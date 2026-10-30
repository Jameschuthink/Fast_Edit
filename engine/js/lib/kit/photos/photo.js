// kit photo: an image cover-cropped into a box (ax, ay = anchors 0-1; zoom > 1 crops tighter, e.g. to drop a corner watermark), optional rounded corners. KIT.photo(g, src, x, y, w, h, {r, ax, ay, zoom}) -> height
(function () {
  const KIT = globalThis.KIT = globalThis.KIT || {};
  KIT.photo = function (g, src, x, y, w, h, o = {}) {
    const im = img(src), s = Math.max(w / im.width, h / im.height) * (o.zoom || 1);
    const sw = w / s, sh = h / s, ax = o.ax ?? 0.5, ay = o.ay ?? 0.45;
    g.save();
    if (o.r) { g.beginPath(); g.roundRect(x, y, w, h, o.r); g.clip(); }
    g.drawImage(im, (im.width - sw) * ax, (im.height - sh) * ay, sw, sh, x, y, w, h);
    g.restore();
    return h;
  };
})();
