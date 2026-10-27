import test from 'node:test';
import assert from 'node:assert/strict';
import { exposeDescription, nativePackets } from './commands.mjs';
const native = { schema: 'json', topic: 'zigbee2mqtt/fixture/set', on: 'ON', off: 'OFF', parametres: { state: { type: 'enum', valeurs: ['ON', 'OFF'] }, brightness: { type: 'integer', min: 0, max: 254 } } };
function packets(spec, value) {
  return nativePackets({ native: { schema: 'json', topic: 'fixture/set', parametres: { parameter: spec } } }, { parameter: value });
}
function description(command = native) {
  return { source_id: 'z2m-1', pont: 'moli-probe/maison/fixture', canaux: {
    light: { id: 'light', domaine: 'light', source: 'fixture/light', commande: structuredClone(command) },
    brightness: { id: 'brightness', source: 'fixture/light/brightness' },
    other: { id: 'other', source: 'fixture/other/brightness' },
  } };
}
test('les commandes publiques restent virtuelles, le routage natif reste privé', () => {
  const bindings = new Map(); const d = exposeDescription(description(), bindings, 2);
  const command = d.canaux.light.commande;
  assert.equal(command.schema, 'virtual');
  assert.equal(command.topic, 'moli-probe/maison/fixture/canal/light/set');
  assert.equal(bindings.get(command.topic).source, 1);
  assert.equal(bindings.get(command.topic).native.topic, native.topic);
  assert.equal(command.on, 'ON'); assert.equal(command.off, 'OFF');
});
test('les retours des lumières utilisent leur canal exact sans inventer une échelle', () => {
  const d = exposeDescription(description(), new Map(), 2);
  const { state, brightness } = d.canaux.light.commande.parametres;
  assert.equal(state.source_channel, 'light');
  assert.deepEqual(state.source_values, [
    { source: 'on', value: 'ON' }, { source: 'off', value: 'OFF' },
    { source: true, value: 'ON' }, { source: false, value: 'OFF' },
    { source: 'ON', value: 'ON' }, { source: 'OFF', value: 'OFF' },
  ]);
  assert.equal(brightness.source_channel, 'brightness');
  assert.equal(brightness.source_scale, undefined);
  const absent = description(); delete absent.canaux.brightness;
  assert.equal(exposeDescription(absent, new Map(), 2).canaux.light.commande.parametres.brightness.source_channel, undefined);
  const ambiguous = description(); ambiguous.canaux.duplicate = { id: 'duplicate', source: 'fixture/light/brightness' };
  assert.equal(exposeDescription(ambiguous, new Map(), 2).canaux.light.commande.parametres.brightness.source_channel, undefined);
});
test('les valeurs et paramètres inconnus sont refusés avant le routage', () => {
  for (const body of [{ brightness: 255 }, { brightness: -1 }, { brightness: 1.5 }, { brightness: '12' }, { state: 'toggle' }, { other: true }, {}, []]) assert.throws(() => nativePackets({ native }, body));
  assert.deepEqual(nativePackets({ native }, { state: 'ON', brightness: 128 }), [{ topic: native.topic, payload: '{"state":"ON","brightness":128}' }]);
  assert.throws(() => nativePackets({ native }, JSON.parse('{"toString":true}')));
});
test('les booléens, nombres bornés et textes bornés gardent leur type JSON', () => {
  assert.equal(packets({ type: 'boolean' }, false)[0].payload, '{"parameter":false}');
  assert.equal(packets({ type: 'number', min: 1, max: 2 }, 1.5)[0].payload, '{"parameter":1.5}');
  assert.equal(packets({ type: 'string', minLength: 1, maxLength: 3 }, 'abc')[0].payload, '{"parameter":"abc"}');
  assert.equal(packets({ type: 'text', maxLength: 3 }, '')[0].payload, '{"parameter":""}');
  assert.equal(packets({ type: 'enum', valeurs: [false, 2, 'mode'] }, false)[0].payload, '{"parameter":false}');
});
test('les contrats incomplets, types inconnus et valeurs non finies sont refusés', () => {
  for (const [spec, value] of [
    [{ type: 'boolean' }, 'false'], [{ type: 'unknown', min: 0, max: 10 }, 1],
    [{ type: 'number', max: 10 }, 1], [{ type: 'number', min: 10, max: 0 }, 1],
    [{ type: 'number', min: 0, max: Infinity }, 1],
    ...[NaN, Infinity, -Infinity].map(value => [{ type: 'number', min: 0, max: 10 }, value]),
    [{ type: 'enum', valeurs: [] }, 'x'], [{ type: 'enum', valeurs: [null] }, null],
    [{ type: 'enum' }, 'x'], [{ type: 'string' }, 'x'],
    [{ type: 'string', maxLength: 3 }, 'long'], [{ type: 'string', minLength: 2, maxLength: 3 }, 'x'],
    [{ type: 'string', maxLength: 3, minLength: -1 }, 'x'],
    [{ type: 'string', maxLength: 3, minLength: '1' }, 'x'],
  ]) assert.throws(() => packets(spec, value));
});
test('la grille part du minimum et tolère les arrondis flottants', () => {
  assert.doesNotThrow(() => packets({ type: 'number', min: 10, max: 20, step: 0.5 }, 10.5));
  assert.doesNotThrow(() => packets({ type: 'number', min: 0, max: 1, step: 0.1 }, 0.30000000000000004));
  assert.throws(() => packets({ type: 'number', min: 10, max: 20, step: 0.5 }, 10.3));
  for (const step of [0, -1, Infinity, NaN, '1']) assert.throws(() => packets({ type: 'number', min: 0, max: 10, step }, 1));
});
test('la borne de payload compte les octets UTF-8 avant et après mapping', () => {
  assert.doesNotThrow(() => packets({ type: 'string', maxLength: 256 }, 'é'.repeat(100)));
  assert.throws(() => packets({ type: 'string', maxLength: 256 }, 'é'.repeat(125)));
  const expanded = structuredClone(native); expanded.on = 'é'.repeat(130);
  assert.throws(() => nativePackets({ native: expanded }, { state: 'ON' }));
  expanded.schema = 'default';
  assert.throws(() => nativePackets({ native: expanded }, { state: 'ON' }));
});
test('JSON préserve les enums sans mapping ON/OFF applicable', () => {
  const modes = { schema: 'json', topic: 'fixture/set', parametres: { state: { type: 'enum', valeurs: ['heat', 'off'] } } };
  assert.equal(nativePackets({ native: modes }, { state: 'heat' })[0].payload, '{"state":"heat"}');
  const command = exposeDescription(description(modes), new Map(), 2).canaux.light.commande;
  assert.equal(command.on, undefined); assert.equal(command.off, undefined);
  assert.equal(command.parametres.state.source_values, undefined);
  assert.equal(command.parametres.state.source_channel, undefined);
  const mapped = structuredClone(native); mapped.on = true; mapped.off = false;
  assert.equal(nativePackets({ native: mapped }, { state: 'OFF' })[0].payload, '{"state":false}');
});
test('default exige un mapping explicite pour state et conserve les topics de paramètres', () => {
  const plain = structuredClone(native); plain.schema = 'default'; plain.on = 'enable'; plain.off = 'disable';
  plain.parametres.brightness.topic = 'fixture/brightness/set';
  assert.deepEqual(nativePackets({ native: plain }, { state: 'ON', brightness: 12 }), [
    { topic: plain.topic, payload: 'enable' }, { topic: 'fixture/brightness/set', payload: '12' },
  ]);
  delete plain.off;
  assert.throws(() => nativePackets({ native: plain }, { state: 'ON' }));
  const modes = { schema: 'default', topic: 'fixture/set', parametres: { state: { type: 'enum', valeurs: ['heat'] } } };
  assert.throws(() => nativePackets({ native: modes }, { state: 'heat' }));
  assert.throws(() => nativePackets({ native: { ...native, schema: 'unknown' } }, { state: 'ON' }));
});

test('l’échelle brightness HA exige toutes les métadonnées explicites', () => {
  const withScale = () => {
    const d = description();
    Object.assign(d.canaux.brightness, { unite: '/255', semantique: 'intensite' });
    return d;
  };
  const command = exposeDescription(withScale(), new Map(), 2).canaux.light.commande;
  assert.deepEqual(command.parametres.brightness.source_scale, { min: 0, max: 255 });
  assert.equal(command.parametres.brightness.max, 254);
  assert.equal(command.parametres.brightness.source_channel, 'brightness');
  assert.equal(nativePackets({ native }, { brightness: 254 })[0].payload, '{"brightness":254}');
  assert.throws(() => nativePackets({ native }, { brightness: 255 }));
  for (const mutate of [
    d => { delete d.canaux.brightness.unite; },
    d => { delete d.canaux.brightness.semantique; },
    d => { d.canaux.brightness.unite = '%'; },
    d => { d.canaux.brightness.semantique = 'autre'; },
    d => { d.canaux.light.domaine = 'cover'; },
    d => { delete d.canaux.brightness; },
    d => { d.canaux.brightness.source = 'fixture/another/brightness'; },
    d => { d.canaux.duplicate = { ...d.canaux.brightness, id: 'duplicate' }; },
  ]) {
    const d = withScale(); mutate(d);
    const exposed = exposeDescription(d, new Map(), 2).canaux.light.commande;
    assert.equal(exposed.parametres.brightness.source_scale, undefined);
  }
});

test('les sorties indépendantes gardent leur commande et leur retour exact', () => {
  const command = outlet => ({ schema: 'default', topic: `fixture/${outlet}/set`, on: JSON.stringify({ [outlet]: 'ON' }), off: JSON.stringify({ [outlet]: 'OFF' }), parametres: { state: { type: 'enum', valeurs: ['ON', 'OFF'] } } });
  const d = { source_id: 'z2m-0', pont: 'moli-probe/maison/fixture', canaux: Object.fromEntries(['one', 'two'].map(id => [id, { id, domaine: 'switch', source: `switch.${id}`, commande: command(id) }])) };
  const bindings = new Map();
  exposeDescription(d, bindings, 1);
  for (const id of ['one', 'two']) {
    const exposed = d.canaux[id].commande;
    assert.equal(exposed.parametres.state.source_channel, id);
    assert.deepEqual(nativePackets(bindings.get(exposed.topic), { state: 'OFF' }), [{ topic: `fixture/${id}/set`, payload: JSON.stringify({ [id]: 'OFF' }) }]);
  }
});

test('la couleur accepte un seul modèle déclaré avec ses coordonnées bornées', () => {
  const spec = { type: 'color', modes: ['xy', 'hs', 'rgb'] };
  for (const color of [{ x: 0.4, y: 0.3 }, { h: 360, s: 100 }, { r: 255, g: 0, b: 128 }]) {
    assert.equal(packets(spec, color)[0].payload, JSON.stringify({ parameter: color }));
  }
  for (const color of [null, [], {}, { x: 0.2 }, { x: 0.8, y: 0.8 }, { x: -1, y: 0.2 }, { x: 0.2, y: '0.3' }, { x: NaN, y: 0.2 }, { h: 361, s: 0 }, { h: 20, s: 101 }, { r: 1.5, g: 0, b: 0 }, { r: 256, g: 0, b: 0 }, { x: 0.2, y: 0.3, r: 12 }, { r: 1, g: 2, b: 3, w: 4 }]) {
    assert.throws(() => packets(spec, color));
  }
  assert.throws(() => packets({ type: 'color', modes: ['xy'] }, { r: 1, g: 2, b: 3 }));
  for (const modes of [undefined, [], ['unknown'], ['xy', 'xy']]) assert.throws(() => packets({ type: 'color', modes }, { x: 0.2, y: 0.3 }));
  assert.throws(() => nativePackets({ native: { schema: 'default', topic: 'fixture/set', parametres: { color: spec } } }, { color: { x: 0.2, y: 0.3 } }));
});

test('les coordonnées de retour couleur sont liées aux seuls canaux exacts', () => {
  const d = description();
  d.canaux.light.commande.parametres.color = { type: 'color', modes: ['xy'] };
  d.canaux.x = { id: 'x', source: 'fixture/light/color_x', semantique: 'color_x', domaine: 'light' };
  d.canaux.y = { id: 'y', source: 'fixture/light/color_y', semantique: 'color_y', domaine: 'light' };
  d.canaux.other = { id: 'other', source: 'fixture/other/color_x', semantique: 'color_x', domaine: 'light' };
  assert.deepEqual(exposeDescription(d, new Map(), 2).canaux.light.commande.parametres.color.source_channels, { xy: { x: 'x', y: 'y' } });
  const ambiguous = description();
  ambiguous.canaux.light.commande.parametres.color = { type: 'color', modes: ['xy'] };
  Object.assign(ambiguous.canaux, { x: d.canaux.x, y: d.canaux.y, duplicate: { ...d.canaux.x, id: 'duplicate' } });
  assert.deepEqual(exposeDescription(ambiguous, new Map(), 2).canaux.light.commande.parametres.color.source_channels, {});
});
