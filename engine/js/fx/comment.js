// fx comment: an Instagram-style comment bar over the face (a face-only slot): a placeholder, the word types in, the Post button presses, then the comment posts as a row above the bar and its heart pops red. keys: word="EDIT" type=0.5 (s, the first letter) cps=8 (letters a second) send=1.15 (s, Post presses) user="you" cy=560 (bar middle) w=880 accent=#BB6F52 ink=#18181b
// made for a test job #16 (no note)
font('SplitInter', JOB.engine + '/fonts/captions/Inter.ttf', { pins: { slnt: 0 } });
const CM = {
  ease: (x) => 1 - Math.pow(1 - Math.min(1, Math.max(0, x)), 3),
  back: (x) => { x = Math.min(1, Math.max(0, x)); const c = 1.7; return 1 + (c + 1) * Math.pow(x - 1, 3) + c * Math.pow(x - 1, 2); },
  ink: (hex, a) => { const n = parseInt(String(hex).slice(1), 16); return `rgba(${n >> 16 & 255},${n >> 8 & 255},${n & 255},${a})`; },
  // the avatar: a ring in the accent, the first letter inside
  avatar(g, x, y, r, c, letter) {
    g.strokeStyle = c; g.lineWidth = 4; g.beginPath(); g.arc(x, y, r, 0, 7); g.stroke();
    g.fillStyle = '#E9E3D6'; g.beginPath(); g.arc(x, y, r - 6, 0, 7); g.fill();
    g.fillStyle = '#16130D'; g.font = `700 ${Math.round(r * 0.8)}px SplitInter`; g.textAlign = 'center'; g.fillText(letter, x, y + 1);
  },
  heart(g, x, y, s, fill) {
    g.save(); g.translate(x, y); g.scale(s, s); g.beginPath();
    g.moveTo(0, 8); g.bezierCurveTo(-16, -2, -14, -16, -6, -16); g.bezierCurveTo(-2, -16, 0, -13, 0, -11);
    g.bezierCurveTo(0, -13, 2, -16, 6, -16); g.bezierCurveTo(14, -16, 16, -2, 0, 8); g.closePath();
    if (fill) { g.fillStyle = fill; g.fill(); } else { g.strokeStyle = 'rgba(24,24,27,0.5)'; g.lineWidth = 2.5; g.stroke(); }
    g.restore();
  },
};
FX.def('comment', { keys: { word: 'EDIT', type: 0.5, cps: 8, send: 1.15, user: 'you', cy: 560, w: 880, accent: '#BB6F52', ink: '#18181b' } }, {
  under(g, f, k) {
    const u = f.u, W = +k.w, bh = 110, word = String(k.word), send = +k.send;
    const e = CM.ease(u / 0.4), s = (0.94 + 0.06 * CM.back(u / 0.45)) * (1 + 0.012 * u);
    if (e <= 0) return;
    const n = Math.max(0, Math.min(word.length, Math.floor((u - +k.type) * +k.cps))), posted = u > send + 0.1;
    g.save(); g.globalAlpha = e; g.translate(f.W / 2, +k.cy); g.scale(s, s); g.translate(-W / 2, 0);
    g.textBaseline = 'middle';
    // the posted comment: rises in above the bar, the heart pops red 0.3 s later
    if (posted) {
      const pe = CM.ease((u - send - 0.1) / 0.3), y = -bh / 2 - 90 + 20 * (1 - pe);
      g.save(); g.globalAlpha = e * pe;
      g.shadowColor = 'rgba(0,0,0,0.18)'; g.shadowBlur = 24; g.shadowOffsetY = 10;
      g.fillStyle = '#ffffff'; g.beginPath(); g.roundRect(0, y - 56, W, 112, 28); g.fill(); g.shadowColor = 'transparent';
      CM.avatar(g, 62, y, 32, k.accent, String(k.user)[0].toUpperCase());
      g.textAlign = 'left'; g.fillStyle = CM.ink(k.ink, 0.55); g.font = '600 28px SplitInter'; g.fillText(k.user, 112, y - 18);
      g.fillStyle = k.ink; g.font = '800 40px SplitInter'; g.fillText(word, 112, y + 18);
      const hb = u > send + 0.4 ? CM.back((u - send - 0.4) / 0.3) : 0;
      CM.heart(g, W - 60, y, 1.6, null);
      if (hb > 0) CM.heart(g, W - 60, y, 1.6 * hb, '#E5484D');
      g.restore();
    }
    // the bar: avatar, the text field (placeholder, then the typed word and a caret), Post
    g.shadowColor = 'rgba(0,0,0,0.18)'; g.shadowBlur = 30; g.shadowOffsetY = 12;
    g.fillStyle = '#ffffff'; g.beginPath(); g.roundRect(0, -bh / 2, W, bh, bh / 2); g.fill(); g.shadowColor = 'transparent';
    CM.avatar(g, 62, 0, 32, k.accent, String(k.user)[0].toUpperCase());
    g.textAlign = 'left';
    const typed = posted ? '' : word.slice(0, n);
    if (typed) { g.fillStyle = k.ink; g.font = '800 42px SplitInter'; g.fillText(typed, 116, 2); }
    else { g.fillStyle = CM.ink(k.ink, 0.4); g.font = '500 34px SplitInter'; g.fillText('Add a comment...', 116, 2); }
    if (!posted && u > +k.type - 0.2 && Math.sin(u * 18) > -0.3) {
      g.font = '800 42px SplitInter'; g.fillStyle = k.accent; g.fillRect(118 + (typed ? g.measureText(typed).width : 0), -22, 3, 44);
    }
    const pd = u - send, ps = pd > 0 && pd < 0.18 ? 0.88 : 1;
    g.save(); g.translate(W - 80, 0); g.scale(ps, ps);
    g.fillStyle = typed ? k.accent : CM.ink(k.accent, 0.35); g.font = '700 34px SplitInter'; g.textAlign = 'center'; g.fillText('Post', 0, 2);
    g.restore();
    g.restore();
  },
});
