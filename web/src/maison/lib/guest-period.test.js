import test from 'node:test';
import assert from 'node:assert/strict';
import { dateTimeInput, editedDateTime } from './guest-period.js';

process.env.TZ = 'Europe/Paris';

test('account dates preserve local time and the exact millisecond', () => {
  const instant = Date.parse('2026-10-20T10:00:01.234Z');
  assert.equal(dateTimeInput(instant), '2026-10-20T12:00:01.234');
  assert.equal(editedDateTime('2026-10-20T12:00:01.234', null), instant);
  assert.equal(editedDateTime('', instant), null);
});

test('nonexistent, ambiguous and malformed local dates are refused', () => {
  for (const value of ['2026-03-29T02:30', '2026-10-25T02:30', '2026-02-31T10:00', 'not-a-date']) {
    assert.throws(() => editedDateTime(value, null));
  }
});

test('editing a name preserves an existing precise timestamp, including either DST fold', () => {
  for (const time of ['2026-10-25T00:30:00Z', '2026-10-25T01:30:00Z']) {
    const instant = Date.parse(time);
    assert.equal(editedDateTime(dateTimeInput(instant), instant), instant);
  }
});
