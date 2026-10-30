/* AIOS3 lib/layout.js — the grid every graphic sits on, for any canvas (reel, cover, square, feed, 16:9, poster):
 * the safe area (margins + the platform's buttons and text), columns, guide lines (thirds and golden ratio), a type
 * scale and named slots. Graphics name a slot instead of typing x / y, so a job cannot drift off the grid;
 * `aios lint` measures every frame against the same grid.
 *
 * job.json "layout" (every key optional; the format follows from the size):
 *   format     reel | story | cover | square | feed | wide | poster
 *   platforms  reel only: ["reels"] (default), "tiktok", "shorts" — their buttons and text are keep-out zones
 *   margin     safe margin, world units (default 6% of the short side)
 *   cols, gutter, unit    columns (6; poster and wide 12), gutter (3 units), spacing unit (8 px; poster 1/120 of the short side)
 *   base, ratio           type scale: px of step 0 and the step ratio (default √φ = 1.272: two steps = φ)
 *   slots      {name: {x, y, w, h, align, valign, dir, gap}} to add or change slots
 *
 *   LAYOUT.safe {x,y,w,h} · LAYOUT.zones [{name,x,y,w,h}] · LAYOUT.cols [{x,w}] · LAYOUT.lines {x: {...}, y: {...}}
 *   LAYOUT.slot(name)   {x, y, w, h, ax, ay, ...}: the box, and the anchor (ax, ay) where an element's centre goes
 *   LAYOUT.size(s)      px on the type scale: a step number or a role (caption label body lead h3 h2 h1 display hero big title)
 *   LAYOUT.place(b, id) resolves slot / named x, y / size role / w in columns ("4col") on a beat or element, and
 *                       notes raw numbers that are off the grid (aios lint prints them)
 *   LAYOUT.guides(g)    draws the grid over a frame (aios still --grid, live page "grid")
 *   g.mark(label) … g.unmark()   group draws into one element that `aios lint` checks (REEL, CAPTIONS, POSTER do it)
 */
(function () {
  const PHI = (1 + Math.sqrt(5)) / 2;
  const [W, H] = JOB.size || [1080, 1920];
  const spec = Object.assign({}, JOB.layout || {});
  const SHORT = Math.min(W, H);
  const near = (a, b, tol = 0.02) => Math.abs(a / b - 1) < tol;
  const format = spec.format || (near(H / W, 16 / 9) ? 'reel' : near(W / H, 16 / 9) ? 'wide' : near(W, H) ? 'square' : near(H / W, 1.25) ? 'feed' : 'poster');
  const VERTICAL = ['reel', 'story', 'cover'].includes(format);
  const k1080 = W / 1080;                                        // the platform numbers are for a 1080-wide frame

  // ── keep-out zones on a 1080 x 1920 frame: CapQA masks (11 Sept 2026); TikTok top bar and Stories: videogrid.app
  // (Sept 2026). Text and key graphics never go in them; the picture may.
  const PLATFORMS = {
    reels: [['Reels header', 0, 0, 1080, 210], ['Reels name, caption and audio', 0, 1590, 1080, 330], ['Reels buttons', 996, 614, 84, 846]],
    tiktok: [['TikTok tabs', 0, 0, 1080, 130], ['TikTok name, caption and sound', 0, 1570, 1080, 350], ['TikTok buttons', 918, 730, 162, 806]],
    shorts: [['Shorts search bar', 0, 0, 1080, 234], ['Shorts title and channel', 0, 1575, 1080, 345], ['Shorts buttons', 936, 499, 144, 961]],
    stories: [['Stories progress and name', 0, 0, 1080, 270], ['Stories reply field', 0, 1540, 1080, 380]],
  };
  // Meta's own rule for Reels ads (14% top, 35% bottom, 6% sides): shown on the guides as a soft line, not checked
  const SOFT = { reels: { x: 65, y: 269, w: 950, h: 979 } };

  const unit = spec.unit ?? (format === 'poster' ? Math.max(1, Math.round(SHORT / 120)) : 8);
  const snap = v => Math.round(v / unit) * unit;
  const margin = spec.margin ?? snap(SHORT * 0.06);
  const platforms = VERTICAL && format !== 'cover' ? (spec.platforms || [format === 'story' ? 'stories' : 'reels']) : [];
  const zones = [];
  for (const p of platforms) {
    if (!PLATFORMS[p]) throw new Error(`layout: unknown platform "${p}" (${Object.keys(PLATFORMS).join(', ')})`);
    for (const [name, x, y, w, h] of PLATFORMS[p]) zones.push({ name, x: x * k1080, y: y * k1080, w: w * k1080, h: h * k1080 });
  }

  // ── the safe area: inside the margins, clear of every zone, centred (so centred text stays on the centre line)
  let top = margin, bottom = H - margin, left = margin, right = W - margin;
  if (format === 'wide') { left = snap(W * 0.05); right = W - left; top = snap(H * 0.05); bottom = H - top; }   // 90% title safe
  if (format === 'cover') { top = 285 * k1080; bottom = 1635 * k1080; }                                     // the 4:5 profile-grid crop
  for (const z of zones) {
    if (z.w >= W * 0.9) { if (z.y <= 1) top = Math.max(top, z.y + z.h); else bottom = Math.min(bottom, z.y); }
    else if (z.x + z.w >= W - 1) right = Math.min(right, z.x);
    else if (z.x <= 1) left = Math.max(left, z.x + z.w);
  }
  const inset = Math.max(left, W - right);
  const safe = { x: inset, y: top, w: W - 2 * inset, h: bottom - top };
  const soft = platforms.includes('reels') && SOFT.reels ? scaleBox(SOFT.reels) : null;
  function scaleBox(b) { return { x: b.x * k1080, y: b.y * k1080, w: b.w * k1080, h: b.h * k1080 }; }

  // ── columns
  const ncol = spec.cols ?? (format === 'poster' || format === 'wide' ? 12 : 6);
  const gutter = spec.gutter ?? unit * 3;
  const cw = (safe.w - gutter * (ncol - 1)) / ncol;
  const cols = [...Array(ncol).keys()].map(i => ({ x: safe.x + i * (cw + gutter), w: cw }));
  const span = (a, b) => ({ x: cols[a - 1].x, w: cols[b - 1].x + cw - cols[a - 1].x });

  // ── guide lines (world units): the safe edges, the centre, thirds and golden sections of the whole frame
  const lines = {
    x: { left: safe.x, right: safe.x + safe.w, center: W / 2, third1: W / 3, third2: 2 * W / 3, phi1: W / PHI / PHI, phi2: W / PHI },
    y: { top: safe.y, bottom: safe.y + safe.h, center: H / 2, third1: H / 3, third2: 2 * H / 3, phi1: H / PHI / PHI, phi2: H / PHI,
      upper: H / PHI ** 3, lower: H - H / PHI ** 3 },
  };

  // ── type scale: px = base * ratio^step; two steps = φ
  const base = spec.base ?? (format === 'poster' ? Math.round(SHORT / 30) : Math.round(44 * SHORT / 1080));
  const ratio = spec.ratio ?? Math.sqrt(PHI);
  const ROLES = { caption: -2, label: -1, body: 0, lead: 1, h3: 2, h2: 3, h1: 4, display: 5, hero: 6, big: 7, title: 8 };
  const step = n => base * ratio ** n;
  function size(s) {
    if (typeof s === 'number') return Math.round(step(s) * 10) / 10;
    if (ROLES[s] !== undefined) return Math.round(step(ROLES[s]) * 10) / 10;
    throw new Error(`layout: unknown size "${s}" (a step number or ${Object.keys(ROLES).join(', ')})`);
  }
  const scale = [...Array(15).keys()].map(i => Math.round(step(i - 4) * 10) / 10);
  const gap = n => snap(unit * PHI ** (n ?? 2));                // spacing steps (unit 8): 0 8, 1 16, 2 24, 3 32, 4 56, 5 88

  // ── slots: rows on the guide lines (every format), and the poster's sections
  const slots = {};
  function row(name, y) {
    const h = 2 * Math.min(y - safe.y, safe.y + safe.h - y);
    if (h <= unit * 4) return;
    const half = (safe.w - gutter) / 2;
    slots[name] = { x: safe.x, y: y - h / 2, w: safe.w, h, ax: W / 2, ay: y, align: 'center', valign: 'center', kind: 'row' };
    slots[name + '-left'] = { x: safe.x, y: y - h / 2, w: half, h, ax: safe.x + half / 2, ay: y, align: 'center', valign: 'center', kind: 'row' };
    slots[name + '-right'] = { x: safe.x + half + gutter, y: y - h / 2, w: half, h, ax: safe.x + half + gutter + half / 2, ay: y, align: 'center', valign: 'center', kind: 'row' };
  }
  row('top', (safe.y + Math.max(lines.y.upper, safe.y + unit * 12)) / 2);  // between the safe top and the upper golden line
  row('title', lines.y.upper);                                    // 0.236: the big title word
  row('stage', lines.y.third1);                                   // 1/3: cards, pictures, compares
  row('golden', lines.y.phi1);                                    // 0.382
  row('center', lines.y.center);                                  // 1/2: one statement
  row('lower', lines.y.phi2);                                     // 0.618: under a full-screen face
  if (format === 'poster' || spec.sections) {
    // header (φ⁻⁴ of the safe height), footer (φ⁻³), the body split φ : 1 (landscape: text left, picture right;
    // portrait: picture on top, text under it)
    const hh = snap(safe.h / PHI ** 4), fh = snap(safe.h / PHI ** 3), g2 = gap(3);
    const body = { x: safe.x, y: safe.y + hh + g2, w: safe.w, h: safe.h - hh - fh - 2 * g2 };
    const foot = { x: safe.x, y: safe.y + safe.h - fh, w: safe.w, h: fh };
    const sec = (b, o) => Object.assign({ x: b.x, y: b.y, w: b.w, h: b.h, ax: b.x + b.w / 2, ay: b.y + b.h / 2, align: 'left', valign: 'center', dir: 'col', kind: 'section' }, o || {});
    slots.brand = sec({ x: safe.x, y: safe.y, w: safe.w / 2 - gutter / 2, h: hh }, { valign: 'top' });
    slots.tag = sec({ x: safe.x + safe.w / 2 + gutter / 2, y: safe.y, w: safe.w / 2 - gutter / 2, h: hh }, { align: 'right', valign: 'top' });
    if (W >= H * 0.95) {
      const mw = snap(body.w / PHI - gutter / 2);
      slots.main = sec({ x: body.x, y: body.y, w: mw, h: body.h }, { valign: 'phi' });
      slots.hero = sec({ x: body.x + mw + gutter, y: body.y, w: body.w - mw - gutter, h: body.h }, { align: 'center' });
    } else {
      const ph = snap(body.h / PHI - gutter / 2);
      slots.hero = sec({ x: body.x, y: body.y, w: body.w, h: ph }, { align: 'center' });
      slots.main = sec({ x: body.x, y: body.y + ph + gutter, w: body.w, h: body.h - ph - gutter });
    }
    slots.rule = sec({ x: foot.x, y: foot.y, w: foot.w, h: 0 });             // gap(3) under the body, gap(3) over the foot's content
    slots['foot-left'] = sec({ x: foot.x, y: foot.y + g2, w: foot.w / 2 - gutter / 2, h: foot.h - g2 }, { dir: 'row' });
    slots['foot-right'] = sec({ x: foot.x + foot.w / 2 + gutter / 2, y: foot.y + g2, w: foot.w / 2 - gutter / 2, h: foot.h - g2 }, { align: 'right' });
    slots.body = sec(body);
    slots.foot = sec(foot);
  }
  for (const [name, o] of Object.entries(spec.slots || {})) {
    const s = Object.assign({ align: 'center', valign: 'center' }, slots[name] || {}, resolveBox(o), { kind: 'custom' });
    s.ax = o.ax !== undefined ? X(o.ax) : s.x + s.w / 2; s.ay = o.ay !== undefined ? Y(o.ay) : s.y + s.h / 2;
    slots[name] = s;
  }
  function resolveBox(o) {
    const r = Object.assign({}, o);
    if (o.cols) { const [a, b] = String(o.cols).split('-').map(Number); const s = span(a, b || a); r.x = s.x; r.w = s.w; }
    for (const k of ['x', 'w']) if (typeof r[k] === 'string') r[k] = k === 'x' ? X(r[k]) : widthOf(r[k]);
    for (const k of ['y', 'h']) if (typeof r[k] === 'string') r[k] = k === 'y' ? Y(r[k]) : Y(r[k]);
    return r;
  }

  function slot(name) {
    const s = slots[name];
    if (!s) throw new Error(`layout: no slot "${name}" (slots: ${Object.keys(slots).filter(n => !/-(left|right)$/.test(n)).join(', ')}, and ROW-left / ROW-right)`);
    return s;
  }
  // named x: center, left, right (the halves), a column "c2" or a span "c2-4" (its centre), a line (third1, phi2, …)
  function X(v) {
    if (typeof v === 'number') return v;
    const m = /^c(\d+)(?:-(\d+))?$/.exec(v);
    if (m) { const s = span(+m[1], +(m[2] || m[1])); return s.x + s.w / 2; }
    if (v === 'left') return slots['center-left'] ? slots['center-left'].ax : safe.x + safe.w / 4;
    if (v === 'right') return slots['center-right'] ? slots['center-right'].ax : safe.x + safe.w * 3 / 4;
    if (lines.x[v] !== undefined) return lines.x[v];
    throw new Error(`layout: unknown x "${v}" (center, left, right, c1..c${ncol}, c2-4, ${Object.keys(lines.x).join(', ')})`);
  }
  function Y(v) {
    if (typeof v === 'number') return v;
    if (lines.y[v] !== undefined) return lines.y[v];
    if (slots[v]) return slots[v].ay;
    throw new Error(`layout: unknown y "${v}" (${Object.keys(lines.y).join(', ')}, or a slot name)`);
  }
  function widthOf(v) {
    if (typeof v === 'number') return v;
    const m = /^(\d+)col$/.exec(v);
    if (m) return Math.min(+m[1], ncol) * cw + (Math.min(+m[1], ncol) - 1) * gutter;
    if (v === 'full') return safe.w;
    if (v === 'half') return (safe.w - gutter) / 2;
    throw new Error(`layout: unknown width "${v}" (4col, half, full, or px)`);
  }

  // ── placing: slot / names -> numbers; raw numbers off the grid are noted for `aios lint`
  const AUDIT = [];
  const xs = () => [W / 2, ...Object.values(lines.x), ...cols.flatMap(c => [c.x, c.x + c.w / 2, c.x + c.w]), ...Object.values(slots).map(s => s.ax)];
  const ys = () => [...Object.values(lines.y), ...Object.values(slots).map(s => s.ay)];
  function nearest(v, list) { let b = list[0]; for (const x of list) if (Math.abs(x - v) < Math.abs(b - v)) b = x; return b; }
  function nameOfY(v) { const s = Object.entries(slots).find(([n, s]) => !/-(left|right)$/.test(n) && Math.abs(s.ay - v) < 0.5); if (s) return 'slot ' + s[0]; const l = Object.entries(lines.y).find(([n, y]) => Math.abs(y - v) < 0.5); return l ? l[0] : Math.round(v); }
  function onScale(px) { return scale.some(s => Math.abs(px / s - 1) <= 0.05); }
  function note(id, msg) { AUDIT.push({ id, msg }); }
  function place(b, id, opts = {}) {
    const tol = unit / 2;
    if (b.slot) {
      const s = slot(b.slot);
      b.__slot = s;
      if (b.x === undefined) b.x = s.ax; else b.x = X(b.x);
      if (b.y === undefined) b.y = s.ay; else b.y = Y(b.y);
    } else {
      if (typeof b.x === 'string') b.x = X(b.x);
      if (typeof b.y === 'string') b.y = Y(b.y);
      if (!opts.free) {
        if (typeof b.y === 'number' && !ys().some(v => Math.abs(v - b.y) <= tol))
          note(id, `y ${Math.round(b.y)} is off the grid (nearest: ${nameOfY(nearest(b.y, ys()))}); use "slot"`);
        if (typeof b.x === 'number' && !xs().some(v => Math.abs(v - b.x) <= tol))
          note(id, `x ${Math.round(b.x)} is off the grid (nearest ${Math.round(nearest(b.x, xs()))}); use "slot" or x "c1".."c${ncol}"`);
      }
    }
    if (typeof b.w === 'string') b.w = widthOf(b.w);
    if (typeof b.size === 'string') b.size = size(b.size);
    else if (typeof b.step === 'number') b.size = size(b.step);
    else if (typeof b.size === 'number' && !opts.free && !onScale(b.size)) note(id, `size ${b.size} px is off the type scale (${scale.filter(s => s > b.size * 0.6 && s < b.size * 1.6).join(', ')})`);
    return b;
  }

  // the slots the guides draw: posters show their sections (not the rows and the body / foot containers); other
  // formats show the rows; custom slots always
  function shown(n, s) {
    if (s.kind === 'custom') return true;
    if (s.kind === 'row') return format !== 'poster' && !/-(left|right)$/.test(n);
    return n !== 'body' && n !== 'foot';
  }

  // ── what `aios lint` reads
  let salientFn = null;
  function info() {
    return { format, W, H, unit, margin, safe, soft, zones, cols, lines, base, ratio, scale, platforms,
      slots: Object.fromEntries(Object.entries(slots).map(([n, s]) => [n, { x: s.x, y: s.y, w: s.w, h: s.h, ax: s.ax, ay: s.ay, kind: s.kind, shown: shown(n, s) }])),
      audit: AUDIT };
  }
  function frame(t) { return { salient: salientFn ? salientFn(t) : [] }; }

  // ── the guides overlay: zones red, safe area green, columns, thirds white, golden lines gold, slot anchors
  function guides(g) {
    const lw = Math.max(1, SHORT / 540);
    g.save(); g.resetTransform(); g.globalAlpha = 1; g.globalCompositeOperation = 'source-over'; g.filter = 'none';
    g.shadowColor = 'transparent'; g.setLineDash([]);
    g.fillStyle = 'rgba(80,200,255,0.07)'; for (const c of cols) g.fillRect(c.x, safe.y, c.w, safe.h);
    g.fillStyle = 'rgba(255,40,40,0.30)'; for (const z of zones) g.fillRect(z.x, z.y, z.w, z.h);
    g.font = `600 ${Math.round(SHORT / 60)}px sans-serif`; g.textBaseline = 'top'; g.textAlign = 'left';
    g.fillStyle = 'rgba(255,255,255,0.85)'; for (const z of zones) g.fillText(z.name, z.x + 8 * lw, z.y + 6 * lw);
    if (soft) { g.strokeStyle = 'rgba(255,170,60,0.8)'; g.lineWidth = lw; g.setLineDash([6 * lw, 6 * lw]); g.strokeRect(soft.x, soft.y, soft.w, soft.h); g.setLineDash([]); }
    g.strokeStyle = 'rgba(60,230,120,0.95)'; g.lineWidth = 2 * lw; g.strokeRect(safe.x, safe.y, safe.w, safe.h);
    const hl = (y, c, d) => { g.strokeStyle = c; g.setLineDash(d); g.beginPath(); g.moveTo(0, y); g.lineTo(W, y); g.stroke(); };
    const vl = (x, c, d) => { g.strokeStyle = c; g.setLineDash(d); g.beginPath(); g.moveTo(x, 0); g.lineTo(x, H); g.stroke(); };
    g.lineWidth = lw;
    for (const v of [lines.x.third1, lines.x.third2]) vl(v, 'rgba(255,255,255,0.55)', [10 * lw, 8 * lw]);
    for (const v of [lines.y.third1, lines.y.third2]) hl(v, 'rgba(255,255,255,0.55)', [10 * lw, 8 * lw]);
    for (const v of [lines.x.phi1, lines.x.phi2]) vl(v, 'rgba(255,200,60,0.75)', [3 * lw, 5 * lw]);
    for (const v of [lines.y.phi1, lines.y.phi2, lines.y.upper, lines.y.lower]) hl(v, 'rgba(255,200,60,0.75)', [3 * lw, 5 * lw]);
    vl(W / 2, 'rgba(255,255,255,0.3)', []);
    g.setLineDash([]);
    g.font = `600 ${Math.round(SHORT / 70)}px sans-serif`; g.textBaseline = 'bottom';
    for (const [n, s] of Object.entries(slots)) {
      if (!shown(n, s)) continue;
      const box = s.kind !== 'row' && s.h > 0;
      if (box) { g.strokeStyle = 'rgba(80,200,255,0.8)'; g.lineWidth = lw; g.strokeRect(s.x, s.y, s.w, s.h); }
      const r = 7 * lw; g.strokeStyle = 'rgba(80,200,255,0.95)'; g.lineWidth = 1.5 * lw;
      g.beginPath(); g.moveTo(s.ax - r, s.ay); g.lineTo(s.ax + r, s.ay); g.moveTo(s.ax, s.ay - r); g.lineTo(s.ax, s.ay + r); g.stroke();
      g.fillStyle = 'rgba(80,200,255,0.95)'; g.fillText(n, (box ? s.x : s.ax) + 10 * lw, (box ? s.y + 16 * lw : s.ay - 4 * lw));
    }
    g.restore();
  }

  globalThis.LAYOUT = { format, W, H, unit, margin, safe, soft, zones, cols, lines, base, ratio, scale, slots, gutter,
    slot, size, gap, place, X, Y, width: widthOf, span, info, frame, guides, note, PHI,
    set salient(fn) { salientFn = fn; } };
})();
