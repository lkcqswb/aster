/* Aster Companion API v1. Original adapter code; contains no model or SDK assets.
 * Browser: AsterCompanion.create(coreModel, profile). Node: require this file.
 * Profiles are data. No eval, callbacks, resource URLs or expression execution.
 */
(function (root) {
  'use strict';
  const STATES = ['idle','listening','thinking','reading','working','checking','waiting','speaking','pleased','concerned'];
  const clone = value => JSON.parse(JSON.stringify(value));
  const own = (object, key) => Object.prototype.hasOwnProperty.call(object, key);
  const record = value => value !== null && typeof value === 'object' && !Array.isArray(value);
  const name = value => typeof value === 'string' && /^[A-Za-z0-9_-]{1,64}$/.test(value);
  const finite = (value, min, max) => typeof value === 'number' && Number.isFinite(value) && value >= min && value <= max;
  const fail = message => { throw new Error(message); };
  function keys(value, allowed, required = allowed) {
    if (!record(value) || Object.keys(value).some(k => !allowed.includes(k)) || required.some(k => !own(value,k))) fail('Unknown or missing fields');
  }
  function validateProfile(p) {
    if (new TextEncoder().encode(JSON.stringify(p)).length > 128000) fail('Profile exceeds 128 KB');
    keys(p,['version','name','model','layout','bindings','emotions','motions']);
    if (p.version !== 1 || typeof p.name !== 'string' || !p.name || new TextEncoder().encode(p.name).length > 120 || /[\x00-\x1f\x7f]/.test(p.name)) fail('Invalid profile version or name');
    if (typeof p.model !== 'string' || new TextEncoder().encode(p.model).length > 512 || !p.model.endsWith('.model3.json') || /[\\:?#%\x00-\x1f\x7f]/.test(p.model) || p.model.split('/').some(s => !s || s === '.' || s === '..')) fail('Use a relative local model path');
    keys(p.layout,['zoom','center','anchor']);
    if (!finite(p.layout.zoom,.1,5) || ['center','anchor'].some(k => !Array.isArray(p.layout[k]) || p.layout[k].length !== 2 || p.layout[k].some(n => !finite(n,0,1)))) fail('Invalid layout');
    if (!record(p.bindings) || !Object.keys(p.bindings).length || Object.keys(p.bindings).length > 128 || Object.entries(p.bindings).some(([k,v]) => !name(k) || !name(v)) || new Set(Object.values(p.bindings)).size !== Object.keys(p.bindings).length) fail('Invalid or duplicate parameter bindings');
    if (!record(p.emotions) || !own(p.emotions,'neutral') || Object.keys(p.emotions).length > 32 || !record(p.motions) || Object.keys(p.motions).length > 32) fail('Define neutral and at most 32 emotions/motions');
    for (const [n, values] of Object.entries(p.emotions)) {
      if (!name(n) || !record(values) || Object.entries(values).some(([k,v]) => !own(p.bindings,k) || !finite(v,-10000,10000))) fail('Invalid emotion: '+n);
    }
    for (const [n, clip] of Object.entries(p.motions)) {
      keys(clip,['duration_ms','blend','tracks']);
      if (!name(n) || !Number.isInteger(clip.duration_ms) || !finite(clip.duration_ms,100,10000) || !['add','set'].includes(clip.blend) || !record(clip.tracks) || !Object.keys(clip.tracks).length || Object.keys(clip.tracks).length > 128) fail('Invalid motion: '+n);
      for (const [channel, frames] of Object.entries(clip.tracks)) {
        if (!own(p.bindings,channel) || !Array.isArray(frames) || frames.length < 2 || frames.length > 64 || frames.some((f,i) => !Array.isArray(f) || f.length !== 2 || !finite(f[0],0,1) || !finite(f[1],-10000,10000) || (i > 0 && frames[i-1][0] >= f[0])) || frames[0][0] !== 0 || frames.at(-1)[0] !== 1) fail('Invalid keyframe track: '+n+'/'+channel);
      }
    }
    return clone(p);
  }
  function create(core, source) {
    const p = validateProfile(source);
    const raw = core.getModel().parameters;
    const parameters = Array.from(raw.ids, (id,i) => ({id:String(id), min:raw.minimumValues[i], max:raw.maximumValues[i], default:raw.defaultValues[i], index:i}));
    if (parameters.length > 4096 || parameters.some(v => !finite(v.min,-1e10,1e10) || !finite(v.max,v.min,1e10) || !finite(v.default,v.min,v.max))) fail('Invalid model parameter metadata');
    const byId = new Map(parameters.map(v => [v.id,v]));
    const bindings = new Map(Object.entries(p.bindings).filter(([,id]) => byId.has(id)));
    const warnings = Object.entries(p.bindings).filter(([,id]) => !byId.has(id)).map(([channel,id]) => 'Unsupported binding '+channel+': '+id);
    let time = 0, state = 'idle', emotion = 'neutral', intensity = 1, clip = null, gaze = null, last = {};
    const supported = channels => channels.filter(channel => bindings.has(channel));
    const description = () => ({version:1, profile:p.name, model:p.model, states:STATES.slice(), emotions:Object.keys(p.emotions), motions:Object.keys(p.motions), bindings:clone(p.bindings), parameters:parameters.map(({index,...v}) => v), warnings:warnings.slice()});
    function setState(value) { if (!STATES.includes(value)) fail('Unknown work state: '+value); state = value; }
    function strength(value) { if (!finite(value,0,1)) fail('Strength must be 0–1'); return value; }
    function setEmotion(value, weight = 1) {
      if (!own(p.emotions,value)) fail('Unknown emotion: '+value);
      strength(weight);
      const channels = supported(Object.keys(p.emotions[value]));
      if (Object.keys(p.emotions[value]).length && !channels.length) fail('Emotion has no supported model parameters');
      emotion = value; intensity = weight;
      return {emotion:value, strength:weight, supported_channels:channels};
    }
    function play(value, weight = 1) {
      if (!own(p.motions,value)) fail('Unknown motion: '+value);
      strength(weight);
      const channels = supported(Object.keys(p.motions[value].tracks));
      if (!channels.length) fail('Motion has no supported model parameters');
      clip = {name:value, start:time, strength:weight};
      return {motion:value, duration_ms:p.motions[value].duration_ms, supported_channels:channels};
    }
    function look(x,y,duration_ms = 2000) {
      if (!finite(x,-1,1) || !finite(y,-1,1) || !Number.isInteger(duration_ms) || !finite(duration_ms,100,10000)) fail('Invalid gaze coordinates or duration');
      const channels = supported(['eye_x','eye_y','head_x','head_y']);
      if (!channels.length) fail('Gaze has no supported model parameters');
      gaze = {x,y,start:time,end:time+duration_ms/1000};
      return {look:[x,y], duration_ms, supported_channels:channels};
    }
    function reset() { emotion='neutral'; intensity=1; clip=null; gaze=null; return {reset:true}; }
    function dispatch(command) {
      if (!record(command)) fail('Expected a command object');
      switch (command.type) {
        case 'emotion': keys(command,['type','name','strength']); return setEmotion(command.name,command.strength);
        case 'motion': keys(command,['type','name','strength']); return play(command.name,command.strength);
        case 'look': keys(command,['type','x','y','duration_ms']); return look(command.x,command.y,command.duration_ms);
        case 'reset': keys(command,['type']); return reset();
        default: fail('Unknown companion command');
      }
    }
    function interpolate(frames, at) {
      const end = frames.findIndex(f => f[0] >= at);
      if (end <= 0) return frames[0][1];
      const a=frames[end-1], b=frames[end], ratio=(at-a[0])/(b[0]-a[0]);
      return a[1]+(b[1]-a[1])*ratio;
    }
    function apply(delta_ms = 100) {
      if (!finite(delta_ms,0,250)) fail('Frame delta must be 0–250 ms');
      time += delta_ms/1000;
      const values = Object.fromEntries(Array.from(bindings, ([channel,id]) => [channel, byId.get(id).default]));
      const put = (key,value) => { if (bindings.has(key)) values[key]=value; };
      const blinkPhase=time%4.7, blink=blinkPhase>4.48?Math.abs((blinkPhase-4.59)/.11):1;
      const thinking=['thinking','reading','checking'].includes(state), attentive=['listening','waiting'].includes(state), sway=Math.sin(time*.8)*.8;
      put('head_x',sway+(thinking?-6:0)+(state==='working'?-3:0));
      put('head_y',Math.sin(time*.6)*.6+(attentive?2:0)); put('head_z',thinking?-2:0); put('body_x',sway*.3);
      put('eye_x',thinking?-.3:0); put('eye_y',state==='reading'?-.18:(thinking?.12:0));
      put('eye_l',Math.max(0,Math.min(state==='concerned'?.7:1,blink))); put('eye_r',Math.max(0,Math.min(state==='concerned'?.7:1,blink)));
      put('breath',.5+Math.sin(time*1.3)*.5); put('mouth_open',state==='speaking'?Math.max(0,Math.sin(time*15)*.65+.25):0);
      put('smile_l',state==='pleased'?.45:0); put('smile_r',state==='pleased'?.45:0);
      for (const [channel,value] of Object.entries(p.emotions.neutral)) put(channel,value);
      for (const [channel,value] of Object.entries(p.emotions[emotion])) if (bindings.has(channel)) put(channel,values[channel]+(value-values[channel])*intensity);
      if (gaze && time >= gaze.end) gaze=null;
      if (gaze) {
        const blend=Math.min(1,(time-gaze.start)/.15,(gaze.end-time)/.15);
        for (const [channel,value] of Object.entries({eye_x:gaze.x,eye_y:gaze.y,head_x:gaze.x*20,head_y:gaze.y*10})) if(bindings.has(channel)) put(channel,values[channel]+(value-values[channel])*blend);
      }
      if (clip) {
        const definition=p.motions[clip.name], elapsed=time-clip.start, duration=definition.duration_ms/1000;
        if (elapsed >= duration) clip=null;
        else {
          const weight=clip.strength*Math.min(1,elapsed/.1,(duration-elapsed)/.1);
          for (const [channel,frames] of Object.entries(definition.tracks)) if (bindings.has(channel)) {
            const value=interpolate(frames,elapsed/duration);
            values[channel] = definition.blend==='add' ? values[channel]+value*weight : values[channel]+(value-values[channel])*weight;
          }
        }
      }
      last={};
      for (const [channel,id] of bindings) {
        const meta=byId.get(id), value=Math.max(meta.min,Math.min(meta.max,values[channel]));
        core.setParameterValueByIndex(meta.index,value); last[id]=value;
      }
      return clone(last);
    }
    const snapshot = () => ({version:1,state,emotion,strength:intensity,motion:clip?.name||null,look:gaze?[gaze.x,gaze.y]:null,time,parameters:clone(last)});
    return Object.freeze({describe:description,state:setState,emotion:setEmotion,motion:play,look,reset,dispatch,apply,snapshot});
  }
  const api=Object.freeze({version:1,validateProfile,create});
  if (typeof module !== 'undefined' && module.exports) module.exports=api;
  else root.AsterCompanion=api;
})(typeof globalThis !== 'undefined' ? globalThis : this);
