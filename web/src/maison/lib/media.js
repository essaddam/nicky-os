// Explicit provider targets only. Network resolution belongs to the hub.
export const SERVICES = Object.freeze(['netflix', 'crunchyroll', 'freetv', 'primevideo', 'disneyplus', 'molotov', 'appletv']);
const MAX_FAVORITES = 50;
const MAX_STORED_BYTES = 256 * 1024;
const bytes = text => new TextEncoder().encode(text).length;
const hasControls = text => /[\u0000-\u001f\u007f-\u009f]/u.test(text);

export function kindsFor(service) {
  if (!SERVICES.includes(service)) return [];
  if (service === 'freetv') return ['app', 'channel', 'replay'];
  if (service === 'molotov') return ['app'];
  return ['app', 'title', 'episode'];
}

export function contentRequest(input) {
  const { service, kind, target } = input ?? {};
  if (!kindsFor(service).includes(kind) || typeof target !== 'string') throw new Error('invalid_target');
  if (hasControls(target) || bytes(target) > 2048) throw new Error('invalid_target');
  const trimmed = target.trim();
  if ((kind === 'app') !== (trimmed === '')) throw new Error('invalid_target');
  const request = JSON.stringify({ service, kind, target: trimmed });
  if (bytes(request) > 4096) throw new Error('invalid_target');
  return request;
}

function favorite(input) {
  if (typeof input?.label !== 'string' || hasControls(input.label)) throw new Error('invalid_favorite');
  const label = input.label.trim();
  if (!label || [...label].length > 80) throw new Error('invalid_favorite');
  return { label, ...JSON.parse(contentRequest(input)) };
}

function storageKey(device) {
  if (typeof device !== 'string' || !device || device.length > 256 || hasControls(device)) throw new Error('invalid_device');
  return `moli-media:${device}`;
}

function readEnvelope(storage, device) {
  try {
    const raw = storage.getItem(storageKey(device));
    if (!raw || bytes(raw) > MAX_STORED_BYTES) return null;
    return JSON.parse(raw);
  } catch { return null; }
}

export function readFavorites(storage, device) {
  const envelope = readEnvelope(storage, device);
  if (envelope?.version !== 1 || !Array.isArray(envelope.items)) return [];
  if (envelope.items.length > MAX_FAVORITES) return [];
  const unique = new Map();
  for (const item of envelope.items) {
    try { const valid = favorite(item); unique.set(contentRequest(valid), valid); } catch { /* Ignore invalid imported entries. */ }
  }
  return [...unique.values()];
}

function writeFavorites(storage, device, items) {
  const envelope = readEnvelope(storage, device);
  if (envelope?.version != null && envelope.version !== 1) throw new Error('unsupported_version');
  storage.setItem(storageKey(device), JSON.stringify({ version: 1, items }));
  return items;
}

export function saveFavorite(storage, device, input) {
  const valid = favorite(input);
  const items = readFavorites(storage, device);
  const existing = items.findIndex(item => contentRequest(item) === contentRequest(valid));
  if (existing >= 0) items[existing] = valid;
  else {
    if (items.length >= MAX_FAVORITES) throw new Error('favorites_full');
    items.push(valid);
  }
  return writeFavorites(storage, device, items);
}

export function removeFavorite(storage, device, input) {
  const target = contentRequest(input);
  const items = readFavorites(storage, device).filter(item => contentRequest(item) !== target);
  return writeFavorites(storage, device, items);
}
