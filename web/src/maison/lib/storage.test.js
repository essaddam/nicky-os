import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { storageLines } from './storage.js';

const fmt = { num: String, when: String };
const keys = (lines) => lines.map((l) => l.key);
const catalog = (lang) => JSON.parse(readFileSync(new URL(`../../../../locales/${lang}/systeme.json`, import.meta.url)));

test('a count that failed is said unavailable, never zero', () => {
  // What /api/system answers when the history table cannot be read.
  const facts = { db_bytes: 4096, wal_bytes: 0, batches_written: 3, batches_failed: 1, rows_merged: 0, rows_dropped: 0, last_error: null, last_prune: null, volume_error: 'database: no such table: samples' };
  const lines = storageLines('history', facts, fmt);
  assert.equal(lines[0].key, 'systeme.stockage.indisponible');
  assert.ok(lines[0].alert);
  assert.ok(!keys(lines).includes('systeme.stockage.lignes_24h'));
  assert.deepEqual(lines[1].params, { ok: '3', failed: '1' });
});

test('missing counters say nothing rather than « 0 en erreur »', () => {
  const lines = storageLines('history', { db_bytes: 4096 }, fmt);
  assert.deepEqual(keys(lines), []);
});

test('a healthy history and energy', () => {
  const history = storageLines('history', { rows_24h: 12, query_ms: 1.5, batches_written: 4, batches_failed: 0, rows_merged: 2, rows_dropped: 0, last_prune: { at: 1, ms: 3, rows: 7 }, last_error: [5, 'disk full'], volume_error: null }, fmt);
  assert.deepEqual(keys(history), ['systeme.stockage.lignes_24h', 'systeme.stockage.lots', 'systeme.stockage.sature', 'systeme.stockage.purge', 'systeme.stockage.erreur']);
  const energy = storageLines('energy', { rows_24h: 48, query_ms: 0.1, writes_ok: 9, writes_failed: 0, last_error: null, volume_error: null }, fmt);
  assert.deepEqual(keys(energy), ['systeme.stockage.lignes_24h', 'systeme.stockage.releves']);
});

test('every line is worded in French and in English', () => {
  const all = [
    ...storageLines('history', { volume_error: 'x', batches_written: 1, batches_failed: 0, rows_dropped: 1, last_prune: null, last_error: [1, 'x'] }, fmt),
    ...storageLines('history', { rows_24h: 1, query_ms: 1, last_prune: { at: 1, ms: 1, rows: 1 } }, fmt),
    ...storageLines('energy', { writes_ok: 1, writes_failed: 0 }, fmt),
  ];
  for (const lang of ['fr', 'en']) {
    const words = catalog(lang);
    for (const l of all) {
      for (const key of [l.key, l.note?.key].filter(Boolean)) {
        assert.ok(words[key] ?? words[`${key}_other`], `${lang}: ${key}`);
      }
    }
  }
});
