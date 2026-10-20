import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';

const source = await readFile(new URL('../../public/data-client.js', import.meta.url), 'utf8');
const {renderQuery, queryData} = await import('data:text/javascript;base64,' +
  Buffer.from(source).toString('base64'));
const user = {id: 'bob', tenant_id: 'tenant-a', credential_version: 'initial'};

test('frontend sends complete TypeQL and preserves the DogRS envelope', async () => {
  const template = await readFile(new URL('../../public/queries/data/resources.tql', import.meta.url), 'utf8');
  const query = renderQuery(template, {identity_id: 'alice', tenant_id: 'tenant-b'}, user);
  assert.ok(query.includes('has identity_id "bob"'));
  assert.ok(query.includes('has tenant_id "tenant-a"'));
  assert.ok(!query.includes('@@'));
  const previousFetch = globalThis.fetch;
  try {
    globalThis.fetch = async (url, options) => {
      assert.equal(url, '/data');
      assert.equal(options.headers['x-service-method'], 'read');
      assert.deepEqual(JSON.parse(options.body), {query});
      return {ok: true, json: async () => ({ok: {answers: [{data: {id: 'files'}}]}})};
    };
    const result = await queryData(query, 'read', {token: 'test-token', tenant: 'tenant-a'});
    assert.equal(result.ok.answers[0].data.id, 'files');
  } finally { globalThis.fetch = previousFetch; }
});

test('literal encoding is single pass and normalizes zoned dates', () => {
  const reason = '@@identity_id@@"; delete $actor;';
  assert.equal(renderQuery('has reason @@reason@@;', {reason}, user),
    'has reason ' + JSON.stringify(reason) + ';');
  assert.equal(renderQuery('@@starts_at@@', {starts_at: '2026-10-20T09:00:00+03:00'}, user),
    '2026-10-20T06:00:00.000');
  assert.throws(() => renderQuery('@@starts_at@@', {starts_at: '2026-10-20T09:00:00'}, user));
  assert.throws(() => renderQuery('@@resource_id@@', {}, user));
});

test('schema-policy frontend owns queries but never chooses authorization time', async () => {
  const template = await readFile(new URL('../../public/queries/data/resources.tql', import.meta.url), 'utf8');
  const query = renderQuery(template, {identity_id:'alice', tenant_id:'tenant-b', now:'2099-01-01'}, user);
  assert.ok(query.includes('request_now()'));
  assert.ok(query.includes('request_permitted($r,"read")'));
  assert.ok(!query.includes('credential_version'));
  assert.ok(!query.includes('2099-01-01'));
  assert.ok(query.includes('has identity_id "bob"'));
  assert.ok(!query.includes('@@'));
});
