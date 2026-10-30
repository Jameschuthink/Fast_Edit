// fx broll: a video clip as a cut-away over the picture (under the words and captions). keys: src="refs/videos/FILE.mp4" (from the job folder) from=0 (clip second at the line's start) fill=1 (1 = width on black, 0 = cover the whole frame) rate=1 (clip speed; 0.5 = half), w=0 (a width in px, the clip centred on black; 0 = fit) box="x,y,w,h" (the clip covers that box: a frame line's window)
const BR_V = {};
FX.def('broll', { keys: { src: '', from: 0, fill: 1, w: 0, rate: 1, box: '' } }, {
  under(g, f, k) {
    const V = BR_V[k.src] || (BR_V[k.src] = video(k.src));
    const fr = V.frame(Math.max(0, k.from + f.u * k.rate)), s = k.w ? k.w / fr.width : k.fill ? Math.min(f.W / fr.width, f.H / fr.height) : Math.max(f.W / fr.width, f.H / fr.height);
    const w = fr.width * s, h = fr.height * s;
    g.fillStyle = '#000'; g.fillRect(0, 0, f.W, f.H);
    if (k.box) {   // cover the box, centred (the frame fx rounds its corners)
      const [bx, by, bw, bh] = (globalThis.FRAME_AT && FRAME_AT(f.t)) || String(k.box).split(',').map(Number), c = Math.max(bw / fr.width, bh / fr.height);
      if (bw < 1 || bh < 1) return;
      g.save(); g.beginPath(); g.rect(bx, by, bw, bh); g.clip();
      g.drawImage(fr, bx + (bw - fr.width * c) / 2, by + (bh - fr.height * c) / 2, fr.width * c, fr.height * c);
      g.restore(); return;
    }
    g.drawImage(fr, (f.W - w) / 2, (f.H - h) / 2, w, h);
  },
});
