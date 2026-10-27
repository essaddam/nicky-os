import { createServer as httpServer } from 'node:http';
import { createServer as tcpServer } from 'node:net';
import { readFile, mkdir, chmod, unlink, stat } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, resolve, join } from 'node:path';
import { spawn, execFile } from 'node:child_process';
import { Aedes } from 'aedes';
import mqtt from 'mqtt';
import { WebSocketServer, createWebSocketStream } from 'ws';
import { exposeDescription, nativePackets } from './commands.mjs';
import { createSocketTickets, permitsWebSocket, validCredentials } from './websocket-access.mjs';
import { mqttWebStream } from './mqtt-web-stream.mjs';

const directory = dirname(fileURLToPath(import.meta.url));
const root = resolve(directory, '../../..');
const portIndex = process.argv.indexOf('--port');
const port = portIndex < 0 ? 1414 : Number(process.argv[portIndex + 1]);
if (!Number.isInteger(port) || port < 1024 || port > 65534) throw Error('port');
const mqttPort = port + 1;
const controlDir = `/tmp/moli-poc-${process.getuid()}`;
await mkdir(controlDir, { recursive: true, mode: 0o700 });
if ((await stat(controlDir)).uid !== process.getuid()) throw Error('control owner');
await chmod(controlDir, 0o700);
const controlPath = join(controlDir, `${port}.sock`);
try { await unlink(controlPath); } catch (e) { if (e.code !== 'ENOENT') throw e; }
const state = { ready: false, password: '', username: '', hosts: [], child: null, runtime: null, rustRss: 0, registries: new Set(), commandQueue: [], commandWorkers: 0, stopping: false };
const bindings = new Map();
const retained = new Map();
const socketTickets = createSocketTickets();
const webDiagnostics = { upgrades: 0, refusedTickets: 0, refusedCredentials: 0, publicOrigins: [] };
const broker = await Aedes.createBroker({ concurrency: 128, queueLimit: 256, connectTimeout: 10000 });
const normalizedIp = ip => String(ip || '').replace(/^::ffff:/, '');
const localIp = ip => ['127.0.0.1', '::1'].includes(normalizedIp(ip));
const publish = packet => broker.publish({ cmd: 'publish', qos: 1, retain: false, ...packet }, () => {});

broker.authenticate = (client, username, password, callback) => {
  const browser = client.conn.pocBrowser === true;
  client.pocRole = browser ? 'browser' : 'bridge';
  const allowed = state.ready && validCredentials(username, password, state.username, state.password);
  if (!allowed) webDiagnostics.refusedCredentials++;
  callback(null, allowed);
};
broker.authorizeSubscribe = (client, subscription, callback) => {
  const allowed = client.pocRole === 'bridge' || subscription.topic === 'moli-probe/maison/#' || subscription.topic === 'moli-probe/runtime/#';
  callback(null, allowed ? subscription : null);
};
broker.authorizePublish = (client, packet, callback) => {
  try {
    if (!client || client.pocRole !== 'browser') {
      if (!packet.topic.startsWith('moli-probe/')) throw Error('namespace');
      if (packet.payload.length > 4 * 1024 * 1024) throw Error('size');
      if (packet.topic.endsWith('/config') && packet.payload.length) {
        const description = JSON.parse(packet.payload.toString());
        if (description.type === 'description') packet.payload = Buffer.from(JSON.stringify(exposeDescription(description, bindings, state.hosts.length)));
      }
      callback(null); return;
    }
    if (packet.retain || packet.payload.length > 256) throw Error('command');
    const binding = bindings.get(packet.topic);
    if (!binding) throw Error('unknown');
    nativePackets(binding, JSON.parse(packet.payload.toString()));
    if (state.commandQueue.length >= 128) throw Error('queue');
    callback(null);
  } catch { callback(new Error('Commande refusée par le contrat')); }
};
broker.on('publish', (packet, client) => {
  if (packet.retain) {
    if (packet.payload.length) retained.set(packet.topic, packet.payload.length); else retained.delete(packet.topic);
  }
  if (client?.pocRole === 'browser' && bindings.has(packet.topic)) {
    state.commandQueue.push({ topic: packet.topic, body: JSON.parse(packet.payload.toString()), binding: bindings.get(packet.topic), expires: Date.now() + 5000 });
    drainCommands();
  }
});
for (const event of ['clientError', 'connectionError']) broker.on(event, () => {});

function drainCommands() {
  while (state.commandWorkers < 4 && state.commandQueue.length) {
    const job = state.commandQueue.shift(); state.commandWorkers++;
    executeCommand(job).finally(() => { state.commandWorkers--; drainCommands(); });
  }
}
async function executeCommand(job) {
  let ok = false;
  if (Date.now() < job.expires) {
    const client = mqtt.connect(`mqtt://${state.hosts[job.binding.source]}:1883`, { username: state.username, password: state.password, reconnectPeriod: 0, clean: true, connectTimeout: Math.max(1, job.expires - Date.now()), queueQoSZero: false, clientId: `moli-poc-human-${Date.now()}-${Math.random().toString(16).slice(2)}` });
    ok = await new Promise(resolveResult => {
      const finish = success => { clearTimeout(timer); client.end(true); resolveResult(success); };
      const timer = setTimeout(() => finish(false), Math.max(1, job.expires - Date.now()));
      client.once('error', () => finish(false));
      client.once('connect', async () => {
        try {
          for (const packet of nativePackets(job.binding, job.body)) {
            if (Date.now() >= job.expires) throw Error('expired');
            await client.publishAsync(packet.topic, packet.payload, { qos: 1, retain: false });
          }
          finish(true);
        } catch { finish(false); }
      });
    });
  }
  publish({ topic: job.topic.replace(/\/set$/, '/result'), payload: Buffer.from(JSON.stringify({ v: 3, type: 'commande_resultat', ok, livraison: ok ? 'transmise_sans_confirmation_physique' : 'non_confirmee', ts: Date.now() })) });
}

const tcp = tcpServer(socket => broker.handle(socket));
await new Promise((resolveReady, reject) => { tcp.once('error', reject); tcp.listen(mqttPort, '127.0.0.1', resolveReady); });
const htmlPath = resolve(root, 'docs/proposition/analyse/poc-interface-mqtt.html');
const ws = new WebSocketServer({ noServer: true, maxPayload: 4 * 1024 * 1024 });
ws.on('connection', socket => { const stream = mqttWebStream(createWebSocketStream(socket)); stream.pocBrowser = true; broker.handle(stream); });

function status() {
  const memory = process.memoryUsage();
  return { ready: state.ready, port, mqtt_port: mqttPort, sources: state.hosts.map((_, index) => ({ index, registry: state.registries.has(index) || (state.runtime?.sources?.[index]?.projection?.channels ?? 0) > 0, projection: state.runtime?.sources?.[index] || null })), hub: state.runtime, memory: { node_rss: memory.rss, node_heap: memory.heapUsed, rust_rss: state.rustRss, retained_payload_bytes: [...retained.values()].reduce((sum, n) => sum + n, 0), retained_topics: retained.size }, clients: broker.connectedClients, web: webDiagnostics };
}
function respond(response, code, body, type = 'application/json') { response.writeHead(code, { 'content-type': `${type}; charset=utf-8`, 'cache-control': 'no-store', 'x-content-type-options': 'nosniff' }); response.end(type === 'application/json' ? JSON.stringify(body) : body); }
async function body(request) { const chunks = []; let size = 0; for await (const chunk of request) { size += chunk.length; if (size > 4 * 1024 * 1024) throw Error('size'); chunks.push(chunk); } return JSON.parse(Buffer.concat(chunks).toString()); }
const web = httpServer(async (request, response) => {
  try {
    const path = new URL(request.url, 'http://localhost').pathname;
    if (request.method === 'GET' && path === '/') {
      let html = await readFile(htmlPath, 'utf8');
      html = html.replace('https://unpkg.com/mqtt@5.10.3/dist/mqtt.min.js', '/mqtt.min.js');
      respond(response, 200, html, 'text/html');
    } else if (request.method === 'GET' && path === '/mqtt.min.js') {
      respond(response, 200, await readFile(resolve(directory, 'node_modules/mqtt/dist/mqtt.min.js')), 'application/javascript');
    } else if (request.method === 'GET' && path === '/api/status') {
      respond(response, 200, { ready: state.ready, port });
    } else if (request.method === 'GET' && path === '/api/socket-ticket') {
      if (!state.ready) { respond(response, 503, { error: 'starting' }); return; }
      try {
        const hint = new URL(request.url, 'http://localhost').searchParams.get('origin');
        if (hint) {
          const origin = new URL(hint);
          if (['http:', 'https:'].includes(origin.protocol) && !webDiagnostics.publicOrigins.includes(origin.origin)) {
            webDiagnostics.publicOrigins.push(origin.origin);
            if (webDiagnostics.publicOrigins.length > 8) webDiagnostics.publicOrigins.shift();
          }
        }
      } catch { /* Diagnostic only, never an authorization input. */ }
      respond(response, 200, socketTickets.issue());
    } else if (request.method === 'POST' && path === '/source/registry' && state.ready) {
      const ip = normalizedIp(request.socket.remoteAddress);
      const source = state.hosts.findIndex(host => host === ip);
      if (source < 0) { respond(response, 403, { error: 'source' }); return; }
      const data = await body(request);
      if (data.source !== source || !Array.isArray(data.registry?.entities) || data.registry.entities.length > 8192) { respond(response, 400, { error: 'registry' }); return; }
      await new Promise((done, reject) => broker.publish({ topic: `moli-probe/source/z2m-${source}/registry`, payload: Buffer.from(JSON.stringify(data.registry)), qos: 1, retain: true }, error => error ? reject(error) : done()));
      state.registries.add(source); respond(response, 200, { accepted: true, source });
    } else if (request.method === 'GET' && path === '/manifest.webmanifest') {
      respond(response, 200, { name: 'Moli — Maison', short_name: 'Moli', start_url: '/', display: 'standalone', background_color: '#edf0f5', theme_color: '#edf0f5', icons: [] }, 'application/json');
    } else respond(response, 404, { error: 'not_found' });
  } catch { respond(response, 400, { error: 'request' }); }
});
web.on('upgrade', (request, socket, head) => {
  webDiagnostics.upgrades++;
  if (!state.ready || !permitsWebSocket(request, socketTickets)) {
    webDiagnostics.refusedTickets++;
    socket.end('HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n');
    return;
  }
  ws.handleUpgrade(request, socket, head, client => ws.emit('connection', client, request));
});
await new Promise((done, reject) => { web.once('error', reject); web.listen(port, '0.0.0.0', done); });

const control = tcpServer(socket => {
  let data = ''; socket.on('data', chunk => { data += chunk; if (data.length > 16384) socket.destroy(); });
  socket.on('end', () => {
    try {
      const config = JSON.parse(data);
      if (config.op === 'status') { socket.end(JSON.stringify(status())); return; }
      if (state.ready) throw Error('configured');
      if (!Array.isArray(config.hosts) || !config.hosts.length || config.hosts.length > 4 || config.hosts.some(h => typeof h !== 'string' || !/^[a-zA-Z0-9.:-]+$/.test(h)) || typeof config.password !== 'string' || !config.password.length) throw Error('config');
      state.hosts = config.hosts; state.username = config.username || 'test-mqtt'; state.password = config.password; state.ready = true;
      const args = state.hosts.flatMap(host => ['--host', host]);
      args.push('--port', '1883', '--username', state.username, '--seconds', '0', '--bridges', String(state.hosts.length), '--republish', 'moli-probe', '--republish-retain', '--virtual-tree', '--virtual-mirror', '--publish-config', '--metadata-memory', '--destination', `127.0.0.1:${mqttPort}`);
      const child = spawn(resolve(root, 'target/mqtt-live-probe/debug/mqtt-live-probe'), args, { cwd: root, env: { ...process.env, MQTT_PASSWORD: state.password }, stdio: ['ignore', 'pipe', 'pipe'] });
      state.child = child;
      let lines = ''; child.stdout.on('data', chunk => { lines += chunk; const split = lines.split('\n'); lines = split.pop(); for (const line of split) { if (line.startsWith('POC_RUNTIME ')) { try { state.runtime = JSON.parse(line.slice(12)); } catch {} } else if (line.startsWith('HOUSE_')) console.log(line); } });
      child.stderr.on('data', chunk => { if (String(chunk).includes('HOUSE_ERROR')) console.log('Un pont attend une reconnexion.'); });
      child.on('exit', code => { console.log(`Processus Rust arrêté (${code}).`); state.child = null; });
      socket.end(JSON.stringify({ ready: true, sources: state.hosts.length, port, mqtt_port: mqttPort }));
      console.log('Ponts Rust démarrés ; réception des registres attendue.');
      publishRuntimeDescription();
    } catch { socket.end(JSON.stringify({ error: 'configuration_refused' })); }
  });
});
await new Promise((done, reject) => { control.once('error', reject); control.listen(controlPath, done); });
await chmod(controlPath, 0o600);

const runtimeFields = [ ['ram_rust_rss', 'Hub et ponts Rust · RSS', 'B'], ['ram_rust_requested', 'Allocations Rust vivantes', 'B'], ['ram_node_rss', 'Broker de démonstration Node · RSS', 'B'], ['ram_node_heap', 'Tas Node utilisé', 'B'], ['retained_bytes', 'Payloads retained du broker virtuel', 'B'], ['retained_topics', 'Topics retained', 'topics'], ['hub_devices', 'Appareils et virtuels dans le Hub Rust', 'appareils'], ['hub_points', 'Points déclarés dans le Hub Rust', 'canaux'] ];
function publishRuntimeDescription() { publish({ topic: 'moli-probe/runtime/config', retain: true, payload: Buffer.from(JSON.stringify({ v: 3, type: 'description', pont: 'moli-probe/runtime', nom: 'Hub', noeuds: [], appareils: [], systeme: runtimeFields.map(([id, nom, unite]) => ({ id, nom, unite })) })) }); }
setInterval(() => {
  if (!state.ready) return;
  if (state.child) execFile('ps', ['-o', 'rss=', '-p', String(state.child.pid)], (error, output) => { if (!error) state.rustRss = Number(output.trim()) * 1024; });
  const s = status(); const values = { ram_rust_rss: s.memory.rust_rss, ram_rust_requested: s.hub?.allocator_live_bytes ?? null, ram_node_rss: s.memory.node_rss, ram_node_heap: s.memory.node_heap, retained_bytes: s.memory.retained_payload_bytes, retained_topics: s.memory.retained_topics, hub_devices: s.hub?.hub_devices ?? null, hub_points: s.hub?.hub_points ?? null };
  publish({ topic: 'moli-probe/runtime/state', retain: true, payload: Buffer.from(JSON.stringify({ v: 3, type: 'systeme', pont: 'moli-probe/runtime', canaux: runtimeFields.map(([id]) => ({ id, valeur: values[id] })), ts: Date.now() })) });
}, 2000).unref();
async function stop() { if (state.stopping) return; state.stopping = true; state.child?.kill('SIGTERM'); web.close(); tcp.close(); control.close(); ws.close(); broker.close(() => process.exit(0)); try { await unlink(controlPath); } catch {} setTimeout(() => process.exit(0), 2000).unref(); }
process.on('SIGINT', stop); process.on('SIGTERM', stop);
console.log(`POC iPhone : http://localhost:${port} · MQTT local : ${mqttPort}`);
console.log('Configuration confidentielle attendue sur le canal local privé.');
