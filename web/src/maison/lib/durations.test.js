import test from 'node:test';
import assert from 'node:assert/strict';
import {durationEditor,durationPayload} from './durations.js';
test('AUTO-K3 keeps typed state, source unit, freshness and window explicit',()=>{
  const value={source:'fixture:lamp/on',source_unit:null,value:true,window_s:3600,max_age_s:60};
  const form=durationEditor(value);
  assert.deepEqual(durationPayload(form),{type:'duration',value});
  assert.equal(typeof durationPayload({...form,value:'true'}).value.value,'string');
  for (const bad of [null,{},[],NaN,Infinity]) assert.equal(durationPayload({...form,value:bad}),null);
  for (const bad of [0,604801,1.5]) {
    assert.equal(durationPayload({...form,window_s:bad}),null);
    assert.equal(durationPayload({...form,max_age_s:bad}),null);
  }
});
