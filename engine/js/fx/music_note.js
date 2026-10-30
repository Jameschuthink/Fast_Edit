// fx music_note: a white music note pops in, bobs and sways, then fades out. keys: x=1440 y=395 size=420 color=#ffffff
// made by note n36 (a test job)
FX.def('music_note', { keys: { x: 1440, y: 395, size: 420, color: '#ffffff' } }, {
  over(g, f, k) {
    const s = k.size / 420;
    const a = Math.min(1, f.u / 0.15) * Math.min(1, (f.dur - f.u) / 0.15);
    const pop = 0.6 + 0.4 * Math.min(1, f.u / 0.2);
    const bob = Math.sin(f.u * 12.56) * 10;
    g.save();
    g.globalAlpha = Math.max(0, a);
    g.translate(k.x, k.y + bob);
    g.scale(s * pop, s * pop);
    g.rotate(Math.sin(f.u * 9.4) * 0.08);
    g.fillStyle = k.color; g.strokeStyle = k.color; g.lineCap = 'round'; g.lineWidth = 18;
    g.beginPath(); g.ellipse(-55, 165, 70, 52, -0.35, 0, 6.28); g.fill();
    g.beginPath(); g.moveTo(10, 150); g.lineTo(10, -210); g.stroke();
    g.beginPath(); g.moveTo(10, -210); g.quadraticCurveTo(45, -120, 100, -80); g.stroke();
    g.restore();
  },
});
