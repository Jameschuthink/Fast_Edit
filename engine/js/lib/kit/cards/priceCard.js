// kit priceCard: rounded card with a label, a big price and one line under it; dark = the filled one. KIT.priceCard(g, x, y, w, {label, price, line, dark}) -> height
(function () {
  const KIT = globalThis.KIT = globalThis.KIT || {};
  KIT.priceCard = function (g, x, y, w, d, o = {}) {
    const C = LOOK.color, h = o.h || 156, pad = 30;
    g.save();
    g.fillStyle = d.dark ? C.accent : C.card; g.beginPath(); g.roundRect(x, y, w, h, 26); g.fill();
    KIT.label(g, x + pad, y + 22, d.label, { color: d.dark ? C.leaf : C.accent2 });
    g.textBaseline = 'top'; g.textAlign = 'left';
    g.font = LOOK.font('display', 58, 800); g.fillStyle = d.dark ? C.gold : C.accent; g.fillText(d.price, x + pad, y + 50);
    g.font = LOOK.font('text', 23, 500); g.fillStyle = d.dark ? C.light : C.ink; g.fillText(d.line, x + pad, y + 118);
    g.restore();
    return h;
  };
})();
