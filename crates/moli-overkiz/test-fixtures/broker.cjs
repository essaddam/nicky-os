// Local integration-test fixture only; never used by a Moli production process.
const { createServer } = require('node:net');
const { resolve } = require('node:path');
const { pathToFileURL } = require('node:url');

let input = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', chunk => {
  input += chunk;
  if (Buffer.byteLength(input) > 4096) process.exit(2);
});
process.stdin.on('end', () => start().catch(() => {
  process.stderr.write('Synthetic broker startup failed\n');
  process.exit(2);
}));

async function start() {
  const config = JSON.parse(input);
  if (!config.prefix.startsWith('moli-probe/') || !config.username || !config.password) {
    throw new Error('Invalid fixture configuration');
  }
  const modulePath = process.env.MOLI_MQTT_TEST_AEDES
    || resolve(__dirname, 'node_modules/aedes/aedes.js');
  const { Aedes } = await import(pathToFileURL(modulePath).href);
  const broker = await Aedes.createBroker({ concurrency: 16, queueLimit: 128, connectTimeout: 5000 });
  broker.authenticate = (_client, username, password, callback) => {
    callback(null, username === config.username && password?.toString() === config.password);
  };
  broker.authorizeSubscribe = (_client, subscription, callback) => {
    callback(null, subscription.topic.startsWith(`${config.prefix}/`) ? subscription : null);
  };
  broker.authorizePublish = (_client, packet, callback) => {
    const allowed = packet.topic.startsWith(`${config.prefix}/`) && packet.payload.length <= 1024 * 1024;
    callback(allowed ? null : new Error('Fixture publication refused'));
  };
  for (const event of ['clientError', 'connectionError']) broker.on(event, () => {});
  const server = createServer(socket => {
    if (socket.remoteAddress !== '127.0.0.1') return socket.destroy();
    socket.setNoDelay(true);
    broker.handle(socket);
  });
  server.maxConnections = 16;
  await new Promise((accept, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', accept);
  });
  process.stdout.write(`${server.address().port}\n`);
  // A crashed test harness cannot leave this test broker running indefinitely.
  setTimeout(() => process.exit(0), 30000).unref();
}
