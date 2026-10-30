// fx blackout: a full-screen colour covers the picture (captions and words stay). keys: color=#000000
// made by note n7 (a test job)
FX.def('blackout', { keys: { color: '#000000' } }, {
  under(g, f, k) { g.fillStyle = k.color; g.fillRect(0, 0, f.W, f.H); },
});
