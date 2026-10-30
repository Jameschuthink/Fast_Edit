/* AIOS3 lib/poster.js — a still graphic (poster, cover, social post) as data on the layout grid. Needs lib "layout".
 *
 *   job.json   "lib": ["layout", "poster"], "size": [1350, 1200] (any units: mm for print), "dur": 1,
 *              "layout": {"format": "poster"}, "poster": "poster.json"
 *   comp.js    function draw(g, t) { POSTER.draw(g, t); }
 *
 * poster.json: {ground, colors {name: hex}, fonts {display, text, mono: "WEIGHT 'Family'"}, elements: [...]}
 * Each element names a slot (LAYOUT slots: brand, tag, main, hero, rule, foot-left, foot-right, body, foot, or the
 * rows top / title / stage / golden / center / lower). The elements of one slot stack in order: down the slot
 * (dir "col") or across it (dir "row"), aligned by the slot's align (left / center / right) and valign
 * (top / center / phi = the stack's centre on the slot's golden line / bottom). Nothing has x / y.
 *
 *   text   {text ("\n" breaks lines), role (display|text|mono), size (role name or step), weight, color, lh, track (em), wrap}
 *   image  {src, h ("full" = the slot's height, or px), w (px or "4col"); default: as big as fits the slot}
 *   rule   {color, weight (line width)}
 *   space  {h}  an empty gap
 * Every element: gap (spacing step before it, default 3), align (its own, across the stack).
 *   POSTER.draw(g, t)   POSTER.boxes() the placed boxes (world units)
 */
(function () {
  if (!globalThis.LAYOUT) throw new Error('lib "poster" needs lib "layout" before it (job.json "lib": ["layout", "poster"])');
  const P = JOB.poster || (JOB.data && JOB.data.poster) || {};
  const EL = (P.elements || []).map((e, i) => Object.assign({ i }, e));
  const COLORS = Object.assign({ ground: '#ffffff', ink: '#111111', mute: '#777777', line: '#dddddd', accent: '#1f5fd6' },
    (globalThis.LOOK && LOOK.color) || {}, JOB.colors || {}, P.colors || {});
  const col = c => (c && COLORS[c]) || c || COLORS.ink;
  const FONTS = Object.assign({ display: "700 sans-serif", text: "400 sans-serif", mono: "500 monospace" }, P.fonts || {});
  function fontOf(e, px) {
    if (e.font) return `${e.weight || 400} ${px}px ${e.font}`;
    if (!P.fonts && globalThis.LOOK) return LOOK.font(e.role || 'text', px, e.weight);
    const f = FONTS[e.role || 'text'] || FONTS.text, m = /^\s*(\d+)\s+(.*)$/.exec(f);
    return m ? `${e.weight || m[1]} ${px}px ${m[2]}` : `${e.weight || 400} ${px}px ${f}`;
  }

  // wrap a line to a width: words for spaced text, characters for CJK
  function wrap(g, text, maxW) {
    const out = [];
    for (const para of String(text).split('\n')) {
      const parts = /\s/.test(para) ? para.split(/(\s+)/) : [...para];
      let line = '';
      for (const p of parts) {
        const tryL = line + p;
        if (line && g.measureText(tryL.trimEnd()).width > maxW) { out.push(line.trimEnd()); line = p.trimStart(); } else line = tryL;
      }
      out.push(line.trimEnd());
    }
    return out;
  }

  // measure every element, then stack each slot's elements
  let PLACED = null;
  function layout(g) {
    if (PLACED) return PLACED;
    const bySlot = new Map();
    for (const e of EL) {
      if (!e.slot) throw new Error(`poster element ${e.i} (${e.type}): no "slot" (slots: ${Object.keys(LAYOUT.slots).filter(n => !/-(left|right)$/.test(n)).join(', ')})`);
      const s = LAYOUT.slot(e.slot);
      if (!bySlot.has(e.slot)) bySlot.set(e.slot, []);
      bySlot.get(e.slot).push(e);
      e.__s = s;
    }
    for (const [name, list] of bySlot) {
      const s = list[0].__s, dir = s.dir || 'col';
      // sizes
      for (const e of list) {
        if (e.type === 'text') {
          e.px = typeof e.size === 'number' && e.size > 20 ? LAYOUT.place({ size: e.size }, `element ${e.i} (text)`).size : LAYOUT.size(e.size ?? 'body');
          e.lhK = e.lh || (e.role === 'display' ? 1.15 : 1.4);
          g.font = fontOf(e, e.px); g.letterSpacing = ((e.track || 0) * e.px) + 'px';
          const maxW = dir === 'col' ? s.w : s.w * 0.9;
          e.lines = e.wrap ? wrap(g, e.text, maxW) : String(e.text).split('\n');
          e.lw = e.lines.map(l => g.measureText(l).width);
          e.w = Math.max(...e.lw); e.h = e.lines.length * e.px * e.lhK;
          const m = g.measureText(e.lines[0] || 'M'); e.asc = m.fontBoundingBoxAscent; e.desc = m.fontBoundingBoxDescent;
          if (e.w > s.w + 0.5) LAYOUT.note(`element ${e.i} (text)`, `"${e.lines[e.lw.indexOf(e.w)]}" is ${Math.round(e.w)} wide, the slot ${name} ${Math.round(s.w)}: wider than its slot`);
        } else if (e.type === 'image') {
          const pic = img(e.src), ar = pic.height / pic.width;
          let w, h;
          if (e.h !== undefined) { h = e.h === 'full' ? s.h : e.h; w = h / ar; }
          else if (e.w !== undefined) { w = LAYOUT.width(e.w); h = w * ar; }
          else { w = s.w; h = w * ar; if (h > s.h) { h = s.h; w = h / ar; } }
          Object.assign(e, { pic, w, h });
        } else if (e.type === 'rule') { e.w = s.w; e.h = 0; }
        else if (e.type === 'space') { e.w = 0; e.h = e.h || LAYOUT.gap(3); }
        else throw new Error(`poster element ${e.i}: unknown type "${e.type}" (text, image, rule, space)`);
      }
      // the stack
      const gapOf = (e, k) => k === 0 ? 0 : LAYOUT.gap(e.gap ?? 3);
      if (dir === 'col') {
        const H = list.reduce((a, e, k) => a + gapOf(e, k) + e.h, 0);
        const va = s.valign || 'center';
        let y = va === 'top' ? s.y : va === 'bottom' ? s.y + s.h - H : va === 'phi' ? s.y + s.h / LAYOUT.PHI / LAYOUT.PHI - H / 2 : s.y + (s.h - H) / 2;
        if (va === 'phi') y = Math.max(s.y, Math.min(y, s.y + s.h - H));
        list.forEach((e, k) => {
          y += gapOf(e, k);
          const al = e.align || s.align || 'left';
          e.x = al === 'center' ? s.x + (s.w - e.w) / 2 : al === 'right' ? s.x + s.w - e.w : s.x; e.y = y; e.al = al;
          y += e.h;
        });
      } else {
        const Wt = list.reduce((a, e, k) => a + gapOf(e, k) + e.w, 0);
        const al = s.align || 'left';
        let x = al === 'center' ? s.x + (s.w - Wt) / 2 : al === 'right' ? s.x + s.w - Wt : s.x;
        list.forEach((e, k) => {
          x += gapOf(e, k);
          const va = e.valign || s.valign || 'center';
          e.x = x; e.y = va === 'top' ? s.y : va === 'bottom' ? s.y + s.h - e.h : s.y + (s.h - e.h) / 2; e.al = 'left';
          x += e.w;
        });
      }
    }
    PLACED = EL;
    return PLACED;
  }

  function draw(g, t) {
    const L = layout(g);
    g.fillStyle = col(P.ground || 'ground'); g.fillRect(0, 0, LAYOUT.W, LAYOUT.H);
    for (const e of L) {
      g.mark({ id: `element ${e.i}`, kind: e.type, text: e.type === 'text' ? e.lines.join(' / ') : e.src || '', slot: e.slot });
      if (e.type === 'text') {
        g.font = fontOf(e, e.px); g.letterSpacing = ((e.track || 0) * e.px) + 'px';
        g.fillStyle = col(e.color); g.textBaseline = 'alphabetic'; g.textAlign = e.al;
        const lh = e.px * e.lhK, half = (lh - (e.asc + e.desc)) / 2;
        const x = e.al === 'center' ? e.x + e.w / 2 : e.al === 'right' ? e.x + e.w : e.x;
        e.lines.forEach((l, k) => g.fillText(l, x, e.y + k * lh + half + e.asc));
      } else if (e.type === 'image') {
        g.drawImage(e.pic, e.x, e.y, e.w, e.h);
      } else if (e.type === 'rule') {
        g.strokeStyle = col(e.color || 'line'); g.lineWidth = e.weight || Math.max(1, LAYOUT.unit / 5);
        g.beginPath(); g.moveTo(e.x, e.y); g.lineTo(e.x + e.w, e.y); g.stroke();
      }
      g.unmark();
    }
  }

  function boxes() { const L = PLACED || []; return L.map(e => ({ i: e.i, slot: e.slot, type: e.type, x: e.x, y: e.y, w: e.w, h: e.h })); }

  globalThis.POSTER = { draw, boxes, colors: COLORS };
})();
