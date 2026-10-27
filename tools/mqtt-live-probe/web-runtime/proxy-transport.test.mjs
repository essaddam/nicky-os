import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { Aedes } from 'aedes';
import mqtt from 'mqtt';
import WebSocket, { WebSocketServer, createWebSocketStream } from 'ws';
import { createSocketTickets, permitsWebSocket, validCredentials } from './websocket-access.mjs';
import { mqttWebStream } from './mqtt-web-stream.mjs';

test('un client derrière proxy reçoit un gros retained, avec ticket et authentification', { timeout: 10000 }, async () => {
  const trace = [];
  const tickets = createSocketTickets();
  const broker = await Aedes.createBroker();
  broker.authenticate = (_client, user, password, done) => {trace.push('authenticate');done(null, validCredentials(user, password, 'fixture', 'fixture-password'));};
  const ws = new WebSocketServer({ noServer: true });
  ws.on('connection', socket => {trace.push('ws_connection');const stream=mqttWebStream(createWebSocketStream(socket));broker.handle(stream);});
  const server = createServer((_request, response) => { response.setHeader('content-type', 'application/json'); response.end(JSON.stringify(tickets.issue())); });
  server.on('upgrade', (request, socket, head) => {
    if (!permitsWebSocket(request, tickets)) { socket.end('HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n'); return; }
    ws.handleUpgrade(request, socket, head, client => ws.emit('connection', client));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const port = server.address().port;
  const payload = Buffer.alloc(1500000, 97);
  await new Promise((resolve, reject) => broker.publish({ topic: 'fixture/description', payload, qos: 1, retain: true }, e => e ? reject(e) : resolve()));
  const ticket = await (await fetch(`http://127.0.0.1:${port}/api/socket-ticket`)).json();
  const options = password => ({ username: 'fixture', password, manualConnect: true, reconnectPeriod: 0, connectTimeout: 3000, clean: true,
    createWebsocket: url => new WebSocket(url, { origin: 'https://preview.example.org', headers: { 'x-forwarded-host': 'preview.example.org', 'x-forwarded-proto': 'https' }, maxPayload: 32768 }) });
  let client;
  try {
    client = mqtt.connect(`ws://127.0.0.1:${port}/mqtt?ticket=${ticket.ticket}`, options('fixture-password'));
    const received = new Promise((resolve, reject) => { client.once('error', reject); client.once('connect', () => client.subscribe('fixture/#', { qos: 1 })); client.once('message', (_topic, data, packet) => resolve({ data, retained: packet.retain })); });
    client.connect();
    const actual = await received;
    assert.equal(actual.retained, true);
    assert.deepEqual(actual.data, payload);
    await client.endAsync(true);
    client = mqtt.connect(`ws://127.0.0.1:${port}/mqtt?ticket=${ticket.ticket}`, options('wrong'));
    const refused = new Promise(resolve => client.once('error', resolve));
    client.connect();
    assert.equal((await refused).code, 5);
  } catch(error) {throw new Error(error.message+' trace='+trace.join(','));
  } finally {
    client?.end(true);
    for (const socket of ws.clients) socket.terminate();
    ws.close();
    await new Promise(resolve => broker.close(resolve));
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
  }
});
