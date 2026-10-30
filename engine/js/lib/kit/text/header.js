// kit header: kicker label, big title, 2nd title line, sub paragraph, centered in a width. KIT.header(g, x, y, w, {kicker, title, title2, sub}) -> height
(function () {
  const KIT = globalThis.KIT = globalThis.KIT || {};
  KIT.header = function (g, x, y, w, d, o = {}) {
    const C = LOOK.color, cx = x + w / 2, y0 = y;
    let big = o.title || 76;
    const big2 = o.title2 || 40, body = o.sub || 27;
    if (d.kicker) y += KIT.label(g, cx, y, d.kicker, { align: 'center' }) + 14;
    g.save(); g.textAlign = 'center'; g.textBaseline = 'top';
    const line = (s, font, color, lh, maxW) => {
      g.font = font; g.fillStyle = color;
      return g.fillTextBox(s, x + (w - maxW) / 2, y, maxW, { align: 'center', lineHeight: lh }).height;
    };
    if (d.title) {  // the title shrinks to stay on one line (min 0.75 of its size)
      g.font = LOOK.font('display', big, 800);
      const fit = Math.max(0.75, Math.min(1, w / g.measureText(d.title).width));
      big = Math.floor(big * fit);
    }
    if (d.title) y += line(d.title, LOOK.font('display', big, 800), C.accent, 1.05, w);
    if (d.title2) y += 2, y += line(d.title2, LOOK.font('display', big2, 600), C.accent2, 1.1, w);
    if (d.sub) y += 22, y += line(d.sub, LOOK.font('text', body, 400), C.ink, 1.35, w - 140);
    g.restore();
    return y - y0;
  };
})();
