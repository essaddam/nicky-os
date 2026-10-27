import test from 'node:test';
import assert from 'node:assert/strict';
import { createSocketTickets, permitsWebSocket, validCredentials } from './websocket-access.mjs';

const fixture = () => createSocketTickets({ key: Buffer.alloc(32, 7), now: () => 10000, lifetime: 1000 });
test('le proxy ne doit pas dépendre des réécritures Host et Origin', () => {
  const tickets = fixture();
  const { ticket } = tickets.issue();
  assert.equal(permitsWebSocket({ url: `/mqtt?ticket=${ticket}`, headers: { host: 'localhost:1414', origin: 'https://preview.example.org' } }, tickets), true);
});
test('aucun upgrade sans ticket même depuis localhost', () => {
  assert.equal(permitsWebSocket({ url: '/mqtt', headers: { host: 'localhost:1414', origin: 'http://localhost:1414' } }, fixture()), false);
});
test('un ticket altéré, expiré, d’un autre serveur ou sur un autre chemin est refusé', () => {
  const tickets = fixture();
  const { ticket } = tickets.issue();
  assert.equal(tickets.verify(ticket.slice(1)), false);
  assert.equal(createSocketTickets({ key: Buffer.alloc(32, 8), now: () => 10000 }).verify(ticket), false);
  assert.equal(createSocketTickets({ key: Buffer.alloc(32, 7), now: () => 11001 }).verify(ticket), false);
  assert.equal(permitsWebSocket({ url: `/other?ticket=${ticket}` }, tickets), false);
});
test('le ticket et le proxy ne remplacent jamais les identifiants MQTT', () => {
  assert.equal(validCredentials('fixture', Buffer.from('fixture-password'), 'fixture', 'fixture-password'), true);
  assert.equal(validCredentials('fixture', undefined, 'fixture', 'fixture-password'), false);
  assert.equal(validCredentials('fixture', Buffer.from('wrong'), 'fixture', 'fixture-password'), false);
  assert.equal(validCredentials('fixture', Buffer.from('fixture-password'), 'other', 'fixture-password'), false);
});
