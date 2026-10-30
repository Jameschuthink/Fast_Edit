// fx tools: an install card in the split panel: rows (a monogram, a name, a role) pop in one after another, each turns from a spinner to a ticked check on its second, the header goes from "Installing tools" to "All set". With fx split on the same span. keys: items="Node.js|FFmpeg|Whisper" sub="runtime|video|captions" at="0.55|0.95|1.35" (s from the line start, one tick per row) title="Installing tools" done="All set" w=760 cy=600 d=0 accent=#BB6F52 ink=#18181b ground=#fcfcfb
// made by note n13 (a test job)
font('SplitInter', JOB.engine + '/fonts/captions/Inter.ttf', { pins: { slnt: 0 } });
const TO = {
  ease: (x) => 1 - Math.pow(1 - Math.min(1, Math.max(0, x)), 3),
  back: (x) => { x = Math.min(1, Math.max(0, x)); const c = 1.7; return 1 + (c + 1) * Math.pow(x - 1, 3) + c * Math.pow(x - 1, 2); },
  ink: (hex, a) => { const n = parseInt(String(hex).slice(1), 16); return `rgba(${n >> 16 & 255},${n >> 8 & 255},${n & 255},${a})`; },
};
FX.def('tools', { keys: { items: 'Node.js|FFmpeg|Whisper', sub: 'runtime|video|captions', at: '0.55|0.95|1.35', title: 'Installing tools', done: 'All set', w: 760, cy: 600, d: 0, accent: '#BB6F52', ink: '#18181b', ground: '#fcfcfb' } }, {
  under(g, f, k) {
    const u = f.u, items = String(k.items).split('|'), subs = String(k.sub).split('|'), ats = String(k.at).split('|').map(Number);
    const n = items.length, rh = 100, W = +k.w, H = 120 + n * rh + 24, pad = 40;
    const t = u - +k.d, e = TO.ease(t / 0.45), s = (0.94 + 0.06 * TO.back(t / 0.45)) * (1 + 0.012 * Math.max(0, t));
    if (e <= 0) return;
    const fin = u > ats[n - 1] + 0.25;
    g.save(); g.globalAlpha = e; g.translate(f.W / 2, +k.cy); g.scale(s, s); g.translate(-W / 2, -H / 2);
    g.shadowColor = 'rgba(0,0,0,0.18)'; g.shadowBlur = 34; g.shadowOffsetY = 14;
    g.fillStyle = k.ground; g.beginPath(); g.roundRect(0, 0, W, H, 28); g.fill(); g.shadowColor = 'transparent';
    g.textBaseline = 'middle'; g.textAlign = 'left';
    g.fillStyle = fin ? k.accent : k.ink; g.font = '700 40px SplitInter'; g.fillText(fin ? k.done : k.title, pad, 62);
    g.fillStyle = TO.ink(k.ink, 0.14); g.fillRect(0, 112, W, 2);
    items.forEach((name, i) => {
      const re = TO.ease((u - 0.1 - i * 0.07) / 0.3);
      if (re <= 0) return;
      const y = 120 + i * rh + rh / 2, tick = u - ats[i];
      g.save(); g.globalAlpha = e * re; g.translate(18 * (1 - re), 0);
      // the monogram: a rounded square, ink at 8%
      g.fillStyle = TO.ink(k.ink, 0.08); g.beginPath(); g.roundRect(pad, y - 30, 60, 60, 16); g.fill();
      g.fillStyle = k.ink; g.font = '700 30px SplitInter'; g.textAlign = 'center'; g.fillText(name[0], pad + 30, y + 1);
      g.textAlign = 'left';
      g.fillStyle = tick > 0 ? k.ink : TO.ink(k.ink, 0.55); g.font = '600 38px SplitInter'; g.fillText(name, pad + 84, y - 12);
      g.fillStyle = TO.ink(k.ink, 0.45); g.font = '500 26px SplitInter'; g.fillText(subs[i] || '', pad + 84, y + 24);
      const cx = W - pad - 26;
      if (tick <= 0) {
        // the spinner: a 270-degree arc turning
        g.strokeStyle = TO.ink(k.ink, 0.3); g.lineWidth = 5; g.lineCap = 'round';
        const a = u * 9; g.beginPath(); g.arc(cx, y, 20, a, a + 4.7); g.stroke();
      } else {
        // the check pops with a small overshoot (MOTION 4)
        const b = TO.back(tick / 0.3);
        g.save(); g.translate(cx, y); g.scale(b, b);
        g.fillStyle = k.accent; g.beginPath(); g.arc(0, 0, 24, 0, 7); g.fill();
        g.strokeStyle = '#ffffff'; g.lineWidth = 5; g.lineCap = 'round'; g.lineJoin = 'round';
        g.beginPath(); g.moveTo(-10, 1); g.lineTo(-3, 9); g.lineTo(11, -8); g.stroke();
        g.restore();
      }
      g.restore();
    });
    g.restore();
  },
});
