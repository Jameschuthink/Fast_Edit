// fx phone: a video playing inside a phone frame picture (a PNG with a see-through screen), in the split panel: it pops in (small overshoot), then a slow push and a small sway. keys: src="media/CLIP.mp4" from=0 rate=1 frame="media/phone frame.png" screen="x,y,w,h" (the frame's screen, image px) r=300 (screen corner, image px) h=700 (phone height px) cy=600 d=0
// made by note n12 (a test job)
const PH_I = {}, PH_V = {};
const PH = {
  ease: (x) => 1 - Math.pow(1 - Math.min(1, Math.max(0, x)), 3),
  back: (x) => { x = Math.min(1, Math.max(0, x)); const c = 1.5; return 1 + (c + 1) * Math.pow(x - 1, 3) + c * Math.pow(x - 1, 2); },
};
FX.def('phone', { keys: { src: '', from: 0, rate: 1, frame: 'media/phone frame.png', screen: '71,64,2450,5442', r: 300, h: 700, cy: 600, d: 0 } }, {
  under(g, f, k) {
    const I = PH_I[k.frame] || (PH_I[k.frame] = img(k.frame)), V = PH_V[k.src] || (PH_V[k.src] = video(k.src));
    const t = f.u - +k.d, e = PH.ease(t / 0.45);
    if (e <= 0) return;
    const sc = +k.h / I.height, s = (0.92 + 0.08 * PH.back(t / 0.5)) * (1 + 0.015 * Math.max(0, t));
    const [sx, sy, sw, sh] = String(k.screen).split(',').map(Number);
    const fr = V.frame(Math.max(0, +k.from + Math.max(0, t) * +k.rate)), c = Math.max(sw / fr.width, sh / fr.height);
    g.save(); g.globalAlpha = e;
    g.translate(f.W / 2, +k.cy + 40 * (1 - e)); g.rotate(1.2 * Math.sin(1.4 * t) * Math.PI / 180); g.scale(s * sc, s * sc);
    g.translate(-I.width / 2, -I.height / 2);
    g.shadowColor = 'rgba(0,0,0,0.3)'; g.shadowBlur = 40 / sc; g.shadowOffsetY = 16 / sc;
    g.fillStyle = '#000'; g.beginPath(); g.roundRect(sx, sy, sw, sh, +k.r); g.fill(); g.shadowColor = 'transparent';
    g.save(); g.beginPath(); g.roundRect(sx, sy, sw, sh, +k.r); g.clip();
    g.drawImage(fr, sx + (sw - fr.width * c) / 2, sy + (sh - fr.height * c) / 2, fr.width * c, fr.height * c);
    g.restore();
    g.drawImage(I, 0, 0, I.width, I.height);
    g.restore();
  },
});
