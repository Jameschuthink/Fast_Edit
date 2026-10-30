// fx chipstack: white chips with black words, all the same width, stacked and centred on x; each pops in on its second. keys: items="a|b|c" at="0|1|2" x=340 y=400 w=564 h=94 size=56 gap=26
// made by note n33 (a test job)
FX.def('chipstack', { keys: { items: '', at: '', x: 340, y: 400, w: 564, h: 94, size: 56, gap: 26 } }, {
  over(g, f, k) {
    const it = String(k.items).split('|'), at = String(k.at).split('|').map(Number);
    const x = +k.x, y = +k.y, w = +k.w, h = +k.h, gap = +k.gap;
    g.font = (+k.size) + 'px MontBlack'; g.textAlign = 'center'; g.textBaseline = 'middle';
    it.forEach((s, i) => {
      const e = Math.min(1, Math.max(0, (f.u - at[i]) / 0.15));
      if (e <= 0) return;
      const sc = 0.8 + 0.2 * e;
      g.save(); g.globalAlpha = e; g.translate(x, y + i * (h + gap)); g.scale(sc, sc);
      g.fillStyle = '#ffffff'; g.fillRect(-w / 2, -h / 2, w, h);
      g.fillStyle = '#000000'; g.fillText(s, 0, 0);
      g.restore();
    });
  },
});
