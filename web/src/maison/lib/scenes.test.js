import { test } from 'node:test';
import assert from 'node:assert/strict';
import { scenePayload, scenePreviewMatches, sceneNeedsVersion } from './scenes.js';
test('scene requests preserve scalar types and reject contradictory target intents', () => {
  const rows = [{ point: 'fixture:lamp/on', value: false }, { point: 'fixture:level/level', value: 0 }];
  assert.deepEqual(scenePayload(rows, 4, 30, 5).value.targets, rows);
  assert.equal(scenePayload([...rows, { ...rows[0], value: true }], 4, 30, 5), null);
  assert.equal(scenePayload([{ ...rows[0], value: { toggle: true } }], 4, 30, 5), null);
});
test('scene execution preview binds the current head independently of an agent draft', () => {
  const view = { reference: { kind: 'scene', id: 'scenes:desired' }, versions: { draft: { version: 3 }, current: { version: 2 } } };
  assert.equal(scenePreviewMatches(view, { id: 'scenes:desired', version: 2 }), true);
  assert.equal(scenePreviewMatches(view, { id: 'scenes:desired', version: 3 }), false);
  assert.equal(scenePreviewMatches(view, { id: 'scenes:other', version: 2 }), false);
});

test('scope rebinding requires a new draft version when the published revision is still current', () => {
  const old = {targets:[{point:'fixture:lamp/on',rooms:['Neutral']}]};
  const next = {targets:[{point:'fixture:lamp/on',rooms:['Other synthetic room']}]};
  const view = {versions:{draft:{version:1},current:{version:1}},scope:old};
  assert.equal(sceneNeedsVersion(view,{scene:old}),false);
  assert.equal(sceneNeedsVersion(view,{scene:next}),true);
  assert.equal(sceneNeedsVersion({...view,versions:{draft:{version:2},current:{version:1}}},{scene:next}),false);
});
