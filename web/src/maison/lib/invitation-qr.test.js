import test from 'node:test';
import assert from 'node:assert/strict';
import jsQR from 'jsqr';
import { invitationLink } from './invitation.js';
import { invitationQr } from './invitation-qr.js';

function raster(qr) {
  const scale = 6;
  const width = qr.size * scale;
  const data = new Uint8ClampedArray(width * width * 4).fill(255);
  for (const match of qr.path.matchAll(/M(\d+) (\d+)h1v1h-1z/g)) {
    const x = Number(match[1]); const y = Number(match[2]);
    for (let dy = 0; dy < scale; dy++) for (let dx = 0; dx < scale; dx++) {
      const offset = ((y * scale + dy) * width + x * scale + dx) * 4;
      data[offset] = data[offset + 1] = data[offset + 2] = 0;
    }
  }
  return { data, width };
}

test('the rendered QR path is independently decoded back to the exact invitation link', async () => {
  for (const base of ['http://127.0.0.1:8790/', 'https://synthetic-long-hostname.example.com:9443/']) {
    const link = invitationLink(base, 'ABCDEFGH');
    const qr = await invitationQr(link);
    assert.ok(qr.size >= 29);
    const { data, width } = raster(qr);
    assert.equal(jsQR(data, width, width)?.data, link);
  }
});

test('QR generation refuses unsafe origins and extra invitation parameters', async () => {
  for (const link of ['javascript:fixture', 'https://example.com/#invite=ABCDEFGH&next=elsewhere', `https://example.com/${'x'.repeat(2048)}`]) {
    await assert.rejects(invitationQr(link));
  }
});
