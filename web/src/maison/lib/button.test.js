import test from 'node:test';
import assert from 'node:assert/strict';
import { buttonRequest, nextButtonReference, sendButton } from '../../lib/button.js';

test('auto_h4 two presses share no identity even with an equal value', async () => {
  let state = { client: 'a'.repeat(32), sequence: 0 };
  const reserve = async () => (state = nextButtonReference(state));
  const [first, second] = await Promise.all([
    buttonRequest('simulated:button/press', reserve),
    buttonRequest('simulated:button/press', reserve),
  ]);
  assert.equal(first.value, null);
  assert.equal(second.value, null);
  assert.notEqual(first.request_id, second.request_id);
  const bodies = [];
  const send = async (_url, options) => {
    bodies.push(options.body);
    if (bodies.length === 1) throw new Error('response lost');
    return { ok: true, status: 200, json: async () => ({ ok: true }) };
  };
  await assert.rejects(sendButton(first, send));
  assert.equal((await sendButton(first, send)).ok, true);
  assert.equal(bodies[0], bodies[1], 'a transport retry preserves the complete intent');
});

test('auto_h4 browser counter rotates before JSON rounding', () => {
  const old = { client: 'a'.repeat(32), sequence: Number.MAX_SAFE_INTEGER };
  const next = nextButtonReference(old);
  assert.equal(next.sequence, 1);
  assert.notEqual(next.client, old.client);
  assert.match(next.client, /^[0-9a-f]{32}$/);
});
