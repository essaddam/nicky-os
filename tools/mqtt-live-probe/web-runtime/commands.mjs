// The gateway advertises virtual commands, keeping native routing server-side.
function isScalar(value) {
  return typeof value === 'string' || typeof value === 'boolean' || (typeof value === 'number' && Number.isFinite(value));
}
function hasStateMapping(native) {
  const state = native.parametres?.state;
  return state?.type === 'enum' && Array.isArray(state.valeurs) && state.valeurs.includes('ON') && state.valeurs.includes('OFF')
    && Object.hasOwn(native, 'on') && Object.hasOwn(native, 'off') && isScalar(native.on) && isScalar(native.off);
}
const colorComponents = { xy: ['x', 'y'], hs: ['h', 's'], rgb: ['r', 'g', 'b'] };
function addChannelFeedback(channel, channels, parameters, native) {
  if (!['light', 'switch'].includes(channel.domaine)) return;
  if (parameters.state && hasStateMapping(native)) {
    parameters.state.source_channel = channel.id;
    parameters.state.source_values = [
      { source: 'on', value: 'ON' }, { source: 'off', value: 'OFF' },
      { source: true, value: 'ON' }, { source: false, value: 'OFF' },
      { source: 'ON', value: 'ON' }, { source: 'OFF', value: 'OFF' },
    ];
  }
  if (channel.domaine !== 'light') return;
  if (parameters.color?.type === 'color' && Array.isArray(parameters.color.modes)) {
    const groups = {};
    for (const mode of parameters.color.modes) {
      const components = colorComponents[mode];
      if (!components) continue;
      const entries = components.map(component => {
        const semantic = `color_${component}`;
        const matches = channels.filter(candidate => candidate.source === `${channel.source}/${semantic}` && candidate.semantique === semantic && candidate.domaine === 'light');
        return matches.length === 1 ? [component, matches[0].id] : null;
      });
      if (entries.every(Boolean)) groups[mode] = Object.fromEntries(entries);
    }
    parameters.color.source_channels = groups;
  }
  if (!parameters.brightness || typeof channel.source !== 'string') return;
  const matches = channels.filter(candidate => candidate.source === `${channel.source}/brightness`);
  if (matches.length === 1) {
    const feedback = matches[0];
    parameters.brightness.source_channel = feedback.id;
    if (feedback.unite === '/255' && feedback.semantique === 'intensite') {
      parameters.brightness.source_scale = { min: 0, max: 255 };
    }
  }
}
export function exposeDescription(description, bindings, sourceCount) {
  const source = Number(String(description.source_id || '').replace(/^z2m-/, ''));
  if (!Number.isInteger(source) || source < 0 || source >= sourceCount) throw Error('source');
  if (!description.pont?.startsWith('moli-probe/maison/')) throw Error('namespace');
  const prefix = `${description.pont}/canal/`;
  for (const topic of bindings.keys()) if (topic.startsWith(prefix)) bindings.delete(topic);
  const channels = Object.values(description.canaux || {});
  for (const c of channels) {
    const native = c.commande;
    if (!native || !['json', 'default'].includes(native.schema)) continue;
    const topic = `${prefix}${c.id}/set`;
    bindings.set(topic, { source, native });
    const parameters = structuredClone(native.parametres || {});
    for (const spec of Object.values(parameters)) if (spec && typeof spec === 'object') delete spec.topic;
    addChannelFeedback(c, channels, parameters, native);
    c.commande = { schema: 'virtual', topic, retain: false, parametres: parameters };
    if (hasStateMapping(native)) Object.assign(c.commande, { on: 'ON', off: 'OFF' });
  }
  return description;
}
function validateParameter(spec, value) {
  if (!spec || typeof spec !== 'object') throw Error('parameter');
  if (spec.type === 'color') {
    if (!Array.isArray(spec.modes) || !spec.modes.length || spec.modes.length > 3 || new Set(spec.modes).size !== spec.modes.length
      || !spec.modes.every(mode => Object.hasOwn(colorComponents, mode)) || !value || typeof value !== 'object' || Array.isArray(value)) throw Error('color');
    const keys = Object.keys(value);
    const mode = spec.modes.find(mode => keys.length === colorComponents[mode].length && colorComponents[mode].every(key => Object.hasOwn(value, key)));
    if (!mode || !keys.every(key => typeof value[key] === 'number' && Number.isFinite(value[key]))) throw Error('color');
    if (mode === 'xy' && (value.x < 0 || value.x > 1 || value.y < 0 || value.y > 1 || value.x + value.y > 1 + 1e-9)) throw Error('color');
    if (mode === 'hs' && (value.h < 0 || value.h > 360 || value.s < 0 || value.s > 100)) throw Error('color');
    if (mode === 'rgb' && !keys.every(key => Number.isInteger(value[key]) && value[key] >= 0 && value[key] <= 255)) throw Error('color');
    return;
  }
  if (spec.type === 'enum') {
    if (!Array.isArray(spec.valeurs) || !spec.valeurs.length || spec.valeurs.length > 128 || !spec.valeurs.every(isScalar) || !spec.valeurs.includes(value)) throw Error('enum');
    return;
  }
  if (spec.type === 'boolean') {
    if (typeof value !== 'boolean') throw Error('boolean');
    return;
  }
  if (['string', 'text'].includes(spec.type)) {
    if (!Number.isInteger(spec.maxLength) || spec.maxLength < 1 || spec.maxLength > 256
      || (spec.minLength !== undefined && (!Number.isInteger(spec.minLength) || spec.minLength < 0 || spec.minLength > spec.maxLength))
      || typeof value !== 'string' || value.length < (spec.minLength ?? 0) || value.length > spec.maxLength) throw Error('string');
    return;
  }
  if (!['integer', 'number'].includes(spec.type)) throw Error('type');
  if (typeof value !== 'number' || !Number.isFinite(value) || !Number.isFinite(spec.min) || !Number.isFinite(spec.max)
    || spec.min > spec.max || value < spec.min || value > spec.max || (spec.type === 'integer' && !Number.isInteger(value))) throw Error('range');
  if (spec.step !== undefined) {
    if (!Number.isFinite(spec.step) || spec.step <= 0) throw Error('step');
    const steps = (value - spec.min) / spec.step;
    if (!Number.isFinite(steps) || Math.abs(steps - Math.round(steps)) > 1e-6) throw Error('step');
  }
}
function stateValue(native, value) {
  if (hasStateMapping(native) && (value === 'ON' || value === 'OFF')) return value === 'ON' ? native.on : native.off;
  if (native.schema === 'json') return value;
  throw Error('state mapping');
}
export function nativePackets(binding, body) {
  if (!body || typeof body !== 'object' || Array.isArray(body) || Object.keys(body).length === 0
    || Buffer.byteLength(JSON.stringify(body), 'utf8') > 256) throw Error('payload');
  const native = binding.native;
  if (!native || !['json', 'default'].includes(native.schema)) throw Error('schema');
  for (const [key, value] of Object.entries(body)) {
    if (!Object.hasOwn(native.parametres || {}, key)) throw Error('parameter');
    if (native.parametres[key].type === 'color' && native.schema !== 'json') throw Error('color schema');
    validateParameter(native.parametres[key], value);
  }
  if (native.schema === 'json') {
    const payload = { ...body };
    if (Object.hasOwn(payload, 'state')) payload.state = stateValue(native, payload.state);
    const serialized = JSON.stringify(payload);
    if (Buffer.byteLength(serialized, 'utf8') > 256) throw Error('payload');
    return [{ topic: native.topic, payload: serialized }];
  }
  const packets = Object.entries(body).map(([key, value]) => ({
    topic: native.parametres[key].topic || native.topic,
    payload: String(key === 'state' ? stateValue(native, value) : value),
  }));
  if (packets.some(packet => Buffer.byteLength(packet.payload, 'utf8') > 256)) throw Error('payload');
  return packets;
}
