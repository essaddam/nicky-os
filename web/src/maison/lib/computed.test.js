import test from 'node:test';
import assert from 'node:assert/strict';
import { computedEditor, computedPayload } from './computed.js';
import { dependencyRequest, previewMatchesShown } from '../../lib/dependency.js';

test('AUTO-K1 editor keeps operation, sources, unit and integer freshness explicit', () => {
  const value={operation:'mean',sources:['fixture:a/value'],unit:'W',max_age_s:60};
  assert.deepEqual(computedPayload(computedEditor(value)),{type:'aggregate',value});
  assert.equal(computedPayload({...computedEditor(value),sources:['x/value','x/value']}),null);
  for (const age of [0,1.5,604801,NaN]) assert.equal(computedPayload({...computedEditor(value),max_age_s:age}),null);
  assert.equal(computedPayload({...computedEditor(value),sources:Array(33).fill('x/value')}),null);
  assert.deepEqual(computedPayload({...computedEditor(value),sources:[]}).value.sources,[]);
});
test('AUTO-K1 transport stages definitions and echoes only the exact shown preview', async (context) => {
  const calls=[];
  context.mock.method(globalThis,'fetch',async (url,options)=>{calls.push({url,options});return {ok:true,json:async()=>({ok:true})};});
  const translate=(key)=>key;
  await dependencyRequest('computed','helpers:average','PUT','',{expected:1,definition:{type:'aggregate',value:{}}},translate);
  assert.equal(calls[0].url,'/api/dependencies/computed/helpers%3Aaverage');
  assert.equal(calls[0].options.headers['x-moli-origin'],'ui');
  assert.equal(JSON.parse(calls[0].options.body).human,undefined);
  const view={reference:{kind:'computed',id:'helpers:average'},versions:{draft:{version:1},current:null}};
  assert.equal(previewMatchesShown(view,{reference:view.reference,versions:{draft:1,current:null}}),true);
  assert.equal(previewMatchesShown(view,{reference:view.reference,versions:{draft:2,current:null}}),false);
  assert.equal(previewMatchesShown(null,null),false);
});
