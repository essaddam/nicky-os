import test from 'node:test';
import assert from 'node:assert/strict';
import { minuteText, parseMinute, weeklyPayload } from './weekly.js';

test('AUTO-H5 strict minute inputs keep midnight and exclusive end', () => {
  assert.equal(minuteText(1440), '24:00');
  assert.equal(parseMinute('00:00'), 0);
  assert.equal(parseMinute('24:00', true), 1440);
  for (const value of ['24:00', '12:60', '1:00', '', ' 12:00']) assert.equal(parseMinute(value), null);
  assert.equal(parseMinute('24:01', true), null);
});
test('AUTO-H5 editor preserves weekday and overnight intent without browser timezone', () => {
  const payload = weeklyPayload('UTC', [{ day: 7, start: '23:00', end: '01:00' }]);
  assert.deepEqual(payload, { type: 'schedule', value: { timezone: 'UTC', slots: [{ day: 7, start: 1380, end: 60 }] } });
  assert.equal(weeklyPayload('UTC', [{day: 1, start: '00:00', end: '00:00'}]), null);
  assert.equal(weeklyPayload('UTC', Array(257).fill({day: 1, start: '00:00', end: '01:00'})), null);
});

test('AUTO-H5 a preview of an unseen newer draft cannot be approved', async () => {
  const { previewMatchesShown } = await import('./weekly.js');
  const view = {reference:{kind:'schedule',id:'helpers:weekly'},versions:{draft:{version:1},current:null}};
  const preview = {reference:view.reference,versions:{draft:1,current:null}};
  assert.equal(previewMatchesShown(view,preview),true);
  assert.equal(previewMatchesShown(view,{...preview,versions:{draft:2,current:null}}),false);
  assert.equal(previewMatchesShown(view,{...preview,versions:{draft:1,current:1}}),false);
  assert.equal(previewMatchesShown(view,{...preview,reference:{kind:'schedule',id:'helpers:other'}}),false);
});
