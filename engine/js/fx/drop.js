// fx drop: clips drop into a folder (style ref De2SEI7JP8r 51.7 s): clip cards (9:16, playing, a REC dot) pop in, a blue folder pops in, each card arcs into the folder behind its front, the folder bumps on each landing, the label goes from accent to ink when the last one lands. With fx split on the same span. keys: src="media/TAKE.mp4" from="2|12|24" (one clip second per card) label="clips/" at=0.08 (s, the cards pop) fold=0.3 (s, the folder pops) fly=0.42 (s, the first card leaves) gap=0.1 (s between cards) cy=580 (folder middle) accent=#BB6F52 ink=#16130D blue=#5AA9E6
// made by note n5 (a test job)
font('SplitInter', JOB.engine + '/fonts/captions/Inter.ttf', { pins: { slnt: 0 } });
const DR_V = {};
const DR = {
  ease: (x) => 1 - Math.pow(1 - Math.min(1, Math.max(0, x)), 3),
  inout: (x) => { x = Math.min(1, Math.max(0, x)); return x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2; },
  back: (x) => { x = Math.min(1, Math.max(0, x)); const c = 1.5; return 1 + (c + 1) * Math.pow(x - 1, 3) + c * Math.pow(x - 1, 2); },
};
FX.def('drop', { keys: { src: '', from: '2|12|24', label: 'clips/', at: 0.08, fold: 0.3, fly: 0.42, gap: 0.1, cy: 580, accent: '#BB6F52', ink: '#16130D', blue: '#5AA9E6' } }, {
  under(g, f, k) {
    const u = f.u, W = f.W, froms = String(k.from).split('|').map(Number), n = froms.length;
    const V = DR_V[k.src] || (DR_V[k.src] = video(k.src));
    const fx = W / 2 + 150, fy = +k.cy + 60, fw = 330, fh = 240, flyT = 0.4;
    const land = (i) => +k.fly + i * +k.gap + flyT;
    // the folder: pops in with a small overshoot, a 4% bump on each landing
    const fe = DR.ease((u - +k.fold) / 0.35);
    let bump = 0;
    for (let i = 0; i < n; i++) { const d = u - land(i); if (d > 0 && d < 0.2) bump = Math.max(bump, Math.sin(Math.PI * d / 0.2)); }
    const fs = (0.9 + 0.1 * DR.back((u - +k.fold) / 0.4)) * (1 + 0.04 * bump);
    const folder = (part) => {
      if (fe <= 0) return;
      g.save(); g.globalAlpha = fe; g.translate(fx, fy); g.scale(fs, fs);
      if (part === 'back') {
        g.shadowColor = 'rgba(0,0,0,0.18)'; g.shadowBlur = 30; g.shadowOffsetY = 12;
        g.fillStyle = k.blue; g.beginPath(); g.roundRect(-fw / 2, -fh / 2 - 30, 120, 50, 14); g.fill();
        g.beginPath(); g.roundRect(-fw / 2, -fh / 2, fw, fh, 20); g.fill(); g.shadowColor = 'transparent';
        g.fillStyle = '#ffffff'; g.beginPath(); g.roundRect(-fw / 2 + 22, -fh / 2 + 16, fw - 44, fh - 40, 8); g.fill();
      } else {
        g.fillStyle = '#8CCBF5'; g.beginPath();
        g.moveTo(-fw / 2 + 8, -fh / 2 + 70); g.lineTo(fw / 2, -fh / 2 + 56); g.lineTo(fw / 2 - 10, fh / 2 - 16);
        g.quadraticCurveTo(fw / 2 - 12, fh / 2, fw / 2 - 30, fh / 2); g.lineTo(-fw / 2 + 20, fh / 2);
        g.quadraticCurveTo(-fw / 2, fh / 2, -fw / 2, fh / 2 - 20); g.closePath(); g.fill();
        const done = u > land(n - 1) + 0.05;
        g.font = '700 42px SplitInter'; g.textAlign = 'center'; g.textBaseline = 'middle';
        g.fillStyle = done ? k.ink : k.accent; g.fillText(k.label, 0, fh / 2 + 48);
      }
      g.restore();
    };
    folder('back');
    // the cards: pop in fanned at the left, then each arcs into the folder (the front hides its lower part)
    const cw = 170, ch = cw * 16 / 9;
    for (let i = n - 1; i >= 0; i--) {
      const pe = DR.ease((u - +k.at - i * 0.06) / 0.35);
      if (pe <= 0) continue;
      const m = DR.inout((u - (+k.fly + i * +k.gap)) / flyT);
      const x0 = W / 2 - 230 + i * 36, y0 = +k.cy - 40 + i * 14, r0 = (-6 + i * 6) * Math.PI / 180;
      const x1 = fx, y1 = fy - 20;
      const x = x0 + (x1 - x0) * m, y = y0 + (y1 - y0) * m - Math.sin(Math.PI * m) * 120;
      const s = (0.92 + 0.08 * DR.back((u - +k.at - i * 0.06) / 0.4)) * (1 - 0.55 * m);
      const fr = V.frame(Math.max(0, froms[i] + u));
      g.save(); g.globalAlpha = pe; g.translate(x, y); g.rotate(r0 * (1 - m)); g.scale(s, s);
      g.shadowColor = 'rgba(0,0,0,0.25)'; g.shadowBlur = 20; g.shadowOffsetY = 8;
      g.fillStyle = '#ffffff'; g.beginPath(); g.roundRect(-cw / 2 - 5, -ch / 2 - 5, cw + 10, ch + 10, 16); g.fill(); g.shadowColor = 'transparent';
      g.save(); const c = Math.max(cw / fr.width, ch / fr.height);
      g.beginPath(); g.roundRect(-cw / 2, -ch / 2, cw, ch, 12); g.clip();
      g.drawImage(fr, -fr.width * c / 2, -fr.height * c / 2, fr.width * c, fr.height * c); g.restore();
      g.fillStyle = '#E5484D'; g.beginPath(); g.arc(-cw / 2 + 20, -ch / 2 + 20, 7, 0, 7); g.fill();
      g.fillStyle = '#ffffff'; g.font = '700 18px SplitInter'; g.textAlign = 'left'; g.textBaseline = 'middle';
      g.fillText('REC', -cw / 2 + 32, -ch / 2 + 21);
      g.restore();
    }
    folder('front');
  },
});
