// fx frame: the reel in one rounded window on black (story ref, Alberta Tech) that changes shape per scene and morphs between them (morph s, ease out); the face is fit to each clip's head into the window; b-roll covers the same box (FRAME_AT). keys: boxes=[[t,x,y,w,h,r],…] (from the scene lines) faces={CLIP:[head_top,chin]} morph=0.45
const FR = (() => {
  const x = ((JOB.edit || {}).fx || []).find(x => x.name === 'frame');
  return x ? { boxes: x.keys.boxes || [], morph: x.keys.morph ?? 0.45 } : null;
})();
// the window at output second t: [x, y, w, h, r]
globalThis.FRAME_AT = (t) => {
  if (!FR || !FR.boxes.length) return null;
  const B = FR.boxes;
  let i = 0;
  while (i + 1 < B.length && t >= B[i + 1][0]) i++;
  const b = B[i].slice(1), u = (t - B[i][0]) / FR.morph;
  if (i === 0 || u >= 1) return b;
  const a = B[i - 1].slice(1), e = 1 - Math.pow(1 - Math.max(0, u), 3);
  return b.map((v, j) => a[j] + (v - a[j]) * e);
};
FX.def('frame', { keys: { morph: 0.45, faces: {} } }, {
  // the face into the window: cover it, the head 62% of its height at most 1x, the head centre at 46% of its height
  video(g, f, k) {
    const b = FRAME_AT(f.t);
    if (!b || b[2] < 1 || b[3] < 1) return;
    const segs = (JOB.edit || {}).segs || [];
    const s = segs.find(x => f.t >= x.o - 1e-6 && f.t < x.o + (x.b - x.a)) || segs[segs.length - 1];
    const fc = s && (k.faces || {})[s.clip];
    const [x, y, w, h] = b, cover = Math.max(w / f.W, h / f.H);
    const sc = fc ? Math.max(cover, Math.min(1, 0.62 * h / (fc[1] - fc[0]))) : cover;
    const mid = fc ? (fc[0] + fc[1]) / 2 : f.H / 2;
    const tx = x + (w - f.W * sc) / 2;
    const ty = Math.min(y, Math.max(y + h - f.H * sc, y + 0.46 * h - mid * sc));
    g.translate(tx, ty); g.scale(sc, sc);
  },
  under(g, f, k) {
    const b = FRAME_AT(f.t);
    if (!b) return;
    const [x, y, w, h, r] = b;
    g.fillStyle = '#000'; g.beginPath(); g.rect(0, 0, f.W, f.H);
    if (w >= 1 && h >= 1) g.roundRect(x, y, w, h, Math.min(r, w / 2, h / 2));
    g.fill('evenodd');
  },
});
