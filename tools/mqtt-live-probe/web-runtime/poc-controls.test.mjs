import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const html = readFileSync(new URL('../../../docs/proposition/analyse/poc-interface-mqtt.html', import.meta.url), 'utf8');
const enumHelpers = html.slice(html.indexOf('function isU32('), html.indexOf('function generic('));
const controls = html.slice(html.indexOf('function isPrimaryChannel('), html.indexOf('function showDetail('));
class Element {
  constructor() { this.dataset = {}; this.children = new Map(); this.rows = []; this.attributes = {}; this.value = ''; this.open = true; }
  set innerHTML(markup) {
    this.markup = markup; this.rows = [];
    for (const match of markup.matchAll(/data-(output|color)-id="([^"]+)"/g)) {
      const row = new Element(); row.dataset[match[1] === 'color' ? 'colorId' : 'outputId'] = match[2]; this.rows.push(row);
      if (match[1] === 'color') row.querySelector('input').value = /type="color" value="([^"]+)"/.exec(markup)?.[1] || '#ffffff';
    }
  }
  get innerHTML() { return this.markup || ''; }
  querySelector(selector) { if (!this.children.has(selector)) this.children.set(selector, new Element()); return this.children.get(selector); }
  querySelectorAll(selector) { return selector === '[data-command-form]' ? [] : this.rows; }
  setAttribute(name, value) { this.attributes[name] = value; }
  contains(element) { return element === this || this.rows.some(row => row.contains(element)) || [...this.children.values()].some(child => child.contains(element)); }
}
function harness() {
  const elements = new Map(); const publications = [];
  const $ = id => { if (!elements.has(id)) elements.set(id, new Element()); return elements.get(id); };
  const state = { lang: 'fr', configs: new Map(), enumCatalogues: new Map(), enumDescriptions: new WeakSet(), enumPending: new Map(), observation: 0, devices: new Map(), commandDrafts: new Map(), colorDrafts: new Map(), connected: true, detail: 'synthetic|fixture', client: { connected: true, publish(topic, body, options, callback) { publications.push({ topic, body, options, callback }); } } };
  const context = { state, $, document: { activeElement: null }, TextEncoder, URL, Date, Intl, setTimeout, clearTimeout,
    key: (p, id) => `${p}|${id}`, t: key => key,
    esc: value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c])),
    formatted: (v, unit) => String(v) + (unit ? ' ' + unit : ''), channelValue: c => String(c.valeur), isActive: c => c.valeur === true || ['on', 'ON'].includes(c.valeur), actionStatus() {}, lightForKey() { return null; } };
  vm.createContext(context); vm.runInContext(enumHelpers, context); vm.runInContext(controls, context);
  return { $, state, context, publications, run: code => vm.runInContext(code, context) };
}
function fixture(domain = 'light') {
  const c = { id: 'main', nom: 'Synthetic output', domaine: domain, source: 'fixture/main', valeur: 'OFF', commande: { schema: 'virtual', topic: 'synthetic/canal/main/set', on: 'ON', off: 'OFF', parametres: { state: { type: 'enum', valeurs: ['ON', 'OFF'], source_channel: 'main' }, color: { type: 'color', modes: ['xy', 'rgb'], source_channels: { xy: { x: 'x', y: 'y' } } } } } };
  if (domain === 'switch') delete c.commande.parametres.color;
  return { id: 'fixture', pont: 'synthetic', nom: 'Synthetic fixture', canaux: [c] };
}
function mount(h, d) { h.context.d = d; h.state.devices.set('synthetic|fixture', d); }

test('le sélecteur respecte les modes déclarés, D65 et les bornes sans gamut inventé', () => {
  const h = harness();
  assert.equal(h.run("colorMode({type:'color',modes:['rgb','hs','xy']})"), 'xy');
  assert.equal(h.run("colorMode({type:'color',modes:['unknown']})"), undefined);
  const white = h.run("colorPayload('#ffffff','xy')");
  assert.ok(Math.abs(white.x - 0.3127) < 0.00001); assert.ok(Math.abs(white.y - 0.3290) < 0.00001);
  const red = h.run("colorPayload('#ff0000','xy')");
  assert.ok(Math.abs(red.x - 0.64) < 0.00001); assert.ok(Math.abs(red.y - 0.33) < 0.00001);
  assert.equal(h.run("colorPayload('#000000','xy')"), null);
  assert.equal(h.run("colorPayload('red','rgb')"), null);
  assert.equal(h.run("JSON.stringify(colorPayload('#00ff00','hs'))"), '{"h":120,"s":100}');
  assert.equal(h.run("JSON.stringify(colorPayload('#ff00ff','rgb'))"), '{"r":255,"g":0,"b":255}');
  for (const hex of ['#ffffff', '#ff0000', '#00ff00', '#0000ff', '#142a8b']) {
    h.context.hex = hex; const xy = h.run("colorPayload(hex,'xy')");
    assert.ok(xy.x >= 0 && xy.x <= 1 && xy.y >= 0 && xy.y <= 1 && xy.x + xy.y <= 1);
  }
});
test('la saisie couleur ne publie rien ; Envoi seul produit le JSON canonique', () => {
  const h = harness(); mount(h, fixture()); h.run('renderColorControls(d)');
  const form = h.$('color-controls').rows[0]; const input = form.querySelector('input');
  h.context.document.activeElement = input; input.value = '#ff0000'; input.oninput();
  assert.equal(h.publications.length, 0); assert.equal(h.$('color-controls').rows[0].querySelector('input'), input);
  form.onsubmit({ preventDefault() {} }); assert.equal(h.publications.length, 1);
  const sent = h.publications[0]; const body = JSON.parse(sent.body);
  assert.equal(sent.topic, 'synthetic/canal/main/set'); assert.equal(body.state, 'ON');
  assert.deepEqual(Object.keys(body.color).sort(), ['x', 'y']); assert.equal(sent.options.retain, false); assert.equal(sent.options.qos, 0);
  sent.callback(); assert.equal(h.run('colorDraft(d,d.canaux[0]).status'), 'colorAwaited');
  h.run("updateColorFeedback({pont:'synthetic',id:'x',valeur:0.64,retenu:true});updateColorFeedback({pont:'synthetic',id:'y',valeur:0.33,retenu:true})");
  assert.equal(h.run('colorDraft(d,d.canaux[0]).status'), 'colorAwaited');
  h.run("updateColorFeedback({pont:'synthetic',id:'x',valeur:0.64})");
  assert.equal(h.run('colorDraft(d,d.canaux[0]).status'), 'colorAwaited');
  h.run("updateColorFeedback({pont:'synthetic',id:'y',valeur:0.33})");
  assert.equal(h.run('colorDraft(d,d.canaux[0]).status'), 'stateConfirmed');
});
test('couleur sans retour déclaré, mauvais pont ou mauvaises coordonnées reste non confirmée', () => {
  const h = harness(); const d = fixture(); mount(h, d); delete d.canaux[0].commande.parametres.color.source_channels;
  h.run("colorDraft(d,d.canaux[0]).raw='#ff0000';publishColor('synthetic|fixture','main');updateColorFeedback({pont:'synthetic',id:'x',valeur:0.64});updateColorFeedback({pont:'synthetic',id:'y',valeur:0.33})");
  assert.equal(h.run('colorDraft(d,d.canaux[0]).status'), 'colorNoFeedback');
  d.canaux[0].commande.parametres.color.source_channels = { xy: { x: 'x', y: 'y' } };
  h.run("publishColor('synthetic|fixture','main');updateColorFeedback({pont:'other',id:'x',valeur:0.64});updateColorFeedback({pont:'synthetic',id:'y',valeur:0.33})");
  assert.equal(h.run('colorDraft(d,d.canaux[0]).status'), 'colorAwaited');
  h.run("updateColorFeedback({pont:'synthetic',id:'x',valeur:0.2})"); assert.equal(h.run('colorDraft(d,d.canaux[0]).status'), 'colorAwaited');
});
test('aucun éditeur couleur pour contrat absent ou mode inconnu ; retour par identité exacte', () => {
  const h = harness(); const d = fixture(); mount(h, d);
  d.canaux.push({ id: 'mode', source: 'fixture/main/color_mode', semantique: 'color_mode', valeur: 'xy' }, { id: 'unrelated', source: 'fixture/other/color_x', semantique: 'color_x', valeur: 0.1 });
  assert.equal(h.run('colorFeedback(d,d.canaux[0]).length'), 1);
  delete d.canaux[0].commande.parametres.color;
  assert.equal(h.run('colorFields(d).length'), 0);
});
test('les sorties ont chacune leur switch, sans commande globale ni état optimiste', () => {
  const h = harness(); const d = fixture('switch');
  d.canaux.push({ ...structuredClone(d.canaux[0]), id: 'second', nom: 'Synthetic second output', commande: { ...structuredClone(d.canaux[0].commande), topic: 'synthetic/canal/second/set' } });
  d.canaux.push({ id: 'diagnostic', domaine: 'switch', categorie: 'diagnostic' }, { id: 'config', domaine: 'switch', semantique: 'configuration' });
  mount(h, d); h.run('renderOutputControls(d)');
  assert.equal(h.$('output-controls').rows.length, 2); assert.equal(h.publications.length, 0);
  assert.match(h.$('output-controls').innerHTML, /deviceMeasuresHint/);
  const first = h.$('output-controls').rows[0]; first.querySelector('button').onclick();
  assert.equal(h.publications.length, 1); assert.equal(h.publications[0].body, '{"state":"ON"}'); assert.equal(h.publications[0].options.qos, 0);
  assert.equal(first.querySelector('button').attributes['aria-checked'], 'false');
  h.publications[0].callback(); assert.equal(first.querySelector('button').attributes['aria-checked'], 'false');
  d.canaux[0].valeur = 'ON'; h.run("updateCommandFeedback({pont:'synthetic',id:'main',valeur:'ON'});renderOutputControls(d)");
  assert.equal(first.querySelector('button').attributes['aria-checked'], 'true');
  assert.equal(h.run('commandFields(d).length'), 0);
});
test('les textes embarqués restent identiques aux catalogues et le script se parse', () => {
  const embedded = JSON.parse(/const MESSAGES\s*= (.*);/.exec(html)[1]);
  for (const lang of ['fr', 'en']) assert.deepEqual(embedded[lang], JSON.parse(readFileSync(new URL(`../../../locales/${lang}/mqtt-poc.json`, import.meta.url), 'utf8')));
  const script = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].at(-1)[1];
  assert.doesNotThrow(() => new vm.Script(script));
});

test('le retour différentiel confirme une coordonnée modifiée avec les autres axes connus', () => {
  for (const mode of ['hs', 'xy']) {
    const h = harness(); const d = fixture();
    const spec = d.canaux[0].commande.parametres.color;
    spec.modes = mode === 'hs' ? ['xy', 'hs'] : ['xy'];
    spec.source_channels = mode === 'hs' ? { hs: { h: 'h', s: 's' } } : { xy: { x: 'x', y: 'y' } };
    const selected = mode === 'hs' ? '#00ff00' : '#ff0000';
    d.canaux.push(...(mode === 'hs' ? [{ id: 'h', valeur: 0 }, { id: 's', valeur: 100 }] : [{ id: 'x', valeur: 0.3 }, { id: 'y', valeur: 0.33 }]));
    mount(h, d); h.context.selected = selected;
    assert.equal(h.run('colorMode(d.canaux[0].commande.parametres.color)'), mode);
    h.run("colorDraft(d,d.canaux[0]).raw=selected;publishColor('synthetic|fixture','main')");
    h.publications[0].callback();
    assert.equal(h.run('colorDraft(d,d.canaux[0]).status'), 'colorAwaited');
    h.run(mode === 'hs' ? "updateColorFeedback({pont:'synthetic',id:'h',valeur:120,retenu:true})" : "updateColorFeedback({pont:'synthetic',id:'x',valeur:0.64,retenu:true})");
    assert.equal(h.run('colorDraft(d,d.canaux[0]).status'), 'colorAwaited');
    h.run(mode === 'hs' ? "updateColorFeedback({pont:'synthetic',id:'h',valeur:120})" : "updateColorFeedback({pont:'synthetic',id:'x',valeur:0.64})");
    assert.equal(h.run('colorDraft(d,d.canaux[0]).status'), 'stateConfirmed');
  }
});

test('hors connexion, aucun geste ne publie et aucune file QoS zéro n’est activée', () => {
  const h = harness(); h.context.navigator = { onLine: false };
  mount(h, fixture()); h.run("publishColor('synthetic|fixture','main')");
  mount(h, fixture('switch')); h.run("toggleOutput('synthetic|fixture','main')");
  assert.equal(h.publications.length, 0);
  assert.match(html, /queueQoSZero:false/);
  assert.match(html, /id="offline-notice"/);
  assert.match(html, /src="\.\/mqtt\.min\.js"/);
});
test('le worker ne traite que la coque sans paramètres, identifiants ni origine tierce', async () => {
  const source = readFileSync(new URL('./public/service-worker.js', import.meta.url), 'utf8');
  const handlers = {}; const stores = new Map(); const requests = []; let offline = false;
  const origin = 'https://example.com/';
  const context = {
    URL, Request, Headers, Response,
    self: { registration: { scope: origin }, addEventListener(name, handler) { handlers[name] = handler; }, async skipWaiting() {}, clients: { async claim() {} } },
    caches: { async open(name) { if (!stores.has(name)) stores.set(name, new Map()); const store = stores.get(name); return { async put(url, response) { store.set(url, response.clone()); }, async match(url) { return store.get(url)?.clone(); } }; }, async keys() { return [...stores.keys()]; }, async delete(name) { return stores.delete(name); } },
    async fetch(request) {
      requests.push(request.url); if (offline) throw Error('offline');
      const path = new URL(request.url).pathname;
      const type = path === '/' ? 'text/html' : path.endsWith('.js') ? 'application/javascript' : path.endsWith('webmanifest') ? 'application/manifest+json' : path.endsWith('svg') ? 'image/svg+xml' : 'image/png';
      return new Response(path === '/' ? '<meta name="moli-poc-shell" content="1">' : 'synthetic static asset', { headers: { 'content-type': type } });
    },
  };
  vm.createContext(context); vm.runInContext(source, context);
  for (const [url, options] of [
    [origin + 'api/status'], [origin + 'api/socket-ticket'], [origin + 'mqtt'],
    [origin + '?ticket=synthetic'], [origin + 'mqtt.min.js?password=synthetic'],
    ['https://other.example.invalid/mqtt.min.js'], ['https://synthetic:synthetic@example.com/'],
    [origin, { headers: { authorization: 'synthetic' } }], [origin, { headers: { 'proxy-authorization': 'synthetic' } }],
    [origin, { method: 'POST' }],
  ]) {
    context.request = { method: options?.method || 'GET', headers: new Headers(options?.headers), url };
    assert.equal(vm.runInContext('allowedRequest(request)', context), false);
    let intercepted = false; handlers.fetch({ request: context.request, respondWith() { intercepted = true; } }); assert.equal(intercepted, false);
  }
  context.request = new Request(origin + 'mqtt.min.js'); assert.equal(vm.runInContext('allowedRequest(request)', context), true);
  let pending; handlers.install({ waitUntil(promise) { pending = promise; } }); await pending;
  const entries = [...stores.values()][0]; assert.equal(entries.size, 7);
  for (const url of entries.keys()) { assert.equal(new URL(url).origin, new URL(origin).origin); assert.equal(new URL(url).search, ''); }
  assert.equal(requests.length, 7);
  context.response = new Response('<html>synthetic sign in</html>', { headers: { 'content-type': 'text/html' } });
  assert.equal(await vm.runInContext(`cacheable(response,'${origin}')`, context), false);
  context.response = { ok: true, redirected: true };
  assert.equal(await vm.runInContext(`cacheable(response,'${origin}')`, context), false);
  stores.set('moli-poc-shell-obsolete', new Map()); stores.set('unrelated-app', new Map());
  handlers.activate({ waitUntil(promise) { pending = promise; } }); await pending;
  assert.equal(stores.has('moli-poc-shell-obsolete'), false); assert.equal(stores.has('unrelated-app'), true);
  offline = true;
  handlers.fetch({ request: new Request(origin), respondWith(promise) { pending = promise; } });
  assert.match(await (await pending).text(), /moli-poc-shell/);
});
test('le manifeste et les icônes locales respectent les tailles déclarées', () => {
  const manifest = JSON.parse(readFileSync(new URL('./public/manifest.webmanifest', import.meta.url), 'utf8'));
  assert.equal(manifest.display, 'standalone'); assert.equal(manifest.start_url, './'); assert.equal(manifest.scope, './');
  for (const [path, size] of [['moli-192.png', 192], ['moli-512.png', 512], ['apple-touch-icon.png', 180]]) {
    const png = readFileSync(new URL(`./public/icons/${path}`, import.meta.url));
    assert.equal(png.readUInt32BE(16), size); assert.equal(png.readUInt32BE(20), size);
  }
  assert.match(html, /rel="apple-touch-icon" sizes="180x180"/);
});

function enumState(code = 0, version = 1) {
  return { id: 'enum-state', pont: 'synthetic', valeur: code, encodage: 'enum', catalogue: 7, catalogue_version: version, qualite: 'connu' };
}
function enumDescription(version = 1, entries = [{ code: 0, native: 'ON', label: 'Synthetic active', active: true }, { code: 4, native: 'OFF', label: 'Synthetic inactive', active: true }]) {
  return { v: 3, pont: 'synthetic', canaux: { 'enum-state': { enumeration: { catalogue: 7, version } } }, catalogues: { 7: { id: 7, version, entries } } };
}
test('un enum attend sa description exacte, sans décodage avec une ancienne version', () => {
  const h = harness(); h.context.c = enumState();
  assert.equal(h.run('channelValue(c)'), 'enumWaiting'); assert.equal(h.run('nativeChannelValue(c)'), undefined);
  assert.equal(h.state.enumPending.size, 1);
  h.state.configs.set('synthetic', enumDescription());
  assert.equal(h.run('channelValue(c)'), 'Synthetic active'); assert.equal(h.run('nativeChannelValue(c)'), 'ON');
  assert.equal(h.state.enumPending.size, 0);
  h.context.c = enumState(0, 2);
  assert.equal(h.run('channelValue(c)'), 'enumWaiting');
  h.state.configs.set('synthetic', enumDescription(2, [{ code: 0, native: 'mode-b', label: 'Synthetic new description', active: true }]));
  assert.equal(h.run('channelValue(c)'), 'Synthetic new description');
  h.context.c = enumState(0, 1);
  assert.equal(h.run('channelValue(c)'), 'enumWaiting');
  h.state.configs.set('synthetic', enumDescription());
  assert.equal(h.run('channelValue(c)'), 'Synthetic active');
});
test('les codes sont indépendants de l’ordre des entrées et un catalogue reste immuable', () => {
  const h = harness(); const description = enumDescription(); description.catalogues[7].entries.reverse();
  h.state.configs.set('synthetic', description); h.context.c = enumState(4);
  assert.equal(h.run('channelValue(c)'), 'Synthetic inactive');
  const changed = enumDescription(); changed.catalogues[7].entries[1].native = 'unexpected'; changed.catalogues[7].entries[1].label = 'Unexpected changed mapping';
  h.state.configs.set('synthetic', changed);
  assert.equal(h.run('nativeChannelValue(c)'), 'OFF'); assert.equal(h.run('channelValue(c)'), 'Synthetic inactive');
});
test('codes inconnus, inactifs et références invalides ont une qualité explicite', () => {
  const h = harness(); h.state.configs.set('synthetic', enumDescription(1, [{ code: 0, native: 'ON', label: 'Unavailable', active: false }]));
  h.context.c = enumState(); assert.equal(h.run('channelValue(c)'), 'enumInactive'); assert.equal(h.run('quality(c)'), 'enumInactive'); assert.equal(h.run('nativeChannelValue(c)'), undefined);
  h.context.c = enumState(8); assert.equal(h.run('quality(c)'), 'enumUnknown');
  h.context.c = enumState(256); assert.equal(h.run('quality(c)'), 'enumInvalid');
  h.context.c = { ...enumState(), catalogue: -1 }; assert.equal(h.run('quality(c)'), 'enumInvalid');
  h.context.c = { ...enumState(), catalogue_version: '1' }; assert.equal(h.run('quality(c)'), 'enumInvalid');
});
test('un petit nombre ou du texte hors enum restent inchangés malgré les métadonnées', () => {
  const h = harness(); h.state.configs.set('synthetic', enumDescription());
  h.context.c = { id: 'enum-state', pont: 'synthetic', valeur: 0 };
  assert.equal(h.run('channelValue(c)'), '0'); assert.equal(h.run('nativeChannelValue(c)'), 0);
  h.context.c.valeur = 'ON'; assert.equal(h.run('channelValue(c)'), 'ON'); assert.equal(h.run('nativeChannelValue(c)'), 'ON');
  h.context.c = { ...enumState(), pont: 'other' }; assert.equal(h.run('channelValue(c)'), 'enumWaiting');
});
test('les retours codés utilisent native pour confirmer sans confondre label et commande', () => {
  const h = harness(); h.state.configs.set('synthetic', enumDescription());
  h.state.commandDrafts.set('synthetic-draft', { raw: 'ON', pending: true, status: 'stateAwaited', pont: 'synthetic', sourceId: 'enum-state', sentObservation: 10, spec: { type: 'enum', valeurs: ['ON', 'OFF'] } });
  h.context.c = { ...enumState(), _observation: 9 }; h.run('updateCommandFeedback(c)');
  assert.equal(h.state.commandDrafts.get('synthetic-draft').status, 'stateAwaited');
  h.context.c._observation = 11; h.run('updateCommandFeedback(c)');
  assert.equal(h.state.commandDrafts.get('synthetic-draft').status, 'stateConfirmed');
});
test('les valeurs codées conservent leurs références pendant hydratation et le suivi est borné', () => {
  const h = harness();
  vm.runInContext(html.slice(html.indexOf('function hydrateDevice('), html.indexOf('function hydrateNode(')), h.context);
  const description = enumDescription(); description.appareils = [{ id: 'fixture', canaux: ['enum-state'] }];
  h.state.configs.set('synthetic', description); h.context.d = { id: 'fixture', pont: 'synthetic', canaux: [enumState()] };
  assert.equal(h.run('nativeChannelValue(hydrateDevice(d).canaux[0])'), 'ON');
  assert.equal(h.run('hydrateDevice(d).canaux[0].catalogue_version'), 1);
  h.run("for(let id=0;id<16390;id++)resolveChannelEnum({id:String(id),pont:'absent',valeur:0,encodage:'enum',catalogue:7,catalogue_version:1})");
  assert.equal(h.state.enumPending.size, 16384);
});

test('app_id codé affiche Application et devient prioritaire sans changer le natif', () => {
  const h = harness(); h.context.t = key => ({ app_name: 'Application', app_id: 'Identifiant application' }[key] || key); const description = enumDescription(1, [{ code: 0, native: 'synthetic.application.id', label: 'Synthetic application', active: true }]);
  h.state.configs.set('synthetic', description);
  const app = { ...enumState(), semantique: 'app_id', nom: 'Synthetic application identifier' };
  h.context.c = app; assert.equal(h.run('channelLabel(c)'), 'Application');
  assert.equal(h.run('channelValue(c)'), 'Synthetic application');
  assert.equal(h.run('nativeChannelValue(c)'), 'synthetic.application.id');
  h.context.c = { semantique: 'app_id', valeur: 'legacy.synthetic.id', nom: 'Synthetic legacy identifier' };
  assert.equal(h.run('channelLabel(c)'), 'Identifiant application'); assert.equal(h.run('channelValue(c)'), 'legacy.synthetic.id');
  h.context.d = { id: 'fixture', pont: 'synthetic', nom: 'Synthetic media', canaux: [
    { semantique: 'media_title', nom: 'Synthetic title', valeur: 'Synthetic playback' },
    { semantique: 'temperature', nom: 'Synthetic temperature', valeur: 22 },
    { semantique: 'energie', nom: 'Synthetic energy', valeur: 10 }, app,
  ] };
  h.context.scopeChannels = d => d.canaux; h.context.icon = () => ''; h.context.countLabel = (count, unit) => `${count} ${unit}`;
  vm.runInContext(html.slice(html.indexOf('function deviceTile('), html.indexOf('function devicesFor(')), h.context);
  const tile = h.run('deviceTile(d)');
  assert.match(tile, /Application : Synthetic application/);
  assert.ok(tile.indexOf('Synthetic application') < tile.indexOf('Synthetic playback'));
  assert.doesNotMatch(tile, /synthetic\.application\.id/);
});
