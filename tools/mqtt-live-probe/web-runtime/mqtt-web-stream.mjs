import { Duplex } from 'node:stream';

// Keep MQTT's byte framing intact, but bound individual WebSocket messages.
export function mqttWebStream(transport, maximum = 8192) {
  const connection = new Duplex({
    read() { transport.resume(); },
    write(chunk, encoding, done) {
      const data = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk, encoding);
      let offset = 0;
      const next = error => {
        if (error) { done(error); return; }
        if (offset >= data.length) { done(); return; }
        const part = data.subarray(offset, offset + maximum);
        offset += part.length;
        transport.write(part, next);
      };
      next();
    },
    final(done) { transport.end(done); },
    destroy(error, done) { transport.destroy(); done(error); },
  });
  transport.on('data', chunk => { if (!connection.push(chunk)) transport.pause(); });
  transport.on('end', () => connection.push(null));
  transport.on('error', error => connection.destroy(error));
  transport.on('close', () => connection.destroy());
  const read = connection.read;
  // Aedes drains a batch with read(null). Explicitly consume available bytes
  // so newer Node versions do not leave the rest of a fragmented packet stuck.
  connection.read = function readBatch(size) {
    return read.call(this, size == null && this.readableLength > 0 ? this.readableLength : size);
  };
  return connection;
}
