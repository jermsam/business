// The browser owns and renders complete TypeQL, as in the DogRS examples.
// Scope is verified again by hooks; the selected user is never trusted as proof.
export function renderQuery(source, bindings, user) {
  const values = {...bindings, identity_id: user.id, tenant_id: user.tenant_id,
    revision: crypto.randomUUID()};
  return source.replace(/@@([a-z_]+)@@/g, (_, name) => {
    const value = values[name];
    if (typeof value === 'number') {
      if (!Number.isSafeInteger(value)) throw new Error('Expected an exact integer: ' + name);
      return JSON.stringify(value);
    }
    if (typeof value !== 'string') throw new Error('Missing query value: ' + name);
    if (name === 'starts_at' || name === 'ends_at') {
      if (!/(Z|[+-]\d{2}:\d{2})$/.test(value)) throw new Error('Timestamp requires timezone');
      const date = new Date(value);
      if (!Number.isFinite(date.valueOf())) throw new Error('Invalid timestamp');
      return date.toISOString().replace(/Z$/, '');
    }
    return JSON.stringify(value);
  });
}
export async function queryData(query, method, {token, tenant, signal} = {}) {
  if (!['read', 'write'].includes(method)) throw new Error('Choose read or write');
  const response = await fetch('/data', {
    method: 'POST', signal,
    headers: {'Content-Type': 'application/json', 'x-service-method': method,
      Authorization: 'Bearer ' + token, 'x-tenant-id': tenant},
    body: JSON.stringify({query}),
  });
  const result = await response.json();
  if (!response.ok) throw new Error(result.message || 'Request failed');
  return result; // Standard DogRS result.ok.answers; not an operation-specific DTO.
}
