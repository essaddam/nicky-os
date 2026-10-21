import test from 'node:test';
import assert from 'node:assert/strict';
import { invitationLink, consumeInvitation, guestRoomList } from './invitation.js';

test('invitation links carry only the existing code in a same-origin fragment', () => {
  assert.equal(invitationLink('https://user:user@example.com/private?state=fixture#/systeme', 'ABCD-EFGH'), 'https://example.com/#invite=ABCDEFGH');
  for (const code of ['<script>', 'ABCDEFGH&next=outside', '', '12345678']) {
    assert.throws(() => invitationLink('https://example.com/', code));
  }
  assert.throws(() => invitationLink('javascript:fixture', 'ABCDEFGH'));
});

test('a valid invitation is consumed once and erased before the house starts', () => {
  const location = new URL('https://example.com/?view=fixture#invite=ABCDEFGH');
  const history = { replaceState(_state, _title, url) { location.href = url; } };
  assert.equal(consumeInvitation(location, history), 'ABCDEFGH');
  assert.equal(location.href, 'https://example.com/?view=fixture#/');
  assert.equal(consumeInvitation(location, history), null);
});

test('unknown invitation parameters never become redirects or persistent secrets', () => {
  for (const hash of ['#invite=ABCDEFGH&next=https://elsewhere.example.com/', '#invite=ABCDEFGH&invite=HGFEDCBA', '#invite=%3Cscript%3E']) {
    const location = new URL(`https://example.com/${hash}`);
    const history = { replaceState(_state, _title, url) { location.href = url; } };
    assert.equal(consumeInvitation(location, history), null);
    assert.equal(location.hash, '#/');
  }
  const location = new URL('https://example.com/#/pieces');
  assert.equal(consumeInvitation(location, { replaceState() { assert.fail('ordinary navigation must remain'); } }), null);
});

test('guest presentation keeps only explicitly permitted rooms', () => {
  const rooms = [{ name: 'Synthetic guest room' }, { name: 'Synthetic private room' }];
  assert.deepEqual(guestRoomList(rooms, { role: 'guest', rooms: ['Synthetic guest room'] }), [rooms[0]]);
  assert.deepEqual(guestRoomList(rooms, { role: 'guest', rooms: [] }), []);
  assert.deepEqual(guestRoomList(rooms, null), []);
});
