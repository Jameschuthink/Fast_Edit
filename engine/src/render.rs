//! Rendering: N workers, each with its own JS runtime and Skia surface, take frames from a shared counter; an
//! ordered writer feeds the encoder (or PNG/JPEG files). Motion blur is adaptive: the frame's shutter ends are
//! compared op by op (hash + device box); a frame where nothing moved costs one paint, a moving frame gets
//! enough samples to keep copies `gap` px apart, painted only inside the region that changed and averaged in
//! linear light.

use crate::assets::Assets;
use crate::canvas::{Analysis, Capture, Elem, Exec, Lift};
use crate::encode::{bgra_to_yuv420, EncOpts, Encoder};
use crate::js::{CompSrc, Js};
use crate::text::Shaper;
use crate::trace;
use anyhow::{anyhow, Result};
use skia_safe::{surfaces, AlphaType, BlendMode, Color, ColorType, IRect, ImageInfo, Paint, Rect, RoundOut, Surface};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

#[derive(Clone, Copy, Debug)]
pub struct Blur {
    pub shutter: f64, // degrees (180 = film look)
    pub gap: f32,     // px between neighbouring copies of a moving edge
    pub smax: usize,  // most samples per frame
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub out_w: u32,
    pub out_h: u32,
    pub k: f32,
    pub fps: f64,
    pub dur: f64,
    pub blur: Option<Blur>,
    pub workers: usize,
    /// how many of the workers paint on the GPU (Vulkan); the rest paint on the CPU
    pub gpu: usize,
}

/// Trace spans for a worker step / a whole frame (recorded when dropped; nothing without AIOS_TRACE).
struct Span(&'static str, Instant);
impl Drop for Span {
    fn drop(&mut self) { let n = self.0; crate::trace::span("worker", self.1, || n.to_string()); }
}
struct FrameSpan(usize, Instant);
impl Drop for FrameSpan {
    fn drop(&mut self) { let f = self.0; crate::trace::span("frame", self.1, || format!("frame {f}")); }
}

#[derive(Default, Clone, Debug)]
pub struct Stats {
    pub frames: usize,
    pub paints: usize,
    pub still: usize,
    pub blurred: usize,
    pub js_ms: f64,
    pub ana_ms: f64,
    pub paint_ms: f64,
    pub post_ms: f64,
    pub warnings: Vec<String>,
}

impl Stats {
    fn add(&mut self, o: &Stats) {
        self.frames += o.frames;
        self.paints += o.paints;
        self.still += o.still;
        self.blurred += o.blurred;
        self.js_ms += o.js_ms;
        self.ana_ms += o.ana_ms;
        self.paint_ms += o.paint_ms;
        self.post_ms += o.post_ms;
        for w in &o.warnings {
            if !self.warnings.contains(w) && self.warnings.len() < 20 {
                self.warnings.push(w.clone());
            }
        }
    }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.
}

struct Luts {
    to_lin: [u32; 256],
    to_srgb: Vec<u8>, // 4096 entries over 0..=65535
}

fn luts() -> &'static Luts {
    static L: OnceLock<Luts> = OnceLock::new();
    L.get_or_init(|| {
        let mut to_lin = [0u32; 256];
        for (i, v) in to_lin.iter_mut().enumerate() {
            let c = i as f64 / 255.;
            let l = if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
            *v = (l * 65535.).round() as u32;
        }
        let mut to_srgb = vec![0u8; 4097];
        for (i, v) in to_srgb.iter_mut().enumerate() {
            let l = (i as f64 * 16.) / 65535.;
            let l = l.min(1.);
            let c = if l <= 0.0031308 { l * 12.92 } else { 1.055 * l.powf(1. / 2.4) - 0.055 };
            *v = (c * 255.).round().clamp(0., 255.) as u8;
        }
        Luts { to_lin, to_srgb }
    })
}

pub struct Worker {
    pub js: Js,
    shaper: Rc<RefCell<Shaper>>,
    surface: Surface,
    assets: Arc<Assets>,
    w: i32,
    h: i32,
    k: f32,
    pub pixels: Vec<u8>,
    in_surface: bool,
    region: Vec<u8>,
    acc: Vec<u32>,
    acc_surface: Option<Surface>,
    pub stats: Stats,
    /// the live page's lift for the next paint (Exec::set_lift)
    lift: Option<(String, Lift)>,
    /// BGRA -> yuv420p on the GPU (yuv.rs), for GPU workers
    yuv: Option<crate::yuv::GpuYuv>,
    /// AIOS_GPU_YUV=check: GPU frames already compared with the CPU path, waiting to be collected
    checked: std::collections::VecDeque<Vec<u8>>,
    gpu: Option<crate::gpu::Gpu>, // last: the surfaces must be dropped before their GPU context
}

impl Worker {
    pub fn new(src: &CompSrc, assets: Arc<Assets>, s: &Settings, use_gpu: bool) -> Result<Worker> {
        let shaper = Rc::new(RefCell::new(Shaper::new()));
        let js = Js::new(src, assets.clone(), shaper.clone())?;
        let mut gpu = None;
        let mut surface = None;
        if use_gpu {
            match crate::gpu::Gpu::new() {
                Ok(mut g) => {
                    let info = ImageInfo::new_n32_premul((s.out_w as i32, s.out_h as i32), Some(skia_safe::ColorSpace::new_srgb()));
                    surface = skia_safe::gpu::surfaces::render_target(&mut g.context, skia_safe::gpu::Budgeted::Yes, &info, None, skia_safe::gpu::SurfaceOrigin::TopLeft, None, false, None);
                    if surface.is_some() { gpu = Some(g); } else { eprintln!("GPU surface failed: painting on the CPU"); }
                }
                Err(e) => eprintln!("GPU unavailable ({e:#}): painting on the CPU"),
            }
        }
        let surface = match surface {
            Some(s) => s,
            None => surfaces::raster(&ImageInfo::new_n32_premul((s.out_w as i32, s.out_h as i32), Some(skia_safe::ColorSpace::new_srgb())), None, None)
                .ok_or_else(|| anyhow!("cannot make a {}x{} surface", s.out_w, s.out_h))?,
        };
        let yuv = match gpu.as_mut().and_then(|g| crate::yuv::GpuYuv::new(g, s.out_w as i32, s.out_h as i32)) {
            Some(Ok(y)) => Some(y),
            Some(Err(e)) => { eprintln!("GPU YUV off ({e:#}): converting on the CPU"); None }
            None => None,
        };
        Ok(Worker {
            js,
            shaper,
            surface,
            assets,
            w: s.out_w as i32,
            h: s.out_h as i32,
            k: s.k,
            pixels: vec![0; (s.out_w * s.out_h * 4) as usize],
            in_surface: false,
            region: Vec::new(),
            acc: Vec::new(),
            acc_surface: None,
            stats: Stats::default(),
            lift: None,
            yuv,
            checked: Default::default(),
            gpu,
        })
    }

    fn analyze(&mut self, ops: &[f64]) -> Analysis {
        let t0 = Instant::now();
        let _g = Span("analyse", t0);
        let mut a = Analysis::default();
        {
            let mut sh = self.shaper.borrow_mut();
            let mut ex = Exec::new(&self.js.strs, &self.assets, &mut sh, self.k, self.w as f32, self.h as f32, None, Some(&mut a));
            ex.run(ops);
            let _ = ex.finish();
        }
        self.stats.ana_ms += ms(t0);
        a
    }

    fn paint(&mut self, ops: &[f64], clip: Option<IRect>) {
        let t0 = Instant::now();
        let _g = Span("paint", t0);
        let warns;
        {
            let canvas = self.surface.canvas();
            canvas.save();
            if let Some(r) = clip {
                canvas.clip_irect(r, None);
            }
            canvas.clear(Color::TRANSPARENT);
            let mut sh = self.shaper.borrow_mut();
            let mut ex = Exec::new(&self.js.strs, &self.assets, &mut sh, self.k, self.w as f32, self.h as f32, Some(canvas), None);
            ex.set_lift(self.lift.clone());
            ex.run(ops);
            warns = ex.finish();
            canvas.restore_to_count(1);
        }
        for w in warns {
            if !self.stats.warnings.contains(&w) && self.stats.warnings.len() < 20 {
                self.stats.warnings.push(w);
            }
        }
        self.stats.paints += 1;
        self.stats.paint_ms += ms(t0);
    }

    /// `aios lint`: the frame's elements (marked groups and texts, device boxes) and the comp's LAYOUT.frame(t)
    /// (the face and other areas graphics should keep off)
    pub fn layout(&mut self, t: f64) -> Result<(Vec<Elem>, serde_json::Value)> {
        let ops = self.record(t)?;
        let mut a = Analysis { capture: Some(Capture::default()), ..Default::default() };
        {
            let mut sh = self.shaper.borrow_mut();
            let mut ex = Exec::new(&self.js.strs, &self.assets, &mut sh, self.k, self.w as f32, self.h as f32, None, Some(&mut a));
            ex.run(&ops);
            let _ = ex.finish();
        }
        let info = self.js.eval_json(&format!("globalThis.LAYOUT ? LAYOUT.frame({t}) : null"))?;
        Ok((a.capture.map(|c| c.elems).unwrap_or_default(), info))
    }

    fn record(&mut self, t: f64) -> Result<Vec<f64>> {
        let t0 = Instant::now();
        let _g = Span("js", t0);
        let r = self.js.record(t);
        self.stats.js_ms += ms(t0);
        r
    }

    fn read(&mut self, r: IRect, into_region: bool) {
        let info = ImageInfo::new((r.width(), r.height()), ColorType::BGRA8888, AlphaType::Premul, None);
        let rb = (r.width() * 4) as usize;
        if into_region {
            self.region.resize(rb * r.height() as usize, 0);
            self.surface.read_pixels(&info, &mut self.region, rb, (r.left, r.top));
        } else {
            self.surface.read_pixels(&info, &mut self.pixels, rb, (r.left, r.top));
        }
    }

    /// What frame t will look like, as a hash: the analysis of its record (and the shutter ends when blurred). Two
    /// frames with the same hash are the same picture; a changed hash means the frame must be painted again.
    pub fn hash(&mut self, t: f64, blur: Option<Blur>, fps: f64, dur: f64) -> Result<u64> {
        let mut h = xxhash_rust::xxh3::Xxh3::new();
        let ops = self.record(t)?;
        h.update(&self.analyze(&ops).hash.to_le_bytes());
        if let Some(b) = blur {
            let half = (b.shutter / 360.) / fps / 2.;
            let tmax = (dur - 1e-6).max(0.);
            for tt in [(t - half).clamp(0., tmax), (t + half).clamp(0., tmax)] {
                let o = self.record(tt)?;
                h.update(&self.analyze(&o).hash.to_le_bytes());
            }
        }
        Ok(h.digest())
    }

    /// Starts converting the finished frame to yuv420p on the GPU and reading it back (yuv.rs); false when this
    /// worker cannot (the caller converts `result()` on the CPU). `gpu_yuv_collect` returns the bytes, oldest first;
    /// at most two frames wait. AIOS_GPU_YUV=check converts both ways and prints the largest difference.
    pub fn gpu_yuv_submit(&mut self) -> bool {
        static CHECK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if !self.in_surface { return false; }
        let (Some(y), Some(g)) = (self.yuv.as_mut(), self.gpu.as_mut()) else { return false };
        if let Err(e) = y.submit(&mut g.context, &mut self.surface) {
            eprintln!("GPU YUV failed ({e:#}): converting on the CPU");
            return false;
        }
        if *CHECK.get_or_init(|| std::env::var("AIOS_GPU_YUV").map(|v| v == "check").unwrap_or(false)) {
            let mut out = Vec::new();
            if let Err(e) = y.collect(&mut out) {
                eprintln!("GPU YUV failed ({e:#}): converting on the CPU");
                return false;
            }
            let (mut cpu, w, h) = (Vec::new(), self.w as usize, self.h as usize);
            bgra_to_yuv420(self.result(), w, h, &mut cpu);
            let (n, h) = (w * h, w * h / 4);
            let max = |a: &[u8], b: &[u8]| a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
            let off = |a: &[u8], b: &[u8]| a.iter().zip(b).filter(|(x, y)| x.abs_diff(**y) > 1).count();
            eprintln!("yuv check: max diff Y {} U {} V {}, {} bytes off by more than 1", max(&cpu[..n], &out[..n]), max(&cpu[n..n + h], &out[n..n + h]), max(&cpu[n + h..], &out[n + h..]), off(&cpu, &out));
            self.checked.push_back(out);
        }
        true
    }

    pub fn gpu_yuv_collect(&mut self, out: &mut Vec<u8>) -> Result<()> {
        if let Some(b) = self.checked.pop_front() {
            *out = b;
            return Ok(());
        }
        self.yuv.as_mut().ok_or_else(|| anyhow!("no GPU YUV"))?.collect(out)
    }

    /// The finished frame (BGRA, premultiplied): straight from the surface, or the blurred composite.
    pub fn result(&mut self) -> &[u8] {
        if self.in_surface {
            if let Some(pm) = self.surface.peek_pixels() {
                if let Some(b) = pm.bytes() {
                    // SAFETY: the pixmap borrows the surface's memory, which lives as long as self
                    return unsafe { std::slice::from_raw_parts(b.as_ptr(), b.len()) };
                }
            }
            let full = IRect::from_wh(self.w, self.h);
            self.read(full, false);
            self.in_surface = false;
        }
        &self.pixels
    }

    /// The live page's lift at t (no motion blur), one part of the frame around element `id` (Lift): Below = the
    /// plate, as JPEG; Solo / Above = on clear, cropped to what was drawn, as PNG + its box in output px (empty:
    /// nothing drawn).
    pub fn lift(&mut self, t: f64, id: &str, part: Lift) -> Result<(Vec<u8>, [i32; 4])> {
        let ops = self.record(t)?;
        let solo = part != Lift::Below;
        self.lift = Some((id.to_string(), part));
        self.paint(&ops, None);
        self.lift = None;
        self.in_surface = true;
        let (w, h) = (self.w as usize, self.h as usize);
        let px = self.result().to_vec();
        let mut bx = [0i32, 0, w as i32, h as i32];
        let (img_px, iw, ih) = if solo {
            let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
            for y in 0..h {
                let row = &px[y * w * 4..(y + 1) * w * 4];
                for x in 0..w {
                    if row[x * 4 + 3] != 0 {
                        x0 = x0.min(x); x1 = x1.max(x + 1); y0 = y0.min(y); y1 = y1.max(y + 1);
                    }
                }
            }
            if x1 <= x0 || y1 <= y0 {
                return Ok((Vec::new(), [0, 0, 0, 0]));
            }
            let mut c = Vec::with_capacity((x1 - x0) * (y1 - y0) * 4);
            for y in y0..y1 {
                c.extend_from_slice(&px[(y * w + x0) * 4..(y * w + x1) * 4]);
            }
            bx = [x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32];
            (c, x1 - x0, y1 - y0)
        } else {
            (px, w, h)
        };
        let info = ImageInfo::new((iw as i32, ih as i32), ColorType::BGRA8888, AlphaType::Premul, None);
        let fmt = if solo { skia_safe::EncodedImageFormat::PNG } else { skia_safe::EncodedImageFormat::JPEG };
        let enc = skia_safe::images::raster_from_data(&info, skia_safe::Data::new_copy(&img_px), iw * 4)
            .and_then(|img| img.encode(None, fmt, 88))
            .ok_or_else(|| anyhow!("lift encode"))?;
        Ok((enc.as_bytes().to_vec(), bx))
    }

    /// Renders the frame at t (see result()).
    pub fn frame(&mut self, t: f64, blur: Option<Blur>, fps: f64, dur: f64) -> Result<()> {
        self.stats.frames += 1;
        self.in_surface = false;
        let full = IRect::from_wh(self.w, self.h);
        let Some(b) = blur else {
            let ops = self.record(t)?;
            self.paint(&ops, None);
            self.in_surface = true;
            return Ok(());
        };
        let half = (b.shutter / 360.) / fps / 2.;
        let tmax = (dur - 1e-6).max(0.);
        let (ta, tb) = ((t - half).clamp(0., tmax), (t + half).clamp(0., tmax));
        let oa = self.record(ta)?;
        let ob = self.record(tb)?;
        let aa = self.analyze(&oa);
        let ab = self.analyze(&ob);
        if aa.hash == ab.hash {
            self.stats.still += 1;
            self.paint(&oa, None);
            self.in_surface = true;
            return Ok(());
        }
        let (disp, _) = diff(&aa, &ab);
        if std::env::var("AIOS_PROF").map(|v| v == "4").unwrap_or(false) {
            for (k, (x, y)) in aa.items.iter().zip(ab.items.iter()).enumerate() {
                if x.h != y.h {
                    eprintln!("  t={t:.3} item {k} op {}: same thing {} same state {} same content {} | a [{:.0},{:.0} {:.0}x{:.0}] b [{:.0},{:.0} {:.0}x{:.0}]", x.kind, x.id == y.id, x.fp == y.fp, x.ext == y.ext,
                        x.b.left, x.b.top, x.b.width(), x.b.height(), y.b.left, y.b.top, y.b.width(), y.b.height());
                }
            }
            if aa.items.len() != ab.items.len() { eprintln!("  t={t:.3} item counts differ: {} vs {}", aa.items.len(), ab.items.len()); }
        }
        if disp < 0.75 {
            // content changed (a video frame, a word, a colour) but nothing travelled: one sample at the frame time
            let ops = self.record(t)?;
            self.paint(&ops, None);
            self.in_surface = true;
            return Ok(());
        }
        let s = ((disp / b.gap).ceil() as usize + 1).clamp(2, b.smax.max(2));
        let times: Vec<f64> = (0..s).map(|k| ta + (tb - ta) * (k as f64 + 0.5) / s as f64).collect();
        let mut recs = Vec::with_capacity(s);
        let mut anas = Vec::with_capacity(s);
        for &tk in &times {
            let o = self.record(tk)?;
            anas.push(self.analyze(&o));
            recs.push(o);
        }
        // a camera never exposes across an edit: samples are grouped by what is drawn (text, image frame, shape);
        // only the group holding the frame time is averaged, so a new word or a video frame flip stays crisp
        let centre = s / 2;
        let (mut g0, mut g1) = (centre, centre);
        while g0 > 0 && anas[g0 - 1].ident == anas[centre].ident { g0 -= 1; }
        while g1 + 1 < s && anas[g1 + 1].ident == anas[centre].ident { g1 += 1; }
        if g0 == g1 {
            self.paint(&recs[centre], None);
            self.in_surface = true;
            return Ok(());
        }
        let recs: Vec<Vec<f64>> = recs.drain(g0..=g1).collect();
        let anas: Vec<Analysis> = anas.drain(g0..=g1).collect();
        let s = recs.len();
        // the region any sample differs from the first one in
        let mut dirty = Rect::new_empty();
        for a in &anas[1..] {
            let (_, d) = diff(&anas[0], a);
            dirty.join(d);
        }
        let mut dirty_i: IRect = dirty.round_out();
        dirty_i.outset((2, 2));
        let Some(dirty_i) = IRect::intersect(&dirty_i, &full) else {
            self.paint(&recs[0], None);
            self.in_surface = true;
            return Ok(());
        };
        let recs = recs;
        self.stats.blurred += 1;
        let t0 = Instant::now();
        let whole = (dirty_i.width() * dirty_i.height()) as usize > (self.w * self.h) as usize * 6 / 10;
        let region = if whole { full } else { dirty_i };
        // sample 0 paints the whole frame (outside the region every sample is the same); the samples are summed in a
        // linear-light half-float surface (Skia converts sRGB -> linear on the way in and back on the way out), on
        // the GPU when this worker paints there
        self.paint(&recs[0], None);
        let (rw, rh) = (region.width(), region.height());
        self.ensure_acc()?;
        let weight = 1.0 / recs.len() as f32;
        let mut add = Paint::default();
        add.set_blend_mode(BlendMode::Plus);
        add.set_alpha_f(weight);
        {
            let acc = self.acc_surface.as_mut().unwrap();
            let c = acc.canvas();
            c.save();
            c.clip_irect(IRect::from_wh(rw, rh), None);
            c.clear(Color::TRANSPARENT);
            c.restore();
        }
        for k in 0..recs.len() {
            if k > 0 {
                self.paint(&recs[k], Some(region));
            }
            let snap = self.surface.image_snapshot_with_bounds(region).ok_or_else(|| anyhow!("snapshot"))?;
            let acc = self.acc_surface.as_mut().unwrap();
            acc.canvas().draw_image(&snap, (0, 0), Some(&add));
        }
        let back = self.acc_surface.as_mut().unwrap().image_snapshot_with_bounds(IRect::from_wh(rw, rh)).ok_or_else(|| anyhow!("snapshot"))?;
        let mut put = Paint::default();
        put.set_blend_mode(BlendMode::Src);
        self.surface.canvas().draw_image(&back, (region.left, region.top), Some(&put));
        self.in_surface = true;
        self.stats.post_ms += ms(t0);
        if std::env::var("AIOS_PROF").map(|v| v == "3").unwrap_or(false) {
            eprintln!("blur t={t:.3} samples={} region={:.0}% disp={disp:.1}px {:.0} ms", recs.len(), 100. * (rw * rh) as f64 / (self.w * self.h) as f64, ms(t0));
        }
        Ok(())
    }

    /// The half-float, linear-light surface the motion-blur samples are summed in (frame size, made once).
    fn ensure_acc(&mut self) -> Result<()> {
        if self.acc_surface.is_some() {
            return Ok(());
        }
        let info = ImageInfo::new((self.w, self.h), ColorType::RGBAF16, AlphaType::Premul, Some(skia_safe::ColorSpace::new_srgb_linear()));
        let s = match self.gpu.as_mut() {
            Some(g) => skia_safe::gpu::surfaces::render_target(&mut g.context, skia_safe::gpu::Budgeted::Yes, &info, None, skia_safe::gpu::SurfaceOrigin::TopLeft, None, false, None),
            None => surfaces::raster(&info, None, None),
        };
        self.acc_surface = Some(s.ok_or_else(|| anyhow!("cannot make the motion-blur surface"))?);
        Ok(())
    }
}

/// Largest edge travel among ops that draw the same thing in both analyses (same identity, other place), and the
/// device region where the two differ at all.
pub fn diff(a: &Analysis, b: &Analysis) -> (f32, Rect) {
    let mut disp = 0f32;
    let mut dirty = Rect::new_empty();
    let travel = |r0: &Rect, r1: &Rect| (r0.left - r1.left).abs().max((r0.top - r1.top).abs()).max((r0.right - r1.right).abs()).max((r0.bottom - r1.bottom).abs());
    if a.items.len() == b.items.len() {
        for (x, y) in a.items.iter().zip(b.items.iter()) {
            if x.h != y.h {
                if !x.b.is_empty() { dirty.join(x.b); }
                if !y.b.is_empty() { dirty.join(y.b); }
                if x.id == y.id && !x.b.is_empty() && !y.b.is_empty() {
                    disp = disp.max(travel(&x.b, &y.b));
                }
            }
        }
    } else {
        // different op lists: what is not in the other list is dirty; travel from ops that share index and identity
        use std::collections::HashMap;
        let mut ca: HashMap<u64, usize> = HashMap::new();
        for x in &a.items { *ca.entry(x.h).or_default() += 1; }
        let mut cb: HashMap<u64, usize> = HashMap::new();
        for y in &b.items { *cb.entry(y.h).or_default() += 1; }
        for x in &a.items { if cb.get(&x.h).copied().unwrap_or(0) == 0 && !x.b.is_empty() { dirty.join(x.b); } }
        for y in &b.items { if ca.get(&y.h).copied().unwrap_or(0) == 0 && !y.b.is_empty() { dirty.join(y.b); } }
        for (x, y) in a.items.iter().zip(b.items.iter()) {
            if x.h != y.h && x.id == y.id && !x.b.is_empty() && !y.b.is_empty() {
                disp = disp.max(travel(&x.b, &y.b));
            }
        }
    }
    (disp, dirty)
}

// ───────────────────────────── the pool ─────────────────────────────

/// What a render task hands back.
#[derive(Clone)]
pub enum Want {
    Yuv,
    Bgra,
    Jpeg,
    File { path: PathBuf, jpg: bool },
    Nothing,
}

enum Task {
    Hash { idx: usize, f: usize },
    Layout { idx: usize, f: usize },
    Render { idx: usize, f: usize, want: Want },
    Lift { f: usize, id: String, part: Lift },
    Quit,
}

enum Out {
    Hash(usize, u64),
    Layout(usize, Vec<Elem>, serde_json::Value),
    Frame(usize, Vec<u8>),
    Lift(Vec<u8>, [i32; 4]),
    Fail(String),
}

/// N render workers that live for the whole command (JS loaded once): they hash frames, then paint the ones needed.
pub struct Pool {
    tx: crossbeam_channel::Sender<Task>,
    rx: crossbeam_channel::Receiver<Out>,
    handles: Vec<std::thread::JoinHandle<Stats>>,
    pub workers: usize,
    pub settings: Settings,
    pub assets: Arc<Assets>,
}

pub struct Report {
    pub frames: usize,
    pub seconds: f64,
    pub stats: Stats,
    pub workers: usize,
    pub images: Vec<(usize, Vec<u8>)>,
}

impl Pool {
    pub fn new(src: &CompSrc, assets: Arc<Assets>, settings: Settings, max_workers: usize) -> Pool {
        let nw = settings.workers.max(1).min(max_workers.max(1));
        let (tx, task_rx) = crossbeam_channel::unbounded::<Task>();
        let (out_tx, rx) = crossbeam_channel::unbounded::<Out>();
        let mut handles = Vec::new();
        for wi in 0..nw {
            let (src, assets, s2, task_rx, out_tx) = (src.clone(), assets.clone(), settings.clone(), task_rx.clone(), out_tx.clone());
            let use_gpu = wi < settings.gpu;
            handles.push(
                std::thread::Builder::new()
                    .stack_size(64 << 20)
                    .name("render".into())
                    .spawn(move || -> Stats {
                        let mut w = match Worker::new(&src, assets, &s2, use_gpu) {
                            Ok(w) => w,
                            Err(e) => {
                                let _ = out_tx.send(Out::Fail(format!("{e:#}")));
                                // keep answering so the caller never waits on a dead worker
                                while let Ok(t) = task_rx.recv() {
                                    if matches!(t, Task::Quit) { break; }
                                    let _ = out_tx.send(Out::Fail(format!("{e:#}")));
                                }
                                return Stats::default();
                            }
                        };
                        let mut yuv = Vec::new();
                        // frames whose YUV is on its way back from the GPU (Worker::gpu_yuv_submit): one is sent once
                        // the next frame is recorded, so the GPU paints and copies it meanwhile; all are sent as soon
                        // as no task waits, and before any other task
                        let mut inflight: std::collections::VecDeque<usize> = Default::default();
                        let send_oldest = |w: &mut Worker, inflight: &mut std::collections::VecDeque<usize>| {
                            if let Some(i) = inflight.pop_front() {
                                let mut b = Vec::new();
                                let o = match w.gpu_yuv_collect(&mut b) { Ok(()) => Out::Frame(i, b), Err(e) => Out::Fail(format!("{e:#}")) };
                                let _ = out_tx.send(o);
                            }
                        };
                        loop {
                            let task = if inflight.is_empty() {
                                match task_rx.recv() { Ok(t) => t, Err(_) => break }
                            } else {
                                match task_rx.try_recv() {
                                    Ok(t) => t,
                                    Err(_) => { while !inflight.is_empty() { send_oldest(&mut w, &mut inflight); } continue; }
                                }
                            };
                            if !matches!(task, Task::Render { want: Want::Yuv, .. }) {
                                while !inflight.is_empty() { send_oldest(&mut w, &mut inflight); }
                            }
                            match task {
                                Task::Quit => break,
                                Task::Hash { idx, f } => {
                                    let t = f as f64 / s2.fps;
                                    match w.hash(t, s2.blur, s2.fps, s2.dur) {
                                        Ok(h) => { let _ = out_tx.send(Out::Hash(idx, h)); }
                                        Err(e) => { let _ = out_tx.send(Out::Fail(format!("frame {f} ({t:.3} s): {e:#}"))); }
                                    }
                                }
                                Task::Layout { idx, f } => {
                                    let t = f as f64 / s2.fps;
                                    match w.layout(t) {
                                        Ok((e, v)) => { let _ = out_tx.send(Out::Layout(idx, e, v)); }
                                        Err(e) => { let _ = out_tx.send(Out::Fail(format!("frame {f} ({t:.3} s): {e:#}"))); }
                                    }
                                }
                                Task::Lift { f, id, part } => {
                                    let t = f as f64 / s2.fps;
                                    match w.lift(t, &id, part) {
                                        Ok((b, bx)) => { let _ = out_tx.send(Out::Lift(b, bx)); }
                                        Err(e) => { let _ = out_tx.send(Out::Fail(format!("{e:#}"))); }
                                    }
                                }
                                Task::Render { idx, f, want } => {
                                    let t = f as f64 / s2.fps;
                                    let _g = FrameSpan(f, Instant::now());
                                    if let Err(e) = w.frame(t, s2.blur, s2.fps, s2.dur) {
                                        let _ = out_tx.send(Out::Fail(format!("frame {f} ({t:.3} s): {e:#}")));
                                        continue;
                                    }
                                    let t0 = Instant::now();
                                    let _p = Span("post", t0);
                                    let out = match want {
                                        Want::Yuv => {
                                            if w.gpu_yuv_submit() {
                                                inflight.push_back(idx);
                                                if inflight.len() > 1 { send_oldest(&mut w, &mut inflight); }
                                                w.stats.post_ms += ms(t0);
                                                continue;
                                            }
                                            let (ow, oh) = (s2.out_w as usize, s2.out_h as usize);
                                            bgra_to_yuv420(w.result(), ow, oh, &mut yuv);
                                            Out::Frame(idx, std::mem::take(&mut yuv))
                                        }
                                        Want::Bgra => Out::Frame(idx, w.result().to_vec()),
                                        Want::Jpeg => {
                                            let px = w.result().to_vec();
                                            let info = ImageInfo::new((s2.out_w as i32, s2.out_h as i32), ColorType::BGRA8888, AlphaType::Premul, None);
                                            let enc = skia_safe::images::raster_from_data(&info, skia_safe::Data::new_copy(&px), (s2.out_w * 4) as usize)
                                                .and_then(|img| img.encode(None, skia_safe::EncodedImageFormat::JPEG, 88));
                                            match enc {
                                                Some(d) => Out::Frame(idx, d.as_bytes().to_vec()),
                                                None => Out::Fail("jpeg encode".into()),
                                            }
                                        }
                                        Want::File { path, jpg } => {
                                            let px = w.result().to_vec();
                                            match write_image(&path, &px, s2.out_w, s2.out_h, jpg) {
                                                Ok(()) => Out::Frame(idx, Vec::new()),
                                                Err(e) => Out::Fail(format!("{e:#}")),
                                            }
                                        }
                                        Want::Nothing => Out::Frame(idx, Vec::new()),
                                    };
                                    w.stats.post_ms += ms(t0);
                                    let _ = out_tx.send(out);
                                }
                            }
                        }
                        w.stats.clone()
                    })
                    .expect("spawn worker"),
            );
        }
        Pool { tx, rx, handles, workers: nw, settings, assets }
    }

    /// Hashes of the given frames (same order).
    pub fn hashes(&self, frames: &[usize]) -> Result<Vec<u64>> {
        for (idx, &f) in frames.iter().enumerate() {
            self.tx.send(Task::Hash { idx, f }).map_err(|_| anyhow!("workers gone"))?;
        }
        let mut out = vec![0u64; frames.len()];
        for _ in 0..frames.len() {
            match self.rx.recv().map_err(|_| anyhow!("workers gone"))? {
                Out::Hash(i, h) => out[i] = h,
                Out::Fail(e) => { self.drain(); return Err(anyhow!(e)); }
                Out::Frame(..) | Out::Layout(..) | Out::Lift(..) => {}
            }
        }
        Ok(out)
    }

    /// The elements of the given frames (same order) and each frame's LAYOUT.frame(t), for `aios lint`.
    pub fn layout(&self, frames: &[usize]) -> Result<Vec<(Vec<Elem>, serde_json::Value)>> {
        for (idx, &f) in frames.iter().enumerate() {
            self.tx.send(Task::Layout { idx, f }).map_err(|_| anyhow!("workers gone"))?;
        }
        let mut out: Vec<(Vec<Elem>, serde_json::Value)> = vec![(Vec::new(), serde_json::Value::Null); frames.len()];
        for _ in 0..frames.len() {
            match self.rx.recv().map_err(|_| anyhow!("workers gone"))? {
                Out::Layout(i, e, v) => out[i] = (e, v),
                Out::Fail(e) => { self.drain(); return Err(anyhow!(e)); }
                Out::Frame(..) | Out::Hash(..) | Out::Lift(..) => {}
            }
        }
        Ok(out)
    }

    /// One part of frame f around element `id` (Worker::lift).
    pub fn lift(&self, f: usize, id: &str, part: Lift) -> Result<(Vec<u8>, [i32; 4])> {
        self.tx.send(Task::Lift { f, id: id.to_string(), part }).map_err(|_| anyhow!("workers gone"))?;
        loop {
            match self.rx.recv().map_err(|_| anyhow!("workers gone"))? {
                Out::Lift(b, bx) => return Ok((b, bx)),
                Out::Fail(e) => return Err(anyhow!(e)),
                _ => {}
            }
        }
    }

    fn drain(&self) {
        while self.rx.try_recv().is_ok() {}
    }

    /// Renders frames and hands each result to `consume` in order (on this thread), keeping at most a window of
    /// frames in flight so memory stays flat.
    pub fn render(&self, frames: &[usize], want: impl Fn(usize) -> Want, mut consume: impl FnMut(usize, Vec<u8>) -> Result<()>, progress: bool) -> Result<f64> {
        let t0 = Instant::now();
        let n = frames.len();
        let per = std::env::var("AIOS_WINDOW").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(3);
        let window = (self.workers * per).max(8).min(n.max(1));
        let (mut sent, mut received, mut done) = (0usize, 0usize, 0usize);
        let mut pending: std::collections::BTreeMap<usize, Vec<u8>> = std::collections::BTreeMap::new();
        let mut last = Instant::now();
        let mut failure: Option<anyhow::Error> = None;
        while sent < window.min(n) {
            self.tx.send(Task::Render { idx: sent, f: frames[sent], want: want(frames[sent]) }).map_err(|_| anyhow!("workers gone"))?;
            sent += 1;
        }
        while received < sent {
            let t_idle = Instant::now();
            let msg = self.rx.recv().map_err(|_| anyhow!("workers gone"))?;
            trace::add_since(trace::C::DispatchIdle, t_idle);
            trace::span("dispatch", t_idle, || "idle: no frame ready".into());
            received += 1;
            if failure.is_some() {
                continue; // drain what is in flight so the pool stays usable
            }
            match msg {
                Out::Frame(i, buf) => { pending.insert(i, buf); }
                Out::Fail(e) => { failure = Some(anyhow!(e)); continue; }
                Out::Hash(..) | Out::Layout(..) | Out::Lift(..) => {}
            }
            while let Some(buf) = pending.remove(&done) {
                let t_c = Instant::now();
                let r = consume(frames[done], buf);
                trace::add_since(trace::C::Consume, t_c);
                trace::span("dispatch", t_c, || format!("write {}", frames[done]));
                if let Err(e) = r {
                    failure = Some(e);
                    break;
                }
                done += 1;
                if sent < n {
                    self.tx.send(Task::Render { idx: sent, f: frames[sent], want: want(frames[sent]) }).map_err(|_| anyhow!("workers gone"))?;
                    sent += 1;
                }
            }
            if progress && last.elapsed().as_secs_f64() > 2. {
                eprintln!("  {done}/{n} frames  {:.1} s", t0.elapsed().as_secs_f64());
                last = Instant::now();
            }
        }
        if let Some(e) = failure {
            return Err(e);
        }
        Ok(t0.elapsed().as_secs_f64())
    }

    /// Stops the workers and returns their summed counters.
    pub fn close(self) -> Stats {
        for _ in 0..self.handles.len() {
            let _ = self.tx.send(Task::Quit);
        }
        let mut st = Stats::default();
        for h in self.handles {
            if let Ok(s) = h.join() { st.add(&s); }
        }
        self.assets.stop();
        st
    }
}

/// One-shot helpers on a fresh pool: a video file, image files, frames in memory, or nothing (bench).
pub fn render_once(src: &CompSrc, assets: Arc<Assets>, settings: Settings, frames: Vec<usize>, sink: Sink, progress: bool) -> Result<Report> {
    let t0 = Instant::now();
    let pool = Pool::new(src, assets, settings.clone(), frames.len());
    let workers = pool.workers;
    let mut images = Vec::new();
    let res = match &sink {
        Sink::Video { out, enc } => {
            let mut e = Encoder::new(out, settings.out_w, settings.out_h, settings.fps, enc)?;
            let r = pool.render(&frames, |_| Want::Yuv, |_, b| e.write(&b), progress);
            match r { Ok(_) => e.finish().map(|_| ()), Err(x) => Err(x) }
        }
        Sink::Frames { dir, jpg } => pool
            .render(&frames, |f| Want::File { path: dir.join(format!("{:05}.{}", f, if *jpg { "jpg" } else { "png" })), jpg: *jpg }, |_, _| Ok(()), progress)
            .map(|_| ()),
        Sink::Memory => pool.render(&frames, |_| Want::Bgra, |f, b| { images.push((f, b)); Ok(()) }, progress).map(|_| ()),
        Sink::Null => pool.render(&frames, |_| Want::Nothing, |_, _| Ok(()), progress).map(|_| ()),
    };
    let stats = pool.close();
    res?;
    Ok(Report { frames: frames.len(), seconds: t0.elapsed().as_secs_f64(), stats, workers, images })
}

pub enum Sink {
    Video { out: PathBuf, enc: EncOpts },
    Frames { dir: PathBuf, jpg: bool },
    Memory,
    Null,
}

pub fn write_image(path: &Path, bgra: &[u8], w: u32, h: u32, jpg: bool) -> Result<()> {
    if jpg {
        let info = ImageInfo::new((w as i32, h as i32), ColorType::BGRA8888, AlphaType::Premul, None);
        let img = skia_safe::images::raster_from_data(&info, skia_safe::Data::new_copy(bgra), (w * 4) as usize).ok_or_else(|| anyhow!("image"))?;
        let data = img.encode(None, skia_safe::EncodedImageFormat::JPEG, 90).ok_or_else(|| anyhow!("jpeg encode"))?;
        std::fs::write(path, data.as_bytes())?;
        return Ok(());
    }
    // PNG, fast deflate, RGBA unpremultiplied
    let mut rgba = vec![0u8; bgra.len()];
    for (o, p) in rgba.chunks_exact_mut(4).zip(bgra.chunks_exact(4)) {
        let a = p[3] as u32;
        if a == 255 || a == 0 {
            o[0] = p[2]; o[1] = p[1]; o[2] = p[0];
        } else {
            o[0] = ((p[2] as u32 * 255 + a / 2) / a).min(255) as u8;
            o[1] = ((p[1] as u32 * 255 + a / 2) / a).min(255) as u8;
            o[2] = ((p[0] as u32 * 255 + a / 2) / a).min(255) as u8;
        }
        o[3] = p[3];
    }
    let f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut e = png::Encoder::new(f, w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.set_compression(png::Compression::Fast);
    let mut wr = e.write_header()?;
    wr.write_image_data(&rgba)?;
    Ok(())
}
