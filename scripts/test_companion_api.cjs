// Asset-free contract tests. Real Cubism validation is a separate local probe.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const api = require('../live2d/companion.js');
const profile = JSON.parse(fs.readFileSync(path.join(__dirname,'../examples/companion-custom.json')));
const copy = () => structuredClone(profile);
function fixture(p = copy(), omit = []) {
  const ids=Object.values(p.bindings).filter(id => !omit.includes(id));
  const raw={ids, minimumValues:ids.map(()=>-30), maximumValues:ids.map(()=>30), defaultValues:ids.map(()=>0)};
  for (let i=0;i<ids.length;i++) if (/Open|Smile|Breath|Heart|Teers/.test(ids[i])) {raw.minimumValues[i]=0;raw.maximumValues[i]=1;}
  const values={};
  return {engine:api.create({getModel:()=>({parameters:raw}),setParameterValueByIndex:(i,v)=>{values[ids[i]]=v;}},p),values};
}
test('profiles validate and remain detached from caller mutations',()=>{
  const p=copy(), valid=api.validateProfile(p);p.emotions.focused.eye_y=99;
  assert.equal(valid.emotions.focused.eye_y,-.2);
  const {engine}=fixture();const description=engine.describe();description.motions.push('fake');assert(!engine.describe().motions.includes('fake'));
});
test('reject invalid paths, extra code, unknown channels and malformed keyframes',()=>{
  for(const edit of [p=>p.model='../a.model3.json',p=>p.javascript='run()',p=>p.emotions.happy.unknown=1,p=>p.motions.nod.tracks.head_y[1][0]=0,p=>p.bindings.head_x=p.bindings.head_y,p=>p.layout.zoom=Infinity]){const p=copy();edit(p);assert.throws(()=>api.validateProfile(p));}
});
test('emotion strengths, neutral reset, gaze and task state are independent',()=>{
  const {engine,values}=fixture();engine.state('checking');engine.emotion('focused',.5);engine.apply(125);
  assert.equal(values.ParamEyeSmile,.075);assert.equal(engine.snapshot().state,'checking');
  engine.look(.4,.2,1000);engine.apply(250);assert(Math.abs(values.ParamEyeBallX-.4)<1e-9);
  engine.reset();engine.apply(125);assert.equal(engine.snapshot().emotion,'neutral');assert.equal(engine.snapshot().state,'checking');assert.equal(values.ParamEyeBallX,-.3);
});
test('keyframe interpolation, strength, additive blending and expiry',()=>{
  const p=copy();p.motions.test={duration_ms:1000,blend:'add',tracks:{head_z:[[0,0],[.5,10],[1,0]]}};
  const {engine,values}=fixture(p);engine.motion('test',.5);engine.apply(250);assert.equal(values.ParamAngleZ,2.5);
  engine.apply(250);assert.equal(values.ParamAngleZ,5);engine.apply(250);engine.apply(250);assert.equal(values.ParamAngleZ,0);assert.equal(engine.snapshot().motion,null);
});
test('clamp to real rig bounds, unsupported channels reported without fabricated success',()=>{
  const p=copy();p.emotions.max={smile_l:99};const {engine,values}=fixture(p);engine.emotion('max');engine.apply(125);assert.equal(values.ParamEyeSmile,1);
  const missing=fixture(p,['ParamEyeSmile']);assert(missing.engine.describe().warnings.some(w=>w.includes('smile_l')));assert.throws(()=>missing.engine.emotion('max'));
});
test('invalid commands never change state; reset removes motion and gaze',()=>{
  const {engine}=fixture();engine.emotion('happy');const before=engine.snapshot();
  for(const command of [{type:'emotion',name:'missing',strength:1},{type:'motion',name:'nod',strength:NaN},{type:'look',x:4,y:0,duration_ms:1000},{type:'reset',code:'bad'}])assert.throws(()=>engine.dispatch(command));
  assert.deepEqual(engine.snapshot(),before);engine.dispatch({type:'motion',name:'nod',strength:.5});engine.look(.1,0);engine.dispatch({type:'reset'});assert.equal(engine.snapshot().motion,null);assert.equal(engine.snapshot().look,null);
});
test('finite deltas and known work states only; custom motion does not fabricate check results',()=>{
  const {engine}=fixture();assert.throws(()=>engine.state('verified'));assert.throws(()=>engine.apply(Infinity));
  engine.state('concerned');engine.motion('nod');engine.apply(125);assert.equal(engine.snapshot().state,'concerned');
});
