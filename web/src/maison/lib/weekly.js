export function minuteText(minute) {
  return `${String(Math.floor(minute / 60)).padStart(2, '0')}:${String(minute % 60).padStart(2, '0')}`;
}
export function parseMinute(text, end = false) {
  if (end && text === '24:00') return 1440;
  if (!/^(?:[01][0-9]|2[0-3]):[0-5][0-9]$/.test(text)) return null;
  return Number(text.slice(0, 2)) * 60 + Number(text.slice(3));
}
export function weeklyPayload(timezone, rows) {
  if (!timezone || rows.length > 256) return null;
  const slots = rows.map((row) => ({day: Number(row.day), start: parseMinute(row.start), end: parseMinute(row.end, true)}));
  if (slots.some((s) => !Number.isInteger(s.day) || s.day < 1 || s.day > 7 || s.start == null || s.end == null || s.start === s.end)) return null;
  return { type: 'schedule', value: { timezone, slots } };
}

/** A server preview must describe exactly the definition currently shown. */
export function previewMatchesShown(view, preview) {
  return view?.reference.kind === preview?.reference.kind
    && view?.reference.id === preview?.reference.id
    && view?.versions.draft.version === preview?.versions.draft
    && (view?.versions.current?.version ?? null) === preview?.versions.current;
}
