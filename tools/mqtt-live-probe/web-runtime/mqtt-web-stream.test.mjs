import test from 'node:test';
import assert from 'node:assert/strict';
import { Duplex } from 'node:stream';
import { mqttWebStream } from './mqtt-web-stream.mjs';

test('le manifeste MQTT reste identique avec des messages WebSocket bornés', async () => {
  const chunks = [];
  const transport = new Duplex({ read() {}, write(chunk, _encoding, done) { chunks.push(Buffer.from(chunk)); setImmediate(done); } });
  const stream = mqttWebStream(transport);
  const manifest = Buffer.alloc(1500000, 97);
  await new Promise((resolve, reject) => stream.write(manifest, error => error ? reject(error) : resolve()));
  assert(chunks.length > 1);
  assert(chunks.every(chunk => chunk.length <= 8192));
  assert.deepEqual(Buffer.concat(chunks), manifest);
  stream.destroy();
});
