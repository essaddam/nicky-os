import { createHmac, randomBytes, timingSafeEqual } from 'node:crypto';

// A short-lived, stateless proof that the WebSocket was opened from this web
// client. It never replaces MQTT authentication and contains no MQTT secret.
export function createSocketTickets({ key = randomBytes(32), now = Date.now, lifetime = 300000 } = {}) {
  const sign = payload => createHmac('sha256', key).update(payload).digest();
  return {
    issue() {
      const expiresAt = now() + lifetime;
      const payload = Buffer.alloc(24);
      payload.writeBigUInt64BE(BigInt(expiresAt));
      randomBytes(16).copy(payload, 8);
      return { ticket: `${payload.toString('base64url')}.${sign(payload).toString('base64url')}`, expiresAt };
    },
    verify(ticket) {
      if (typeof ticket !== 'string' || ticket.length > 160 || !/^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/.test(ticket)) return false;
      const [body, mac] = ticket.split('.');
      const payload = Buffer.from(body, 'base64url');
      const signature = Buffer.from(mac, 'base64url');
      if (payload.length !== 24 || signature.length !== 32 || !timingSafeEqual(signature, sign(payload))) return false;
      const expiresAt = Number(payload.readBigUInt64BE());
      return Number.isSafeInteger(expiresAt) && expiresAt > now() && expiresAt <= now() + lifetime;
    },
  };
}

export function permitsWebSocket(request, tickets) {
  try {
    const url = new URL(request.url, 'http://localhost');
    return url.pathname === '/mqtt' && tickets.verify(url.searchParams.get('ticket'));
  } catch { return false; }
}

export function validCredentials(username, password, expectedUsername, expectedPassword) {
  const equal = (a, b) => {
    const left = Buffer.from(String(a ?? ''));
    const right = Buffer.from(String(b ?? ''));
    return left.length === right.length && timingSafeEqual(left, right);
  };
  return Boolean(expectedPassword) && equal(username, expectedUsername) && equal(password?.toString(), expectedPassword);
}
