// fx zoom: a slow push-in on the video (and the person matte), linear over the line, around a point (the face). keys: from=1 to=1.08 x=540 y=700
FX.def('zoom', { keys: { from: 1, to: 1.08, x: 540, y: 700 } }, {
  video(g, f, k) {
    const s = k.from + (k.to - k.from) * f.p;
    g.translate(k.x, k.y); g.scale(s, s); g.translate(-k.x, -k.y);
  },
});
