// lib edit: draws a job made from ONE file, jobs/JOB/edit.txt (engine/src/editfile.rs compiles it into JOB.edit).
// The picture of each kept part, read from its clip's prep (turned and cropped by the engine) at the raw second, so
// the video is never cut again when a skip or a sentence turns on or off; then each text line with its style. A style with
// `behind` puts the person (the matte, made by the engine) over its words.
//
// style keys: font, size, color, y (x is the middle), shadow (flag), fit (most px wide), weight=700, track=-60 (letter spacing, 1/1000 em)
//   fade=0.6          the bottom 60% of the word fades from full to 0 (top to bottom)
//   pop=0.08           alpha 0->1 and scale 0.9->1, linear over that many seconds (small words)
//   in=0.12 grow=1.12  alpha 0->1 (cubic ease) and scale grow->1 over `in` seconds (big words)
//   from=up|down dist=90  the word moves in from above (up) or below (down) while it fades in (pop or in time)
//   bg=#000000         a full-screen colour under the word while it is on (a title card: over the video and the captions)
//   enter=soft|rise|blur|wipe|pop|slam|blurpop  exit=fade|blur|drift  (named moves, MOVES below; over pop/in/grow)
//   enterdur= exitdur= (s, else the move's own)  steps=15 (the move on a 15 fps step clock: stop-motion text)
// text keys (override the style): at=x,y, size, color, y, fit, from, dist
// pic: a picture over the video; fill = the whole width, centred on black (a cut-away)
// fx: named effects, the skill library (engine/js/fx/NAME.js; a job's own in jobs/JOB/fx/). A file:
//   // fx NAME: what it does. keys: amp=18 hz=14
//   FX.def('NAME', { keys: { amp: 18, hz: 14 } }, {
//     video(g, f, k) {},   // transform the video picture (and the person matte) only, e.g. g.translate(dx, dy)
//     under(g, f, k) {},   // draw over the picture, under the words and captions
//     over(g, f, k) {},    // draw over everything
//   });
// f = {t, t0, t1, u: t - t0, p: 0..1 over the line, dur, W, H, matte(), part()}; k = the line's keys over the defaults.
// f.part(g, 'title', () => { ...draws... }): one part the page can drag on its own (writes nudge.title= on the line).
// f.matte() = the person at t (RGB = the clip, A = the person) or null; a file with the line `// needs: matte` gets one.
globalThis.FX = { defs: {}, def(name, meta, hooks) { this.defs[name] = { meta: meta || {}, hooks: hooks || {} }; } };

globalThis.EDIT = (function () {
  const E = JOB.edit || { items: [], styles: {}, segs: [], mattes: [] };
  const W = JOB.size[0], H = JOB.size[1];
  const V = {};
  for (const [id, path] of Object.entries(E.videos || {})) V[id] = video(path);
  const PICS = (E.pics || []).map(p => ({ ...p, I: img(p.src) }));
  const MATTES = (E.mattes || []).map(m => ({ ...m, S: seq(m.pattern, m.n, m.fps) }));

  // output second -> raw second of a clip (the matte is in raw time, so a skip never moves it)
  function raw(t, clip) {
    for (const s of E.segs) if (s.clip === clip && t >= s.o - 1e-6 && t < s.o + (s.b - s.a) + 1e-6) return s.a + (t - s.o);
    return null;
  }

  function drawText(g, t, it) {
    const s = E.styles[it.style] || {};
    const size = it.size ?? s.size ?? 84, color = it.color ?? s.color ?? '#ffffff';
    const [x, y] = it.at || [W / 2, it.y ?? s.y ?? H / 2];
    let a, k, e;
    const enter = it.enter ?? s.enter, exit = it.exit ?? s.exit;
    let mv = null;   // a named move (MOVES): {op, sc, ty (em), blur (px), wipe 0..1}
    if (it.wt && String(it.wt).includes(';')) return drawWords(g, t, it, s, size, color, x, y, enter, exit);
    if (enter || exit) mv = move(t, it, s, enter, exit, size);
    if (mv) { a = mv.op; k = mv.sc; e = 1; }
    else if (s.grow) {
      const p = Math.max(0, Math.min(1, (t - it.t0) / (s.in ?? 0.12)));
      e = 1 - Math.pow(1 - p, 3);
      a = e; k = s.grow - (s.grow - 1) * e;
    } else {
      const p = Math.min(1, (t - it.t0) / (s.pop ?? 0.08));
      a = p; k = 0.9 + 0.1 * p; e = 1 - Math.pow(1 - p, 3);
    }
    const from = it.from ?? s.from, dy = from ? (from === 'up' ? -1 : 1) * (it.dist ?? s.dist ?? 90) * (1 - e) : 0;
    g.mark({ id: it.id, kind: 'text', text: String(it.text) });
    g.save(); g.globalAlpha = a; g.translate(x, y + dy + (mv ? mv.ty * size : 0)); g.scale(k, k);
    g.font = `${s.weight ? s.weight + ' ' : ''}${size}px ${s.font || 'sans-serif'}`; g.textAlign = 'center'; g.textBaseline = 'middle';
    if (s.track) g.letterSpacing = s.track / 1000 * size;
    const fit = it.fit ?? s.fit;
    if (fit) { const m = g.measureText(it.text).width; if (m > fit) g.scale(fit / m, fit / m); }
    if (mv && mv.blur > 0.3) g.filter = `blur(${(mv.blur * size / 108).toFixed(1)}px)`;
    if (mv && mv.wipe < 1) { const m = g.measureText(it.text).width; g.beginPath(); g.rect(-m / 2 - 0.1 * size, -size, (m + 0.2 * size) * mv.wipe, 2 * size); g.clip(); }
    if (s.shadow) { g.shadowColor = 'rgba(0,0,0,0.35)'; g.shadowBlur = 12; g.shadowOffsetY = 3; }
    const fade = it.fade ?? s.fade;
    if (fade) {   // the word in its own layer, then a mask: full at the top, 0 at the bottom (descenders included)
      g.beginLayer({});
      g.fillStyle = color; g.fillText(it.text, 0, 0);
      const top = -0.38 * size, bot = 0.48 * size, m = g.createLinearGradient(0, top + (bot - top) * (1 - fade), 0, bot);
      m.addColorStop(0, 'rgba(0,0,0,1)'); m.addColorStop(1, 'rgba(0,0,0,0)');
      g.shadowColor = 'transparent'; g.globalCompositeOperation = 'destination-in'; g.fillStyle = m;
      const mw = g.measureText(it.text).width + 0.3 * size;   // only over the word (its box stays the word's)
      g.fillRect(-mw / 2, top - 0.3 * size, mw, bot - top + 0.6 * size);
      g.endLayer();
    } else { g.fillStyle = color; g.fillText(it.text, 0, 0); }
    g.restore();
    g.unmark();
  }

  // a card row word by word (key wt = "0;0.21;…", seconds after t0 per word): each word its own enter move, one exit
  function drawWords(g, t, it, s, size, color, x, y, enter, exit) {
    const ws = String(it.text).split(' '), off = String(it.wt).split(';').map(Number);
    g.mark({ id: it.id, kind: 'text', text: String(it.text) });
    g.save(); g.translate(x, y);
    g.font = `${s.weight ? s.weight + ' ' : ''}${size}px ${s.font || 'sans-serif'}`; g.textAlign = 'center'; g.textBaseline = 'middle';
    if (s.track) g.letterSpacing = s.track / 1000 * size;
    const fit = it.fit ?? s.fit, all = g.measureText(it.text).width;
    if (fit && all > fit) g.scale(fit / all, fit / all);
    const sp = g.measureText(' ').width, wd = ws.map(w => g.measureText(w).width);
    let cx = -(wd.reduce((a, b) => a + b, 0) + sp * (ws.length - 1)) / 2;
    ws.forEach((w, i) => {
      const mv = move(t, { ...it, t0: it.t0 + (off[i] || 0) }, s, enter || 'soft', exit, size);
      if (t >= it.t0 + (off[i] || 0) && mv.op > 0.002) {
        g.save(); g.globalAlpha = mv.op; g.translate(cx + wd[i] / 2, mv.ty * size); g.scale(mv.sc, mv.sc);
        if (mv.blur > 0.3) g.filter = `blur(${(mv.blur * size / 108).toFixed(1)}px)`;
        if (s.shadow) { g.shadowColor = 'rgba(0,0,0,0.35)'; g.shadowBlur = 12; g.shadowOffsetY = 3; }
        g.fillStyle = color; g.fillText(w, 0, 0); g.restore();
      }
      cx += wd[i] + sp;
    });
    g.restore(); g.unmark();
  }

  // Named moves (keys enter= exit=, enterdur= exitdur= in s, steps=FPS). Numbers from the editorial-film-edit
  // package (media/references/editorial-film/, typeLayers.js): an AI-tuned preset over 16 versions, not measured
  // from a real editor. ty is in em (x the type size, + = down); blur is px at 108 px type, scaled with the size.
  // The exit runs in the last exitdur seconds of the line, so the line's times do not change.
  const cl = v => Math.max(0, Math.min(1, v));
  const expo = u => (u >= 1 ? 1 : 1 - Math.pow(2, -10 * u));
  const cubicOut = u => 1 - Math.pow(1 - u, 3), cubicIn = u => u * u * u;
  const backOut = u => 1 + 2.70158 * Math.pow(u - 1, 3) + 1.70158 * Math.pow(u - 1, 2);
  const MOVES = {
    enter: {
      soft:    [0.32, (l, d) => { const e = expo(cl(l / d)); return { ty: (1 - e) * 0.32, blur: (1 - e) * 9, op: cl(l / d * 4.5) }; }],
      rise:    [0.22, (l, d) => { const e = expo(cl(l / d)); return { ty: (1 - e) * 0.45, op: cl(l / 0.06) }; }],
      blur:    [0.22, (l, d) => { const e = cubicOut(cl(l / d)); return { ty: (1 - e) * 0.08, blur: (1 - e) * 14, op: cl(l / 0.12) }; }],
      wipe:    [0.16, (l, d) => ({ wipe: cubicOut(cl(l / d)) })],
      pop:     [0.18, (l, d) => ({ sc: 0.82 + 0.18 * backOut(cl(l / d)), op: cl(l / 0.08) })],
      slam:    [0.14, (l, d) => { const e = expo(cl(l / d)); return { sc: 1.55 - 0.55 * e, blur: (1 - e) * 10, op: cl(l / 0.05) }; }],
      blurpop: [0.12, (l, d) => { const e = expo(cl(l / d)); return { sc: 1.3 - 0.3 * e, blur: (1 - e) * 8, op: cl(l / 0.04) }; }],
    },
    exit: {   // r = seconds left
      fade:  [0.12, (r, d) => ({ op: cl(r / d) })],
      blur:  [0.12, (r, d) => { const e = 1 - cl(r / d); return { blur: e * 10, op: 1 - 0.7 * e }; }],
      drift: [0.28, (r, d) => { const e = cubicIn(1 - cl(r / d)); return { ty: -e * 0.12, blur: e * 8, op: 1 - e }; }],
    },
  };
  function move(t, it, s, enter, exit, size) {
    const steps = it.steps ?? s.steps, fps = JOB.fps || 30;
    // stepped clock (stop-motion text): from the frame's own time, so the motion-blur samples of one frame agree
    const q = l => (steps ? Math.floor(Math.round(l * fps) / fps * steps + 1e-6) / steps : l);
    const m = { op: 1, sc: 1, ty: 0, blur: 0, wipe: 1 };
    const add = o => { if (o.op != null) m.op *= o.op; if (o.sc != null) m.sc *= o.sc; m.ty += o.ty || 0; m.blur = Math.max(m.blur, o.blur || 0); if (o.wipe != null) m.wipe = Math.min(m.wipe, o.wipe); };
    const en = MOVES.enter[enter], ex = MOVES.exit[exit];
    if (en) add(en[1](q(t - it.t0), it.enterdur ?? s.enterdur ?? en[0]));
    if (ex) { const d = it.exitdur ?? s.exitdur ?? ex[0], r = it.t1 - t; if (r < d) add(ex[1](d - q(d - r), d)); }
    if (!en) { const p = Math.min(1, (t - it.t0) / (s.pop ?? 0.08)); m.op *= p; m.sc *= 0.9 + 0.1 * p; }   // no enter: the old pop
    return m;
  }

  // the effects on at t, each with its hooks, f and k
  function fxOn(t) {
    return (E.fx || []).filter(x => t >= x.t0 && t < x.t1).map(x => {
      const d = FX.defs[x.name];
      if (!d) throw new Error(`fx ${x.id}: no FX.def('${x.name}') (its file must call FX.def with that name)`);
      const dur = Math.max(1e-6, x.t1 - x.t0);
      return { id: x.id, h: d.hooks, f: { t, t0: x.t0, t1: x.t1, u: t - x.t0, p: Math.min(1, (t - x.t0) / dur), dur, W, H, matte: () => matteAt(t),
        part: (g, name, paint) => { g.mark({ id: x.id + '.' + name, kind: 'part', text: name }); try { paint(); } finally { g.unmark(); } } }, k: { ...(d.meta.keys || {}), ...(x.keys || {}) } };
    });
  }
  // the person matte at output second t, or null. Frame k of the matte = the video frame at from + k/fps, picked by
  // rounding, as video() does.
  function matteAt(t) {
    for (const m of MATTES) {
      const r = raw(t, m.clip);
      if (r !== null && r >= m.from - 1e-6 && r < m.from + m.n / m.fps) return m.S.frame(Math.round((r - m.from) * m.fps + 1e-6));
    }
    return null;
  }
  function fxRun(g, on, hook) {
    for (const x of on) if (x.h[hook]) { g.mark({ id: x.id, kind: 'fx', text: x.id }); g.save(); x.h[hook](g, x.f, x.k); g.restore(); g.unmark(); }
  }
  function draw(g, t) {
    const segs = E.segs || [], fx = fxOn(t), vfx = fx.filter(x => x.h.video);
    const video = (paint) => { g.save(); for (const x of vfx) x.h.video(g, x.f, x.k); paint(); g.restore(); };
    let s = segs.find(x => t >= x.o - 1e-6 && t < x.o + (x.b - x.a)) || segs[segs.length - 1];
    if (vfx.length) { g.fillStyle = '#000'; g.fillRect(0, 0, W, H); }
    if (s && V[s.clip]) video(() => g.drawImage(V[s.clip].frame(Math.min(s.b, s.a + Math.max(0, t - s.o))), 0, 0, W, H));
    for (const p of PICS) if (t >= p.t0 && t < p.t1) {
      const I = p.I, k = Math.min(W / I.width, H / I.height), w = I.width * k, h = I.height * k;
      g.mark({ id: p.id, kind: 'pic', text: p.id });
      if (p.fill) { g.fillStyle = '#000'; g.fillRect(0, 0, W, H); }
      g.drawImage(I, (W - w) / 2, (H - h) / 2, w, h);
      g.unmark();
    }
    fxRun(g, fx, 'under');
    const on = E.items.filter(it => t >= it.t0 && t < it.t1);
    const card = on.filter(it => (E.styles[it.style] || {}).bg);
    const back = on.filter(it => (E.styles[it.style] || {}).behind && !card.includes(it));
    for (const it of back) drawText(g, t, it);
    // the person over the words, only while a word is behind (the cut-out is the video again: any drift would show).
    const pm = back.length ? matteAt(t) : null;
    if (pm) video(() => g.drawImage(pm, 0, 0, W, H));
    for (const it of on) if (!back.includes(it) && !card.includes(it)) drawText(g, t, it);
    if (globalThis.CAPTIONS) CAPTIONS.draw(g, t);   // `cap` lines (lib captions, approved v3)
    if (card.length) {   // a title card: its colour over everything, then its words
      g.fillStyle = E.styles[card[0].style].bg; g.fillRect(0, 0, W, H);
      for (const it of card) drawText(g, t, it);
    }
    fxRun(g, fx, 'over');
  }
  globalThis.draw = draw;
  return { items: E.items, raw };
})();
