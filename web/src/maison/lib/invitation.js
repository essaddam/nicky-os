// The existing code is carried only in a fragment; no new invitation secret.
function invitationCode(value) {
  const code = String(value ?? '').replaceAll('-', '').toUpperCase();
  if (!/^[A-HJKMNP-Z2-9]{8}$/.test(code)) throw new Error('invalid invitation code');
  return code;
}

export function invitationLink(base, code) {
  const current = new URL(base);
  if (!['https:', 'http:'].includes(current.protocol)) throw new Error('invalid invitation origin');
  const link = new URL('/', current.origin);
  link.hash = `invite=${invitationCode(code)}`;
  return link.href;
}

export function consumeInvitation(location, history) {
  const parameters = new URLSearchParams(location.hash.slice(1));
  if (!parameters.has('invite')) return null;
  const clean = new URL(location.href);
  clean.hash = '/';
  history.replaceState(null, '', clean.href);
  const entries = [...parameters.entries()];
  if (entries.length !== 1 || entries[0][0] !== 'invite') return null;
  try { return invitationCode(entries[0][1]); }
  catch { return null; }
}

export function guestRoomList(rooms, person) {
  if (person?.role !== 'guest' || !Array.isArray(person.rooms)) return [];
  const allowed = new Set(person.rooms);
  return rooms.filter((room) => allowed.has(room.name));
}
