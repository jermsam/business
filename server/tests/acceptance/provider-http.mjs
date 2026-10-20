// Explicit real-provider acceptance. Run manually with private environment values;
// never part of the default offline test suite, never use a live connection.
import assert from 'node:assert/strict';
import {randomUUID} from 'node:crypto';
const required = key => { assert(process.env[key], `${key} required`); return process.env[key]; };
const base = new URL(required('BUSINESS_TEST_BASE_URL'));
assert(base.origin === 'https://jitpomi-business-validation.onrender.com' ||
  (base.hostname === '127.0.0.1' && base.protocol === 'http:'), 'Validation target required');
const provider = required('BUSINESS_TEST_PROVIDER');
assert(['stripe', 'mercury'].includes(provider));
const tenant = required('BUSINESS_TEST_TENANT');
const headers = {'content-type':'application/json', 'x-tenant-id':tenant};
let token;
async function call(path, method='GET', data, extra={}) {
  const response = await fetch(new URL(path, base), {
    method, headers:{...headers, ...(token ? {authorization:`Bearer ${token}`} : {}), ...extra},
    body:data === undefined ? undefined : JSON.stringify(data),
    signal:AbortSignal.timeout(90000), redirect:'error',
  });
  return {status:response.status, body:await response.json()};
}
function ok(result, label) {
  assert(result.status >= 200 && result.status < 300,
    `${label}: HTTP ${result.status} (${result.body?.name || 'unknown'})`);
  return result.body;
}
const anonymous = await call(`/${provider}/${provider==='stripe'?'customers':'accounts'}?limit=1`);
assert.equal(anonymous.status,401);console.log('PASS anonymous provider access denied');
const login = ok(await call('/authentication','POST',{
  strategy:'local', email:required('BUSINESS_TEST_OWNER_EMAIL'),
  password:required('BUSINESS_TEST_OWNER_PASSWORD'),
}), 'owner login');
assert(login.accessToken && !login.user.password); token=login.accessToken;
const action = (resource, method, data) => call(`/stripe/${resource}`,'POST',data,
  {'x-service-method':method,'idempotency-key':randomUUID()});
const create = (resource, data, key=randomUUID()) => call(`/stripe/${resource}`,'POST',data,{'idempotency-key':key});
const cleanup=[];
try {
  const denied=await call(`/${provider}/${provider==='stripe'?'customers':'accounts'}?limit=1`, 'GET', undefined,
    {'x-tenant-id':'00000000-0000-4000-8000-000000000001'});
  assert.equal(denied.status,401);console.log('PASS cross-tenant provider access denied');
  if(provider==='mercury') {
    const failures=[];
    for(const resource of ['accounts','customers','invoices','transactions']) {
      const result=await call(`/mercury/${resource}?limit=1`);
      if(result.status===200) console.log(`PASS Mercury ${resource} read`);
      else failures.push(`${resource}: HTTP ${result.status} (${result.body?.name||'unknown'})`);
    }
    assert.equal(failures.length,0,`Mercury sandbox verification failed: ${failures.join('; ')}`);
  } else {
    assert.equal(process.env.BUSINESS_ALLOW_PROVIDER_TEST_WRITES,'1','Explicit test-write approval required');
    const key=randomUUID();
    const data={name:`Business HTTPS acceptance ${key}`,email:'contract-test@example.invalid',metadata:{purpose:'synthetic-provider-test',run:key}};
    const customer=ok(await create('customers',data,key),'customer create');
    assert.equal(customer.livemode,false,'TEST connection required');assert(customer.id?.startsWith('cus_'));
    cleanup.push(async()=>ok(await call(`/stripe/customers/${customer.id}`,'DELETE'),'customer cleanup'));
    assert.equal(ok(await create('customers',data,key),'idempotent create').id,customer.id);
    assert.equal(ok(await call(`/stripe/customers/${customer.id}`),'customer read').id,customer.id);
    ok(await call(`/stripe/customers/${customer.id}`,'PATCH',{metadata:{verified:'true'}},{'idempotency-key':randomUUID()}),'customer update');
    const list=ok(await call('/stripe/customers?limit=1'),'customer list');assert(Array.isArray(list));
    console.log('PASS Stripe customer create/replay/read/update/list');
    const raw=await create('payment-intents',{amount:100,currency:'usd',payment_method_data:{type:'card',card:{number:'4242424242424242',cvc:'123'}}});
    assert.equal(raw.status,400);console.log('PASS raw card data rejected');
    for(const resource of ['setup-intents','payment-intents']) {
      const body={customer:customer.id,payment_method_types:['card'],...(resource==='payment-intents'?{amount:100,currency:'usd'}:{})};
      const object=ok(await create(resource,body),`${resource} create`);assert.equal(object.livemode,false);
      ok(await action(resource,'cancel',{id:object.id}),`${resource} cancel`);
      console.log(`PASS Stripe ${resource} unconfirmed create/cancel`);
    }
    const checkout=ok(await create('checkout-sessions',{mode:'payment',customer:customer.id,
      success_url:'https://example.invalid/success',cancel_url:'https://example.invalid/cancel',
      line_items:[{price_data:{currency:'usd',unit_amount:100,product_data:{name:'Synthetic contract test'}},quantity:1}]}),'checkout create');
    assert.equal(checkout.livemode,false);
    ok(await action('checkout-sessions','line_items',{id:checkout.id}),'checkout line items');
    ok(await action('checkout-sessions','expire',{id:checkout.id}),'checkout expire');
    console.log('PASS Stripe checkout create/read/expire (no payment)');
    const invoice=ok(await create('invoices',{customer:customer.id,auto_advance:false,collection_method:'send_invoice',days_until_due:30}),'draft invoice create');
    assert.equal(invoice.livemode,false);assert.equal(invoice.status,'draft');
    cleanup.push(async()=>ok(await call(`/stripe/invoices/${invoice.id}`,'DELETE'),'draft invoice cleanup'));
    ok(await create('invoice-items',{customer:customer.id,invoice:invoice.id,amount:100,currency:'usd',description:'Synthetic contract test'}),'invoice item create');
    const fetched=ok(await call(`/stripe/invoices/${invoice.id}`),'draft invoice read');
    assert.equal(fetched.status,'draft');assert.equal(fetched.total,100);
    console.log('PASS Stripe draft invoice/item/read (no finalization, email or collection)');
  }
} finally {
  const errors=[];
  for(const remove of cleanup.reverse()) {try{await remove()}catch(e){errors.push(e.message)}}
  const logout=await call('/authentication','DELETE');
  if(logout.status!==200)errors.push(`Logout: HTTP ${logout.status}`);
  const revoked=await call(`/${provider}/${provider==='stripe'?'customers':'accounts'}?limit=1`);
  if(revoked.status!==401)errors.push(`Revoked token: HTTP ${revoked.status}`);
  assert.equal(errors.length,0,errors.join('; '));
  console.log('PASS synthetic cleanup, logout and revoked-token denial');
}
