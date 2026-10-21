const pad = (number, width = 2) => String(number).padStart(width, '0');

export function dateTimeInput(milliseconds) {
  if (milliseconds === null || milliseconds === undefined) return '';
  const date = new Date(milliseconds);
  if (!Number.isFinite(date.getTime())) throw new Error('invalid account date');
  let text = `${pad(date.getFullYear(), 4)}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
  if (date.getSeconds() || date.getMilliseconds()) text += `:${pad(date.getSeconds())}`;
  if (date.getMilliseconds()) text += `.${pad(date.getMilliseconds(), 3).replace(/0+$/, '')}`;
  return text;
}

function localTimestamp(text) {
  const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::(\d{2})(?:\.(\d{1,3}))?)?$/.exec(text);
  if (!match) throw new Error('invalid account date');
  const [year, month, day, hour, minute, second] = match.slice(1, 7).map((value) => Number(value ?? 0));
  const milliseconds = Number((match[7] ?? '').padEnd(3, '0'));
  if (year < 1970 || month < 1 || month > 12 || day < 1 || day > 31 || hour > 23 || minute > 59 || second > 59) throw new Error('invalid account date');
  const civil = Date.UTC(year, month - 1, day, hour, minute, second, milliseconds);
  const offsets = new Set([-48, 0, 48].map((hours) => new Date(civil + hours * 3_600_000).getTimezoneOffset()));
  const candidates = [...offsets].map((offset) => civil + offset * 60_000).filter((instant) => {
    const date = new Date(instant);
    return date.getFullYear() === year && date.getMonth() + 1 === month && date.getDate() === day && date.getHours() === hour && date.getMinutes() === minute && date.getSeconds() === second && date.getMilliseconds() === milliseconds;
  });
  if (candidates.length !== 1) throw new Error('ambiguous or nonexistent account date');
  return candidates[0];
}

export function editedDateTime(text, previous) {
  if (previous !== null && previous !== undefined && text === dateTimeInput(previous)) return previous;
  if (!text) return null;
  return localTimestamp(text);
}
