// fx timeline: an editor timeline in the split panel (notes n7, n8): your clips with grey pauses and a red retake; it zooms in on a pause, the pause snaps shut, it zooms out, the retake drops out and the rest snaps together; then the timeline falls away, 3 still frames pop in (no motion), and on "motion" they start to play and motion graphics pop onto them. With fx split on the same span(s). keys: src="media/take_proxy.mp4" (thumbnails) zin=0.3 cut1=0.65 zout=1.0 cut2=1.42 drop=2.12 stills=2.24 motion=2.68 (s from the line start) w=980 cy=590 accent=#BB6F52 ink=#18181b ground=#fcfcfb
// made by note n7 (a test job)
font('SplitInter', JOB.engine + '/fonts/captions/Inter.ttf', { pins: { slnt: 0 } });
const TL_V = {};
const TL = {
  ease: (x) => 1 - Math.pow(1 - Math.min(1, Math.max(0, x)), 3),
  inout: (x) => { x = Math.min(1, Math.max(0, x)); return x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2; },
  back: (x) => { x = Math.min(1, Math.max(0, x)); const c = 1.7; return 1 + (c + 1) * Math.pow(x - 1, 3) + c * Math.pow(x - 1, 2); },
  ink: (hex, a) => { const n = parseInt(String(hex).slice(1), 16); return `rgba(${n >> 16 & 255},${n >> 8 & 255},${n & 255},${a})`; },
  // the track: [kind, length, thumbnail second]; c = a clip, p = a pause, r = a retake
  segs: [['c', 1.2, 3], ['p', 0.5], ['c', 1.6, 9], ['r', 0.9, 14], ['c', 1.0, 20], ['p', 0.45], ['c', 1.3, 27]],
};
FX.def('timeline', { keys: { src: 'media/take_proxy.mp4', zin: 0.3, cut1: 0.65, zout: 1.0, cut2: 1.42, drop: 2.12, stills: 2.24, motion: 2.68, w: 980, cy: 590, accent: '#BB6F52', ink: '#18181b', ground: '#fcfcfb' } }, {
  under(g, f, k) {
    const u = f.u, W = f.W, V = TL_V[k.src] || (TL_V[k.src] = video(k.src));
    const cw = +k.w, ch = 400, cy = +k.cy;
    // the timeline card: in over 0.35 s, falls away at drop (MOTION 3: the exit shorter than the entrance)
    const de = TL.ease((u - +k.drop) / 0.3), e = TL.ease(u / 0.35) * (1 - de);
    if (e > 0.002) {
      const s = 0.94 + 0.06 * TL.back(u / 0.45);
      g.save(); g.globalAlpha = e; g.translate(W / 2, cy + 560 * de * de); g.rotate(-0.1 * de); g.scale(s, s); g.translate(-cw / 2, -ch / 2);
      g.shadowColor = 'rgba(0,0,0,0.18)'; g.shadowBlur = 34; g.shadowOffsetY = 14;
      g.fillStyle = k.ground; g.beginPath(); g.roundRect(0, 0, cw, ch, 26); g.fill(); g.shadowColor = 'transparent';
      g.textBaseline = 'middle'; g.textAlign = 'left'; g.fillStyle = k.ink; g.font = '700 32px SplitInter'; g.fillText('Timeline', 36, 46);
      ['#FF5F57', '#FEBC2E', '#28C840'].forEach((c, i) => { g.fillStyle = c; g.beginPath(); g.arc(cw - 110 + i * 28, 46, 8, 0, 7); g.fill(); });
      g.fillStyle = TL.ink(k.ink, 0.12); g.fillRect(0, 84, cw, 2);
      // each gap's cut: 0 -> 1 with a small overshoot, so the clips snap and settle (MOTION 4)
      const cut = (i) => i === 1 ? TL.back((u - +k.cut1) / 0.3) : i === 3 ? TL.back((u - +k.cut2) / 0.3) : i === 5 ? TL.back((u - +k.cut2 - 0.08) / 0.3) : 0;
      const total0 = TL.segs.reduce((a, s) => a + s[1], 0), unit = (cw - 120) / total0;
      const ws = TL.segs.map((s, i) => s[1] * unit * (1 - cut(i)));
      const x0 = (cw - ws.reduce((a, b) => a + b, 0)) / 2;
      // the zoom: in on the first pause, then out again (one morph, eased both ways)
      const zi = TL.inout((u - +k.zin) / 0.3), zo = TL.inout((u - +k.zout) / 0.35), mix = zi * (1 - zo), z = 1 + 1.3 * mix;
      const fxp = x0 + ws[0] + ws[1] / 2, Fx = cw / 2 + (fxp - cw / 2) * mix, ty = 230;
      g.save(); g.beginPath(); g.rect(20, 88, cw - 40, ch - 100); g.clip();
      g.translate(cw / 2, ty); g.scale(z, z); g.translate(-Fx, -ty);
      // the ruler
      g.fillStyle = TL.ink(k.ink, 0.25);
      for (let x = x0 % (unit / 2); x < cw; x += unit / 2) g.fillRect(x, 104, 2, Math.round((x - x0) / (unit / 2)) % 2 ? 8 : 16);
      // the clips, pauses and the retake; the audio under them
      let x = x0;
      TL.segs.forEach((sg, i) => {
        const w = ws[i];
        if (w > 1) {
          const kind = sg[0], dropR = kind === 'r' ? TL.ease((u - +k.cut2) / 0.3) : 0;
          g.save(); g.globalAlpha = 1 - dropR; g.translate(0, 80 * dropR);
          if (kind === 'p') {
            g.fillStyle = TL.ink(k.ink, 0.06); g.beginPath(); g.roundRect(x + 2, 140, w - 4, 100, 10); g.fill();
            g.save(); g.clip(); g.strokeStyle = TL.ink(k.ink, 0.14); g.lineWidth = 3;
            for (let hx = x - 100; hx < x + w; hx += 16) { g.beginPath(); g.moveTo(hx, 240); g.lineTo(hx + 100, 140); g.stroke(); }
            g.restore();
            if (w > 70) { g.fillStyle = TL.ink(k.ink, 0.5); g.font = '600 22px SplitInter'; g.textAlign = 'center'; g.fillText('pause', x + w / 2, 190); }
            g.fillStyle = TL.ink(k.ink, 0.15); g.fillRect(x + 2, 278, w - 4, 3);
          } else {
            const fr = V.frame(sg[2]), th = 100, tw = fr.width * th / fr.height;
            g.save(); g.beginPath(); g.roundRect(x + 2, 140, w - 4, 100, 10); g.clip();
            for (let tx = x + 2; tx < x + w; tx += tw) g.drawImage(fr, tx, 140, tw, th);
            if (kind === 'r') { g.fillStyle = 'rgba(229,72,77,0.45)'; g.fillRect(x, 140, w, 100); }
            g.restore();
            if (kind === 'r' && w > 70) { g.fillStyle = '#ffffff'; g.font = '700 24px SplitInter'; g.textAlign = 'center'; g.fillText('retake', x + w / 2, 190); }
            // the audio: bars from a fixed pattern (no random, MOTION 1)
            g.fillStyle = kind === 'r' ? 'rgba(229,72,77,0.6)' : k.accent;
            for (let b = 0, bx = x + 6; bx < x + w - 6; b++, bx += 9) {
              const h = 8 + 34 * Math.abs(Math.sin(b * 0.7 + i) * Math.cos(b * 0.23));
              g.fillRect(bx, 280 - h / 2, 5, h);
            }
          }
          g.restore();
        }
        x += w;
      });
      // the playhead
      const px = x0 + 40 + 60 * u;
      g.fillStyle = k.accent; g.fillRect(px - 1.5, 112, 3, 210);
      g.beginPath(); g.moveTo(px - 12, 108); g.lineTo(px + 12, 108); g.lineTo(px, 124); g.closePath(); g.fill();
      g.restore();
      g.restore();
    }
    // the still frames: pop in still; on "motion" they play, push in, and graphics pop onto them
    if (u > +k.stills) {
      const fw = 230, fh = fw * 16 / 9, secs = [5, 16, 30];
      for (let i = 0; i < 3; i++) {
        const t = u - +k.stills - i * 0.07, pe = TL.ease(t / 0.35);
        if (pe <= 0) continue;
        const mo = u - +k.motion - i * 0.08, fr = V.frame(secs[i] + Math.max(0, mo));
        const sc = 0.92 + 0.08 * TL.back(t / 0.4), z2 = 1 + 0.08 * TL.ease(mo / 0.8);
        g.save(); g.globalAlpha = pe; g.translate(W / 2 + (i - 1) * 270, cy + 20 + 30 * (1 - pe)); g.rotate((i - 1) * 3 * Math.PI / 180); g.scale(sc, sc);
        g.shadowColor = 'rgba(0,0,0,0.25)'; g.shadowBlur = 26; g.shadowOffsetY = 12;
        g.fillStyle = '#ffffff'; g.beginPath(); g.roundRect(-fw / 2 - 6, -fh / 2 - 6, fw + 12, fh + 12, 20); g.fill(); g.shadowColor = 'transparent';
        g.save(); g.beginPath(); g.roundRect(-fw / 2, -fh / 2, fw, fh, 14); g.clip();
        const c = Math.max(fw / fr.width, fh / fr.height) * z2;
        g.drawImage(fr, -fr.width * c / 2, -fr.height * c / 2, fr.width * c, fr.height * c);
        if (mo > 0) {
          const me = TL.ease(mo / 0.3), mb = TL.back(mo / 0.35);
          g.globalAlpha = pe * me; g.textAlign = 'center'; g.textBaseline = 'middle';
          if (i === 0) {
            // a caption pill rises in at the bottom
            g.translate(0, 20 * (1 - me)); g.fillStyle = 'rgba(0,0,0,0.75)'; g.beginPath(); g.roundRect(-96, fh / 2 - 82, 192, 44, 22); g.fill();
            g.fillStyle = '#ffffff'; g.font = '700 22px SplitInter'; g.fillText('cut every pause', 0, fh / 2 - 59);
          } else if (i === 1) {
            // a title chip scales in at the top
            g.translate(0, -fh / 2 + 56); g.scale(mb, mb); g.fillStyle = k.accent; g.beginPath(); g.roundRect(-80, -24, 160, 48, 14); g.fill();
            g.fillStyle = '#ffffff'; g.font = '700 26px SplitInter'; g.fillText('Step 2.', 0, 1);
          } else {
            // a big word pops in the middle
            g.scale(mb, mb); g.shadowColor = 'rgba(0,0,0,0.5)'; g.shadowBlur = 12;
            g.fillStyle = '#ffffff'; g.font = '800 44px SplitInter'; g.fillText('MOTION', 0, -20);
          }
        }
        g.restore();
        g.restore();
      }
    }
  },
});
