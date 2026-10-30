// fx progress: a render card in the split panel, with fx split on the same span: a file name, a track and a fill that springs to each value (stiffness 120, damping 18, mass 1: one small overshoot, settled in about 0.5 s); only the fill moves, the percent counts with it, "Done" when it ends at 1. keys: vals="0.35|0.72|1" at="1.15|1.65|2.1" (s from the line start, one per value) name="my_reel.mp4" label="Rendering" done="Done" w=820 cy=600 d=0.08 accent=#BB6F52 ink=#16130D track=#E9E3D6
// made by note n11 (a test job)
font('SplitInter', JOB.engine + '/fonts/captions/Inter.ttf', { pins: { slnt: 0 } });
const PG = {
  ease: (x) => 1 - Math.pow(1 - Math.min(1, Math.max(0, x)), 3),
  back: (x) => { x = Math.min(1, Math.max(0, x)); const c = 1.4; return 1 + (c + 1) * Math.pow(x - 1, 3) + c * Math.pow(x - 1, 2); },
  // the step response of a damped spring (k 120, c 18, m 1), 0 -> 1, a closed form so each frame stands alone (MOTION 1)
  spring(t) {
    if (t <= 0) return 0;
    const w0 = Math.sqrt(120), z = 18 / (2 * w0), wd = w0 * Math.sqrt(1 - z * z);
    return 1 - Math.exp(-z * w0 * t) * (Math.cos(wd * t) + (z * w0 / wd) * Math.sin(wd * t));
  },
};
FX.def('progress', { keys: { vals: '0.35|0.72|1', at: '1.15|1.65|2.1', name: 'my_reel.mp4', label: 'Rendering', done: 'Done', w: 820, cy: 600, d: 0.08, accent: '#BB6F52', ink: '#16130D', track: '#E9E3D6' } }, {
  under(g, f, k) {
    const vals = String(k.vals).split('|').map(Number), ats = String(k.at).split('|').map(Number);
    let v = 0, prev = 0;
    vals.forEach((x, i) => { v += (x - prev) * PG.spring(f.u - ats[i]); prev = x; });
    const fin = vals[vals.length - 1] >= 1 && f.u > ats[ats.length - 1] + 0.5;
    // the card: 0.45 s in with a small overshoot, then a 1.5%/s push (MOTION 4, 10)
    const t = f.u - +k.d, e = PG.ease(t / 0.45), s = (0.94 + 0.06 * PG.back(t / 0.45)) * (1 + 0.015 * Math.max(0, t));
    if (e <= 0) return;
    const W = +k.w, H = 250, pad = 56, tw = W - 2 * pad, th = 26;
    g.save(); g.globalAlpha = e; g.translate(f.W / 2, +k.cy); g.scale(s, s); g.translate(-W / 2, -H / 2);
    g.shadowColor = 'rgba(0,0,0,0.18)'; g.shadowBlur = 30; g.shadowOffsetY = 12;
    g.fillStyle = '#ffffff'; g.beginPath(); g.roundRect(0, 0, W, H, 28); g.fill(); g.shadowColor = 'transparent';
    g.textBaseline = 'middle';
    g.textAlign = 'left'; g.fillStyle = fin ? k.accent : k.ink; g.font = '700 44px SplitInter';
    g.fillText(fin ? k.done : k.label, pad, 78);
    g.fillStyle = '#8A8478'; g.font = '500 32px SplitInter'; g.fillText(k.name, pad, 186);
    g.textAlign = 'right'; g.fillStyle = k.ink; g.font = '700 44px SplitInter';
    g.fillText(Math.round(Math.min(1, Math.max(0, v)) * 100) + '%', W - pad, 78);
    // the track stays still; only the fill's width moves
    g.fillStyle = k.track; g.beginPath(); g.roundRect(pad, 120, tw, th, th / 2); g.fill();
    const fw = Math.max(0, Math.min(1.03, v)) * tw;
    if (fw > 1) { g.fillStyle = k.accent; g.beginPath(); g.roundRect(pad, 120, Math.max(th, fw), th, th / 2); g.fill(); }
    g.restore();
  },
});
