export function supportsTimezone(zone) {
  try { new Intl.DateTimeFormat('en', { timeZone: zone }); return true; } catch { return false; }
}
// Civil strings never pass through the browser's implicit local time zone.
export function temporalEditor(spec, value, utc = false) {
  const text = String(value ?? '');
  if (spec.format !== 'datetime') return { local: text, offset: '' };
  if (!text.endsWith('Z') && !(utc && text.length === 25)) return { local: text.slice(0, 19), offset: text.slice(19) };
  const zone = utc ? 'UTC' : spec.timezone;
  const date = new Date(text);
  if (!Number.isFinite(date.getTime())) return { local: '', offset: '' };
  const parts = new Intl.DateTimeFormat('en-CA', { timeZone: zone, year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit', hourCycle: 'h23' }).formatToParts(date);
  const get = (key) => parts.find((p) => p.type === key)?.value;
  const name = new Intl.DateTimeFormat('en', { timeZone: zone, timeZoneName: 'longOffset' }).formatToParts(date).find((p) => p.type === 'timeZoneName')?.value;
  const offset = name === 'GMT' ? '+00:00' : name?.replace('GMT', '');
  return { local: `${get('year').padStart(4, '0')}-${get('month')}-${get('day')}T${get('hour')}:${get('minute')}:${get('second')}`, offset: /^[+-]\d\d:\d\d$/.test(offset ?? '') ? offset : '' };
}
export function temporalPayload(format, local, offset = '', utc = false) {
  if (format === 'date') return local;
  const seconds = local.length === (format === 'time' ? 5 : 16) ? `${local}:00` : local;
  return format === 'time' ? seconds : seconds + (utc ? 'Z' : offset);
}
export function formatTemporal(spec, value, language) {
  if (spec.format === 'time') return value;
  const date = new Date(spec.format === 'date' ? `${value}T00:00:00Z` : value);
  if (!Number.isFinite(date.getTime())) return value;
  const options = spec.format === 'date' ? { dateStyle: 'medium', timeZone: 'UTC' } : { dateStyle: 'medium', timeStyle: 'medium', timeZone: spec.timezone };
  try { return new Intl.DateTimeFormat(language, options).format(date); } catch { return value; }
}
