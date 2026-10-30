// fx pip: a clip in a square-cornered box above the middle of the frame (where the eye looks, over the forehead), hard cut in and out, under the words and captions. keys: src="media/broll/FILE.mp4" from=0 rate=1 w=640 (box width px) y=620 (box middle, world px)
const PIP_V = {};
FX.def('pip', { keys: { src: '', from: 0, rate: 1, w: 640, y: 620 } }, {
  under(g, f, k) {
    const V = PIP_V[k.src] || (PIP_V[k.src] = video(k.src));
    const fr = V.frame(Math.max(0, k.from + f.u * k.rate)), h = fr.height * k.w / fr.width;
    g.drawImage(fr, (f.W - k.w) / 2, k.y - h / 2, k.w, h);
  },
});
