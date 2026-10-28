import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import test from 'node:test';
const html = readFileSync(new URL('../../docs/proposition/analyse/poc-ha-multimaison.html', import.meta.url), 'utf8');
const script = html.match(/<script>([\s\S]*?)<\/script>/)[1];
const context = { URLSearchParams, location: { pathname: '/ha/', search: '' }, navigator: { language: 'fr' }, TextEncoder, Intl, structuredClone, console };
runInNewContext(script.split('// Browser wiring')[0] + '\nglobalThis.logic = { MESSAGES, presets, parseDraft, selectedHouses, eligiblePromotion, setStatus, getDraft: () => draftText, setDraft: s => {draftText = s; dirty = true;}, getState: () => state };', context);
const ui = context.logic;
test('embedded catalogues exactly match the two shared JSON files', () => {
 for (const lang of ['fr', 'en']) assert.deepEqual(JSON.parse(JSON.stringify(ui.MESSAGES[lang])), JSON.parse(readFileSync(new URL(`../../locales/${lang}/ha-poc.json`, import.meta.url))));
 assert.deepEqual(Object.keys(ui.MESSAGES.fr), Object.keys(ui.MESSAGES.en));
});
test('cross replication gives each server one active and one passive home', () => {
 const p = ui.presets.cross;
 assert.equal(p.houses[0].primary, p.houses[1].standbys[0].node);
 assert.equal(p.houses[1].primary, p.houses[0].standbys[0].node);
 assert.equal(ui.selectedHouses(p.houses, 'all').length, 2);
 assert.equal(ui.selectedHouses(p.houses, 'house-b')[0].id, 'house-b');
});
test('polling preserves an edited configuration and rejects unsafe or oversized input', () => {
 const draft = JSON.stringify(ui.presets.local);
 ui.setDraft(draft);
 ui.setStatus({ topology: ui.presets.cross, configured: true });
 assert.equal(ui.getDraft(), draft);
 assert.throws(() => ui.parseDraft(' '.repeat(16385)));
 assert.throws(() => ui.parseDraft('{'));
 assert.throws(() => ui.parseDraft(JSON.stringify({...ui.presets.cross, password: 'no'})));
 const duplicate = JSON.parse(JSON.stringify(ui.presets.cross)); duplicate.nodes[1].id = duplicate.nodes[0].id;
 assert.throws(() => ui.parseDraft(JSON.stringify(duplicate)));
});
test('offline, isolated or outdated replicas never offer promotion', () => {
 const copy = {node:'node-b', role:'passive', in_sync:true};
 assert.equal(ui.eligiblePromotion(copy, {online:false, isolated:false}), false);
 assert.equal(ui.eligiblePromotion(copy, {online:true, isolated:true}), false);
 assert.equal(ui.eligiblePromotion({...copy,in_sync:false}, {online:true}), false);
 assert.equal(ui.eligiblePromotion(copy, {online:true,isolated:false}), true);
 assert.equal(ui.eligiblePromotion(copy, {online:true,isolated:false}, {lease_valid:true}), false);
 assert.equal(ui.eligiblePromotion(copy, {online:true,isolated:false}, {lease_valid:false}), true);
});
function browser(pathname, fail = false) {
 const elements=new Map(), requests=[], timers=[];
 const element=id=>{if(!elements.has(id))elements.set(id,{value:'',textContent:'',innerHTML:'',disabled:false,checked:false,dataset:{},classList:{toggle(){}},listeners:{},addEventListener(event,fn){this.listeners[event]=fn;}});return elements.get(id);};
 const sandbox={URLSearchParams,TextEncoder,Intl,AbortSignal,location:{pathname,search:''},navigator:{language:'fr'},document:{documentElement:{},getElementById:element,querySelectorAll:()=>[],addEventListener(){}},setTimeout:fn=>timers.push(fn),fetch:async(url,options)=>{requests.push({url,options});if(fail)throw Error('network');return{ok:true,json:async()=>url==='/api/socket-ticket'?{ticket:'ephemeral-test-ticket'}:{configured:false,topology:JSON.parse(JSON.stringify(ui.presets.cross)),nodes:[],houses:[]}};}};
 runInNewContext(script,sandbox);
 return{elements,requests,timers,sandbox};
}
const settle=()=>new Promise(resolve=>setImmediate(resolve));
test('boot and failed polls never issue or queue a simulation action', async()=>{
 const b=browser('/ha/',true);await settle();
 assert.equal(b.requests.length,1);assert.equal(b.requests[0].url,'/ha/api/status');
 assert.equal(b.requests[0].options.method,'GET');
 b.timers.shift()();await settle();
 assert.ok(b.requests.every(r=>r.options.method==='GET'));
 const layout=b.elements.get('layout');layout.value=JSON.stringify(ui.presets.cloud);layout.listeners.input({target:layout});
 b.timers.shift()();await settle();
 assert.equal(layout.value,JSON.stringify(ui.presets.cloud));
});
test('gateway mutations acquire a fresh ticket, while direct service mutations do not',async()=>{
 for(const pathname of ['/ha/','/']){
  const b=browser(pathname);await settle();
  await b.elements.get('apply').listeners.click();await settle();
  const post=b.requests.find(r=>r.options.method==='POST');assert.ok(post);
  assert.equal(post.url,pathname==='/ha/'?'/ha/api/configure':'/api/configure');
  assert.equal(JSON.parse(post.options.body).topology.houses.length,2);
  assert.equal(post.options.credentials,'same-origin');
  assert.equal(post.options.headers['x-moli-ticket'],pathname==='/ha/'?'ephemeral-test-ticket':undefined);
  assert.equal(b.requests.some(r=>r.url==='/api/socket-ticket'),pathname==='/ha/');
 }
});
test('a successful apply clears the draft marker without discarding edits made during the request',async()=>{
 const b=browser('/');await settle();
 await b.elements.get('apply').listeners.click();
 assert.equal(b.elements.get('draft-status').textContent,'');
 let complete;
 b.sandbox.fetch=()=>new Promise(resolve=>{complete=resolve;});
 const pending=b.elements.get('apply').listeners.click();
 const layout=b.elements.get('layout'),edited=JSON.stringify(ui.presets['local-cloud']);
 layout.value=edited;layout.listeners.input({target:layout});
 complete({ok:true,json:async()=>({configured:true,topology:ui.presets.cross,nodes:[],houses:[]})});
 await pending;
 assert.equal(layout.value,edited);
 assert.notEqual(b.elements.get('draft-status').textContent,'');
});
