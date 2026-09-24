/* Test double for Aster's Live2D renderer check (src/live2d.rs, `stub_renderer_*` tests).
 *
 * This is NOT Pixi, Cubism Core or pixi-live2d-display and contains none of their code. It imitates
 * only the API surface live2d/renderer.html and live2d/companion.js use, with the same update order
 * as pixi-live2d-display's Cubism 4 internal model (motion -> save -> expression -> SDK blink/breath
 * -> model update -> load). Each frame is drawn with WebGL scissor clears: a crude face plus a strip
 * of 4x4 blocks in the top-left corner that encodes the rendered parameter values exactly (R,G =
 * 16-bit value, B = 90), so a test can read them back from lossless PNG frames.
 */
(() => {
  'use strict';
  // id, min, max, default: every parameter the default profile (live2d/profiles/nongyu.json) binds,
  // then two it does not bind, which must stay at rest.
  const PARAMS = [
    ['ParamAngleX', -30, 30, 0], ['ParamAngleY', -30, 30, 0], ['ParamAngleZ', -30, 30, 0],
    ['ParamBodyAngleX', -10, 10, 0], ['ParamEyeBallX', -1, 1, 0], ['ParamEyeBallY', -1, 1, 0],
    ['ParamEyeLOpen', 0, 1, 1], ['ParamEyeROpen', 0, 1, 1], ['ParamBreath', 0, 1, 0],
    ['ParamMouthOpenY', 0, 1, 0], ['ParamEyeSmile', 0, 1, 0], ['ParamEyeSmileR', 0, 1, 0],
    ['ParamHeartEye', 0, 1, 0], ['ParamTeers3', 0, 1, 0], ['ParamTeers5', 0, 1, 0], ['ParamTeers2', 0, 1, 0],
    ['ParamMouthForm', -1, 1, 0], ['ParamHairFront', -1, 1, 0],
  ];
  const RAW = { x: -200, y: -100, width: 800, height: 2000 }; // a tall full-body rig in model units
  const stats = { motionsStarted: 0, autoIdleRequests: 0, expressionsApplied: 0, frames: 0 };

  class Emitter {
    constructor() { this.handlers = {}; }
    on(name, fn) { (this.handlers[name] = this.handlers[name] || []).push(fn); return this; }
    emit(name, ...args) { for (const fn of this.handlers[name] || []) fn(...args); }
  }

  class Core {
    constructor() {
      this.ids = PARAMS.map(p => p[0]);
      this.min = PARAMS.map(p => p[1]);
      this.max = PARAMS.map(p => p[2]);
      this.def = PARAMS.map(p => p[3]);
      this.values = this.def.slice();
      this.saved = this.values.slice();
      this.rendered = this.values.slice();
      this.unknown = new Map();
    }
    // The shape of Cubism Core's model.parameters.
    getModel() {
      return { parameters: { ids: this.ids.slice(), count: this.ids.length, minimumValues: this.min.slice(),
        maximumValues: this.max.slice(), defaultValues: this.def.slice(), values: this.values.slice() } };
    }
    getParameterCount() { return this.ids.length; }
    // Like the Cubism framework: an unknown id still receives an index, past the real parameters.
    getParameterIndex(id) {
      const i = this.ids.indexOf(id);
      if (i >= 0) return i;
      if (!this.unknown.has(id)) this.unknown.set(id, this.ids.length + this.unknown.size);
      return this.unknown.get(id);
    }
    getParameterMinimumValue(i) { return this.min[i]; }
    getParameterMaximumValue(i) { return this.max[i]; }
    getParameterDefaultValue(i) { return this.def[i]; }
    getParameterValueByIndex(i) { return i < this.values.length ? this.values[i] : 0; }
    setParameterValueByIndex(i, v, w = 1) {
      if (i >= this.values.length) return;
      v = Math.min(this.max[i], Math.max(this.min[i], v));
      this.values[i] = w === 1 ? v : this.values[i] * (1 - w) + v * w;
    }
    addParameterValueByIndex(i, v, w = 1) { this.setParameterValueByIndex(i, this.getParameterValueByIndex(i) + v * w); }
    getParameterValueById(id) { return this.getParameterValueByIndex(this.getParameterIndex(id)); }
    setParameterValueById(id, v, w) { this.setParameterValueByIndex(this.getParameterIndex(id), v, w); }
    addParameterValueById(id, v, w) { this.addParameterValueByIndex(this.getParameterIndex(id), v, w); }
    saveParameters() { this.saved = this.values.slice(); }
    loadParameters() { this.values = this.saved.slice(); }
    update() { this.rendered = this.values.slice(); }
  }

  const fetchJson = url => fetch(url).then(r => { if (!r.ok) throw Error(`stub fetch ${r.status} ${url}`); return r.json(); });

  class ExpressionManager {
    constructor(model) { this.model = model; this.definitions = model.settings.FileReferences.Expressions || []; this.current = null; }
    async setExpression(id) {
      const index = typeof id === 'number' ? id : this.definitions.findIndex(d => d.Name === id);
      const d = this.definitions[index];
      if (!d) return false;
      this.current = await fetchJson(this.model.base + d.File);
      stats.expressionsApplied++;
      return true;
    }
    resetExpression() { this.current = null; }
    update(core) {
      for (const p of (this.current && this.current.Parameters) || []) {
        const i = core.getParameterIndex(p.Id);
        if (p.Blend === 'Multiply') core.setParameterValueByIndex(i, core.getParameterValueByIndex(i) * p.Value);
        else if (p.Blend === 'Overwrite') core.setParameterValueByIndex(i, p.Value);
        else core.addParameterValueByIndex(i, p.Value);
      }
    }
  }

  class MotionManager extends Emitter {
    constructor(model) {
      super();
      this.model = model;
      this.definitions = model.settings.FileReferences.Motions || {};
      this.groups = { idle: 'Idle' };
      this.playing = false;
      this.current = null;
      this.requesting = false;
      this.expressionManager = new ExpressionManager(model);
    }
    async startMotion(group, index, priority = 2) {
      const defs = this.definitions[group];
      if (!defs || !defs.length) return false;
      if (this.current && priority <= this.current.priority) return false;
      const i = index === undefined ? Math.floor(Math.random() * defs.length) : index;
      const json = await fetchJson(this.model.base + defs[i].File);
      this.current = { priority, start: null, duration: (json.Meta && json.Meta.Duration) || 1, amplitude: json.StubAmplitude || 8 };
      this.playing = true;
      stats.motionsStarted++;
      return true;
    }
    startRandomMotion(group, priority) { return this.startMotion(group, undefined, priority); }
    update(core, now) {
      if (!this.current) {
        this.playing = false;
        // pixi-live2d-display requests an idle motion whenever nothing plays.
        const idle = this.definitions[this.groups.idle];
        if (idle && idle.length && !this.requesting) {
          this.requesting = true;
          stats.autoIdleRequests++;
          this.startRandomMotion(this.groups.idle, 1).finally(() => { this.requesting = false; });
        }
        return false;
      }
      const m = this.current;
      if (m.start === null) m.start = now;
      const p = (now - m.start) / m.duration;
      if (p >= 1) { this.current = null; this.playing = false; return false; }
      const fade = Math.min(1, p / 0.2, (1 - p) / 0.2);
      const i = core.getParameterIndex('ParamAngleX');
      const source = core.getParameterValueByIndex(i);
      core.setParameterValueByIndex(i, source + (m.amplitude * Math.sin(Math.PI * p) - source) * fade);
      return true;
    }
  }

  class InternalModel extends Emitter {
    constructor(model) {
      super();
      this.coreModel = new Core();
      this.motionManager = new MotionManager(model);
      // The SDK's own layers. If the renderer failed to replace them they would show up as a hard
      // eye close every second and a fast 10 degree head wobble.
      this.eyeBlink = { updateParameters: (core, dt) => { this.blinkClock = ((this.blinkClock || 0) + dt) % 1; if (this.blinkClock < 0.1) { core.setParameterValueById('ParamEyeLOpen', 0); core.setParameterValueById('ParamEyeROpen', 0); } } };
      this.breath = { updateParameters: (core, dt) => { this.breathClock = (this.breathClock || 0) + dt; core.addParameterValueById('ParamAngleX', 10 * Math.sin(this.breathClock * 5)); } };
    }
    update(dt, now) {
      dt /= 1000; now /= 1000;
      const core = this.coreModel;
      this.emit('beforeMotionUpdate');
      const moved = this.motionManager.update(core, now);
      this.emit('afterMotionUpdate');
      core.saveParameters();
      this.motionManager.expressionManager.update(core, now);
      if (!moved && this.eyeBlink) this.eyeBlink.updateParameters(core, dt);
      if (this.breath) this.breath.updateParameters(core, dt);
      this.emit('beforeModelUpdate');
      core.update();
      core.loadParameters();
    }
  }

  class Live2DModel extends Emitter {
    static fromSync(url, options = {}) { const model = new Live2DModel(); model.load(url, options); return model; }
    constructor() {
      super();
      this.x = 0; this.y = 0;
      this.scale = { x: 1, y: 1, set: (x, y = x) => { this.scale.x = x; this.scale.y = y; } };
      this.position = { set: (x, y) => { this.x = x; this.y = y; } };
      this.textures = []; this.deltaTime = 0; this.elapsedTime = 0; this.internalModel = undefined;
    }
    async load(url, options) {
      try {
        this.settings = await fetchJson(url);
        this.base = url.slice(0, url.lastIndexOf('/') + 1);
        const moc = await fetch(this.base + this.settings.FileReferences.Moc);
        if (!moc.ok) throw Error('stub moc missing');
        this.textures = await Promise.all(this.settings.FileReferences.Textures.map(t => PIXI.Texture.fromURL(this.base + t)));
        this.internalModel = new InternalModel(this);
        this.emit('modelLoaded');
        if (options.onLoad) options.onLoad();
      } catch (e) {
        if (options.onError) options.onError(e); else throw e;
      }
    }
    getBounds() {
      const s = this.scale.x;
      return { x: this.x + RAW.x * s, y: this.y + RAW.y * s, width: RAW.width * s, height: RAW.height * s };
    }
    update(dt) { this.deltaTime += dt; this.elapsedTime += dt; }
    motion(group, index, priority) { return this.internalModel.motionManager.startMotion(group, index, priority); }
    expression(id) { return this.internalModel.motionManager.expressionManager.setExpression(id); }
    renderInto(app) {
      this.internalModel.update(this.deltaTime, this.elapsedTime);
      this.deltaTime = 0;
      stats.frames++;
      const core = this.internalModel.coreModel;
      const v = id => core.rendered[core.ids.indexOf(id)];
      const s = this.scale.x;
      const faceX = this.x + (RAW.x + RAW.width / 2) * s + v('ParamAngleX') * s * 3;
      const faceY = this.y + (RAW.y + RAW.height * 0.19) * s - v('ParamAngleY') * s * 3;
      const r = RAW.width * 0.22 * s;
      app.rect(faceX - r * 1.3, faceY + r, r * 2.6, app.view.height, [70, 64, 92]);
      app.rect(faceX - r, faceY - r, r * 2, r * 2.2, [236, 214, 200]);
      for (const [side, id] of [[-1, 'ParamEyeLOpen'], [1, 'ParamEyeROpen']]) {
        const ex = faceX + side * r * 0.45 + v('ParamEyeBallX') * r * 0.12;
        const ey = faceY - r * 0.1 - v('ParamEyeBallY') * r * 0.1;
        const h = Math.max(1, r * 0.3 * v(id));
        app.rect(ex - r * 0.14, ey - h / 2, r * 0.28, h, [40, 60, 110]);
      }
      const mouth = Math.max(1, r * 0.35 * v('ParamMouthOpenY'));
      app.rect(faceX - r * 0.2, faceY + r * 0.55, r * 0.4, mouth, [150, 40, 60]);
      PARAMS.forEach(([id, min, max], k) => {
        const n = Math.round((v(id) - min) / (max - min) * 65535);
        app.rect(k * 4, 0, 4, 4, [n >> 8, n & 255, 90]);
      });
      [stats.motionsStarted, stats.autoIdleRequests, stats.expressionsApplied, stats.frames].forEach((n, k) => {
        n &= 65535;
        app.rect((PARAMS.length + k) * 4, 0, 4, 4, [n >> 8, n & 255, 91]);
      });
    }
  }

  class Application {
    constructor(options) {
      this.view = options.view;
      this.view.width = options.width; this.view.height = options.height;
      this.gl = this.view.getContext('webgl', { preserveDrawingBuffer: !!options.preserveDrawingBuffer, antialias: false, alpha: false });
      if (!this.gl) throw Error('stub: WebGL unavailable');
      const c = options.backgroundColor || 0;
      this.background = [(c >> 16) & 255, (c >> 8) & 255, c & 255];
      this.stage = { children: [], addChild(child) { this.children.push(child); return child; } };
      const app = this;
      this.renderer = {
        resize(w, h) { app.view.width = w; app.view.height = h; },
        render(stage) { app.draw(stage); },
      };
    }
    rect(x, y, w, h, rgb) {
      const gl = this.gl;
      x = Math.round(x); y = Math.round(y); w = Math.round(w); h = Math.round(h);
      if (w <= 0 || h <= 0) return;
      gl.scissor(x, this.view.height - y - h, w, h);
      gl.clearColor(rgb[0] / 255, rgb[1] / 255, rgb[2] / 255, 1);
      gl.clear(gl.COLOR_BUFFER_BIT);
    }
    draw(stage) {
      const gl = this.gl;
      gl.viewport(0, 0, this.view.width, this.view.height);
      gl.disable(gl.SCISSOR_TEST);
      gl.clearColor(this.background[0] / 255, this.background[1] / 255, this.background[2] / 255, 1);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.enable(gl.SCISSOR_TEST);
      for (const child of stage.children) child.renderInto(this);
      gl.disable(gl.SCISSOR_TEST);
    }
  }

  const Texture = {
    fromURL(url) {
      return new Promise((resolve, reject) => {
        const img = new Image();
        img.onload = () => resolve(Texture.from(img));
        img.onerror = () => reject(Error('stub texture failed: ' + url));
        img.src = url;
      });
    },
    from(source) { return { baseTexture: { width: source.width, height: source.height, resource: { source } }, destroy() {} }; },
  };

  window.PIXI = { Application, Texture, live2d: { Live2DModel, MotionPriority: { NONE: 0, IDLE: 1, NORMAL: 2, FORCE: 3 } } };
})();
