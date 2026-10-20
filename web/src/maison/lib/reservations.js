// Only a human selects a live calendar's opaque reservation identity.
export function reservationBinding(value) {
  if (!value) return null;
  const match = /^([a-zA-Z0-9_-]{1,64}):([a-fA-F0-9]{32})$/.exec(value);
  if (!match) throw new Error('invalid reservation reference');
  return { instance: match[1], id: match[2] };
}

export function reservationChoices(devices, drivers) {
  const calendars = new Set(drivers.filter((driver) => driver.kind === 'reservations').map((driver) => driver.instance));
  const choices = new Map();
  for (const device of Object.values(devices)) {
    if (!calendars.has(device.instance) || device.online !== true || !device.id.startsWith(`${device.instance}:next-`)) continue;
    const id = device.state?.reservation_id?.value;
    const start = device.state?.start?.value;
    const end = device.state?.end?.value;
    if (typeof id !== 'string' || !/^[a-fA-F0-9]{32}$/.test(id) || !Number.isFinite(start) || !Number.isFinite(end) || start >= end) continue;
    const value = `${device.instance}:${id}`;
    choices.set(value, { value, start, end, name: device.label?.name ?? device.native_name });
  }
  return [...choices.values()].sort((a, b) => a.start - b.start || a.value.localeCompare(b.value));
}
