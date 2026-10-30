// fx shot: a picture card in the split panel (a screenshot, a prompt, a logo), with fx split on the same span: it pops in (scale 0.94 -> 1, a small overshoot), then a slow push. click="x,y" (image px) adds a cursor that glides in from the lower right and clicks at at= s (a dip + a ring). keys: src="media/FILE.png" w=880 h=640 (the box the picture fits in, px) cy=600 (card middle) r=24 (corner) d=0.08 (delay s: 0.08 after a step label rises, else 0) click="" at=0.6 cur=2.2 accent=#BB6F52
// made by note n3 (a test job)
const SH_I = {};
const SH = {
  ease: (x) => 1 - Math.pow(1 - Math.min(1, Math.max(0, x)), 3),
  inout: (x) => { x = Math.min(1, Math.max(0, x)); return x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2; },
  back: (x) => { x = Math.min(1, Math.max(0, x)); const c = 1.4; return 1 + (c + 1) * Math.pow(x - 1, 3) + c * Math.pow(x - 1, 2); },
  // a macOS arrow: black, white edge, tip at 0,0
  arrow(g, s) {
    g.save(); g.scale(s, s);
    g.beginPath(); g.moveTo(0, 0); g.lineTo(0, 34); g.lineTo(8, 26); g.lineTo(14, 39); g.lineTo(20, 36); g.lineTo(14, 24); g.lineTo(25, 24); g.closePath();
    g.shadowColor = 'rgba(0,0,0,0.3)'; g.shadowBlur = 6; g.shadowOffsetY = 3;
    g.fillStyle = '#111'; g.fill(); g.shadowColor = 'transparent';
    g.lineWidth = 2.5; g.strokeStyle = '#fff'; g.stroke();
    g.restore();
  },
};
FX.def('shot', { keys: { src: '', w: 880, h: 640, cy: 600, r: 24, d: 0.08, click: '', at: 0.6, cur: 2.2, accent: '#BB6F52' } }, {
  under(g, f, k) {
    const I = SH_I[k.src] || (SH_I[k.src] = img(k.src));
    const sc = Math.min(+k.w / I.width, +k.h / I.height), w = I.width * sc, h = I.height * sc;
    // the card: 0.45 s in with a small overshoot, then a 1.5%/s push (MOTION 3, 4, 10)
    const t = f.u - +k.d, e = SH.ease(t / 0.45), s = (0.94 + 0.06 * SH.back(t / 0.45)) * (1 + 0.015 * Math.max(0, t));
    if (e <= 0) return;
    g.save(); g.globalAlpha = e; g.translate(f.W / 2, +k.cy); g.scale(s, s); g.translate(-w / 2, -h / 2);
    g.shadowColor = 'rgba(0,0,0,0.2)'; g.shadowBlur = 36; g.shadowOffsetY = 14;
    g.fillStyle = '#ffffff'; g.beginPath(); g.roundRect(0, 0, w, h, +k.r); g.fill(); g.shadowColor = 'transparent';
    g.save(); g.beginPath(); g.roundRect(0, 0, w, h, +k.r); g.clip(); g.drawImage(I, 0, 0, w, h); g.restore();
    if (k.click) {
      const [bx, by] = String(k.click).split(',').map(Number), px = bx * sc, py = by * sc, at = +k.at, pd = f.u - at;
      // the ring grows from the click point
      if (pd > 0 && pd < 0.3) {
        g.strokeStyle = k.accent; g.globalAlpha = e * 0.7 * (1 - pd / 0.3); g.lineWidth = 5;
        g.beginPath(); g.arc(px, py, 24 + 110 * SH.ease(pd / 0.3), 0, 7); g.stroke(); g.globalAlpha = e;
      }
      // the cursor: in from the lower right on an arc, lands at `at - 0.08`, dips at the click, drifts after
      const m = SH.inout((f.u - 0.1) / Math.max(0.1, at - 0.18)), sx = w + 80, sy = h + 120;
      let cx = sx + (px - sx) * m, cy = sy + (py - sy) * m - Math.sin(Math.PI * m) * 60;
      if (pd > 0.25) { cx += 10 * SH.ease((pd - 0.25) / 0.6); cy += 14 * SH.ease((pd - 0.25) / 0.6); }
      if (f.u > 0.1) { g.translate(cx, cy); SH.arrow(g, +k.cur * (pd > 0 && pd < 0.14 ? 0.86 : 1)); }
    }
    g.restore();
  },
});
