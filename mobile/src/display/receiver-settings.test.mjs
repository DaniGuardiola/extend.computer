import assert from "node:assert/strict";
import { test } from "node:test";
import { appliedDraft, chooseSize, matchingCompletion, observedRequest, settingsFromConfig, sizeOptions, sizeValue } from "./receiver-settings.ts";

const config = { mode:"extend",width:1138,height:710,fps:90,bitrate:12_000_000,retina:false,auto_size:false,codec:"hevc",separate_cursor:true };
const status = { id:"session",supported:true,closed:false,available:true,revision:1,config,pending:null,
  applied:{revision:1,config},result:{id:3,revision:1,outcome:"applied",error:null} };
test("custom size, frame rate and quality remain exact until edited", () => {
  const draft = settingsFromConfig(config);
  assert.deepEqual(draft, {width:1138,height:710,auto_size:false,fps:90,bitrate:12_000_000});
  assert.equal(sizeValue(draft),"1138x710");
  assert.equal(sizeOptions(config,draft).filter(option => option.value === "1138x710").length,1);
  assert.deepEqual(chooseSize(draft,"fit"),{...draft,auto_size:true});
  assert.deepEqual(chooseSize(draft,"1920x1200"),{...draft,width:1920,height:1200});
  assert.equal("codec" in draft,false); assert.equal("retina" in draft,false); assert.equal("mode" in draft,false);
});
test("retina presets obey encoded extent rather than logical extent", () => {
  const options=sizeOptions({...config,retina:true},settingsFromConfig(config));
  assert.equal(options.some(option => option.value === "2560x1600"),false);
  assert.equal(options.some(option => option.value === "1920x1200"),true);
});
test("only this request's matching applied revision may replace its draft", () => {
  assert.deepEqual(appliedDraft(status,3),settingsFromConfig(config));
  assert.equal(appliedDraft(status,2),null); assert.equal(appliedDraft(status,null),null);
  assert.equal(appliedDraft({...status,applied:{revision:0,config}},3),null);
  for(const outcome of ["rejected","stale","failed","closed","timed_out"]) {
    const rejected={...status,result:{...status.result,outcome}};
    assert.equal(matchingCompletion(rejected,3).outcome,outcome);
    assert.equal(appliedDraft(rejected,3),null);
  }
});
test("reopening during native apply follows that pending request to its final draft", () => {
  const pending={...status,pending:3,result:null,applied:{revision:0,config:{...config,fps:60}}};
  const request=observedRequest(pending,null);
  assert.equal(request,3); assert.equal(appliedDraft(pending,request),null);
  assert.deepEqual(appliedDraft(status,request),settingsFromConfig(config));
  assert.equal(observedRequest({...pending,pending:8},3),3);
  assert.equal(observedRequest({...status,pending:null},null),null);
});
