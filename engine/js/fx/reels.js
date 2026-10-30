// fx reels: a hook title over the face, then 3 video cards (9:16, playing, no sound) pop in above the head, fanned, one after another, with a small overshoot, then sway; the title leaves when the first card pops. For a face-only slot (no split). The title is its own part (the page drags it alone: nudge.title=). keys: title="Let Claude edit" (the big line) small="(in 30 seconds)" (the small bracket, line 2 left) title2="like a $300 editor" (line 2) smallscale=0.58 (the small bracket size, x the line 2 size; 1 = the same size) fit=980 (px, the widest line) srcs="media/refs/a.mp4|media/refs/b.mp4|media/refs/c.mp4" from="0|0|0" (clip seconds at the pop) at=2.2 (s from the line start, the first pop) gap=0.08 (s between pops) ty=190 (title middle) cy=520 (cards middle) w=230 (card px) dx=280 (card spacing) tilt=5 (deg) size=150 out=0.25 (s, the title exit)
// made by note n2, title part + exit by note n14 (a test job)
font('SplitInter', JOB.engine + '/fonts/captions/Inter.ttf', { pins: { slnt: 0 } });
font('TitleSerif', JOB.engine + '/fonts/captions/apple_garamond/AppleGaramond.ttf');
const RL_V = {};
const RL = {
  ease: (x) => 1 - Math.pow(1 - Math.min(1, Math.max(0, x)), 3),
  back: (x) => { x = Math.min(1, Math.max(0, x)); const c = 1.7; return 1 + (c + 1) * Math.pow(x - 1, 3) + c * Math.pow(x - 1, 2); },
};
FX.def('reels', { keys: { title: 'Let Claude edit', small: '', title2: '', smallscale: 0.58, fit: 980, srcs: '', from: '0|0|0', at: 2.2, gap: 0.08, ty: 190, cy: 520, w: 230, dx: 280, tilt: 5, size: 150, out: 0.25 } }, {
  under(g, f, k) {
    const u = f.u, W = f.W;
    // the title leads (MOTION 5): rise 30 px + fade over 0.35 s, a slow push; it fades and drifts up as the first card pops (MOTION 3: exit ~70% of the entrance)
    const xo = RL.ease((u - +k.at) / +k.out);
    if (k.title && xo < 1) f.part(g, 'title', () => {
      const e = RL.ease(u / 0.35) * (1 - xo);
      g.save(); g.globalAlpha = e; g.translate(W / 2, +k.ty + 30 * (1 - RL.ease(u / 0.35)) - 18 * xo); g.scale(1 + 0.01 * u, 1 + 0.01 * u);
      // the ref composition (media/refs/title_composition.png): a big line, under it a small (bracket) + a medium line, as one block
      g.textBaseline = 'alphabetic'; g.shadowColor = 'rgba(0,0,0,0.4)'; g.shadowBlur = 16; g.shadowOffsetY = 3; g.fillStyle = '#ffffff';
      const big = +k.size, mid = big * 0.62, sm = mid * +k.smallscale;
      g.font = `${big}px TitleSerif`; const w1 = g.measureText(k.title).width;
      g.font = `${sm}px TitleSerif`; const ws = k.small ? g.measureText(k.small).width + sm * 0.4 : 0;
      g.font = `${mid}px TitleSerif`; const w2 = k.title2 ? g.measureText(k.title2).width : 0;
      const fit = Math.min(1, +k.fit / Math.max(w1, ws + w2)); g.scale(fit, fit);
      g.textAlign = 'center'; g.font = `${big}px TitleSerif`; g.fillText(k.title, 0, 0);
      if (k.title2) {
        const x0 = -(ws + w2) / 2, y2 = big * 0.78;
        g.textAlign = 'left';
        if (k.small) { g.font = `${sm}px TitleSerif`; g.fillText(k.small, x0, y2 - (mid - sm) * 0.52); }
        g.font = `${mid}px TitleSerif`; g.fillText(k.title2, x0 + ws, y2);
      }
      g.restore();
    });
    // the cards: outer ones first, the middle (the hero) last and on top
    const srcs = String(k.srcs).split('|').filter(Boolean), froms = String(k.from).split('|').map(Number);
    const order = srcs.length === 3 ? [0, 2, 1] : srcs.map((_, i) => i);
    const cw = +k.w, ch = cw * 16 / 9;
    order.forEach((i, n) => {
      const t = u - (+k.at + n * +k.gap), e = RL.ease(t / 0.4);
      if (e <= 0) return;
      const V = RL_V[srcs[i]] || (RL_V[srcs[i]] = video(srcs[i]));
      const fr = V.frame(Math.max(0, (froms[i] || 0) + t)), side = i - (srcs.length - 1) / 2;
      const s = (0.9 + 0.1 * RL.back(t / 0.45)) * (1 + 0.012 * Math.max(0, t));
      const rot = (side * +k.tilt + 1.2 * Math.sin(1.6 * t + i)) * Math.PI / 180;
      g.save(); g.globalAlpha = e;
      g.translate(W / 2 + side * +k.dx, +k.cy + Math.abs(side) * 24 + 40 * (1 - e)); g.rotate(rot); g.scale(s, s);
      g.shadowColor = 'rgba(0,0,0,0.35)'; g.shadowBlur = 30; g.shadowOffsetY = 14;
      g.fillStyle = '#ffffff'; g.beginPath(); g.roundRect(-cw / 2 - 6, -ch / 2 - 6, cw + 12, ch + 12, 24); g.fill(); g.shadowColor = 'transparent';
      const c = Math.max(cw / fr.width, ch / fr.height);
      g.beginPath(); g.roundRect(-cw / 2, -ch / 2, cw, ch, 18); g.clip();
      g.drawImage(fr, -fr.width * c / 2, -fr.height * c / 2, fr.width * c, fr.height * c);
      g.restore();
    });
  },
});
