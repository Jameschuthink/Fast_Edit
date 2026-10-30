// fx flash: the person turns into a solid white silhouette on single frames (the "flash transition": white, normal, white), for a cut or the first word. keys: seq=X.X color=#ffffff soft=4
// needs: matte
// seq: one character per frame from the line's start, X = the person flashes, . = the normal picture. soft = feather px.
FX.def('flash', { keys: { seq: 'X.X', color: '#ffffff', soft: 4 } }, {
  under(g, f, k) {
    const fps = JOB.fps || 30, i = Math.round(f.t * fps) - Math.round(f.t0 * fps);
    if (String(k.seq)[i] !== 'X') return;
    const m = f.matte();
    if (!m) return;
    g.beginLayer(k.soft ? { filter: `blur(${k.soft}px)` } : {});
    g.drawImage(m, 0, 0, f.W, f.H); g.drawImage(m, 0, 0, f.W, f.H);   // twice: a solid body where the matte is thin
    g.globalCompositeOperation = 'source-in';
    g.fillStyle = k.color; g.fillRect(0, 0, f.W, f.H);
    g.endLayer();
  },
});
