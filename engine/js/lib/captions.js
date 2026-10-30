/* AIOS3 lib/captions.js — spoken captions, approved prompt v3 (user, 2026-10-06), renderer from a test job.
 *
 * Data: job.json "captions": "captions.json" — made by .claude/skills/reel/captions/build.py from the words and the
 * agent's tiers.json (subject, hero, emphasis, name units). Each caption: {start, end, lines: [[{w, tier, t}]], center}.
 * job.json "faces": "faces.json" (face_src.py, clip px) — each caption is placed from the face where the reel layout
 * puts it during that caption (full screen, box, zooms). Old files with mouth_bottom / head_top per caption still work. Tiers: BASE Inter Bold 72 px (91 px when a single word), EMPHASIS Apple
 * Garamond italic 91 px, HERO Aston Script up to 188 px fitted to the 800 px safe width. One shared shadow.
 * Placement: block bottom at y 1528; if it would cover the mouth, above the head; else the HERO shrinks.
 *
 *   CAPTIONS.draw(g, t)                    (REEL.draw calls it)
 *   CAPTIONS.opts = {anchor, safeW, topSafe, shadow}   to adjust
 */
(function () {
  const { clamp, sstep, ease } = OM;
  const E = JOB.engine + '/fonts/captions/';
  font('CapInter', E + 'Inter.ttf', { pins: { slnt: 0 } });
  font('CapGaramond', E + 'apple_garamond/AppleGaramond-Italic.ttf');
  font('CapScript', E + 'aston_script/Aston Script.ttf');
  // job.json "captionStyle": {"scale": 1.0, "anchor": 1528}; a frame line adds emphLine (an EMPHASIS run on its own
  // line), emphScale (its size x), emphColor, safeW (the widest line, px)
  const CS = JOB.captionStyle || {};
  // v3 LANDSCAPE (width > height, e.g. an edit-file job at 1920x1080): S = 56 px (72 x 56/72), block bottom above the
  // bottom 10%, 64 px margins, centred on the frame. Vertical keeps the approved 1080x1920 numbers.
  const [JW, JH] = JOB.size || [1080, 1920], LAND = JW > JH;
  const opts = LAND
    ? { anchor: CS.anchor ?? Math.round(JH * 0.9), safeW: JW - 128, topSafe: 64, scale: (CS.scale ?? 1) * 56 / 72, cx: JW / 2, mid: JH / 2 }
    : { anchor: CS.anchor ?? 1528, safeW: CS.lineW ?? 800, topSafe: 250, scale: CS.scale ?? 1, cx: 540, mid: 960 };
  const caps = () => (JOB.captions && JOB.captions.captions) || [];
  function style(tier, single) {
    const k = opts.scale;
    if (tier === 'HERO') return { font: px => `${px}px CapScript`, px: Math.round(188 * k), track: 0, lh: 1.2 };
    if (tier === 'EMPHASIS') return { font: px => `${px}px CapGaramond`, px: Math.round(91 * k * (CS.emphScale || 1)), track: -0.03, lh: CS.emphLine ? 1.0 : 1.2 };
    return { font: px => `700 ${px}px CapInter`, px: Math.round((single ? 91 : 72) * k), track: -0.06, lh: 1.2 };
  }
  const rnd8 = v => Math.round(v / 8) * 8;
  // the face in world units during a caption: job.json "faces" (clip px, from face_src.py) mapped through the reel
  // layout at each sampled frame (full screen, box, zooms); worst case over the caption, else the caption's own numbers.
  // A caption across a full screen <-> box move is placed for the layout it spends most of its time in.
  const headTop = v => Math.max(v.hair_top ?? -1e9, v.head_top - 60, 90);   // the hair; never > 60 px over the face (a raised hand)
  function faceWorld(c) {
    const F = JOB.faces && JOB.faces.frames, R = globalThis.REEL;
    if (!F || !R || !R.state || !R.state.V) return { mouth_bottom: c.mouth_bottom, head_top: c.head_top };
    const FY = R.state.face[1], P = [[], []];
    const add = f => {
      const v = F[f]; if (!v) return;
      const a = R.clipAt(f / 30);
      P[a.m >= 0.5 ? 1 : 0].push([a.py + a.s * (v.mouth_bottom - FY), a.py + a.s * (headTop(v) - FY)]);
    };
    for (let f = Math.ceil(((c.start - 0.1) * 30 - 1e-6) / 6) * 6; f <= (c.end + 0.1) * 30 + 1e-6; f += 6) add(f);
    if (!P[0].length && !P[1].length) {                    // no face sampled during it: the nearest sampled frame
      const g0 = Math.round(c.start * 5) * 6;
      for (let d = 0; d <= 30 && !P[0].length && !P[1].length; d += 6) { add(g0 - d); if (!P[0].length && !P[1].length) add(g0 + d); }
    }
    const Q = P[1].length === P[0].length ? P[R.clipAt((c.start + c.end) / 2).m >= 0.5 ? 1 : 0] : P[1].length > P[0].length ? P[1] : P[0];
    if (!Q.length) return { mouth_bottom: c.mouth_bottom, head_top: c.head_top };
    return { mouth_bottom: Math.round(Math.max(...Q.map(q => q[0]))), head_top: Math.round(Math.min(...Q.map(q => q[1]))) };
  }
  const LAYOUT = new Map();
  function layout(g, c) {
    let L = LAYOUT.get(c);
    if (L) return L;
    const single = c.lines.length === 1 && c.lines[0].length === 1;
    // emphLine: split each line where it goes in or out of EMPHASIS ("syntax / *patterns*")
    const src = !CS.emphLine ? c.lines : c.lines.flatMap(l => l.reduce((a, x) => {
      const e = x.tier === 'EMPHASIS', last = a[a.length - 1];
      if (last && (last[0].tier === 'EMPHASIS') === e) last.push(x); else a.push([x]);
      return a;
    }, []));
    const lines = src.map(l => {
      const ws = l.map(x => {
        const st = style(x.tier, single); let px = st.px;
        g.font = st.font(px); g.letterSpacing = (st.track * px) + 'px'; let w = g.measureText(x.w).width;
        if (x.tier === 'HERO') { while (w * 1.04 > opts.safeW && px > 60) { px -= 2; g.font = st.font(px); w = g.measureText(x.w).width; } w *= 1.04; }
        if (x.tier === 'EMPHASIS' && CS.emphLine) while (w > (CS.safeW || opts.safeW) && px > 60) { px -= 2; g.font = st.font(px); w = g.measureText(x.w).width; }
        return Object.assign({}, x, { st, px, ww: w });
      });
      const w = ws.reduce((a, x) => a + x.ww, 0) + 16 * (ws.length - 1), h = rnd8(Math.max(...ws.map(x => x.px * x.st.lh)));
      return { ws, w, h };
    });
    let HH = lines.reduce((a, l) => a + l.h, 0) + 8 * (lines.length - 1), bottom = opts.anchor;
    if (c.y !== undefined) { L = { lines, H: HH, bottom: c.y + HH / 2 }; LAYOUT.set(c, L); return L; }   // `cap ... y=PX`: the caption's middle, placed by the user
    const fw = faceWorld(c);
    // a HERO caption sits in the middle of the frame (a test job), when the middle keeps off the face (box layouts);
    // over a full-screen face it is placed like any caption (reel02: 'soul' low); center: "force" = middle always (user pick)
    if (c.center && (c.center === 'force' || fw.mouth_bottom === undefined || opts.mid - HH / 2 >= fw.mouth_bottom + 24 || opts.mid + HH / 2 <= fw.head_top - 24)) {
      L = { lines, H: HH, bottom: opts.mid + HH / 2 }; LAYOUT.set(c, L); return L;
    }
    if (fw.mouth_bottom !== undefined && opts.anchor - HH < fw.mouth_bottom + 24) {        // (1) low does not clear the mouth
      if (fw.head_top - 24 - HH >= opts.topSafe) bottom = fw.head_top - 24;                 // (2) above the head
      else {                                                                                // (3) low, HERO shrunk
        const hero = lines.find(l => l.ws.some(x => x.tier === 'HERO'));
        if (hero) {
          const room = opts.anchor - (fw.mouth_bottom + 24) - (HH - hero.h), x = hero.ws[0], px = Math.floor(room / 1.2);
          g.font = x.st.font(px); x.ww = g.measureText(x.w).width * 1.04; x.px = px; hero.w = x.ww; hero.h = rnd8(px * 1.2) > room ? room : rnd8(px * 1.2);
          HH = lines.reduce((a, l) => a + l.h, 0) + 8 * (lines.length - 1);
        }
      }
    }
    L = { lines, H: HH, bottom };
    LAYOUT.set(c, L);
    return L;
  }
  function draw(g, t) {
    const ci = caps().findIndex(c => t >= c.start && t < c.end); if (ci < 0) return;
    const c = caps()[ci];
    g.save();
    g.resetTransform();
    g.mark(c.__label || (c.__label = { id: c.moment || `caption ${ci + 1}`, kind: 'caption', text: c.lines.flat().map(x => x.w).join(' ') }));
    const L = layout(g, c);
    let y = L.bottom - L.H;
    const heroEnd = Math.max(0, ...c.lines.flat().filter(x => x.tier === 'HERO').map(x => x.t + 0.4));
    const heroLine = L.lines.findIndex(q => q.ws.some(z => z.tier === 'HERO'));
    L.lines.forEach((l, li) => {
      let x = opts.cx - l.w / 2; const cy = y + l.h / 2;
      for (const w of l.ws) {
        let op = 1, s = 1, dy = 0; const u = t - c.start;
        if (w.tier === 'HERO') op = clamp((t - c.start) / 0.4);
        else if (heroEnd && heroLine >= 0 && heroLine < li) op = sstep(heroEnd - 0.05, heroEnd + 0.05, t);   // the line under a HERO waits
        else if (w.tier === 'EMPHASIS') { const k = ease.cubicOut(clamp(u / 0.16)); op = k; s = 0.92 + 0.08 * k; dy = (1 - k) * 16; }
        else { const k = ease.backOut(clamp(u / 0.1)); op = clamp(u / 0.06); s = 0.9 + 0.1 * k; }
        g.save(); g.globalAlpha = op; g.translate(x + w.ww / 2, cy + dy); g.scale(s, s);
        g.font = w.st.font(w.px); g.letterSpacing = (w.st.track * w.px) + 'px'; g.textAlign = 'center'; g.textBaseline = 'middle';
        g.shadowColor = c.color ? 'rgba(0,0,0,0)' : 'rgba(0,0,0,0.55)'; g.shadowBlur = 10; g.shadowOffsetY = 2; g.fillStyle = c.color || (w.tier === 'EMPHASIS' && CS.emphColor) || '#fff'; g.fillText(w.w, 0, 0);
        g.restore(); x += w.ww + 16;
      }
      y += l.h + 8;
    });
    g.unmark();
    g.restore();
  }
  // what `aios lint` reports: the sampled frames (faces.json, every 0.2 s) where a shown caption covers the face
  function check() {
    const F = JOB.faces && JOB.faces.frames, R = globalThis.REEL;
    if (!F || !R || !R.state || !R.state.V) {   // no reel layout (an edit-file job): each caption's own face numbers
      const own = caps().filter(c => c.mouth_bottom !== undefined); if (!own.length) return null;
      const over = own.filter(c => { const L = layout(globalThis.__ctx, c), top = L.bottom - L.H; return !(top >= c.mouth_bottom + 23 || L.bottom <= c.head_top - 23); })
        .map(c => ({ t: +c.start.toFixed(1), text: c.lines.flat().map(x => x.w).join(' '), move: false }));
      return { frames: own.length, over };
    }
    const FY = R.state.face[1], over = []; let frames = 0;
    caps().forEach(c => {
      const L = layout(globalThis.__ctx, c), top = L.bottom - L.H;
      for (let f = Math.ceil(c.start * 5 - 1e-6) * 6; f < c.end * 30; f += 6) {
        const v = F[f]; if (!v) continue;
        const a = R.clipAt(f / 30), mb = a.py + a.s * (v.mouth_bottom - FY), ht = a.py + a.s * (headTop(v) - FY);
        frames++;
        if (!(top >= mb + 23 || L.bottom <= ht - 23))
          over.push({ t: +(f / 30).toFixed(1), text: c.lines.flat().map(x => x.w).join(' '), move: a.m > 0.02 && a.m < 0.98 });
      }
    });
    return { frames, over };
  }
  globalThis.CAPTIONS = { draw, opts, layout, faceWorld, check };
})();
