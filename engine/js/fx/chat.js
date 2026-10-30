// fx chat: a Claude chat card (the look of media/AI Answer Rank: ink at alpha steps on an off-white ground, one accent): your note types into the user bubble, then Claude answers with a check. With fx split on the same span. keys: q="Move the title up a little" a="Done. I moved the title up." type=0.3 (s, typing starts) cps=22 (letters a second) reply=0.25 (s after the last letter, the answer starts) w=900 cy=590 d=0 accent=#BB6F52 ink=#18181b ground=#fcfcfb
// made by note n10 (a test job)
font('SplitInter', JOB.engine + '/fonts/captions/Inter.ttf', { pins: { slnt: 0 } });
const CH = {
  ease: (x) => 1 - Math.pow(1 - Math.min(1, Math.max(0, x)), 3),
  back: (x) => { x = Math.min(1, Math.max(0, x)); const c = 1.4; return 1 + (c + 1) * Math.pow(x - 1, 3) + c * Math.pow(x - 1, 2); },
  ink: (hex, a) => { const n = parseInt(String(hex).slice(1), 16); return `rgba(${n >> 16 & 255},${n >> 8 & 255},${n & 255},${a})`; },
  // the Claude mark: 8 round rays
  mark(g, x, y, r, c) {
    g.save(); g.translate(x, y); g.strokeStyle = c; g.lineCap = 'round'; g.lineWidth = r * 0.32;
    for (let i = 0; i < 8; i++) { const a = i * Math.PI / 4; g.beginPath(); g.moveTo(Math.cos(a) * r * 0.25, Math.sin(a) * r * 0.25); g.lineTo(Math.cos(a) * r, Math.sin(a) * r); g.stroke(); }
    g.restore();
  },
};
FX.def('chat', { keys: { q: 'Move the title up a little', a: 'Done. I moved the title up.', type: 0.3, cps: 22, reply: 0.25, w: 900, cy: 590, d: 0, accent: '#BB6F52', ink: '#18181b', ground: '#fcfcfb' } }, {
  under(g, f, k) {
    const u = f.u, W = +k.w, H = 430, pad = 44;
    const t = u - +k.d, e = CH.ease(t / 0.45), s = (0.94 + 0.06 * CH.back(t / 0.45)) * (1 + 0.012 * Math.max(0, t));
    if (e <= 0) return;
    g.save(); g.globalAlpha = e; g.translate(f.W / 2, +k.cy); g.scale(s, s); g.translate(-W / 2, -H / 2);
    g.shadowColor = 'rgba(0,0,0,0.18)'; g.shadowBlur = 34; g.shadowOffsetY = 14;
    g.fillStyle = k.ground; g.beginPath(); g.roundRect(0, 0, W, H, 28); g.fill(); g.shadowColor = 'transparent';
    g.textBaseline = 'middle';
    // the header
    CH.mark(g, pad + 18, 62, 20, k.accent);
    g.fillStyle = k.ink; g.font = '700 36px SplitInter'; g.textAlign = 'left'; g.fillText('Claude', pad + 54, 63);
    g.fillStyle = CH.ink(k.ink, 0.14); g.fillRect(0, 112, W, 2);
    // the user bubble: grows with the typed letters, a caret while typing
    const q = String(k.q), nq = Math.max(0, Math.min(q.length, Math.floor((u - +k.type) * +k.cps)));
    const typed = q.slice(0, nq), typing = u > +k.type && nq < q.length;
    g.font = '500 38px SplitInter';
    const tw = g.measureText(typed).width, bw = Math.max(90, tw + 56), bx = W - pad - bw, by = 150, bh = 76;
    if (u > +k.type - 0.1) {
      g.fillStyle = CH.ink(k.ink, 0.07); g.beginPath(); g.roundRect(bx, by, bw, bh, 26); g.fill();
      g.fillStyle = k.ink; g.fillText(typed, bx + 28, by + bh / 2 + 1);
      if (typing || (nq === 0 && u > +k.type - 0.1)) {
        if (Math.sin(u * 18) > -0.3) { g.fillStyle = k.accent; g.fillRect(bx + 30 + tw, by + 20, 3, bh - 40); }
      }
    }
    // the answer: three dots, then the line and a check
    const ta = +k.type + q.length / +k.cps + +k.reply, ay = 300;
    if (u > ta) {
      const ae = CH.ease((u - ta) / 0.3);
      g.globalAlpha = e * ae;
      CH.mark(g, pad + 18, ay, 18, k.accent);
      if (u < ta + 0.35) {
        for (let i = 0; i < 3; i++) { g.fillStyle = CH.ink(k.ink, 0.25 + 0.35 * Math.max(0, Math.sin(u * 12 - i))); g.beginPath(); g.arc(pad + 70 + i * 26, ay, 8, 0, 7); g.fill(); }
      } else {
        const le = CH.ease((u - ta - 0.35) / 0.3);
        g.globalAlpha = e * le; g.translate(0, 12 * (1 - le));
        g.fillStyle = k.ink; g.font = '500 38px SplitInter'; g.fillText(k.a, pad + 56, ay + 1);
        const cx = pad + 56 + g.measureText(k.a).width + 40;
        g.fillStyle = k.accent; g.beginPath(); g.arc(cx, ay, 20, 0, 7); g.fill();
        g.strokeStyle = '#ffffff'; g.lineWidth = 5; g.lineCap = 'round'; g.lineJoin = 'round';
        g.beginPath(); g.moveTo(cx - 9, ay + 1); g.lineTo(cx - 2, ay + 8); g.lineTo(cx + 10, ay - 7); g.stroke();
        g.fillStyle = CH.ink(k.ink, 0.08); g.beginPath(); g.roundRect(pad + 56, ay + 44, W - 2 * pad - 260, 18, 9); g.fill();
      }
    }
    g.restore();
  },
});
