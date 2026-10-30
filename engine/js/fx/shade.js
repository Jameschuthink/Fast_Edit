// fx shade: a black layer at some opacity covers the picture from x to the right edge. keys: x=745 alpha=0.3 color=#000000
// made by note n36 (a test job)
FX.def('shade', { keys: { x: 745, alpha: 0.3, color: '#000000' } }, {
  under(g, f, k) { g.save(); g.globalAlpha = k.alpha; g.fillStyle = k.color; g.fillRect(k.x, 0, f.W - k.x, f.H); g.restore(); },
});
