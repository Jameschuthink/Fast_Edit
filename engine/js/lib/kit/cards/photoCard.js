// kit photoCard: card with 1-2 photos on top ("+" disc between two), then date label, heading, sub line. KIT.photoCard(g, x, y, w, {date, photos, title, sub}, {h, ph, dir}) -> height; a photo = "file" or {src, ax, ay, zoom}
(function () {
  const KIT = globalThis.KIT = globalThis.KIT || {};
  KIT.photoCard = function (g, x, y, w, d, o = {}) {
    const C = LOOK.color, h = o.h || 340, ph = o.ph || 226, r = 22, pad = 20, dir = o.dir || '';
    g.save();
    g.beginPath(); g.roundRect(x, y, w, h, r); g.clip();
    g.fillStyle = C.card; g.fillRect(x, y, w, h);
    const p = d.photos || [];
    const pic = (e, px, pw) => typeof e === 'string' ? KIT.photo(g, dir + e, px, y, pw, ph) : KIT.photo(g, dir + e.src, px, y, pw, ph, e);
    if (p.length === 1) pic(p[0], x, w);
    else if (p.length === 2) {
      const hw = (w - 6) / 2;
      g.fillStyle = C.ground; g.fillRect(x + hw, y, 6, ph);
      pic(p[0], x, hw);
      pic(p[1], x + hw + 6, hw);
      g.fillStyle = C.ground; g.beginPath(); g.arc(x + w / 2, y + ph / 2, 24, 0, Math.PI * 2); g.fill();
      g.font = LOOK.font('text', 28, 800); g.fillStyle = C.accent; g.textAlign = 'center'; g.textBaseline = 'middle';
      g.fillText('+', x + w / 2, y + ph / 2 - 2);
    }
    g.restore();
    g.save();
    let ty = y + ph + 18;
    if (d.date) ty += KIT.label(g, x + pad, ty, d.date) + 4;
    g.textAlign = 'left'; g.textBaseline = 'top';
    let px = 28;  // the heading shrinks to stay on one line (min 22), so the sub line stays inside the card
    g.font = LOOK.font('display', px, 750);
    while (px > 22 && g.measureText(d.title).width > w - 2 * pad) g.font = LOOK.font('display', --px, 750);
    g.fillStyle = C.accent; g.fillText(d.title, x + pad, ty + (28 - px) / 2); ty += 28 * 1.12;
    if (d.sub) { g.font = LOOK.font('text', 23, 600); g.fillStyle = C.accent2; g.fillText(d.sub, x + pad, ty + 2); }
    g.restore();
    return h;
  };
})();
