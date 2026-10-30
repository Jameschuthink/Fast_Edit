// fx scan: a bright line sweeps down the picture with a soft glow trail behind it. keys: band=260 alpha=0.35 color=#78e6ff
// made by note n4 (a test job)
FX.def('scan', { keys: { band: 260, alpha: 0.35, color: '#78e6ff' } }, {
  under(g, f, k) {
    const y = -k.band + f.p * (f.H + k.band);
    g.save();
    const gr = g.createLinearGradient(0, y - k.band, 0, y);
    gr.addColorStop(0, 'rgba(120,230,255,0)');
    gr.addColorStop(1, 'rgba(120,230,255,' + k.alpha + ')');
    g.fillStyle = gr;
    g.fillRect(0, y - k.band, f.W, k.band);
    g.globalAlpha = 0.9;
    g.fillStyle = k.color;
    g.fillRect(0, y - 3, f.W, 6);
    g.restore();
  },
});
