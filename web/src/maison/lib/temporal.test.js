import test from 'node:test';
import assert from 'node:assert/strict';
import { temporalEditor, temporalPayload, formatTemporal, supportsTimezone } from '../../lib/temporal.js';
const spec = { format: 'datetime', timezone: 'Europe/Paris' };
test('auto_h3 fold occurrences keep distinct offsets and UTC identities', () => {
  for (const [value, offset] of [['2026-10-25T00:30:00Z', '+02:00'], ['2026-10-25T01:30:00Z', '+01:00']]) {
    const parsed = temporalEditor(spec, value);
    assert.equal(parsed.local, '2026-10-25T02:30:00');
    assert.equal(parsed.offset, offset);
    assert.equal(temporalPayload('datetime', parsed.local, parsed.offset), `2026-10-25T02:30:00${offset}`);
    assert.equal(temporalEditor(spec, value, true).local + 'Z', value);
    assert.equal(temporalEditor(spec, `2026-10-25T02:30:00${offset}`, true).local + 'Z', value);
  }
});
test('auto_h3 civil values and explicit UTC never depend on the browser zone', () => {
  assert.equal(temporalPayload('time', '12:30'), '12:30:00');
  assert.equal(temporalPayload('datetime', '2026-01-01T00:00', '', true), '2026-01-01T00:00:00Z');
  assert.equal(temporalEditor(spec, '2026-01-01T23:30:00Z').local, '2026-01-02T00:30:00');
  assert.equal(formatTemporal({ format: 'date' }, '2026-01-01', 'en-GB'), '1 Jan 2026');
  assert.equal(formatTemporal({ format: 'time' }, '23:59:59', 'fr'), '23:59:59');
});

test('auto_h3 unsupported browser zones retain an explicit UTC representation', () => {
  assert.equal(supportsTimezone('Unknown/Fictional'), false);
  assert.equal(formatTemporal({ format: 'datetime', timezone: 'Unknown/Fictional' }, '2026-01-01T00:00:00Z', 'en'), '2026-01-01T00:00:00Z');
});
