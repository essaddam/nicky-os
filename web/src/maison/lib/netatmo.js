/** The browser follows the server's consent URL only for its current origin.
 *  The operator configured that origin; no installation URL is discovered. */
export function authorizationUrl(value, origin, instance) {
  if (typeof value !== 'string' || value.length > 8192 || typeof instance !== 'string'
      || !/^[a-zA-Z0-9_-]{1,64}$/.test(instance)) return null;
  try {
    if (new URL(origin).origin !== origin) return null;
    const url = new URL(value);
    const query = url.searchParams;
    const keys = ['client_id', 'redirect_uri', 'response_type', 'scope', 'state'];
    if (url.origin !== 'https://api.netatmo.com' || url.pathname !== '/oauth2/authorize'
        || url.username || url.password || url.hash
        || [...query].length !== keys.length || keys.some((key) => query.getAll(key).length !== 1)
        || query.get('response_type') !== 'code' || query.get('scope') !== 'read_station'
        || !/^[0-9a-fA-F]{64}$/.test(query.get('state'))
        || !/^[\x21-\x7e]{1,256}$/.test(query.get('client_id'))
        || query.get('redirect_uri') !== `${origin}/api/integrations/netatmo/${instance}/callback`) return null;
    return url.href;
  } catch {
    return null;
  }
}

/** Bound the decoded response, including chunked or compressed HTTP bodies. */
export async function readJson(response) {
  if (!response.body) throw new Error('Invalid response');
  const reader = response.body.getReader();
  const chunks = [];
  let size = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > 8192) {
        await reader.cancel();
        throw new Error('Response too large');
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
}
