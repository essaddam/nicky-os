import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { reservationBinding, reservationChoices } from './reservations.js';

const id = 'a'.repeat(32);
const fixture = (instance, online = true) => ({
  id: `${instance}:next-1`, instance, online, native_name: 'Synthetic stay',
  state: { reservation_id: { value: id }, start: { value: 1000 }, end: { value: 2000 } },
});

describe('human reservation binding', () => {
  it('keeps the calendar namespace and refuses malformed or private identities', () => {
    assert.deepEqual(reservationBinding(`calendar:${id}`), { instance: 'calendar', id });
    assert.equal(reservationBinding(''), null);
    for (const value of ['calendar:supplier@example.com', `${'x'.repeat(65)}:${id}`, `calendar:${id}:extra`]) {
      assert.throws(() => reservationBinding(value));
    }
  });
  it('offers only identified, online calendar slots', () => {
    const devices = { a: fixture('calendar'), b: fixture('offline', false), c: fixture('other') };
    const drivers = [{ instance: 'calendar', kind: 'reservations' }, { instance: 'offline', kind: 'reservations' }, { instance: 'other', kind: 'helpers' }];
    assert.deepEqual(reservationChoices(devices, drivers).map((choice) => choice.value), [`calendar:${id}`]);
    devices.a.state.end.value = 500;
    assert.deepEqual(reservationChoices(devices, drivers), []);
  });
  it('deduplicates slots without merging identical ids from different calendars', () => {
    const devices = { a: fixture('first'), b: fixture('first'), c: fixture('second') };
    const drivers = ['first', 'second'].map((instance) => ({ instance, kind: 'reservations' }));
    assert.equal(reservationChoices(devices, drivers).length, 2);
  });
});
