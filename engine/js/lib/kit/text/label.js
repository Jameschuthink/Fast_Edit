// kit label: one small bold caps line (kicker, section name). KIT.label(g, x, y, text, {size, color, align}) -> height
(function () {
  const KIT = globalThis.KIT = globalThis.KIT || {};
  KIT.label = function (g, x, y, text, o = {}) {
    const px = o.size || 20;
    g.save();
    g.font = LOOK.font('text', px, 700); g.fillStyle = o.color || LOOK.color.accent2;
    g.textAlign = o.align || 'left'; g.textBaseline = 'top';
    g.fillText(text, x, y);
    g.restore();
    return Math.round(px * 1.2);
  };
})();
