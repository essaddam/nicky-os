import test from 'node:test';
import assert from 'node:assert/strict';
import { SERVICES, kindsFor, contentRequest, readFavorites, saveFavorite, removeFavorite } from './media.js';

function storage() {
  const data = new Map();
  return { getItem: key => data.get(key) ?? null, setItem: (key, value) => data.set(key, value), data };
}
const entry = (target = '12345678') => ({ label: 'Episode exemple', service: 'netflix', kind: 'episode', target });

test('seven providers expose only supported target kinds, including Free TV replay', () => {
  assert.equal(SERVICES.length, 7);
  assert.deepEqual(kindsFor('freetv'), ['app', 'channel', 'replay']);
  assert.deepEqual(kindsFor('molotov'), ['app']);
  assert.deepEqual(kindsFor('unknown'), []);
  assert.deepEqual(kindsFor('appletv'), ['app', 'title', 'episode']);
});
test('requests preserve explicit episode identity and enforce byte bounds', () => {
  const raw = contentRequest(entry());
  assert.deepEqual(JSON.parse(raw), { service: 'netflix', kind: 'episode', target: '12345678' });
  assert.equal(JSON.parse(contentRequest({ service: 'molotov', kind: 'app', target: '' })).target, '');
  for (const invalid of [entry(''), entry('a\nb'), entry('é'.repeat(1025)), { ...entry(), kind: 'channel' }, { ...entry(), service: '__proto__' }]) {
    assert.throws(() => contentRequest(invalid));
  }
});
test('favorites stay isolated by television and duplicate targets update their label', () => {
  const store = storage();
  saveFavorite(store, 'one:tv', entry());
  saveFavorite(store, 'one:tv', { ...entry(), label: 'Autre nom' });
  saveFavorite(store, 'two:tv', entry('87654321'));
  assert.deepEqual(readFavorites(store, 'one:tv'), [{ ...entry(), label: 'Autre nom' }]);
  assert.deepEqual(readFavorites(store, 'two:tv'), [entry('87654321')]);
});
test('removing one favorite preserves other televisions and entries', () => {
  const store = storage();
  saveFavorite(store, 'one:tv', entry());
  saveFavorite(store, 'one:tv', entry('87654321'));
  saveFavorite(store, 'two:tv', entry());
  assert.deepEqual(removeFavorite(store, 'one:tv', entry()), [entry('87654321')]);
  assert.deepEqual(readFavorites(store, 'two:tv'), [entry()]);
});
test('future storage versions are not overwritten by a downgrade', () => {
  const store = storage();
  saveFavorite(store, 'one:tv', entry());
  const key = [...store.data.keys()][0];
  store.setItem(key, JSON.stringify({ version: 2, items: [entry()] }));
  const previous = store.getItem(key);
  assert.deepEqual(readFavorites(store, 'one:tv'), []);
  assert.throws(() => saveFavorite(store, 'one:tv', entry('87654321')), /unsupported_version/);
  assert.throws(() => removeFavorite(store, 'one:tv', entry()), /unsupported_version/);
  assert.equal(store.getItem(key), previous);
});
test('invalid storage is ignored and new input is validated', () => {
  const store = storage();
  saveFavorite(store, 'one:tv', entry());
  const key = [...store.data.keys()][0];
  for (const bad of ['{', 'null', '[]', JSON.stringify({ version: 1, items: [{ ...entry(), service: 'unknown' }] })]) {
    store.setItem(key, bad);
    assert.deepEqual(readFavorites(store, 'one:tv'), []);
  }
  for (const invalid of [{ ...entry(), label: '' }, { ...entry(), label: 'a'.repeat(81) }, { ...entry(), label: 'x\ny' }, { ...entry(), target: 'x'.repeat(2049) }]) {
    assert.throws(() => saveFavorite(store, 'one:tv', invalid));
  }
});
test('fifty-favorite limit still permits updating an existing entry', () => {
  const store = storage();
  for (let id = 1; id <= 50; id++) saveFavorite(store, 'one:tv', entry(String(id)));
  assert.equal(readFavorites(store, 'one:tv').length, 50);
  assert.throws(() => saveFavorite(store, 'one:tv', entry('51')), /favorites_full/);
  saveFavorite(store, 'one:tv', { ...entry('1'), label: 'Mis à jour' });
  assert.equal(readFavorites(store, 'one:tv')[0].label, 'Mis à jour');
});
