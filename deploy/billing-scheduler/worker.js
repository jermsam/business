// External trigger: invoices survive application restarts and sleeping free hosts.
export default {
  async scheduled(_event, env, ctx) {
    ctx.waitUntil(run(env));
  },
};
export async function run(env) {
  const base = new URL(env.BUSINESS_URL);
  if (base.protocol !== 'https:' || base.username || base.password || base.search || base.hash) throw new Error('BUSINESS_URL must be a trusted HTTPS origin');
  if (!env.BILLING_TICK_SECRET || env.BILLING_TICK_SECRET.length < 32) throw new Error('Missing billing scheduler secret');
  const response = await fetch(new URL('/internal/billing/tick', base), {
    method: 'POST', redirect: 'error',
    headers: { Authorization: `Bearer ${env.BILLING_TICK_SECRET}` },
  });
  if (!response.ok) throw new Error(`Billing scheduler returned HTTP ${response.status}`);
  const result = await response.json();
  if (result.errors) throw new Error(`Billing scheduler reported ${result.errors} operations requiring attention`);
}
