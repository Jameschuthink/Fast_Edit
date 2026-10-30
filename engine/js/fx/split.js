// fx split: the split screen (style ref De2SEI7JP8r, Michael Quoc Ly): the top half a cream grid paper with a "Step N." label, the face moved into the bottom half; hard cut in and out (the ref cuts, no slide). The label rises in on the first slot of a step (intro=1), a placeholder card (ph="#3 Claude app icon") pops in under it, both settle with a slow push. panel=0 = no split, the card alone over the face (a face-only slot). keys: y=960 (the seam, px) dy=300 (the face moves down, px) step="Step 1." title="Set it up" intro=1 ph="" bg=#DFD6C4 ink=#16130D accent=#BB6F52 grid=48 (px, 0 = none) cy=560 (card middle) py=520 (panel=0 card middle)
font('SplitInter', JOB.engine + '/fonts/captions/Inter.ttf', { pins: { slnt: 0 } });
const SPL = {
  ease: (x) => 1 - Math.pow(1 - Math.min(1, Math.max(0, x)), 3),
  // a small overshoot (MOTION 4): past the mark by ~6%, then settle
  back: (x) => { x = Math.min(1, Math.max(0, x)); const c = 1.4; return 1 + (c + 1) * Math.pow(x - 1, 3) + c * Math.pow(x - 1, 2); },
};
FX.def('split', { keys: { y: 960, dy: 300, step: '', title: '', intro: 1, ph: '', bg: '#DFD6C4', ink: '#16130D', accent: '#BB6F52', grid: 48, cy: 560, py: 520, panel: 1 } }, {
  video(g, f, k) { if (+k.panel) g.translate(0, +k.dy); },
  under(g, f, k) {
    const W = f.W, Y = +k.y, u = f.u;
    if (+k.panel) {
      g.fillStyle = k.bg; g.fillRect(0, 0, W, Y);
      if (+k.grid) {
        g.strokeStyle = k.ink; g.globalAlpha = 0.06; g.lineWidth = 2; g.beginPath();
        for (let x = (W / 2) % k.grid; x < W; x += +k.grid) { g.moveTo(x, 0); g.lineTo(x, Y); }
        for (let y = Y % k.grid; y < Y; y += +k.grid) { g.moveTo(0, y); g.lineTo(W, y); }
        g.stroke(); g.globalAlpha = 1;
      }
      // the label: rise 24 px + fade over 0.35 s on a step's first slot, else already there
      if (k.step || k.title) {
        const e = +k.intro ? SPL.ease(u / 0.35) : 1;
        g.save(); g.globalAlpha = e; g.translate(0, 24 * (1 - e));
        g.font = '700 64px SplitInter'; g.textBaseline = 'middle'; g.textAlign = 'left';
        const a = k.step ? k.step + ' ' : '', wa = g.measureText(a).width, wb = g.measureText(k.title).width;
        const x0 = (W - wa - wb) / 2;
        g.fillStyle = k.accent; g.fillText(a, x0, 190);
        g.fillStyle = k.ink; g.fillText(k.title, x0 + wa, 190);
        g.restore();
      }
    }
    if (!k.ph) return;
    // the placeholder card: 80 ms after the label, scale 0.94 -> 1 with a small overshoot, then a 1.5%/s push (MOTION 10)
    const t = u - (+k.panel && +k.intro ? 0.08 : 0), e = SPL.ease(t / 0.45), s = (0.94 + 0.06 * SPL.back(t / 0.45)) * (1 + 0.015 * Math.max(0, t));
    if (e <= 0) return;
    const cw = 760, ch = 400, cy = +k.panel ? +k.cy : +k.py;
    g.save(); g.globalAlpha = e; g.translate(W / 2, cy); g.scale(s, s);
    g.shadowColor = 'rgba(0,0,0,0.18)'; g.shadowBlur = 30; g.shadowOffsetY = 12;
    g.fillStyle = '#ffffff'; g.beginPath(); g.roundRect(-cw / 2, -ch / 2, cw, ch, 28); g.fill();
    g.shadowColor = 'transparent';
    g.setLineDash([18, 12]); g.strokeStyle = k.accent; g.lineWidth = 4;
    g.beginPath(); g.roundRect(-cw / 2 + 18, -ch / 2 + 18, cw - 36, ch - 36, 18); g.stroke(); g.setLineDash([]);
    const m = String(k.ph).match(/^(#\d+)\s*(.*)$/) || [null, '', k.ph];
    g.textAlign = 'center'; g.textBaseline = 'middle';
    g.fillStyle = k.accent; g.font = '700 110px SplitInter'; g.fillText(m[1], 0, -60);
    g.fillStyle = k.ink; g.font = '500 40px SplitInter';
    // the idea on up to 2 lines
    const words = m[2].split(' '), lines = [''];
    for (const w of words) { const L = lines[lines.length - 1]; if (g.measureText(L + ' ' + w).width > cw - 100 && L) lines.push(w); else lines[lines.length - 1] = (L ? L + ' ' : '') + w; }
    lines.slice(0, 2).forEach((L, i) => g.fillText(L, 0, 60 + i * 52));
    g.restore();
  },
});
