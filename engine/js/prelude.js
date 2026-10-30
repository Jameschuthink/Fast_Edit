/* AIOS3 prelude — loaded first in every render worker (QuickJS).
 *
 * A composition is a classic script that defines   function draw(g, t) { ... }   — a pure function of time that
 * draws one frame with the HTML Canvas2D API on `g`. Nothing is drawn here: every call is recorded as numbers in
 * OPS (strings are interned once) and the Rust engine paints the record with Skia, on every core in parallel.
 *
 *   assets   img(path) · video(path, {loop}) → V.frame(t) · seq(pattern, n, fps) → S.frame(i) · font(family, file)
 *   canvas   the whole Canvas2D API on g (paths, Path2D, gradients, text, shadows, filter, clip, composite …)
 *   extras   g.beginLayer({alpha, blend, filter}) / g.endLayer() · g.fillTextBox(...) · new Image() for ported comps
 *   data     JOB (job.json + words + clip info) · W, H (world size) · FPS
 *
 * World units: the comp always draws in its own size (meta.size, default 1080x1920); drafts are the same drawing
 * scaled down by the engine. Shadows, blur filters and line widths are in world units too.
 */
'use strict';
(function () {

// ───────────────────────────── op codes (must match engine/src/canvas.rs) ─────────────────────────────
const OP = {
  SAVE: 1, RESTORE: 2, SET_TRANSFORM: 3, TRANSFORM: 4, TRANSLATE: 5, SCALE: 6, ROTATE: 7, RESET_TRANSFORM: 8,
  ALPHA: 10, COMPOSITE: 11, FILL_STYLE: 12, STROKE_STYLE: 13, FILL_GRAD: 14, STROKE_GRAD: 15, LINE_WIDTH: 16,
  LINE_CAP: 17, LINE_JOIN: 18, MITER: 19, DASH: 20, DASH_OFFSET: 21, SHADOW_COLOR: 22, SHADOW_BLUR: 23,
  SHADOW_OFFSET: 24, FONT: 25, TEXT_ALIGN: 26, TEXT_BASELINE: 27, LETTER_SPACING: 28, FILTER: 29, SMOOTHING: 30,
  WORD_SPACING: 31, FILL_PATTERN: 32, STROKE_PATTERN: 33,
  BEGIN_PATH: 40, MOVE_TO: 41, LINE_TO: 42, QUAD_TO: 43, BEZIER_TO: 44, ARC: 45, ARC_TO: 46, ELLIPSE: 47, RECT: 48,
  ROUND_RECT: 49, CLOSE_PATH: 50, PATH_SVG: 51, PATH2D_BEGIN: 52, PATH2D_END: 53, PATH2D_POP: 54,
  FILL: 60, STROKE: 61, CLIP: 62, FILL_RECT: 63, STROKE_RECT: 64, CLEAR_RECT: 65, FILL_TEXT: 66, STROKE_TEXT: 67,
  DRAW_IMAGE: 68, BEGIN_LAYER: 70, END_LAYER: 71, TEXT_BOX: 72, MARK: 80,
};
const COMPOSITE = { 'source-over': 0, 'source-in': 1, 'source-out': 2, 'source-atop': 3, 'destination-over': 4,
  'destination-in': 5, 'destination-out': 6, 'destination-atop': 7, 'lighter': 8, 'copy': 9, 'xor': 10,
  'multiply': 11, 'screen': 12, 'overlay': 13, 'darken': 14, 'lighten': 15, 'color-dodge': 16, 'color-burn': 17,
  'hard-light': 18, 'soft-light': 19, 'difference': 20, 'exclusion': 21, 'hue': 22, 'saturation': 23, 'color': 24,
  'luminosity': 25, 'plus-lighter': 8 };
const CAP = { butt: 0, round: 1, square: 2 }, JOIN = { miter: 0, round: 1, bevel: 2 };
const ALIGN = { start: 0, left: 0, center: 1, right: 2, end: 2 };
const BASE = { alphabetic: 0, top: 1, hanging: 2, middle: 3, ideographic: 4, bottom: 5 };

// ───────────────────────────── the record ─────────────────────────────
let OPS = new Float64Array(1 << 15), N = 0;
const SI = new Map(); let NEWS = [];
function grow(k) { if (N + k > OPS.length) { const b = new Float64Array(Math.max(OPS.length * 2, N + k + 1024)); b.set(OPS.subarray(0, N)); OPS = b; } }
function op1(a) { grow(1); OPS[N++] = a; }
function op2(a, b) { grow(2); OPS[N++] = a; OPS[N++] = b; }
function op3(a, b, c) { grow(3); OPS[N++] = a; OPS[N++] = b; OPS[N++] = c; }
function op5(a, b, c, d, e) { grow(5); OPS[N++] = a; OPS[N++] = b; OPS[N++] = c; OPS[N++] = d; OPS[N++] = e; }
function opn(arr) { grow(arr.length); for (let i = 0; i < arr.length; i++) OPS[N++] = arr[i]; }
function sid(s) { s = String(s); let i = SI.get(s); if (i === undefined) { i = SI.size; SI.set(s, i); NEWS.push(s); } return i; }
const num = v => { v = +v; return Number.isFinite(v) ? v : NaN; };

// ───────────────────────────── assets ─────────────────────────────
const IMGS = new Map();
function img(path) {                                 // a still image: png, jpg, webp, svg
  let o = IMGS.get(path);
  if (!o) { const r = __img(String(path)); o = { __id: r[0], __f: 0, width: r[1], height: r[2], naturalWidth: r[1], naturalHeight: r[2], complete: true, src: path }; IMGS.set(path, o); }
  return o;
}
const VIDS = new Map();
function video(path, opts) {                         // a video file; V.frame(t) is the frame shown at second t of the video
  opts = opts || {};
  const key = path + '|' + JSON.stringify(opts);
  let o = VIDS.get(key);
  if (!o) {
    const r = __video(String(path), JSON.stringify(opts));
    const id = r[0], w = r[1], h = r[2], fps = r[3], n = r[4];
    o = { __id: id, width: w, height: h, fps, n, dur: n / fps, loop: !!opts.loop, src: path,
      index(t) { let f = Math.round(t * fps + 1e-6); if (this.loop) { f = ((f % n) + n) % n; } return f < 0 ? 0 : f > n - 1 ? n - 1 : f; },
      frame(t) { return { __id: id, __f: this.index(t), width: w, height: h }; } };
    VIDS.set(key, o);
  }
  return o;
}
const SEQS = new Map();
function seq(pattern, n, fps) {                      // an image sequence 'dir/%04d.jpg' (1-based files); S.frame(i)
  let o = SEQS.get(pattern);
  if (!o) {
    const r = __seq(String(pattern), n | 0);
    const id = r[0], w = r[1], h = r[2];
    n = r[4];                                         // the files that exist when n was not given
    o = { __id: id, width: w, height: h, n, first: r[3], fps: fps || 30, src: pattern,
      frame(i) { i = Math.floor(i); i = i < 0 ? 0 : i > n - 1 ? n - 1 : i; return { __id: id, __f: i, width: w, height: h }; },
      at(t) { return this.frame(t * this.fps); } };
    SEQS.set(pattern, o);
  }
  return o;
}
function font(family, file, opts) { __font(String(family), String(file), JSON.stringify(opts || {})); }

// Browser shims so AIOS2 comps run unchanged: new Image() + src, decode(), document.fonts, FontFace, performance.
// A file name ending in a frame number (frames/00012.jpg, cutout/00001.webp) is read as a sequence frame; when
// job.json "seqVideo" maps the pattern to a video (AIOS2 frame folders made from the clip), the video is decoded.
const SEQ_RE = /^(.*?)(\d{3,})(\.[A-Za-z0-9]+)$/;
class Image {
  constructor() { this.__id = -1; this.__f = 0; this.width = 0; this.height = 0; this.naturalWidth = 0; this.naturalHeight = 0; this.complete = false; this._src = ''; }
  set src(p) {
    this._src = p; if (!p) return;
    const m = SEQ_RE.exec(p);
    let o = null;
    if (m) {
      const pattern = m[1] + '%0' + m[2].length + 'd' + m[3], num = parseInt(m[2], 10);
      const map = (typeof JOB !== 'undefined' && JOB.seqVideo) ? JOB.seqVideo[pattern] : null;
      if (map) {
        const V = video(map.video, map.opts || {});
        o = { __id: V.__id, __f: Math.max(0, Math.min(V.n - 1, num + (map.offset ?? -1))), width: map.w || V.width, height: map.h || V.height };
        o.__kx = V.width / o.width; o.__ky = V.height / o.height;
      } else {
        const S = seq(pattern, 0);
        o = { __id: S.__id, __f: Math.max(0, num - S.first), width: S.width, height: S.height };
      }
    } else o = img(p);
    this.__id = o.__id; this.__f = o.__f; this.__kx = o.__kx; this.__ky = o.__ky;
    this.width = this.naturalWidth = o.width; this.height = this.naturalHeight = o.height; this.complete = true;
  }
  get src() { return this._src; }
  decode() { return Promise.resolve(); }
}
class FontFace { constructor(family, source, desc) { this.family = family; this.source = source; this.desc = desc || {}; } load() { return Promise.resolve(this); } }
const document = {
  fonts: { load: () => Promise.resolve([]), add() {}, ready: Promise.resolve(), check: () => true },
  documentElement: { style: { setProperty() {} } },
  getElementById: () => ({ style: {}, textContent: '', getContext: () => g }),
  querySelector: () => ({ style: {}, getContext: () => g }),
  createElement: () => ({ style: {}, getContext: () => g }),
};
const performance = { now: () => 0 };
const location = { search: '' };
const URLSearchParams = class { constructor() {} has() { return false; } get() { return null; } };
function requestAnimationFrame() {}
const console = { log: (...a) => __log(a.map(x => typeof x === 'string' ? x : JSON.stringify(x)).join(' ')),
  warn: (...a) => __log('warn: ' + a.join(' ')), error: (...a) => __log('error: ' + a.join(' ')) };
Math.random = () => { throw new Error('Math.random is not allowed: a frame must be a pure function of t (use OM.rng(seed))'); };

// ───────────────────────────── gradients, patterns, Path2D ─────────────────────────────
class CanvasGradient {
  constructor(kind, c) { this.kind = kind; this.c = c; this.stops = []; }
  addColorStop(o, color) { this.stops.push(+o, sid(color)); }
  __emit(code) { grow(4 + this.c.length + this.stops.length); OPS[N++] = code; OPS[N++] = this.kind; for (const v of this.c) OPS[N++] = v; OPS[N++] = this.stops.length / 2; for (const v of this.stops) OPS[N++] = v; }
}
const GRAD_ARGS = [4, 6, 3];                         // linear x0 y0 x1 y1 · radial x0 y0 r0 x1 y1 r1 · conic angle x y
class CanvasPattern {
  constructor(src, rep) { this.src = src; this.rep = rep; this.m = null; }
  setTransform(m) { this.m = m ? [m.a, m.b, m.c, m.d, m.e, m.f] : null; }
  __emit(code) { const r = { 'repeat': 0, 'repeat-x': 1, 'repeat-y': 2, 'no-repeat': 3 }[this.rep || 'repeat'] ?? 0;
    const m = this.m || [1, 0, 0, 1, 0, 0]; opn([code, this.src.__id, this.src.__f || 0, r, ...m]); }
}
class Path2D {
  constructor(p) { this.ops = []; if (typeof p === 'string') this.ops.push(OP.PATH_SVG, sid(p)); else if (p instanceof Path2D) this.ops.push(...p.ops); }
  moveTo(x, y) { this.ops.push(OP.MOVE_TO, +x, +y); }
  lineTo(x, y) { this.ops.push(OP.LINE_TO, +x, +y); }
  quadraticCurveTo(a, b, x, y) { this.ops.push(OP.QUAD_TO, +a, +b, +x, +y); }
  bezierCurveTo(a, b, c, d, x, y) { this.ops.push(OP.BEZIER_TO, +a, +b, +c, +d, +x, +y); }
  arc(x, y, r, a0, a1, ccw) { this.ops.push(OP.ARC, +x, +y, +r, +a0, +a1, ccw ? 1 : 0); }
  arcTo(x1, y1, x2, y2, r) { this.ops.push(OP.ARC_TO, +x1, +y1, +x2, +y2, +r); }
  ellipse(x, y, rx, ry, rot, a0, a1, ccw) { this.ops.push(OP.ELLIPSE, +x, +y, +rx, +ry, +rot, +a0, +a1, ccw ? 1 : 0); }
  rect(x, y, w, h) { this.ops.push(OP.RECT, +x, +y, +w, +h); }
  roundRect(x, y, w, h, r) { this.ops.push(OP.ROUND_RECT, +x, +y, +w, +h, ...radii(r)); }
  closePath() { this.ops.push(OP.CLOSE_PATH); }
  addPath(p) { this.ops.push(...p.ops); }
}
function radii(r) {                                   // → 8 numbers: tl.x tl.y tr.x tr.y br.x br.y bl.x bl.y
  const one = v => (typeof v === 'object' && v) ? [+(v.x ?? 0), +(v.y ?? v.x ?? 0)] : [+(v || 0), +(v || 0)];
  if (!Array.isArray(r)) r = [r ?? 0];
  let a;
  if (r.length === 1) a = [r[0], r[0], r[0], r[0]];
  else if (r.length === 2) a = [r[0], r[1], r[0], r[1]];
  else if (r.length === 3) a = [r[0], r[1], r[2], r[1]];
  else a = [r[0], r[1], r[2], r[3]];
  return a.flatMap(one);
}

// ───────────────────────────── the context ─────────────────────────────
const MEASURE = new Map();
function mul(m, n) { return [m[0] * n[0] + m[2] * n[1], m[1] * n[0] + m[3] * n[1], m[0] * n[2] + m[2] * n[3], m[1] * n[2] + m[3] * n[3], m[0] * n[4] + m[2] * n[5] + m[4], m[1] * n[4] + m[3] * n[5] + m[5]]; }
class DOMMatrix_ { constructor(m) { [this.a, this.b, this.c, this.d, this.e, this.f] = m; } }
function freshState() {
  return { fillStyle: '#000000', strokeStyle: '#000000', globalAlpha: 1, lineWidth: 1, lineCap: 'butt', lineJoin: 'miter',
    miterLimit: 10, lineDashOffset: 0, dash: [], shadowColor: 'rgba(0, 0, 0, 0)', shadowBlur: 0, shadowOffsetX: 0, shadowOffsetY: 0,
    font: '10px sans-serif', textAlign: 'start', textBaseline: 'alphabetic', letterSpacing: '0px', wordSpacing: '0px', filter: 'none',
    globalCompositeOperation: 'source-over', imageSmoothingEnabled: true, imageSmoothingQuality: 'low', direction: 'ltr', m: [1, 0, 0, 1, 0, 0] };
}
const pxOf = v => { if (typeof v === 'number') return v; const m = /^\s*(-?[\d.]+)(px|em)?\s*$/.exec(String(v)); return m ? +m[1] : 0; };

class Ctx {
  constructor() { this.S = freshState(); this.stack = []; this.canvas = { width: 1080, height: 1920 }; }
  __reset() { this.S = freshState(); this.stack = []; }
  // state
  save() { this.stack.push(Object.assign({}, this.S, { dash: this.S.dash.slice(), m: this.S.m.slice() })); op1(OP.SAVE); }
  restore() { if (this.stack.length) { this.S = this.stack.pop(); op1(OP.RESTORE); } }
  reset() { while (this.stack.length) this.restore(); this.S = freshState(); op1(OP.RESET_TRANSFORM); this.__resync(); }
  __resync() { const s = this.S; op2(OP.ALPHA, 1); op2(OP.COMPOSITE, 0); op2(OP.FILL_STYLE, sid(s.fillStyle)); op2(OP.STROKE_STYLE, sid(s.strokeStyle)); }
  get globalAlpha() { return this.S.globalAlpha; }
  set globalAlpha(v) { v = +v; if (!(v >= 0 && v <= 1)) return; this.S.globalAlpha = v; op2(OP.ALPHA, v); }
  get globalCompositeOperation() { return this.S.globalCompositeOperation; }
  set globalCompositeOperation(v) { const k = COMPOSITE[v]; if (k === undefined) return; this.S.globalCompositeOperation = v; op2(OP.COMPOSITE, k); }
  get fillStyle() { return this.S.fillStyle; }
  set fillStyle(v) { this.S.fillStyle = v; if (v instanceof CanvasGradient) v.__emit(OP.FILL_GRAD); else if (v instanceof CanvasPattern) v.__emit(OP.FILL_PATTERN); else op2(OP.FILL_STYLE, sid(v)); }
  get strokeStyle() { return this.S.strokeStyle; }
  set strokeStyle(v) { this.S.strokeStyle = v; if (v instanceof CanvasGradient) v.__emit(OP.STROKE_GRAD); else if (v instanceof CanvasPattern) v.__emit(OP.STROKE_PATTERN); else op2(OP.STROKE_STYLE, sid(v)); }
  get lineWidth() { return this.S.lineWidth; }
  set lineWidth(v) { v = +v; if (!(v > 0) || !Number.isFinite(v)) return; this.S.lineWidth = v; op2(OP.LINE_WIDTH, v); }
  get lineCap() { return this.S.lineCap; }
  set lineCap(v) { if (CAP[v] === undefined) return; this.S.lineCap = v; op2(OP.LINE_CAP, CAP[v]); }
  get lineJoin() { return this.S.lineJoin; }
  set lineJoin(v) { if (JOIN[v] === undefined) return; this.S.lineJoin = v; op2(OP.LINE_JOIN, JOIN[v]); }
  get miterLimit() { return this.S.miterLimit; }
  set miterLimit(v) { v = +v; if (!(v > 0)) return; this.S.miterLimit = v; op2(OP.MITER, v); }
  setLineDash(a) { a = (a || []).map(Number); if (a.some(x => !(x >= 0) || !Number.isFinite(x))) return; if (a.length % 2) a = a.concat(a); this.S.dash = a; grow(2 + a.length); OPS[N++] = OP.DASH; OPS[N++] = a.length; for (const x of a) OPS[N++] = x; }
  getLineDash() { return this.S.dash.slice(); }
  get lineDashOffset() { return this.S.lineDashOffset; }
  set lineDashOffset(v) { v = +v; if (!Number.isFinite(v)) return; this.S.lineDashOffset = v; op2(OP.DASH_OFFSET, v); }
  get shadowColor() { return this.S.shadowColor; }
  set shadowColor(v) { this.S.shadowColor = v; op2(OP.SHADOW_COLOR, sid(v)); }
  get shadowBlur() { return this.S.shadowBlur; }
  set shadowBlur(v) { v = +v; if (!(v >= 0)) return; this.S.shadowBlur = v; op2(OP.SHADOW_BLUR, v); }
  get shadowOffsetX() { return this.S.shadowOffsetX; }
  set shadowOffsetX(v) { v = +v; if (!Number.isFinite(v)) return; this.S.shadowOffsetX = v; op3(OP.SHADOW_OFFSET, v, this.S.shadowOffsetY); }
  get shadowOffsetY() { return this.S.shadowOffsetY; }
  set shadowOffsetY(v) { v = +v; if (!Number.isFinite(v)) return; this.S.shadowOffsetY = v; op3(OP.SHADOW_OFFSET, this.S.shadowOffsetX, v); }
  get font() { return this.S.font; }
  set font(v) { this.S.font = String(v); op2(OP.FONT, sid(v)); }
  get textAlign() { return this.S.textAlign; }
  set textAlign(v) { if (ALIGN[v] === undefined) return; this.S.textAlign = v; op2(OP.TEXT_ALIGN, ALIGN[v]); }
  get textBaseline() { return this.S.textBaseline; }
  set textBaseline(v) { if (BASE[v] === undefined) return; this.S.textBaseline = v; op2(OP.TEXT_BASELINE, BASE[v]); }
  get letterSpacing() { return this.S.letterSpacing; }
  set letterSpacing(v) { this.S.letterSpacing = typeof v === 'number' ? v + 'px' : String(v); op2(OP.LETTER_SPACING, pxOf(v)); }
  get wordSpacing() { return this.S.wordSpacing; }
  set wordSpacing(v) { this.S.wordSpacing = typeof v === 'number' ? v + 'px' : String(v); op2(OP.WORD_SPACING, pxOf(v)); }
  get direction() { return this.S.direction; }
  set direction(v) { this.S.direction = v; }
  get filter() { return this.S.filter; }
  set filter(v) { this.S.filter = String(v); op2(OP.FILTER, sid(v)); }
  get imageSmoothingEnabled() { return this.S.imageSmoothingEnabled; }
  set imageSmoothingEnabled(v) { this.S.imageSmoothingEnabled = !!v; op3(OP.SMOOTHING, v ? 1 : 0, { low: 0, medium: 1, high: 2 }[this.S.imageSmoothingQuality] ?? 0); }
  get imageSmoothingQuality() { return this.S.imageSmoothingQuality; }
  set imageSmoothingQuality(v) { this.S.imageSmoothingQuality = v; op3(OP.SMOOTHING, this.S.imageSmoothingEnabled ? 1 : 0, { low: 0, medium: 1, high: 2 }[v] ?? 0); }
  // transforms (the JS side keeps a copy so getTransform works)
  setTransform(a, b, c, d, e, f) {
    if (typeof a === 'object' && a) ({ a, b, c, d, e, f } = Object.assign({ a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 }, a));
    else if (a === undefined) [a, b, c, d, e, f] = [1, 0, 0, 1, 0, 0];
    this.S.m = [+a, +b, +c, +d, +e, +f]; grow(7); OPS[N++] = OP.SET_TRANSFORM; for (const v of this.S.m) OPS[N++] = v;
  }
  resetTransform() { this.S.m = [1, 0, 0, 1, 0, 0]; op1(OP.RESET_TRANSFORM); }
  transform(a, b, c, d, e, f) { this.S.m = mul(this.S.m, [+a, +b, +c, +d, +e, +f]); grow(7); OPS[N++] = OP.TRANSFORM; OPS[N++] = +a; OPS[N++] = +b; OPS[N++] = +c; OPS[N++] = +d; OPS[N++] = +e; OPS[N++] = +f; }
  translate(x, y) { x = +x; y = +y; const m = this.S.m; m[4] += m[0] * x + m[2] * y; m[5] += m[1] * x + m[3] * y; op3(OP.TRANSLATE, x, y); }
  scale(x, y) { x = +x; y = y === undefined ? x : +y; const m = this.S.m; m[0] *= x; m[1] *= x; m[2] *= y; m[3] *= y; op3(OP.SCALE, x, y); }
  rotate(a) { a = +a; this.S.m = mul(this.S.m, [Math.cos(a), Math.sin(a), -Math.sin(a), Math.cos(a), 0, 0]); op2(OP.ROTATE, a); }
  getTransform() { return new DOMMatrix_(this.S.m); }
  // paths
  beginPath() { op1(OP.BEGIN_PATH); }
  closePath() { op1(OP.CLOSE_PATH); }
  moveTo(x, y) { op3(OP.MOVE_TO, num(x), num(y)); }
  lineTo(x, y) { op3(OP.LINE_TO, num(x), num(y)); }
  quadraticCurveTo(a, b, x, y) { op5(OP.QUAD_TO, num(a), num(b), num(x), num(y)); }
  bezierCurveTo(a, b, c, d, x, y) { grow(7); OPS[N++] = OP.BEZIER_TO; OPS[N++] = num(a); OPS[N++] = num(b); OPS[N++] = num(c); OPS[N++] = num(d); OPS[N++] = num(x); OPS[N++] = num(y); }
  arc(x, y, r, a0, a1, ccw) { if (r < 0) throw new RangeError('arc: negative radius'); grow(7); OPS[N++] = OP.ARC; OPS[N++] = num(x); OPS[N++] = num(y); OPS[N++] = num(r); OPS[N++] = num(a0); OPS[N++] = num(a1); OPS[N++] = ccw ? 1 : 0; }
  arcTo(x1, y1, x2, y2, r) { if (r < 0) throw new RangeError('arcTo: negative radius'); grow(6); OPS[N++] = OP.ARC_TO; OPS[N++] = num(x1); OPS[N++] = num(y1); OPS[N++] = num(x2); OPS[N++] = num(y2); OPS[N++] = num(r); }
  ellipse(x, y, rx, ry, rot, a0, a1, ccw) { if (rx < 0 || ry < 0) throw new RangeError('ellipse: negative radius'); opn([OP.ELLIPSE, num(x), num(y), num(rx), num(ry), num(rot), num(a0), num(a1), ccw ? 1 : 0]); }
  rect(x, y, w, h) { op5(OP.RECT, num(x), num(y), num(w), num(h)); }
  roundRect(x, y, w, h, r) { opn([OP.ROUND_RECT, num(x), num(y), num(w), num(h), ...radii(r)]); }
  __path2d(p) { op1(OP.PATH2D_BEGIN); opn(p.ops); op1(OP.PATH2D_END); }
  fill(a, b) { let rule = a; if (a instanceof Path2D) { this.__path2d(a); rule = b; } op2(OP.FILL, rule === 'evenodd' ? 1 : 0); if (a instanceof Path2D) op1(OP.PATH2D_POP); }
  stroke(p) { if (p instanceof Path2D) { this.__path2d(p); op1(OP.STROKE); op1(OP.PATH2D_POP); } else op1(OP.STROKE); }
  clip(a, b) { let rule = a; if (a instanceof Path2D) { this.__path2d(a); rule = b; } op2(OP.CLIP, rule === 'evenodd' ? 1 : 0); if (a instanceof Path2D) op1(OP.PATH2D_POP); }
  isPointInPath() { return false; }
  isPointInStroke() { return false; }
  // direct draws
  fillRect(x, y, w, h) { op5(OP.FILL_RECT, num(x), num(y), num(w), num(h)); }
  strokeRect(x, y, w, h) { op5(OP.STROKE_RECT, num(x), num(y), num(w), num(h)); }
  clearRect(x, y, w, h) { op5(OP.CLEAR_RECT, num(x), num(y), num(w), num(h)); }
  fillText(text, x, y, maxWidth) { op5(OP.FILL_TEXT, sid(text), num(x), num(y), maxWidth === undefined ? -1 : num(maxWidth)); }
  strokeText(text, x, y, maxWidth) { op5(OP.STROKE_TEXT, sid(text), num(x), num(y), maxWidth === undefined ? -1 : num(maxWidth)); }
  measureText(text) {
    text = String(text);
    const key = this.S.font + '|' + this.S.letterSpacing + '|' + text;
    let m = MEASURE.get(key);
    if (!m) {
      const r = __measure(this.S.font, pxOf(this.S.letterSpacing), text);
      m = { width: r[0], actualBoundingBoxLeft: r[1], actualBoundingBoxRight: r[2], actualBoundingBoxAscent: r[3], actualBoundingBoxDescent: r[4],
        fontBoundingBoxAscent: r[5], fontBoundingBoxDescent: r[6], emHeightAscent: r[5], emHeightDescent: r[6], alphabeticBaseline: 0 };
      if (MEASURE.size > 20000) MEASURE.clear();
      MEASURE.set(key, m);
    }
    // the box depends on textAlign / textBaseline like the browser's
    const al = ALIGN[this.S.textAlign], w = m.width, o = Object.assign({}, m);
    if (al === 1) { o.actualBoundingBoxLeft = m.actualBoundingBoxLeft + w / 2; o.actualBoundingBoxRight = m.actualBoundingBoxRight - w / 2; }
    else if (al === 2) { o.actualBoundingBoxLeft = m.actualBoundingBoxLeft + w; o.actualBoundingBoxRight = m.actualBoundingBoxRight - w; }
    return o;
  }
  drawImage(src, a, b, c, d, e, f, g2, h) {
    if (!src || src.__id === undefined || src.__id < 0) return;            // not loaded / not an engine image: skip like the browser
    const W0 = src.width, H0 = src.height;
    let sx = 0, sy = 0, sw = W0, sh = H0, dx, dy, dw, dh;
    if (c === undefined) { dx = a; dy = b; dw = W0; dh = H0; }
    else if (e === undefined) { dx = a; dy = b; dw = c; dh = d; }
    else { sx = a; sy = b; sw = c; sh = d; dx = e; dy = f; dw = g2; dh = h; }
    if (src.__kx) { sx *= src.__kx; sw *= src.__kx; sy *= src.__ky; sh *= src.__ky; }
    grow(11); OPS[N++] = OP.DRAW_IMAGE; OPS[N++] = src.__id; OPS[N++] = src.__f || 0;
    OPS[N++] = num(sx); OPS[N++] = num(sy); OPS[N++] = num(sw); OPS[N++] = num(sh); OPS[N++] = num(dx); OPS[N++] = num(dy); OPS[N++] = num(dw); OPS[N++] = num(dh);
  }
  createLinearGradient(x0, y0, x1, y1) { return new CanvasGradient(0, [+x0, +y0, +x1, +y1]); }
  createRadialGradient(x0, y0, r0, x1, y1, r1) { return new CanvasGradient(1, [+x0, +y0, +r0, +x1, +y1, +r1]); }
  createConicGradient(a, x, y) { return new CanvasGradient(2, [+a, +x, +y]); }
  createPattern(src, rep) { return src && src.__id >= 0 ? new CanvasPattern(src, rep) : null; }
  getImageData() { throw new Error('getImageData is not available in AIOS3 (frames never come back to JS)'); }
  putImageData() { throw new Error('putImageData is not available in AIOS3'); }
  // AIOS3 extras
  // mark(label) … unmark(): the draws between are one element for `aios lint` (label: a string or {id, kind, text, bleed}).
  // Marks paint nothing and do not change the frame's hash.
  // A mark whose id has a nudge (JOB.edit.nudge[id] = [dx, dy, k], from nudge= on its line) draws moved by dx,dy and
  // scaled k about the frame centre; a mark inside a nudged one is not nudged again, except an fx part (kind 'part',
  // nudge.PART=), which moves inside its layer's nudge.
  mark(label) {
    op2(OP.MARK, sid('+' + (typeof label === 'string' ? JSON.stringify({ id: label }) : JSON.stringify(label))));
    const st = this.__nudged || (this.__nudged = []), id = typeof label === 'string' ? label : label && label.id;
    const nd = globalThis.JOB && JOB.edit && JOB.edit.nudge, part = label && label.kind === 'part', v = nd && (part || !st.includes(true)) ? nd[id] : null;
    st.push(!!v);
    if (v) { const cx = this.canvas.width / 2, cy = this.canvas.height / 2; this.save(); this.translate(cx + v[0], cy + v[1]); this.scale(v[2], v[2]); this.translate(-cx, -cy); }
  }
  unmark() { if (this.__nudged && this.__nudged.pop()) this.restore(); op2(OP.MARK, sid('-')); }
  beginLayer(o) { o = o || {}; this.save(); op5(OP.BEGIN_LAYER, o.alpha ?? 1, COMPOSITE[o.blend || 'source-over'] ?? 0, sid(o.filter || 'none'), 0); }
  endLayer() { op1(OP.END_LAYER); this.restore(); }
  // wrapped text in a box: lines break at maxWidth; align 'left'|'center'|'right'; lineHeight in em. Returns {lines, height}.
  fillTextBox(text, x, y, maxWidth, o) {
    o = o || {};
    grow(8); OPS[N++] = OP.TEXT_BOX; OPS[N++] = sid(text); OPS[N++] = num(x); OPS[N++] = num(y); OPS[N++] = num(maxWidth);
    OPS[N++] = ALIGN[o.align || 'left'] ?? 0; OPS[N++] = +(o.lineHeight || 1.2); OPS[N++] = o.stroke ? 1 : 0;
    const r = __measure_box(this.S.font, pxOf(this.S.letterSpacing), String(text), +maxWidth, +(o.lineHeight || 1.2));
    return { lines: r[0], height: r[1], width: r[2] };
  }
}
const g = new Ctx();

// ───────────────────────────── frame entry (called by the engine) ─────────────────────────────
let ERR = null;
function __begin(w, h, fps) { G.innerWidth = w; G.innerHeight = h; g.canvas.width = w; g.canvas.height = h; G.FPS = fps; }
function __frame_begin(t) {
  N = 0; NEWS = []; ERR = null; g.__reset();
  const f = typeof G.__frame_draw === 'function' ? G.__frame_draw : G.draw;
  if (typeof f !== 'function') throw new Error('the comp must define  function draw(g, t) { ... }');
  const r = f(g, t);
  if (r && typeof r.then === 'function') r.then(() => guides(), e => { ERR = e; });   // async draw: the engine runs the jobs, then __frame_end
  else guides();
}
// job.json "guides": true (aios still --grid, the live page's grid box): the layout grid over the frame
function guides() { if (G.JOB && G.JOB.guides && G.LAYOUT) { while (g.stack.length) g.restore(); G.LAYOUT.guides(g); } }
function __frame_end() {
  if (ERR) { const e = ERR; ERR = null; throw e; }
  while (g.stack.length) g.restore();                 // an unbalanced save() must not leak into the next frame
  const news = NEWS; NEWS = [];
  return [N, news];
}
function __ops() { return OPS; }

const G = globalThis;
const EXPORTS = { img, video, seq, font, Image, FontFace, document, performance, location, URLSearchParams, requestAnimationFrame,
  console, Path2D, CanvasGradient, CanvasPattern, __begin, __frame_begin, __frame_end, __ops, __ctx: g, window: G, self: G,
  innerWidth: 1080, innerHeight: 1920, FPS: 30, devicePixelRatio: 1 };
for (const k in EXPORTS) Object.defineProperty(G, k, { value: EXPORTS[k], writable: true, configurable: true, enumerable: false });
})();
