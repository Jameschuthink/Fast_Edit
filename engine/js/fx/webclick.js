// fx webclick: a web page (a screenshot, cropped) in a browser window; a cursor glides in, clicks a button (the button presses), then a download bubble fills. For a split panel slot (with fx split on the same span) or over the face. keys: src="media/refs/web/FILE.png" crop="x,y,w,h" (image px) click="x,y" (image px, the button middle) btn="w,h" (image px, the pressed area) at=0.45 (click second from the line start) url="claude.com/download" w=860 (window px) cy=560 (window middle) dl=1 (the download bubble) crop0="x,y,w,h" (the start view, zooms into crop; same aspect) zoom="0.05,0.4" (s) cur=1.5 (cursor size) full=1 (the page fills the split panel: no window, no bar, hard cut; the crops at the panel aspect)
font('SplitInter', JOB.engine + '/fonts/captions/Inter.ttf', { pins: { slnt: 0 } });
const WC_I = {};
const WC = {
  ease: (x) => 1 - Math.pow(1 - Math.min(1, Math.max(0, x)), 3),
  inout: (x) => { x = Math.min(1, Math.max(0, x)); return x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2; },
  // a macOS arrow: black, white edge, tip at 0,0
  arrow(g, s) {
    g.save(); g.scale(s, s);
    g.beginPath(); g.moveTo(0, 0); g.lineTo(0, 34); g.lineTo(8, 26); g.lineTo(14, 39); g.lineTo(20, 36); g.lineTo(14, 24); g.lineTo(25, 24); g.closePath();
    g.shadowColor = 'rgba(0,0,0,0.3)'; g.shadowBlur = 6; g.shadowOffsetY = 3;
    g.fillStyle = '#111'; g.fill(); g.shadowColor = 'transparent';
    g.lineWidth = 2.5; g.strokeStyle = '#fff'; g.stroke();
    g.restore();
  },
};
FX.def('webclick', { keys: { src: '', crop: '', crop0: '', zoom: '0.05,0.4', click: '', btn: '106,36', at: 0.45, url: '', w: 860, cy: 560, dl: 1, cur: 1.5, full: 0 } }, {
  under(g, f, k) {
    const I = WC_I[k.src] || (WC_I[k.src] = img(k.src));
    // crop0 (the whole page, same aspect as crop) zooms into crop over zoom="from,to" s; the click maps on the end view
    const C1 = String(k.crop).split(',').map(Number), C0 = k.crop0 ? String(k.crop0).split(',').map(Number) : C1;
    const [z0, z1] = String(k.zoom).split(',').map(Number), zm = WC.inout((f.u - z0) / Math.max(0.01, z1 - z0));
    const [cx0, cy0, cw, chh] = C1;
    const V = C0.map((v, i) => v + (C1[i] - v) * zm);
    const [bx, by] = String(k.click).split(',').map(Number), [bw, bh] = String(k.btn).split(',').map(Number);
    const u = f.u, at = +k.at, full = +k.full, W = full ? f.W : +k.w, sc = W / cw, bar = full ? 0 : 64, H = chh * sc + bar;
    // the window: scale 0.94 -> 1 + fade over 0.4 s, then a 1%/s push (MOTION 3, 10)
    const e = full ? 1 : WC.ease(u / 0.4), s = full ? 1 : (0.94 + 0.06 * e) * (1 + 0.01 * Math.max(0, u - 0.4));
    if (e <= 0) return;
    g.save(); g.globalAlpha = e; if (!full) { g.translate(f.W / 2, +k.cy); g.scale(s, s); g.translate(-W / 2, -H / 2); }
    if (!full) {
    g.shadowColor = 'rgba(0,0,0,0.22)'; g.shadowBlur = 40; g.shadowOffsetY = 16;
    g.fillStyle = '#ffffff'; g.beginPath(); g.roundRect(0, 0, W, H, 22); g.fill(); g.shadowColor = 'transparent';
    // the bar: three dots + the address pill
    g.fillStyle = '#F1EFEA'; g.beginPath(); g.roundRect(0, 0, W, bar, [22, 22, 0, 0]); g.fill();
    ['#FF5F57', '#FEBC2E', '#28C840'].forEach((c, i) => { g.fillStyle = c; g.beginPath(); g.arc(34 + i * 30, bar / 2, 9, 0, 7); g.fill(); });
    g.fillStyle = '#ffffff'; g.beginPath(); g.roundRect(140, 14, W - 280, bar - 28, 18); g.fill();
    g.fillStyle = '#5E5D59'; g.font = '500 26px SplitInter'; g.textAlign = 'center'; g.textBaseline = 'middle';
    g.fillText(k.url, W / 2, bar / 2 + 1);
    }
    // the page
    g.save(); g.beginPath(); g.roundRect(0, bar, W, H - bar, full ? 0 : [0, 0, 22, 22]); g.clip();
    g.drawImage(I, V[0], V[1], V[2], V[3], 0, bar, W, chh * sc);
    // the button press: a dark wash + squash for 0.18 s at the click
    const pd = u - at, px = (bx - cx0) * sc, py = bar + (by - cy0) * sc;
    if (pd > 0 && pd < 0.25) {
      const q = Math.sin(Math.PI * Math.min(1, pd / 0.25));
      g.fillStyle = `rgba(0,0,0,${0.12 * q})`; g.beginPath(); g.roundRect(px - bw * sc / 2, py - bh * sc / 2, bw * sc, bh * sc, 8 * sc); g.fill();
      // a ring that grows from the click point
      g.strokeStyle = `rgba(187,111,82,${0.6 * (1 - pd / 0.25)})`; g.lineWidth = 4; g.beginPath(); g.arc(px, py, 20 + 90 * WC.ease(pd / 0.25), 0, 7); g.stroke();
    }
    g.restore();
    // the download bubble, top right of the bar: pops in after the click, its ring fills
    if (+k.dl && !full && pd > 0.1) {
      const b = WC.ease((pd - 0.1) / 0.3), r = 22, x = W - 60, y = bar / 2;
      g.save(); g.translate(x, y); g.scale(0.6 + 0.4 * b, 0.6 + 0.4 * b); g.globalAlpha = b;
      g.fillStyle = '#ffffff'; g.beginPath(); g.arc(0, 0, r, 0, 7); g.fill();
      g.strokeStyle = '#E3E0D8'; g.lineWidth = 5; g.beginPath(); g.arc(0, 0, r, 0, 7); g.stroke();
      g.strokeStyle = '#BB6F52'; g.beginPath(); g.arc(0, 0, r, -Math.PI / 2, -Math.PI / 2 + 2 * Math.PI * WC.inout((pd - 0.2) / 0.9)); g.stroke();
      g.strokeStyle = '#16130D'; g.lineWidth = 3.5; g.lineCap = 'round';
      g.beginPath(); g.moveTo(0, -9); g.lineTo(0, 8); g.moveTo(-7, 2); g.lineTo(0, 9); g.lineTo(7, 2); g.stroke();
      g.restore();
    }
    // the cursor: in from the lower right on an S-curve, lands on the button at `at - 0.08`, dips at the click, drifts after
    const m = WC.inout((u - 0.05) / Math.max(0.1, at - 0.13));
    const sx = W + 60, sy = H + 80;
    let cxp = sx + (px - sx) * m, cyp = sy + (py - sy) * m - Math.sin(Math.PI * m) * 60;
    if (pd > 0.25) { cxp += 10 * WC.ease((pd - 0.25) / 0.6); cyp += 14 * WC.ease((pd - 0.25) / 0.6); }
    const dip = pd > 0 && pd < 0.14 ? 0.86 : 1;
    if (u > 0.05) { g.translate(cxp, cyp); WC.arrow(g, +k.cur * dip); }
    g.restore();
  },
});
