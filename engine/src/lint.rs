//! `aios lint`, the layout part (jobs with lib "layout"): every element of every frame — the marked groups of REEL,
//! CAPTIONS and POSTER (g.mark … g.unmark) and loose texts — measured against the job's LAYOUT grid
//! (engine/js/lib/layout.js): the safe area and the platforms' zones, the face (LAYOUT.frame(t).salient), overlaps,
//! text contrast, type sizes, and the grid notes LAYOUT.place made. Only settled frames count (full opacity, not
//! moving): an entrance or an exit is motion, not layout. Warnings only; the user judges.

use crate::canvas::Elem;
use crate::job::Job;
use anyhow::Result;
use serde_json::Value;
use skia_safe::Rect;
use std::collections::{BTreeMap, HashMap};

const LABEL: [&str; 7] = ["safe area", "face", "overlap", "contrast", "type sizes", "type scale", "grid"];

/// the jobs this check runs for
pub fn wants(job: &Job) -> bool {
    job.cfg.get("lib").and_then(|l| l.as_array()).map(|a| a.iter().any(|x| x.as_str() == Some("layout"))).unwrap_or(false)
}

#[derive(Clone)]
struct Txt {
    text: String,
    px: f32,
    color: Option<[f32; 4]>,
    b: Rect,
    alpha: f32,
}

/// one element in one frame (world units)
struct Seen {
    f: usize,
    b: Rect,
    alpha: f32,
    texts: Vec<Txt>,
    settled: bool,
}

/// one element over the frames it is in
struct Track {
    id: String,
    kind: String,
    text: String,
    bleed: bool,
    caption: bool,
    seen: Vec<Seen>,
}

impl Track {
    fn new(e: &Elem) -> Track {
        if e.label.is_empty() {
            let t = e.texts.first().map(|t| t.text.clone()).unwrap_or_default();
            return Track { id: format!("text \"{}\"", short(&t, 28)), kind: "text".into(), text: String::new(), bleed: false, caption: false, seen: vec![] };
        }
        let v: Value = serde_json::from_str(&e.label).unwrap_or(Value::Null);
        let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
        let id = if v["id"].is_string() { s("id") } else { e.label.clone() };
        Track { id, kind: s("kind"), text: s("text"), bleed: v["bleed"].as_bool().unwrap_or(false), caption: s("kind") == "caption", seen: vec![] }
    }
    /// beat 3 (text "5 hours"), beat 5 (card shot.png), caption 12 (caption "so I"), text "loose words"
    fn name(&self) -> String {
        if self.id.starts_with("text \"") {
            return self.id.clone();
        }
        if self.text.is_empty() {
            return if self.kind.is_empty() { self.id.clone() } else { format!("{} ({})", self.id, self.kind) };
        }
        let t = &self.text;
        let low = t.to_lowercase();
        let pathy = t.contains('/') || [".png", ".jpg", ".jpeg", ".webp", ".svg", ".mp4", ".mov", ".gif"].iter().any(|x| low.ends_with(x));
        if pathy {
            let mut parts = t.rsplit('/');
            let base = parts.next().unwrap_or(t);
            let base = if base.contains('%') { format!("{}/{base}", parts.next().unwrap_or("")) } else { base.to_string() };
            format!("{} ({} {})", self.id, self.kind, base)
        } else {
            format!("{} ({} \"{}\")", self.id, self.kind, short(t, 28))
        }
    }
}

struct Issue {
    kind: usize,
    subject: String,
    frames: Vec<usize>,
    worst: f32,
    msg: String,
}

#[derive(Default)]
struct Issues {
    list: Vec<Issue>,
    at: HashMap<(usize, String), usize>,
}

impl Issues {
    /// one line per (check, subject): its frames, and the message of its worst frame
    fn add(&mut self, kind: usize, subject: String, f: Option<usize>, metric: f32, msg: String) {
        self.add_keyed(kind, subject.clone(), subject, f, metric, msg)
    }
    fn add_keyed(&mut self, kind: usize, key: String, subject: String, f: Option<usize>, metric: f32, msg: String) {
        let i = *self.at.entry((kind, key)).or_insert_with(|| {
            self.list.push(Issue { kind, subject, frames: vec![], worst: f32::MIN, msg: String::new() });
            self.list.len() - 1
        });
        let x = &mut self.list[i];
        if let Some(f) = f {
            x.frames.push(f);
        }
        if metric > x.worst {
            x.worst = metric;
            x.msg = msg;
        }
    }
}

fn short(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n - 1).collect::<String>() + "…" }
}

fn inter(a: &Rect, b: &Rect) -> Option<Rect> {
    let mut r = *a;
    (r.intersect(b) && r.width() > 0. && r.height() > 0.).then_some(r)
}

/// the side of the safe area a platform zone is on
fn zone_side(z: &Rect, safe: &Rect) -> &'static str {
    if z.bottom <= safe.top + 1. {
        "top"
    } else if z.top >= safe.bottom - 1. {
        "bottom"
    } else if z.left >= safe.right - 1. {
        "right"
    } else {
        "left"
    }
}

fn area(r: &Rect) -> f32 {
    r.width().max(0.) * r.height().max(0.)
}

/// " 2.1-3.4 s, 9.0 s" (frames grouped into runs; gaps under half a second join a run)
fn spans(frames: &[usize], fps: f64, one: bool) -> String {
    let join = (fps * 0.5).round().max(1.) as usize;
    if one || frames.is_empty() {
        return String::new();
    }
    let mut v = frames.to_vec();
    v.sort();
    v.dedup();
    let mut runs: Vec<(usize, usize)> = vec![];
    for f in v {
        match runs.last_mut() {
            Some(r) if f <= r.1 + join => r.1 = f,
            _ => runs.push((f, f)),
        }
    }
    let s: Vec<String> = runs
        .iter()
        .take(3)
        .map(|&(a, b)| if b - a < 3 { format!("{:.1}", a as f64 / fps) } else { format!("{:.1}-{:.1}", a as f64 / fps, (b + 1) as f64 / fps) })
        .collect();
    let more = if runs.len() > 3 { format!(" (+{} more)", runs.len() - 3) } else { String::new() };
    format!(" {} s{}", s.join(", "), more)
}

/// LAYOUT.info() after the first frame (the slots, the scale, and the grid notes LAYOUT.place made)
fn layout_info(job: &Job) -> Result<Value> {
    let assets = crate::assets::Assets::new(job.dir.clone(), job.fps, true, true, 0.25);
    let shaper = std::rc::Rc::new(std::cell::RefCell::new(crate::text::Shaper::new()));
    let r = crate::js::Js::new(&job.src, assets.clone(), shaper).and_then(|mut js| {
        js.record(0.)?;
        js.eval_json("globalThis.LAYOUT ? LAYOUT.info() : null")
    });
    assets.stop();
    r
}

// WCAG 2 relative luminance and contrast ratio
fn lin(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}
fn lum(r: f32, g: f32, b: f32) -> f32 {
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}
fn ratio(a: f32, b: f32) -> f32 {
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// The contrast of one text on a painted frame (BGRA, `k` px per world unit): the pixels near the text's own colour
/// are the glyphs; the rest of its box, without the 1-px ring around the glyphs (anti-aliasing), is the ground. The
/// ratio is taken against the ground's median luminance. None when the text cannot be measured.
fn contrast(buf: &[u8], w: u32, h: u32, k: f32, t: &Txt) -> Option<f32> {
    let c = t.color?;
    if c[3] * t.alpha < 0.6 {
        return None;
    }
    let x0 = (t.b.left * k).floor().max(0.) as i64;
    let y0 = (t.b.top * k).floor().max(0.) as i64;
    let x1 = ((t.b.right * k).ceil() as i64).min(w as i64);
    let y1 = ((t.b.bottom * k).ceil() as i64).min(h as i64);
    if x1 - x0 < 4 || y1 - y0 < 4 {
        return None;
    }
    let (bw, bh) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut glyph = vec![false; bw * bh];
    let mut lums = vec![0f32; bw * bh];
    for y in 0..bh {
        for x in 0..bw {
            let i = (((y0 as usize + y) * w as usize) + x0 as usize + x) * 4;
            let a = (buf[i + 3] as f32 / 255.).max(1e-3);
            let (b, g, r) = ((buf[i] as f32 / 255. / a).min(1.), (buf[i + 1] as f32 / 255. / a).min(1.), (buf[i + 2] as f32 / 255. / a).min(1.));
            glyph[y * bw + x] = (r - c[0]).abs().max((g - c[1]).abs()).max((b - c[2]).abs()) <= 0.12;
            lums[y * bw + x] = lum(r, g, b);
        }
    }
    let n_glyph = glyph.iter().filter(|&&g| g).count();
    let mut ground = Vec::with_capacity(bw * bh);
    for y in 0..bh {
        for x in 0..bw {
            if glyph[y * bw + x] {
                continue;
            }
            let ring = (y.saturating_sub(1)..(y + 2).min(bh)).any(|yy| (x.saturating_sub(1)..(x + 2).min(bw)).any(|xx| glyph[yy * bw + xx]));
            if !ring {
                ground.push(lums[y * bw + x]);
            }
        }
    }
    let n = bw * bh;
    if n_glyph * 50 < n || ground.len() * 10 < n {
        return None;
    }
    ground.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Some(ratio(lum(c[0], c[1], c[2]), ground[ground.len() / 2]))
}

/// Measures the frames' elements against the grid and prints the warnings.
pub fn report(job: &Job, frames: &[usize], data: Vec<(Vec<Elem>, Value)>, k: f32) -> Result<()> {
    let t0 = std::time::Instant::now();
    let info = layout_info(job)?;
    if info.is_null() {
        return Ok(());
    }
    let num = |v: &Value| v.as_f64().unwrap_or(0.) as f32;
    let rect = |v: &Value| Rect::from_xywh(num(&v["x"]), num(&v["y"]), num(&v["w"]), num(&v["h"]));
    let (fw, fh) = (num(&info["W"]), num(&info["H"]));
    let unit = num(&info["unit"]).max(1.);
    let tol = unit / 4.;
    let safe = rect(&info["safe"]);
    let zones: Vec<(String, Rect)> =
        info["zones"].as_array().map(|a| a.iter().map(|z| (z["name"].as_str().unwrap_or("").to_string(), rect(z))).collect()).unwrap_or_default();
    let scale: Vec<f32> = info["scale"].as_array().map(|a| a.iter().map(num).collect()).unwrap_or_default();
    let one = frames.len() == 1;
    let fps = job.fps;
    let mm = job.cfg["print"]["mm"].as_array().map(|m| m.len() == 2 && num(&m[0]) == fw && num(&m[1]) == fh).unwrap_or(false);
    let u = if mm { "mm" } else { "px" };
    let ground = |b: &Rect| inter(b, &Rect::from_wh(fw, fh)).map(|r| area(&r) >= 0.9 * fw * fh).unwrap_or(false);

    // the elements, followed across frames
    let mut tracks: Vec<Track> = vec![];
    let mut by_key: HashMap<String, usize> = HashMap::new();
    let mut by_frame: Vec<Vec<(usize, usize)>> = vec![vec![]; frames.len()];
    let mut salient: Vec<Vec<(String, Rect)>> = Vec::with_capacity(frames.len());
    let world = |r: Rect| Rect::from_ltrb(r.left / k, r.top / k, r.right / k, r.bottom / k);
    for (i, (elems, fv)) in data.into_iter().enumerate() {
        salient.push(fv["salient"].as_array().map(|a| a.iter().map(|s| (s["name"].as_str().unwrap_or("face").to_string(), rect(s))).collect()).unwrap_or_default());
        for e in elems {
            if e.b.is_empty() && e.texts.is_empty() {
                continue;
            }
            let key = if e.label.is_empty() { format!("text:{}", e.texts.first().map(|t| t.text.as_str()).unwrap_or("")) } else { e.label.clone() };
            let ti = *by_key.entry(key).or_insert_with(|| {
                tracks.push(Track::new(&e));
                tracks.len() - 1
            });
            let texts: Vec<Txt> = e.texts.iter().map(|t| Txt { text: t.text.clone(), px: t.px, color: t.color, b: world(t.b), alpha: t.alpha }).collect();
            let tr = &mut tracks[ti];
            if let Some(last) = tr.seen.last_mut().filter(|s| s.f == frames[i]) {
                last.b.join(world(e.b));
                last.alpha = last.alpha.max(e.alpha);
                last.texts.extend(texts);
                continue;
            }
            tr.seen.push(Seen { f: frames[i], b: world(e.b), alpha: e.alpha, texts, settled: false });
            by_frame[i].push((ti, tr.seen.len() - 1));
        }
    }
    // settled: at the element's full opacity and where it was two frames ago
    let still = unit * 0.1;
    let near = |a: &Rect, b: &Rect| (a.left - b.left).abs() <= still && (a.top - b.top).abs() <= still && (a.right - b.right).abs() <= still && (a.bottom - b.bottom).abs() <= still;
    for tr in tracks.iter_mut() {
        let amax = tr.seen.iter().map(|s| s.alpha).fold(0f32, f32::max);
        for j in 0..tr.seen.len() {
            // held for three frames: a spring's turning point (one or two still frames) is still motion
            let held = one || (j >= 2 && tr.seen[j - 2].f + 2 == tr.seen[j].f && near(&tr.seen[j - 2].b, &tr.seen[j].b) && near(&tr.seen[j - 1].b, &tr.seen[j].b));
            tr.seen[j].settled = amax > 0.05 && tr.seen[j].alpha >= 0.98 * amax && held;
        }
    }

    let mut iss = Issues::default();
    for (i, list) in by_frame.iter().enumerate() {
        let f = frames[i];
        let live: Vec<(usize, &Seen)> = list.iter().map(|&(ti, si)| (ti, &tracks[ti].seen[si])).filter(|(_, s)| s.settled).collect();
        let solid = |ti: usize, s: &Seen| !tracks[ti].bleed && !ground(&s.b) && !s.b.is_empty();
        // 1. the safe area: the margins and the platforms' buttons and text
        for &(ti, s) in live.iter().filter(|(ti, s)| solid(*ti, s)) {
            let over = [("left", safe.left - s.b.left), ("right", s.b.right - safe.right), ("top", safe.top - s.b.top), ("bottom", s.b.bottom - safe.bottom)];
            let d = over.iter().map(|o| o.1).fold(0f32, f32::max);
            if d > tol {
                // each side out, and what is there: a platform zone on that side, else the margin
                let parts: Vec<String> = over
                    .iter()
                    .filter(|o| o.1 > tol)
                    .map(|&(side, v)| {
                        let hit: Vec<&str> = zones
                            .iter()
                            .filter(|(_, z)| zone_side(z, &safe) == side && inter(&s.b, z).map(|r| r.width().min(r.height()) > tol).unwrap_or(false))
                            .map(|(n, _)| n.as_str())
                            .collect();
                        format!("{side} {v:.0} {u} ({})", if hit.is_empty() { "margin".to_string() } else { hit.join(", ") })
                    })
                    .collect();
                iss.add(0, tracks[ti].name(), Some(f), d, format!("outside the safe area: {}", parts.join(", ")));
            }
        }
        // 2. the face (or the comp's other salient areas): graphics keep off it; captions have their own check
        for (name, sb) in &salient[i] {
            let sa = area(sb);
            if sa <= 0. {
                continue;
            }
            for &(ti, s) in live.iter().filter(|(ti, s)| solid(*ti, s) && !tracks[*ti].caption) {
                if let Some(r) = inter(&s.b, sb) {
                    let p = area(&r) / sa;
                    if p > 0.03 {
                        iss.add(1, tracks[ti].name(), Some(f), p, format!("covers {:.0}% of the {name}", p * 100.));
                    }
                }
            }
        }
        // 3. overlaps: two elements partly over each other (one fully inside the other is a layer, e.g. text on a card)
        for x in 0..live.len() {
            for y in x + 1..live.len() {
                let ((ta, a), (tb, b)) = (live[x], live[y]);
                if !solid(ta, a) || !solid(tb, b) || tracks[ta].caption && tracks[tb].caption {
                    continue;
                }
                let Some(r) = inter(&a.b, &b.b) else { continue };
                let small = area(&a.b).min(area(&b.b));
                let (p, depth) = (area(&r) / small.max(1e-6), r.width().min(r.height()));
                if p > 0.01 && p < 0.95 && depth > unit / 2. {
                    let pn = |t: &Track| if t.caption { "captions".to_string() } else { t.name() };
                    let (first, second) = if ta < tb { (ta, tb) } else { (tb, ta) };
                    let smaller = if area(&a.b) <= area(&b.b) { &tracks[ta] } else { &tracks[tb] };
                    let what = if smaller.caption { format!("{} \"{}\"", smaller.id, short(&smaller.text, 24)) } else { smaller.id.clone() };
                    iss.add(2, format!("{} + {}", pn(&tracks[first]), pn(&tracks[second])), Some(f), p, format!("{:.0}% of {what} is covered, {depth:.0} {u} deep", p * 100.));
                }
            }
        }
        // 5. type sizes at once (captions keep their own approved sizes)
        let mut px: Vec<f32> = live.iter().filter(|(ti, _)| !tracks[*ti].caption).flat_map(|(_, s)| s.texts.iter().filter(|t| t.alpha > 0.05).map(|t| t.px)).collect();
        px.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mut groups: Vec<f32> = vec![];
        for p in px {
            if groups.last().map(|g| p > g * 1.05).unwrap_or(true) {
                groups.push(p);
            }
        }
        if groups.len() > 4 {
            let l: Vec<String> = groups.iter().map(|g| format!("{g:.0}")).collect();
            iss.add(4, String::new(), Some(f), groups.len() as f32, format!("{} sizes at once ({} {u}); keep to 3-4", groups.len(), l.join(", ")));
        }
    }

    // 6. the type scale: each element's main (largest) size, where it settles
    let audit: Vec<(String, String)> =
        info["audit"].as_array().map(|a| a.iter().map(|x| (x["id"].as_str().unwrap_or("").to_string(), x["msg"].as_str().unwrap_or("").to_string())).collect()).unwrap_or_default();
    let noted = |id: &str, what: &str| audit.iter().any(|(a, m)| (a == id || a.starts_with(&format!("{id} "))) && m.contains(what));
    if !scale.is_empty() {
        for tr in tracks.iter().filter(|t| !t.caption) {
            let mut count: BTreeMap<i64, (usize, usize)> = BTreeMap::new();
            for s in tr.seen.iter().filter(|s| s.settled) {
                if let Some(m) = s.texts.iter().map(|t| t.px).fold(None, |a: Option<f32>, p| Some(a.map_or(p, |a| a.max(p)))) {
                    let e = count.entry((m * 2.).round() as i64).or_insert((0, s.f));
                    e.0 += 1;
                }
            }
            let Some((&key, &(_, f))) = count.iter().max_by_key(|(_, v)| v.0) else { continue };
            let p = key as f32 / 2.;
            if !scale.iter().any(|s| (p / s - 1.).abs() <= 0.05) && !noted(&tr.id, "type scale") {
                let lo = scale.iter().copied().filter(|&s| s < p).fold(f32::NAN, f32::max);
                let hi = scale.iter().copied().filter(|&s| s > p).fold(f32::NAN, f32::min);
                let near: Vec<String> = [lo, hi].iter().filter(|v| v.is_finite()).map(|v| format!("{v:.1}")).collect();
                iss.add(5, tr.name(), Some(f), 1., format!("{p:.0} {u} is off the type scale (nearest steps: {})", near.join(" or ")));
            }
        }
    }
    // 7. the grid notes LAYOUT.place made (raw numbers off the grid, sizes off the scale, text wider than its slot)
    for (id, msg) in &audit {
        iss.add_keyed(6, format!("{id}\n{msg}"), id.clone(), None, 0., msg.clone());
    }

    // 4. contrast: each text on its painted ground, at its first and last settled frame
    let mut samples: BTreeMap<usize, Vec<(usize, Txt)>> = BTreeMap::new();
    for (ti, tr) in tracks.iter().enumerate().filter(|(_, t)| !t.caption) {
        let set: Vec<&Seen> = tr.seen.iter().filter(|s| s.settled && s.texts.iter().any(|t| t.color.is_some())).collect();
        let (Some(a), Some(b)) = (set.first(), set.last()) else { continue };
        for s in if a.f == b.f { vec![*a] } else { vec![*a, *b] } {
            for t in s.texts.iter().filter(|t| t.color.is_some()) {
                samples.entry(s.f).or_default().push((ti, t.clone()));
            }
        }
    }
    if !samples.is_empty() {
        let look = crate::Look { width: Some(540.min(job.world.0)), draft: false, shutter: 0., gap: None, smax: 2, workers: None, no_hw: false, no_cache: true, gpu: Some(0) };
        let (s, assets) = crate::parts(job, &look, true);
        let list: Vec<usize> = samples.keys().copied().collect();
        let pool = crate::render::Pool::new(&job.src, assets, s.clone(), list.len());
        let mut worst: HashMap<usize, (f32, usize)> = HashMap::new();
        let r = pool.render(
            &list,
            |_| crate::render::Want::Bgra,
            |f, buf| {
                for (ti, t) in &samples[&f] {
                    let r = contrast(&buf, s.out_w, s.out_h, s.k, t);
                    if std::env::var("AIOS_LINT_DEBUG").is_ok() {
                        eprintln!("contrast {} \"{}\" {:.0} px at {:.2} s: {:?}", tracks[*ti].id, short(&t.text, 20), t.px, f as f64 / fps, r);
                    }
                    if let Some(c) = r {
                        let w = worst.entry(*ti).or_insert((f32::MAX, f));
                        if c < w.0 {
                            *w = (c, f);
                        }
                    }
                }
                Ok(())
            },
            false,
        );
        pool.close();
        match r {
            Ok(_) => {
                for (ti, (c, f)) in worst {
                    if c < 3. {
                        iss.add(3, tracks[ti].name(), Some(f), 3. - c, format!("{c:.1}:1 against its ground (large text needs 3:1)"));
                    }
                }
            }
            Err(e) => println!("  warning: contrast check: {e:#}"),
        }
    }

    // the report
    let n_el = tracks.iter().filter(|t| t.seen.iter().any(|s| s.settled)).count();
    let plat: Vec<String> = info["platforms"].as_array().map(|a| a.iter().filter_map(|p| p.as_str()).map(|p| p.to_string()).collect()).unwrap_or_default();
    let desc = format!(
        "{} {}x{}{}{}",
        info["format"].as_str().unwrap_or("?"),
        fw,
        fh,
        if mm { " mm" } else { "" },
        if plat.is_empty() { String::new() } else { format!(", {} zones", plat.join(" + ")) }
    );
    let ms = t0.elapsed().as_secs_f64();
    if iss.list.is_empty() {
        println!("layout ({desc}): {n_el} elements, all on the grid ({ms:.2} s)");
        return Ok(());
    }
    iss.list.sort_by_key(|x| (x.kind, x.frames.iter().min().copied().unwrap_or(0)));
    println!("layout ({desc}): {} warnings on {n_el} elements ({ms:.2} s; the grid: aios still JOB --grid)", iss.list.len());
    for x in iss.list.iter().take(40) {
        let subj = if x.subject.is_empty() { String::new() } else { format!(" {}", x.subject) };
        println!("  {:<10}{}{}: {}", LABEL[x.kind], subj, spans(&x.frames, fps, one), x.msg);
    }
    if iss.list.len() > 40 {
        println!("  … {} more", iss.list.len() - 40);
    }
    Ok(())
}
