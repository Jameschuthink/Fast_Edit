// kit page offer: header, 2 price cards (none = no row), section label, photo-card grid, thank-you line; prints "bottom Y / H". KIT.offer(g, d, {cols, dir, h, ph}) (h, ph = card and photo height)
(function () {
  const KIT = globalThis.KIT = globalThis.KIT || {};
  let told = false;
  KIT.offer = function (g, d, o = {}) {
    const C = LOOK.color, S = LAYOUT.safe, W = LAYOUT.W, H = LAYOUT.H;
    const x = S.x, w = S.w, cols = o.cols || 3, gap = 20;
    g.fillStyle = C.ground; g.fillRect(0, 0, W, H);
    let y = 50;
    y += KIT.header(g, x, y, w, d) + 30;
    const pw = (w - 24) / 2;
    (d.prices || []).forEach((p, i) => KIT.priceCard(g, x + i * (pw + 24), y, pw, p));
    if (d.prices?.length) y += 156 + 30;
    if (d.section) y += KIT.label(g, x, y, d.section) + 12;
    const cw = (w - (cols - 1) * gap) / cols;
    let rowH = 0;
    (d.cards || []).forEach((c, k) => {
      if (k && k % cols === 0) { y += rowH + gap; rowH = 0; }
      rowH = Math.max(rowH, KIT.photoCard(g, x + (k % cols) * (cw + gap), y, cw, c, { dir: o.dir, h: o.h, ph: o.ph }));
    });
    y += rowH + 26;
    if (d.thanks) {
      g.font = LOOK.font('text', 23, 600); g.fillStyle = C.accent; g.textAlign = 'center'; g.textBaseline = 'top';
      g.fillText(d.thanks, W / 2, y); y += 30;
    }
    if (!told) { console.log(`bottom ${Math.round(y)} / ${H}`); told = true; }
    return y;
  };
})();
