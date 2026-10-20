import { test } from 'node:test';
import assert from 'node:assert/strict';
import { authorizationUrl, readJson } from './netatmo.js';

const origin = 'https://weather.example.com';
const instance = 'weather';
const callback = `${origin}/api/integrations/netatmo/${instance}/callback`;
const state = '0'.repeat(64);
function authorization() {
  const url = new URL('https://api.netatmo.com/oauth2/authorize');
  url.search = new URLSearchParams({ client_id: 'synthetic', redirect_uri: callback, response_type: 'code', scope: 'read_station', state }).toString();
  return url;
}

test('consent navigates only to the weather provider with the exact return', () => {
  const url = authorization().href;
  assert.equal(authorizationUrl(url, origin, instance), url);
  assert.equal(authorizationUrl(url, 'https://other.example.com', instance), null);
  assert.equal(authorizationUrl(url, origin, 'other'), null);
});

test('canonical callbacks match browser origins for configured case and default ports', () => {
  for (const [configured, canonical] of [
    ['https://WEATHER.EXAMPLE.COM:443', 'https://weather.example.com'],
    ['https://Weather.EXAMPLE.COM:8443', 'https://weather.example.com:8443'],
    ['http://LOCALHOST:80', 'http://localhost'],
    ['http://127.0.0.1:80', 'http://127.0.0.1'],
    ['http://[0:0:0:0:0:0:0:1]:80', 'http://[::1]'],
    ['https://[::ffff:192.0.2.1]:443', 'https://[::ffff:c000:201]'],
    ['https://[::FFFF:C000:0201]', 'https://[::ffff:c000:201]'],
  ]) {
    assert.equal(new URL(configured).origin, canonical);
    const url = authorization();
    // Exact native callback contract, also asserted in the Rust config tests.
    url.searchParams.set('redirect_uri', `${canonical}/api/integrations/netatmo/${instance}/callback`);
    assert.equal(authorizationUrl(url.href, new URL(configured).origin, instance), url.href);
    for (const suffix of ['?x=1', '#fragment', '/', '/../callback', '%2f']) {
      const hostile = new URL(url);
      hostile.searchParams.set('redirect_uri', url.searchParams.get('redirect_uri') + suffix);
      assert.equal(authorizationUrl(hostile.href, canonical, instance), null);
    }
    const ipv4 = new URL(url);
    ipv4.searchParams.set('redirect_uri', `https://192.0.2.1/api/integrations/netatmo/${instance}/callback`);
    assert.equal(authorizationUrl(ipv4.href, canonical, instance), null);
  }
});

test('hostile destinations and changed consent parameters never navigate', () => {
  const mutations = [
    (url) => { url.protocol = 'http:'; },
    (url) => { url.hostname = 'example.org'; },
    (url) => { url.username = 'synthetic'; },
    (url) => { url.pathname = '/other'; },
    (url) => { url.hash = 'fragment'; },
    (url) => { url.searchParams.set('scope', 'read_station write_thermostat'); },
    (url) => { url.searchParams.set('redirect_uri', 'https://example.org/return'); },
    (url) => { url.searchParams.set('state', 'short'); },
    (url) => { url.searchParams.append('state', state); },
    (url) => { url.searchParams.set('client_secret', 'synthetic'); },
    (url) => { url.searchParams.set('response_type', 'token'); },
  ];
  for (const mutate of mutations) {
    const url = authorization();
    mutate(url);
    assert.equal(authorizationUrl(url.href, origin, instance), null);
  }
  for (const input of [null, {}, '', 'javascript:alert(1)', 'x'.repeat(8193)]) {
    assert.equal(authorizationUrl(input, origin, instance), null);
  }
  assert.equal(authorizationUrl(authorization().href, origin, '../weather'), null);
});

test('JSON responses are limited after decompression and excessive streams are cancelled', async () => {
  let cancelled = false;
  const stream = new ReadableStream({
    start(controller) {
      controller.enqueue(new Uint8Array(4096));
      controller.enqueue(new Uint8Array(4097));
    },
    cancel() { cancelled = true; },
  });
  await assert.rejects(readJson(new Response(stream)));
  assert.equal(cancelled, true);
});

test('bounded JSON is decoded across chunks and invalid JSON is refused', async () => {
  const encoder = new TextEncoder();
  const stream = new ReadableStream({
    start(controller) {
      controller.enqueue(encoder.encode('{"instances":'));
      controller.enqueue(encoder.encode('[]}'));
      controller.close();
    },
  });
  assert.deepEqual(await readJson(new Response(stream)), { instances: [] });
  await assert.rejects(readJson(new Response('invalid')));
  await assert.rejects(readJson(new Response(null)));
});
