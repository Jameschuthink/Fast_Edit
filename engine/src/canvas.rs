//! The Canvas2D interpreter: replays the op record a composition made in JS onto a Skia canvas (Draw), or computes
//! what every draw would touch without painting (Analyze: one (hash, device box) per draw op), which the renderer
//! uses to skip unchanged frames and to size motion blur.
//!
//! Semantics follow the HTML spec where they matter for motion graphics: paths are fixed in device space when a
//! point is added (so a transform change mid-path behaves like the browser), shadows and filters ignore the
//! composition's transform, stroke widths follow it. The engine's base scale K (draft size / world size) applies
//! to everything, so a 540 px draft is the 1080 px frame scaled — shadows and blur included.

use crate::assets::Assets;
use crate::color;
use crate::text::Shaper;
use skia_safe::{
    canvas::{SaveLayerRec, SrcRectConstraint},
    color_filters, image_filters, BlendMode, BlurStyle, Canvas, ClipOp, Color4f, ColorMatrix, CubicResampler, FilterMode,
    ImageFilter, MaskFilter, Matrix, MipmapMode, Paint, PaintCap, PaintJoin, PaintStyle, Path, PathBuilder, PathEffect,
    PathFillType, Point, Rect, SamplingOptions, Shader, TileMode, M44,
};
use std::rc::Rc;
use xxhash_rust::xxh3::Xxh3;

// op codes — keep in step with js/prelude.js
pub const SAVE: u32 = 1;
pub const RESTORE: u32 = 2;
pub const SET_TRANSFORM: u32 = 3;
pub const TRANSFORM: u32 = 4;
pub const TRANSLATE: u32 = 5;
pub const SCALE: u32 = 6;
pub const ROTATE: u32 = 7;
pub const RESET_TRANSFORM: u32 = 8;
pub const ALPHA: u32 = 10;
pub const COMPOSITE: u32 = 11;
pub const FILL_STYLE: u32 = 12;
pub const STROKE_STYLE: u32 = 13;
pub const FILL_GRAD: u32 = 14;
pub const STROKE_GRAD: u32 = 15;
pub const LINE_WIDTH: u32 = 16;
pub const LINE_CAP: u32 = 17;
pub const LINE_JOIN: u32 = 18;
pub const MITER: u32 = 19;
pub const DASH: u32 = 20;
pub const DASH_OFFSET: u32 = 21;
pub const SHADOW_COLOR: u32 = 22;
pub const SHADOW_BLUR: u32 = 23;
pub const SHADOW_OFFSET: u32 = 24;
pub const FONT: u32 = 25;
pub const TEXT_ALIGN: u32 = 26;
pub const TEXT_BASELINE: u32 = 27;
pub const LETTER_SPACING: u32 = 28;
pub const FILTER: u32 = 29;
pub const SMOOTHING: u32 = 30;
pub const WORD_SPACING: u32 = 31;
pub const FILL_PATTERN: u32 = 32;
pub const STROKE_PATTERN: u32 = 33;
pub const BEGIN_PATH: u32 = 40;
pub const MOVE_TO: u32 = 41;
pub const LINE_TO: u32 = 42;
pub const QUAD_TO: u32 = 43;
pub const BEZIER_TO: u32 = 44;
pub const ARC: u32 = 45;
pub const ARC_TO: u32 = 46;
pub const ELLIPSE: u32 = 47;
pub const RECT: u32 = 48;
pub const ROUND_RECT: u32 = 49;
pub const CLOSE_PATH: u32 = 50;
pub const PATH_SVG: u32 = 51;
pub const PATH2D_BEGIN: u32 = 52;
pub const PATH2D_END: u32 = 53;
pub const PATH2D_POP: u32 = 54;
pub const FILL: u32 = 60;
pub const STROKE: u32 = 61;
pub const CLIP: u32 = 62;
pub const FILL_RECT: u32 = 63;
pub const STROKE_RECT: u32 = 64;
pub const CLEAR_RECT: u32 = 65;
pub const FILL_TEXT: u32 = 66;
pub const STROKE_TEXT: u32 = 67;
pub const DRAW_IMAGE: u32 = 68;
pub const BEGIN_LAYER: u32 = 70;
pub const END_LAYER: u32 = 71;
pub const TEXT_BOX: u32 = 72;
pub const MARK: u32 = 80;

/// Strings a worker's JS interned, by id: (text, content hash).
#[derive(Default)]
pub struct StrTable {
    pub list: Vec<(Rc<str>, u64)>,
}

impl StrTable {
    pub fn push(&mut self, s: String) {
        let h = xxhash_rust::xxh3::xxh3_64(s.as_bytes());
        self.list.push((Rc::from(s), h));
    }
    fn get(&self, id: f64) -> (&str, u64) {
        match self.list.get(id as usize) {
            Some((s, h)) => (s, *h),
            None => ("", 0),
        }
    }
}

#[derive(Clone)]
enum Style {
    Color(Color4f),
    Grad(Rc<Grad>),
    Pat(Rc<Pat>),
}

struct Grad {
    kind: u8,
    c: [f32; 6],
    stops: Vec<(f32, Color4f)>,
    hash: u64,
}

struct Pat {
    id: usize,
    frame: usize,
    rep: u8,
    m: Matrix,
    hash: u64,
}

enum FOp {
    Blur(f32),
    Matrix(ColorMatrix),
    Opacity(f32),
    Shadow(f32, f32, f32, Color4f),
}

struct Filter {
    ops: Vec<FOp>,
    hash: u64,
    outset: f32, // world units the filter can spread a draw (3 sigma + offsets)
}

#[derive(Clone)]
struct St {
    ctm: Matrix,
    alpha: f32,
    comp: u8,
    fill: Style,
    stroke: Style,
    lw: f32,
    cap: u8,
    join: u8,
    miter: f32,
    dash: Rc<Vec<f32>>,
    dash_off: f32,
    sh_col: Color4f,
    sh_blur: f32,
    sh_x: f32,
    sh_y: f32,
    font: Rc<str>,
    font_h: u64,
    align: u8,
    baseline: u8,
    ls: f32,
    ws: f32,
    filter: Option<Rc<Filter>>,
    smooth: bool,
    smooth_q: u8,
    clip: Rect,
    clip_h: u64,
    layer_h: u64,
    layer_out: f32,
    fp: Option<u64>,
    fp_k: [Option<u64>; 6],
}

/// One draw op as the analysis sees it: `id` says what is drawn (text, font, image + frame, path shape),
/// `h` adds where and how (transform, style, clip), `b` is the device box it can touch.
#[derive(Clone, Copy)]
pub struct Item {
    pub id: u64,
    pub h: u64,
    pub b: Rect,
    pub kind: u32,
    pub fp: u64,    // the state part of the hash (debugging)
    pub ext: u64,   // the content part
}

/// What a frame touches: one entry per draw op, in order.
#[derive(Default, Clone)]
pub struct Analysis {
    pub items: Vec<Item>,
    pub hash: u64,
    pub ident: u64,
    /// `aios lint` only (None otherwise): the frame's elements, for the layout check
    pub capture: Option<Capture>,
}

/// The elements of one frame: the draws between g.mark(label) and g.unmark() are one element; a text drawn outside
/// any mark is an element of its own (label ""). Boxes are device boxes without shadows, clipped.
#[derive(Default, Clone, Debug)]
pub struct Capture {
    pub elems: Vec<Elem>,
    open: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct Elem {
    pub label: String,
    pub b: Rect,
    pub alpha: f32,
    pub texts: Vec<TextDraw>,
}

#[derive(Clone, Debug)]
pub struct TextDraw {
    pub text: String,
    /// world px of the font as drawn (the transform's scale included)
    pub px: f32,
    /// the fill colour (unpremultiplied sRGB); None for a gradient, a pattern or a stroke
    pub color: Option<[f32; 4]>,
    pub b: Rect,
    pub alpha: f32,
}

/// The px size in a CSS font string ("600 92px 'Geist', sans-serif" -> 92).
fn font_px(font: &str) -> f32 {
    font.split_whitespace().find_map(|w| w.strip_suffix("px").and_then(|n| n.parse::<f32>().ok())).unwrap_or(10.)
}

pub static PROF: std::sync::Mutex<Option<std::collections::HashMap<u32, (u64, u64)>>> = std::sync::Mutex::new(None);

pub fn prof_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("AIOS_PROF").map(|v| v == "2").unwrap_or(false))
}

pub fn prof_report() {
    if let Some(m) = PROF.lock().unwrap().as_ref() {
        let mut v: Vec<_> = m.iter().collect();
        v.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
        for (op, (n, ns)) in v {
            eprintln!("  op {op:>3}: {n:>7} calls {:>9.1} ms total {:>7.3} ms each", *ns as f64 / 1e6, *ns as f64 / 1e6 / *n as f64);
        }
    }
}

pub struct Exec<'a> {
    pub strs: &'a StrTable,
    pub assets: &'a Assets,
    pub shaper: &'a mut Shaper,
    canvas: Option<&'a Canvas>,
    ana: Option<&'a mut Analysis>,
    /// the next record()'s box for the lint capture when it is tighter than the paint box (a stroke without its miter allowance)
    core: Option<Rect>,
    base: Matrix,
    k: f32,
    size: Rect,
    st: St,
    stack: Vec<St>,
    path: PathBuilder,
    path_h: u64,
    verbs_h: u64,
    has_sub: bool,
    cur: Point,
    start: Point,
    stash: Vec<(PathBuilder, u64, u64, bool, Point, Point)>,
    layers: Vec<usize>,
    pub warnings: Vec<String>,
    frame_h: Xxh3,
    /// the live page's lift (a frame drag): which part of the frame to paint around one marked element.
    /// State ops still run, so transforms and clips stay right.
    lift: Option<(String, Lift)>,
    lift_marks: Vec<bool>,
    lift_depth: usize,
    lift_closed: bool,
}

/// The three parts of a frame around one element (marked id): what is drawn before it (the plate), the element
/// alone, what is drawn after it (the head matte over a word behind it, the captions). Stacked, they are the frame.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Lift {
    Below,
    Solo,
    Above,
}

fn fresh(base: Matrix, size: Rect) -> St {
    St {
        ctm: base,
        alpha: 1.,
        comp: 0,
        fill: Style::Color(Color4f::new(0., 0., 0., 1.)),
        stroke: Style::Color(Color4f::new(0., 0., 0., 1.)),
        lw: 1.,
        cap: 0,
        join: 0,
        miter: 10.,
        dash: Rc::new(Vec::new()),
        dash_off: 0.,
        sh_col: Color4f::new(0., 0., 0., 0.),
        sh_blur: 0.,
        sh_x: 0.,
        sh_y: 0.,
        font: Rc::from("10px sans-serif"),
        font_h: 0,
        align: 0,
        baseline: 0,
        ls: 0.,
        ws: 0.,
        filter: None,
        smooth: true,
        smooth_q: 0,
        clip: size,
        clip_h: 0,
        layer_h: 0,
        layer_out: 0.,
        fp: None,
        fp_k: [None; 6],
    }
}

fn blend(k: u8) -> BlendMode {
    match k {
        1 => BlendMode::SrcIn,
        2 => BlendMode::SrcOut,
        3 => BlendMode::SrcATop,
        4 => BlendMode::DstOver,
        5 => BlendMode::DstIn,
        6 => BlendMode::DstOut,
        7 => BlendMode::DstATop,
        8 => BlendMode::Plus,
        9 => BlendMode::Src,
        10 => BlendMode::Xor,
        11 => BlendMode::Multiply,
        12 => BlendMode::Screen,
        13 => BlendMode::Overlay,
        14 => BlendMode::Darken,
        15 => BlendMode::Lighten,
        16 => BlendMode::ColorDodge,
        17 => BlendMode::ColorBurn,
        18 => BlendMode::HardLight,
        19 => BlendMode::SoftLight,
        20 => BlendMode::Difference,
        21 => BlendMode::Exclusion,
        22 => BlendMode::Hue,
        23 => BlendMode::Saturation,
        24 => BlendMode::Color,
        25 => BlendMode::Luminosity,
        _ => BlendMode::SrcOver,
    }
}

fn mat(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) -> Matrix {
    Matrix::new_all(a as f32, c as f32, e as f32, b as f32, d as f32, f as f32, 0., 0., 1.)
}

fn hash_f32s(h: &mut Xxh3, v: &[f32]) {
    for x in v {
        h.update(&x.to_bits().to_le_bytes());
    }
}

/// Quantised hashing: differences below what can show (1/64 px; 1/8192 of scale or skew) must not make a draw look
/// "changed" — a decaying camera shake keeps adding 1e-35 rad for ever, which would otherwise dirty every frame.
fn hash_q(h: &mut Xxh3, v: &[f32], step: f32) {
    for x in v {
        let q = (x / step).round();
        let q = if q.is_finite() { q as i64 } else { i64::MAX };
        h.update(&q.to_le_bytes());
    }
}

fn hash_mat(h: &mut Xxh3, m: &Matrix) {
    let v = mat_vals(m);
    hash_q(h, &v[..4], 1. / 8192.);
    hash_q(h, &v[4..], 1. / 64.);
}

fn mat_vals(m: &Matrix) -> [f32; 6] {
    [m.scale_x(), m.skew_y(), m.skew_x(), m.scale_y(), m.translate_x(), m.translate_y()]
}

/// sqrt(|det|) of the linear part: how much the transform scales lengths on average.
fn mean_scale(m: &Matrix) -> f32 {
    let det = m.scale_x() * m.scale_y() - m.skew_x() * m.skew_y();
    det.abs().sqrt()
}

impl<'a> Exec<'a> {
    pub fn new(strs: &'a StrTable, assets: &'a Assets, shaper: &'a mut Shaper, k: f32, w: f32, h: f32,
        canvas: Option<&'a Canvas>, ana: Option<&'a mut Analysis>) -> Self {
        let base = Matrix::scale((k, k));
        let size = Rect::from_wh(w, h);
        Exec {
            strs, assets, shaper, canvas, ana, base, k, size,
            st: fresh(base, size),
            stack: Vec::new(),
            path: PathBuilder::new(),
            path_h: 0,
            verbs_h: 0,
            has_sub: false,
            cur: Point::default(),
            start: Point::default(),
            stash: Vec::new(),
            layers: Vec::new(),
            warnings: Vec::new(),
            frame_h: Xxh3::new(),
            core: None,
            lift: None,
            lift_marks: Vec::new(),
            lift_depth: 0,
            lift_closed: false,
        }
    }

    pub fn set_lift(&mut self, lift: Option<(String, Lift)>) {
        self.lift = lift;
    }

    /// a draw the lift leaves out
    fn muted(&self) -> bool {
        let inside = self.lift_depth > 0;
        match self.lift.as_ref().map(|l| l.1) {
            None => false,
            Some(Lift::Solo) => !inside,
            Some(Lift::Below) => inside || self.lift_closed,
            Some(Lift::Above) => inside || !self.lift_closed,
        }
    }

    // ───────────────────────────── the loop ─────────────────────────────
    pub fn run(&mut self, ops: &[f64]) {
        let n = ops.len();
        let mut i = 0;
        macro_rules! a {
            ($k:expr) => {
                ops.get(i + $k).copied().unwrap_or(0.)
            };
        }
        let prof = prof_on() && self.canvas.is_some();
        let mut local: std::collections::HashMap<u32, (u64, u64)> = std::collections::HashMap::new();
        while i < n {
            let op = ops[i] as u32;
            i += 1;
            let t_op = if prof { Some(std::time::Instant::now()) } else { None };
            match op {
                SAVE => {
                    self.stack.push(self.st.clone());
                    if let Some(c) = self.canvas {
                        c.save();
                    }
                }
                RESTORE => {
                    if let Some(s) = self.stack.pop() {
                        // a layer opened after the matching save closes with it
                        while let Some(&depth) = self.layers.last() {
                            if depth > self.stack.len() {
                                self.layers.pop();
                                if let Some(c) = self.canvas {
                                    c.restore();
                                }
                            } else {
                                break;
                            }
                        }
                        self.st = s;
                        if let Some(c) = self.canvas {
                            c.restore();
                        }
                    }
                }
                SET_TRANSFORM => {
                    let m = mat(a!(0), a!(1), a!(2), a!(3), a!(4), a!(5));
                    i += 6;
                    if m.is_finite() {
                        self.set_ctm(Matrix::concat(&self.base, &m));
                    }
                }
                TRANSFORM => {
                    let m = mat(a!(0), a!(1), a!(2), a!(3), a!(4), a!(5));
                    i += 6;
                    if m.is_finite() {
                        self.set_ctm(Matrix::concat(&self.st.ctm, &m));
                    }
                }
                TRANSLATE => {
                    let (x, y) = (a!(0) as f32, a!(1) as f32);
                    i += 2;
                    if x.is_finite() && y.is_finite() {
                        self.set_ctm(Matrix::concat(&self.st.ctm, &Matrix::translate((x, y))));
                    }
                }
                SCALE => {
                    let (x, y) = (a!(0) as f32, a!(1) as f32);
                    i += 2;
                    if x.is_finite() && y.is_finite() {
                        self.set_ctm(Matrix::concat(&self.st.ctm, &Matrix::scale((x, y))));
                    }
                }
                ROTATE => {
                    let r = a!(0) as f32;
                    i += 1;
                    if r.is_finite() {
                        self.set_ctm(Matrix::concat(&self.st.ctm, &Matrix::rotate_rad(r)));
                    }
                }
                RESET_TRANSFORM => self.set_ctm(self.base),
                ALPHA => {
                    self.st.alpha = (a!(0) as f32).clamp(0., 1.);
                    i += 1;
                    self.st.fp_k = [None; 6];
                }
                COMPOSITE => {
                    self.st.comp = a!(0) as u8;
                    i += 1;
                    self.st.fp_k = [None; 6];
                }
                FILL_STYLE | STROKE_STYLE | SHADOW_COLOR => {
                    let (s, _) = self.strs.get(a!(0));
                    i += 1;
                    if let Some(c) = color::parse(s) {
                        match op {
                            FILL_STYLE => self.st.fill = Style::Color(c),
                            STROKE_STYLE => self.st.stroke = Style::Color(c),
                            _ => self.st.sh_col = c,
                        }
                        self.st.fp_k = [None; 6];
                    }
                }
                FILL_GRAD | STROKE_GRAD => {
                    let kind = a!(0) as u8;
                    let nc = [4usize, 6, 3][(kind as usize).min(2)];
                    let mut c = [0f32; 6];
                    for j in 0..nc {
                        c[j] = a!(1 + j) as f32;
                    }
                    let ns = a!(1 + nc) as usize;
                    let mut h = Xxh3::new();
                    h.update(&[kind]);
                    hash_f32s(&mut h, &c);
                    let mut stops = Vec::with_capacity(ns);
                    for j in 0..ns {
                        let off = a!(2 + nc + 2 * j) as f32;
                        let (cs, ch) = self.strs.get(a!(3 + nc + 2 * j));
                        h.update(&off.to_bits().to_le_bytes());
                        h.update(&ch.to_le_bytes());
                        if let Some(col) = color::parse(cs) {
                            if off.is_finite() && (0. ..=1.).contains(&off) {
                                stops.push((off, col));
                            }
                        }
                    }
                    i += 2 + nc + 2 * ns;
                    stops.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
                    let g = Style::Grad(Rc::new(Grad { kind, c, stops, hash: h.digest() }));
                    if op == FILL_GRAD { self.st.fill = g } else { self.st.stroke = g }
                    self.st.fp_k = [None; 6];
                }
                FILL_PATTERN | STROKE_PATTERN => {
                    let (id, frame, rep) = (a!(0) as usize, a!(1) as usize, a!(2) as u8);
                    let m = mat(a!(3), a!(4), a!(5), a!(6), a!(7), a!(8));
                    i += 9;
                    let mut h = Xxh3::new();
                    h.update(&self.assets.content_hash(id, frame).to_le_bytes());
                    h.update(&[rep]);
                    hash_f32s(&mut h, &mat_vals(&m));
                    let p = Style::Pat(Rc::new(Pat { id, frame, rep, m, hash: h.digest() }));
                    if op == FILL_PATTERN { self.st.fill = p } else { self.st.stroke = p }
                    self.st.fp_k = [None; 6];
                }
                LINE_WIDTH => { self.st.lw = a!(0) as f32; i += 1; self.st.fp_k = [None; 6]; }
                LINE_CAP => { self.st.cap = a!(0) as u8; i += 1; self.st.fp_k = [None; 6]; }
                LINE_JOIN => { self.st.join = a!(0) as u8; i += 1; self.st.fp_k = [None; 6]; }
                MITER => { self.st.miter = a!(0) as f32; i += 1; self.st.fp_k = [None; 6]; }
                DASH => {
                    let n2 = a!(0) as usize;
                    let v: Vec<f32> = (0..n2).map(|j| a!(1 + j) as f32).collect();
                    i += 1 + n2;
                    self.st.dash = Rc::new(v);
                    self.st.fp_k = [None; 6];
                }
                DASH_OFFSET => { self.st.dash_off = a!(0) as f32; i += 1; self.st.fp_k = [None; 6]; }
                SHADOW_BLUR => { self.st.sh_blur = a!(0) as f32; i += 1; self.st.fp_k = [None; 6]; }
                SHADOW_OFFSET => { self.st.sh_x = a!(0) as f32; self.st.sh_y = a!(1) as f32; i += 2; self.st.fp_k = [None; 6]; }
                FONT => {
                    let (s, h) = self.strs.get(a!(0));
                    i += 1;
                    if self.shaper.spec(s).is_some() {
                        self.st.font = Rc::from(s);
                        self.st.font_h = h;
                        self.st.fp_k = [None; 6];
                    }
                }
                TEXT_ALIGN => { self.st.align = a!(0) as u8; i += 1; self.st.fp_k = [None; 6]; }
                TEXT_BASELINE => { self.st.baseline = a!(0) as u8; i += 1; self.st.fp_k = [None; 6]; }
                LETTER_SPACING => { self.st.ls = a!(0) as f32; i += 1; self.st.fp_k = [None; 6]; }
                WORD_SPACING => { self.st.ws = a!(0) as f32; i += 1; self.st.fp_k = [None; 6]; }
                FILTER => {
                    let (s, h) = self.strs.get(a!(0));
                    i += 1;
                    self.st.filter = parse_filter(s, h);
                    self.st.fp_k = [None; 6];
                }
                SMOOTHING => { self.st.smooth = a!(0) != 0.; self.st.smooth_q = a!(1) as u8; i += 2; self.st.fp_k = [None; 6]; }
                // ── paths ──
                BEGIN_PATH => self.begin_path(),
                MOVE_TO => { let p = self.dev(a!(0), a!(1)); i += 2; self.move_to(p); }
                LINE_TO => { let p = self.dev(a!(0), a!(1)); i += 2; self.line_to(p); }
                QUAD_TO => {
                    let (c1, p) = (self.dev(a!(0), a!(1)), self.dev(a!(2), a!(3)));
                    i += 4;
                    self.ensure(c1);
                    self.path.quad_to(c1, p);
                    self.cur = p;
                    self.hpath(&[QUAD_TO as f32, c1.x, c1.y, p.x, p.y]);
                }
                BEZIER_TO => {
                    let (c1, c2, p) = (self.dev(a!(0), a!(1)), self.dev(a!(2), a!(3)), self.dev(a!(4), a!(5)));
                    i += 6;
                    self.ensure(c1);
                    self.path.cubic_to(c1, c2, p);
                    self.cur = p;
                    self.hpath(&[BEZIER_TO as f32, c1.x, c1.y, c2.x, c2.y, p.x, p.y]);
                }
                ARC => {
                    let (x, y, r, a0, a1, ccw) = (a!(0), a!(1), a!(2), a!(3), a!(4), a!(5) != 0.);
                    i += 6;
                    self.ellipse(x, y, r, r, 0., a0, a1, ccw);
                }
                ELLIPSE => {
                    let (x, y, rx, ry, rot, a0, a1, ccw) = (a!(0), a!(1), a!(2), a!(3), a!(4), a!(5), a!(6), a!(7) != 0.);
                    i += 8;
                    self.ellipse(x, y, rx, ry, rot, a0, a1, ccw);
                }
                ARC_TO => {
                    let (x1, y1, x2, y2, r) = (a!(0), a!(1), a!(2), a!(3), a!(4));
                    i += 5;
                    self.arc_to(x1, y1, x2, y2, r);
                }
                RECT => {
                    let (x, y, w, h) = (a!(0), a!(1), a!(2), a!(3));
                    i += 4;
                    self.rect_path(x, y, w, h);
                }
                ROUND_RECT => {
                    let (x, y, w, h) = (a!(0), a!(1), a!(2), a!(3));
                    let mut r = [0f64; 8];
                    for j in 0..8 {
                        r[j] = a!(4 + j);
                    }
                    i += 12;
                    self.round_rect(x, y, w, h, r);
                }
                CLOSE_PATH => {
                    if self.has_sub {
                        self.path.close();
                        self.cur = self.start;
                        self.hpath(&[CLOSE_PATH as f32]);
                    }
                }
                PATH_SVG => {
                    let (s, _) = self.strs.get(a!(0));
                    i += 1;
                    if let Some(p) = Path::from_svg(s) {
                        let d = p.make_transform(&self.st.ctm);
                        for (k, v) in d.points().iter().enumerate() {
                            if k < 64 { self.hpath(&[v.x, v.y]); }
                        }
                        self.path.add_path(&d, None);
                        if let Some(lp) = d.last_pt() {
                            self.cur = lp;
                            self.has_sub = true;
                        }
                    } else {
                        self.warn(format!("Path2D: cannot parse SVG path '{}'", &s[..s.len().min(40)]));
                    }
                }
                PATH2D_BEGIN => {
                    let pb = std::mem::replace(&mut self.path, PathBuilder::new());
                    self.stash.push((pb, self.path_h, self.verbs_h, self.has_sub, self.cur, self.start));
                    self.path_h = 0;
                    self.verbs_h = 0;
                    self.has_sub = false;
                }
                PATH2D_END => {}
                PATH2D_POP => {
                    if let Some((pb, h, vh, hs, c, s)) = self.stash.pop() {
                        self.path = pb;
                        self.path_h = h;
                        self.verbs_h = vh;
                        self.has_sub = hs;
                        self.cur = c;
                        self.start = s;
                    }
                }
                // ── drawing ──
                FILL => {
                    let evenodd = a!(0) != 0.;
                    i += 1;
                    let p = self.path_snapshot(evenodd);
                    if !self.muted() { self.fill_dev(&p, None); }
                }
                STROKE => {
                    let p = self.path_snapshot(false);
                    if !self.muted() { self.stroke_dev(&p); }
                }
                CLIP => {
                    let evenodd = a!(0) != 0.;
                    i += 1;
                    let p = self.path_snapshot(evenodd);
                    self.clip_dev(&p);
                }
                FILL_RECT | STROKE_RECT | CLEAR_RECT => {
                    let (x, y, w, h) = (a!(0) as f32, a!(1) as f32, a!(2) as f32, a!(3) as f32);
                    i += 4;
                    if !(x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite()) || self.muted() {
                        continue;
                    }
                    let r = Rect::from_xywh(x, y, w, h);
                    let user = if op == STROKE_RECT {
                        let mut pb = PathBuilder::new();
                        pb.move_to((r.left, r.top)).line_to((r.right, r.top)).line_to((r.right, r.bottom)).line_to((r.left, r.bottom)).close();
                        pb.detach()
                    } else {
                        Path::rect(r.sorted(), None)
                    };
                    let p = user.make_transform(&self.st.ctm);
                    match op {
                        FILL_RECT => self.fill_dev(&p, Some(&[op as f32, x, y, w, h])),
                        STROKE_RECT => self.stroke_dev(&p),
                        _ => self.clear_dev(&p),
                    }
                }
                FILL_TEXT | STROKE_TEXT => {
                    let (sid, x, y, mw) = (a!(0), a!(1) as f32, a!(2) as f32, a!(3) as f32);
                    i += 4;
                    if !self.muted() { self.text(sid, x, y, mw, op == STROKE_TEXT, None); }
                }
                TEXT_BOX => {
                    let (sid, x, y, mw, al, lh, stroke) = (a!(0), a!(1) as f32, a!(2) as f32, a!(3) as f32, a!(4) as u8, a!(5) as f32, a!(6) != 0.);
                    i += 7;
                    if !self.muted() { self.text(sid, x, y, -1., stroke, Some((mw, al, lh))); }
                }
                DRAW_IMAGE => {
                    let id = a!(0) as usize;
                    let frame = a!(1) as usize;
                    let v: [f32; 8] = [a!(2) as f32, a!(3) as f32, a!(4) as f32, a!(5) as f32, a!(6) as f32, a!(7) as f32, a!(8) as f32, a!(9) as f32];
                    i += 10;
                    if !self.muted() { self.image(id, frame, v); }
                }
                BEGIN_LAYER => {
                    let (alpha, comp) = (a!(0) as f32, a!(1) as u8);
                    let (fs, fh) = self.strs.get(a!(2));
                    i += 4;
                    self.begin_layer(alpha, comp, fs, fh);
                }
                END_LAYER => {
                    if let Some(depth) = self.layers.pop() {
                        let _ = depth;
                        if let Some(c) = self.canvas {
                            c.restore();
                        }
                    }
                }
                MARK => {
                    let strs = self.strs;
                    let (label, _) = strs.get(a!(0));
                    i += 1;
                    if let Some((id, _)) = &self.lift {
                        if let Some(l) = label.strip_prefix('+') {
                            let hit = serde_json::from_str::<serde_json::Value>(l).ok().map_or(false, |v| v["id"].as_str() == Some(id.as_str()));
                            self.lift_marks.push(hit);
                            if hit { self.lift_depth += 1; }
                        } else if self.lift_marks.pop() == Some(true) {
                            self.lift_depth -= 1;
                            if self.lift_depth == 0 { self.lift_closed = true; }
                        }
                    }
                    if let Some(c) = self.ana.as_deref_mut().and_then(|a| a.capture.as_mut()) {
                        if let Some(l) = label.strip_prefix('+') {
                            c.elems.push(Elem { label: l.to_string(), b: Rect::new_empty(), alpha: 0., texts: Vec::new() });
                            c.open.push(c.elems.len() - 1);
                        } else {
                            c.open.pop();
                        }
                    }
                }
                _ => {
                    self.warn(format!("unknown op {op} at {}", i - 1));
                    break;
                }
            }
            if let Some(t0) = t_op {
                let e = local.entry(op).or_default();
                e.0 += 1;
                e.1 += t0.elapsed().as_nanos() as u64;
            }
        }
        if prof {
            let mut g = PROF.lock().unwrap();
            let m = g.get_or_insert_with(Default::default);
            for (k, (a, b)) in local {
                let e = m.entry(k).or_default();
                e.0 += a;
                e.1 += b;
            }
        }
    }

    pub fn finish(self) -> Vec<String> {
        if let Some(a) = self.ana {
            let mut h = Xxh3::new();
            let mut hi = Xxh3::new();
            h.update(&self.k.to_bits().to_le_bytes());
            for it in &a.items {
                h.update(&it.h.to_le_bytes());
                hi.update(&it.id.to_le_bytes());
            }
            a.hash = h.digest();
            a.ident = hi.digest();
        }
        self.warnings
    }

    fn warn(&mut self, s: String) {
        if self.warnings.len() < 20 && !self.warnings.contains(&s) {
            self.warnings.push(s);
        }
    }

    fn set_ctm(&mut self, m: Matrix) {
        self.st.ctm = m;
        self.st.fp_k = [None; 6];
    }

    // ───────────────────────────── paths (device space) ─────────────────────────────
    fn dev(&self, x: f64, y: f64) -> Point {
        self.st.ctm.map_point((x as f32, y as f32))
    }

    fn hpath(&mut self, v: &[f32]) {
        let mut h = Xxh3::new();
        h.update(&self.path_h.to_le_bytes());
        hash_q(&mut h, v, 1. / 64.);
        self.path_h = h.digest();
        if let Some(&verb) = v.first() {
            // the shape's identity: the verbs only, so a path that moves or grows stays "the same thing"
            let mut hv = Xxh3::new();
            hv.update(&self.verbs_h.to_le_bytes());
            hv.update(&verb.to_bits().to_le_bytes());
            self.verbs_h = hv.digest();
        }
    }

    fn begin_path(&mut self) {
        self.path = PathBuilder::new();
        self.path_h = 0;
        self.verbs_h = 0;
        self.has_sub = false;
    }

    fn move_to(&mut self, p: Point) {
        if !(p.x.is_finite() && p.y.is_finite()) {
            return;
        }
        self.path.move_to(p);
        self.has_sub = true;
        self.cur = p;
        self.start = p;
        self.hpath(&[MOVE_TO as f32, p.x, p.y]);
    }

    fn line_to(&mut self, p: Point) {
        if !(p.x.is_finite() && p.y.is_finite()) {
            return;
        }
        if !self.has_sub {
            return self.move_to(p);
        }
        self.path.line_to(p);
        self.cur = p;
        self.hpath(&[LINE_TO as f32, p.x, p.y]);
    }

    fn ensure(&mut self, p: Point) {
        if !self.has_sub {
            self.move_to(p);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn ellipse(&mut self, x: f64, y: f64, rx: f64, ry: f64, rot: f64, a0: f64, a1: f64, ccw: bool) {
        if ![x, y, rx, ry, rot, a0, a1].iter().all(|v| v.is_finite()) {
            return;
        }
        let tau = std::f64::consts::TAU;
        let sweep = if !ccw && a1 - a0 >= tau {
            tau
        } else if ccw && a0 - a1 >= tau {
            -tau
        } else if !ccw {
            (a1 - a0).rem_euclid(tau)
        } else {
            -((a0 - a1).rem_euclid(tau))
        };
        let pt = |a: f64| -> (f64, f64) {
            let (c, s) = (a.cos(), a.sin());
            let (ex, ey) = (rx * c, ry * s);
            let (cr, sr) = (rot.cos(), rot.sin());
            (x + ex * cr - ey * sr, y + ex * sr + ey * cr)
        };
        let (sx, sy) = pt(a0);
        let sp = self.dev(sx, sy);
        if self.has_sub { self.line_to(sp) } else { self.move_to(sp) }
        if sweep == 0. || rx == 0. || ry == 0. {
            let (ex, ey) = pt(a0 + sweep);
            let ep = self.dev(ex, ey);
            self.line_to(ep);
            return;
        }
        let segs = ((sweep.abs() / (std::f64::consts::FRAC_PI_2)).ceil() as usize).max(1);
        let d = sweep / segs as f64;
        let kappa = 4. / 3. * (d / 4.).tan();
        let (cr, sr) = (rot.cos(), rot.sin());
        let tp = |a: f64| -> (f64, f64) {
            // derivative direction scaled by radii
            let (dx, dy) = (-rx * a.sin(), ry * a.cos());
            (dx * cr - dy * sr, dx * sr + dy * cr)
        };
        let mut a = a0;
        for _ in 0..segs {
            let b = a + d;
            let (p0x, p0y) = pt(a);
            let (p3x, p3y) = pt(b);
            let (t0x, t0y) = tp(a);
            let (t1x, t1y) = tp(b);
            let c1 = self.dev(p0x + kappa * t0x, p0y + kappa * t0y);
            let c2 = self.dev(p3x - kappa * t1x, p3y - kappa * t1y);
            let p3 = self.dev(p3x, p3y);
            self.path.cubic_to(c1, c2, p3);
            self.cur = p3;
            self.hpath(&[ARC as f32, c1.x, c1.y, c2.x, c2.y, p3.x, p3.y]);
            a = b;
        }
    }

    fn arc_to(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, r: f64) {
        if ![x1, y1, x2, y2, r].iter().all(|v| v.is_finite()) {
            return;
        }
        let p1d = self.dev(x1, y1);
        if !self.has_sub {
            self.move_to(p1d);
            return;
        }
        let inv = match self.st.ctm.invert() {
            Some(m) => m,
            None => return,
        };
        let p0 = inv.map_point(self.cur);
        let (x0, y0) = (p0.x as f64, p0.y as f64);
        let (dx0, dy0) = (x0 - x1, y0 - y1);
        let (dx2, dy2) = (x2 - x1, y2 - y1);
        let l0 = (dx0 * dx0 + dy0 * dy0).sqrt();
        let l2 = (dx2 * dx2 + dy2 * dy2).sqrt();
        let cross = dx0 * dy2 - dy0 * dx2;
        if l0 < 1e-9 || l2 < 1e-9 || r == 0. || cross.abs() < 1e-9 {
            self.line_to(p1d);
            return;
        }
        let cos = ((dx0 * dx2 + dy0 * dy2) / (l0 * l2)).clamp(-1., 1.);
        let theta = cos.acos();
        let dist = r / (theta / 2.).tan();
        let (u0x, u0y) = (dx0 / l0, dy0 / l0);
        let (u2x, u2y) = (dx2 / l2, dy2 / l2);
        let (t1x, t1y) = (x1 + u0x * dist, y1 + u0y * dist);
        let (t2x, t2y) = (x1 + u2x * dist, y1 + u2y * dist);
        // centre: from t1 perpendicular towards the inside of the corner
        let (bx, by) = (u0x + u2x, u0y + u2y);
        let bl = (bx * bx + by * by).sqrt();
        let cd = r / (theta / 2.).sin();
        let (cx, cy) = (x1 + bx / bl * cd, y1 + by / bl * cd);
        let a0 = (t1y - cy).atan2(t1x - cx);
        let a1 = (t2y - cy).atan2(t2x - cx);
        let ccw = cross > 0.;
        let t1d = self.dev(t1x, t1y);
        self.line_to(t1d);
        self.ellipse(cx, cy, r, r, 0., a0, a1, ccw);
    }

    fn rect_path(&mut self, x: f64, y: f64, w: f64, h: f64) {
        if ![x, y, w, h].iter().all(|v| v.is_finite()) {
            return;
        }
        let p0 = self.dev(x, y);
        self.move_to(p0);
        let (p1, p2, p3) = (self.dev(x + w, y), self.dev(x + w, y + h), self.dev(x, y + h));
        self.line_to(p1);
        self.line_to(p2);
        self.line_to(p3);
        self.path.close();
        self.hpath(&[CLOSE_PATH as f32]);
        self.move_to(p0);
    }

    fn round_rect(&mut self, x: f64, y: f64, w: f64, h: f64, r: [f64; 8]) {
        if ![x, y, w, h].iter().all(|v| v.is_finite()) || r.iter().any(|v| !v.is_finite() || *v < 0.) {
            return;
        }
        // negative sizes flip the rectangle (and swap the corners) like the browser
        let (mut x, mut y, mut w, mut h) = (x, y, w, h);
        let mut rr = [(r[0], r[1]), (r[2], r[3]), (r[4], r[5]), (r[6], r[7])]; // tl tr br bl
        if w < 0. {
            x += w;
            w = -w;
            rr = [rr[1], rr[0], rr[3], rr[2]];
        }
        if h < 0. {
            y += h;
            h = -h;
            rr = [rr[3], rr[2], rr[1], rr[0]];
        }
        let scale = [
            w / (rr[0].0 + rr[1].0).max(1e-9),
            h / (rr[1].1 + rr[2].1).max(1e-9),
            w / (rr[2].0 + rr[3].0).max(1e-9),
            h / (rr[3].1 + rr[0].1).max(1e-9),
        ]
        .iter()
        .fold(1f64, |a, &b| a.min(b));
        for c in rr.iter_mut() {
            c.0 *= scale;
            c.1 *= scale;
        }
        let k = 0.5522847498;
        let [tl, tr, br, bl] = rr;
        let p = |s: &Self, px: f64, py: f64| s.dev(px, py);
        let m0 = p(self, x + tl.0, y);
        self.move_to(m0);
        let l1 = p(self, x + w - tr.0, y);
        self.line_to(l1);
        self.corner(x + w - tr.0, y, x + w, y + tr.1, tr, k, 0);
        let l2 = p(self, x + w, y + h - br.1);
        self.line_to(l2);
        self.corner(x + w, y + h - br.1, x + w - br.0, y + h, br, k, 1);
        let l3 = p(self, x + bl.0, y + h);
        self.line_to(l3);
        self.corner(x + bl.0, y + h, x, y + h - bl.1, bl, k, 2);
        let l4 = p(self, x, y + tl.1);
        self.line_to(l4);
        self.corner(x, y + tl.1, x + tl.0, y, tl, k, 3);
        self.path.close();
        self.hpath(&[CLOSE_PATH as f32]);
        let s0 = self.dev(x, y);
        self.move_to(s0);
    }

    /// Quarter ellipse from (ax, ay) to (bx, by) for corner `which` (0 tr, 1 br, 2 bl, 3 tl).
    #[allow(clippy::too_many_arguments)]
    fn corner(&mut self, ax: f64, ay: f64, bx: f64, by: f64, r: (f64, f64), k: f64, which: u8) {
        if r.0 <= 0. || r.1 <= 0. {
            let p = self.dev(bx, by);
            self.line_to(p);
            return;
        }
        let (c1, c2) = match which {
            0 => ((ax + r.0 * k, ay), (bx, by - r.1 * k)),
            1 => ((ax, ay + r.1 * k), (bx + r.0 * k, by)),
            2 => ((ax - r.0 * k, ay), (bx, by + r.1 * k)),
            _ => ((ax, ay - r.1 * k), (bx - r.0 * k, by)),
        };
        let (c1, c2, e) = (self.dev(c1.0, c1.1), self.dev(c2.0, c2.1), self.dev(bx, by));
        self.path.cubic_to(c1, c2, e);
        self.cur = e;
        self.hpath(&[BEZIER_TO as f32, c1.x, c1.y, c2.x, c2.y, e.x, e.y]);
    }

    fn path_snapshot(&self, evenodd: bool) -> Path {
        let p = self.path.snapshot();
        p.with_fill_type(if evenodd { PathFillType::EvenOdd } else { PathFillType::Winding })
    }

    // ───────────────────────────── state fingerprint (Analyze) ─────────────────────────────
    /// The state that can change what this kind of draw looks like — and nothing else, so a colour set for a later
    /// text does not make a full-frame image look "changed" (which would make motion blur repaint the whole frame).
    fn fp(&mut self, kind: u32) -> u64 {
        let slot = match kind {
            DRAW_IMAGE => 0,
            FILL => 1,
            STROKE => 2,
            FILL_TEXT => 3,
            STROKE_TEXT => 4,
            _ => 5,
        };
        if let Some(f) = self.st.fp_k[slot] {
            return f;
        }
        let s = &self.st;
        let mut h = Xxh3::new();
        hash_mat(&mut h, &s.ctm);
        h.update(&s.clip_h.to_le_bytes());
        h.update(&s.layer_h.to_le_bytes());
        if slot != 5 {
            // everything but clearRect: opacity, blend, shadow, filter
            hash_q(&mut h, &[s.alpha], 1. / 1024.);
            hash_q(&mut h, &[s.layer_out], 1. / 64.);
            h.update(&[s.comp]);
            let shadow = s.sh_col.a > 0. && (s.sh_blur > 0. || s.sh_x != 0. || s.sh_y != 0.);
            if shadow {
                hash_q(&mut h, &[s.sh_blur, s.sh_x, s.sh_y], 1. / 64.);
                hash_q(&mut h, &[s.sh_col.r, s.sh_col.g, s.sh_col.b, s.sh_col.a], 1. / 1024.);
            }
            h.update(&s.filter.as_ref().map(|f| f.hash).unwrap_or(0).to_le_bytes());
        }
        match slot {
            0 => h.update(&[s.smooth as u8, s.smooth_q]),
            1 => hash_style(&mut h, &s.fill),
            2 | 4 => {
                hash_style(&mut h, &s.stroke);
                hash_q(&mut h, &[s.lw, s.miter, s.dash_off], 1. / 256.);
                hash_q(&mut h, &s.dash, 1. / 256.);
                h.update(&[s.cap, s.join]);
            }
            _ => {}
        }
        if slot == 3 || slot == 4 {
            if slot == 3 {
                hash_style(&mut h, &s.fill);
            }
            h.update(&s.font_h.to_le_bytes());
            hash_q(&mut h, &[s.ls, s.ws], 1. / 256.);
            h.update(&[s.align, s.baseline]);
        }
        let f = h.digest();
        self.st.fp_k[slot] = Some(f);
        f
    }

    fn shadow_on(&self) -> bool {
        self.st.sh_col.a > 0. && (self.st.sh_blur > 0. || self.st.sh_x != 0. || self.st.sh_y != 0.)
    }

    /// Records one draw for Analyze: its hash and device box (with shadow / filter spread, clipped).
    fn record(&mut self, kind: u32, ident: u64, extra: &[f32], extra_h: u64, dev: Rect) {
        let core = self.core.take();
        // (a draw at under 15% opacity — a fading flash, a ghost — is not part of the element's box)
        if let Some(c) = self.ana.as_deref_mut().and_then(|a| a.capture.as_mut()).filter(|_| self.st.alpha >= 0.15) {
            let mut core = core.unwrap_or(dev);
            if let Some(&ei) = c.open.last() {
                if core.intersect(self.st.clip) {
                    let e = &mut c.elems[ei];
                    e.b.join(core);
                    e.alpha = e.alpha.max(self.st.alpha);
                }
            }
        }
        let fp = self.fp(kind);
        let shadow = self.shadow_on();
        let k = self.k;
        let s = &self.st;
        let mut b = dev;
        if shadow {
            let mut sb = dev.with_offset((s.sh_x * k, s.sh_y * k));
            sb.outset((s.sh_blur * k * 1.5, s.sh_blur * k * 1.5));
            b.join(sb);
        }
        let spread = s.filter.as_ref().map(|f| f.outset).unwrap_or(0.) + s.layer_out;
        if spread > 0. {
            b.outset((spread * k, spread * k));
        }
        let clipped = if b.intersect(s.clip) { b } else { Rect::new_empty() };
        let mut h = Xxh3::new();
        h.update(&kind.to_le_bytes());
        h.update(&fp.to_le_bytes());
        h.update(&extra_h.to_le_bytes());
        hash_q(&mut h, extra, 1. / 1024.);
        let ih = h.digest();
        let mut hi = Xxh3::new();
        hi.update(&kind.to_le_bytes());
        hi.update(&ident.to_le_bytes());
        let id = hi.digest();
        if let Some(a) = self.ana.as_deref_mut() {
            a.items.push(Item { id, h: ih, b: clipped, kind, fp, ext: extra_h });
        }
    }

    // ───────────────────────────── paints ─────────────────────────────
    fn style_paint(&self, style: &Style, local: Option<&Matrix>) -> Option<Paint> {
        let mut p = Paint::default();
        p.set_anti_alias(true);
        p.set_blend_mode(blend(self.st.comp));
        match style {
            Style::Color(c) => {
                let mut c = *c;
                c.a *= self.st.alpha;
                p.set_color4f(c, None);
            }
            Style::Grad(g) => {
                p.set_shader(grad_shader(g, local)?);
                p.set_alpha_f(self.st.alpha);
            }
            Style::Pat(pt) => {
                let img = self.assets.picture(pt.id, pt.frame).ok()?;
                let (tx, ty) = match pt.rep {
                    1 => (TileMode::Repeat, TileMode::Decal),
                    2 => (TileMode::Decal, TileMode::Repeat),
                    3 => (TileMode::Decal, TileMode::Decal),
                    _ => (TileMode::Repeat, TileMode::Repeat),
                };
                let lm = match local {
                    Some(l) => Matrix::concat(l, &pt.m),
                    None => pt.m,
                };
                p.set_shader(img.to_shader((tx, ty), self.sampling(1.), Some(&lm))?);
                p.set_alpha_f(self.st.alpha);
            }
        }
        Some(p)
    }

    fn shadow_paint(&self) -> Paint {
        let mut p = Paint::default();
        p.set_anti_alias(true);
        p.set_blend_mode(blend(self.st.comp));
        let mut c = self.st.sh_col;
        c.a *= self.st.alpha;
        p.set_color4f(c, None);
        let sigma = self.st.sh_blur * self.k / 2.;
        if sigma > 0.01 {
            p.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, sigma, false));
        }
        p
    }

    fn sampling(&self, scale: f32) -> SamplingOptions {
        if !self.st.smooth {
            return SamplingOptions::new(FilterMode::Nearest, MipmapMode::None);
        }
        if self.st.smooth_q >= 2 && scale > 0.5 {
            return SamplingOptions::from(CubicResampler::mitchell());
        }
        if scale < 0.5 || (self.st.smooth_q >= 1 && scale < 0.75) {
            SamplingOptions::new(FilterMode::Linear, MipmapMode::Linear)
        } else {
            SamplingOptions::new(FilterMode::Linear, MipmapMode::None)
        }
    }

    /// Runs `f` inside a layer carrying the CSS filter (device space), or directly when there is none.
    fn with_filter(&mut self, bounds: Rect, f: impl FnOnce(&mut Self)) {
        let filter = self.st.filter.clone();
        match (filter, self.canvas) {
            (Some(flt), Some(c)) => {
                let Some(imf) = build_filter(&flt, self.k) else { return f(self) };
                let mut lp = Paint::default();
                lp.set_image_filter(imf);
                c.save();
                c.reset_matrix();
                let mut b = bounds;
                b.outset((flt.outset * self.k + 2., flt.outset * self.k + 2.));
                let rec = SaveLayerRec::default().bounds(&b).paint(&lp);
                c.save_layer(&rec);
                f(self);
                c.restore();
                c.restore();
            }
            _ => f(self),
        }
    }

    // ───────────────────────────── draws ─────────────────────────────
    fn fill_dev(&mut self, p: &Path, rect_args: Option<&[f32]>) {
        if p.is_empty() {
            return;
        }
        let b = *p.bounds();
        if self.ana.is_some() {
            let extra: Vec<f32> = rect_args.map(|r| r.to_vec()).unwrap_or_default();
            let h = if rect_args.is_some() { 0 } else { self.path_h ^ (p.fill_type() as u64) };
            let mut hb = Xxh3::new();
            hash_q(&mut hb, &[b.left, b.top, b.right, b.bottom], 1. / 64.);
            let ident = if rect_args.is_some() { 1 } else { self.verbs_h };
            self.record(FILL, ident, &extra, h ^ hb.digest(), b);
            return;
        }
        let Some(c) = self.canvas else { return };
        let ctm = self.st.ctm;
        let fill = self.st.fill.clone();
        let Some(paint) = self.style_paint(&fill, Some(&ctm)) else { return };
        let shadow = self.shadow_on();
        self.with_filter(b, |s| {
            if shadow {
                let sp = s.shadow_paint();
                c.set_matrix(&M44::from(Matrix::translate((s.st.sh_x * s.k, s.st.sh_y * s.k))));
                c.draw_path(p, &sp);
            }
            c.reset_matrix();
            c.draw_path(p, &paint);
        });
    }

    fn stroke_paint(&self, local: Option<&Matrix>) -> Option<Paint> {
        let stroke = self.st.stroke.clone();
        let mut p = self.style_paint(&stroke, local)?;
        self.apply_stroke(&mut p);
        Some(p)
    }

    fn apply_stroke(&self, p: &mut Paint) {
        p.set_style(PaintStyle::Stroke);
        p.set_stroke_width(self.st.lw);
        p.set_stroke_cap(match self.st.cap {
            1 => PaintCap::Round,
            2 => PaintCap::Square,
            _ => PaintCap::Butt,
        });
        p.set_stroke_join(match self.st.join {
            1 => PaintJoin::Round,
            2 => PaintJoin::Bevel,
            _ => PaintJoin::Miter,
        });
        p.set_stroke_miter(self.st.miter);
        if !self.st.dash.is_empty() && self.st.dash.iter().any(|v| *v > 0.) {
            if let Some(pe) = PathEffect::dash(&self.st.dash, self.st.dash_off) {
                p.set_path_effect(pe);
            }
        }
    }

    fn stroke_dev(&mut self, p: &Path) {
        if p.is_empty() {
            return;
        }
        let ctm = self.st.ctm;
        let Some(inv) = ctm.invert() else { return };
        let mut b = *p.bounds();
        let grow = self.st.lw * mean_scale(&ctm) * if self.st.join == 0 { self.st.miter.max(1.) * 0.5 } else { 0.5 } + 1.;
        b.outset((grow, grow));
        if self.ana.is_some() {
            if self.ana.as_ref().map(|a| a.capture.is_some()).unwrap_or(false) {
                let half = self.st.lw * mean_scale(&ctm) * 0.5;
                self.core = Some(p.bounds().with_outset((half, half)));
            }
            let h = self.path_h;
            let mut hb = Xxh3::new();
            hash_q(&mut hb, &[b.left, b.top, b.right, b.bottom], 1. / 64.);
            let ident = self.verbs_h;
            self.record(STROKE, ident, &[], h ^ hb.digest(), b);
            return;
        }
        let Some(c) = self.canvas else { return };
        let user = p.make_transform(&inv);
        let Some(paint) = self.stroke_paint(None) else { return };
        let shadow = self.shadow_on();
        self.with_filter(b, |s| {
            if shadow {
                let mut sp = s.shadow_paint();
                s.apply_stroke(&mut sp);
                let m = Matrix::concat(&Matrix::translate((s.st.sh_x * s.k, s.st.sh_y * s.k)), &ctm);
                c.set_matrix(&M44::from(m));
                c.draw_path(&user, &sp);
            }
            c.set_matrix(&M44::from(ctm));
            c.draw_path(&user, &paint);
        });
    }

    fn clear_dev(&mut self, p: &Path) {
        let b = *p.bounds();
        if self.ana.is_some() {
            let mut hb = Xxh3::new();
            hash_q(&mut hb, &[b.left, b.top, b.right, b.bottom], 1. / 64.);
            let d = hb.digest();
            self.record(CLEAR_RECT, 2, &[], d, b);
            return;
        }
        let Some(c) = self.canvas else { return };
        let mut paint = Paint::default();
        paint.set_blend_mode(BlendMode::Clear);
        paint.set_anti_alias(true);
        c.reset_matrix();
        c.draw_path(p, &paint);
    }

    fn clip_dev(&mut self, p: &Path) {
        let b = *p.bounds();
        let mut nc = self.st.clip;
        if !nc.intersect(b) {
            nc = Rect::new_empty();
        }
        self.st.clip = nc;
        let mut h = Xxh3::new();
        h.update(&self.st.clip_h.to_le_bytes());
        h.update(&self.path_h.to_le_bytes());
        hash_q(&mut h, &[b.left, b.top, b.right, b.bottom], 1. / 64.);
        self.st.clip_h = h.digest();
        self.st.fp_k = [None; 6];
        if let Some(c) = self.canvas {
            c.reset_matrix();
            c.clip_path(p, ClipOp::Intersect, true);
        }
    }

    fn text(&mut self, sid: f64, x: f32, y: f32, max_w: f32, stroke: bool, layout: Option<(f32, u8, f32)>) {
        if !(x.is_finite() && y.is_finite()) {
            return;
        }
        let (text, th) = self.strs.get(sid);
        let text = text.to_string();
        let font = self.st.font.clone();
        let shaped = self.shaper.shape(&font, self.st.ls, self.st.ws, &text, layout);
        let Some(blob) = shaped.blob.clone() else { return };
        let w = shaped.width;
        // anchor
        let (mut dx, mut dy) = (0f32, 0f32);
        if layout.is_none() {
            dx = match self.st.align {
                1 => -w / 2.,
                2 => -w,
                _ => 0.,
            };
            dy = match self.st.baseline {
                1 => shaped.ascent,
                2 => shaped.ascent * 0.8,
                3 => (shaped.ascent - shaped.descent) / 2.,
                4 | 5 => -shaped.descent,
                _ => 0.,
            };
        }
        let sx = if max_w >= 0. && w > max_w && w > 0. { max_w / w } else { 1. };
        let mut m = Matrix::concat(&self.st.ctm, &Matrix::translate((x, y)));
        if sx != 1. {
            m = Matrix::concat(&m, &Matrix::scale((sx, 1.)));
        }
        m = Matrix::concat(&m, &Matrix::translate((dx, dy)));
        let ink = if shaped.ink.is_empty() { *blob.bounds() } else { shaped.ink };
        let (mut dev, _) = m.map_rect(ink);
        if stroke {
            let g = self.st.lw * mean_scale(&m);
            dev.outset((g, g));
        }
        if self.ana.is_some() {
            let mut hh = Xxh3::new();
            hh.update(&th.to_le_bytes());
            hash_mat(&mut hh, &m);
            hash_q(&mut hh, &[max_w], 1. / 64.);
            if let Some((a, b2, c2)) = layout {
                hash_f32s(&mut hh, &[a, b2 as f32, c2]);
            }
            let d = hh.digest();
            let mut hid = Xxh3::new();
            hid.update(&th.to_le_bytes());
            hid.update(&self.st.font_h.to_le_bytes());
            hash_f32s(&mut hid, &[self.st.ls, self.st.ws]);
            if let Some((a, b2, c2)) = layout {
                hash_f32s(&mut hid, &[a, b2 as f32, c2]);
            }
            let ident = hid.digest();
            if let Some(c) = self.ana.as_deref_mut().and_then(|a| a.capture.as_mut()).filter(|_| self.st.alpha >= 0.15) {
                let mut core = dev;
                if core.intersect(self.st.clip) {
                    let color = match &self.st.fill { Style::Color(c) if !stroke => Some([c.r, c.g, c.b, c.a]), _ => None };
                    let td = TextDraw { text: text.clone(), px: font_px(&font) * mean_scale(&m) / self.k, color, b: core, alpha: self.st.alpha };
                    match c.open.last() {
                        Some(&ei) => c.elems[ei].texts.push(td),
                        None => c.elems.push(Elem { label: String::new(), b: core, alpha: self.st.alpha, texts: vec![td] }),
                    }
                }
            }
            self.record(if stroke { STROKE_TEXT } else { FILL_TEXT }, ident, &[], d, dev);
            return;
        }
        let Some(c) = self.canvas else { return };
        let paint = if stroke {
            self.stroke_paint(None)
        } else {
            let fill = self.st.fill.clone();
            self.style_paint(&fill, None)
        };
        let Some(paint) = paint else { return };
        let shadow = self.shadow_on();
        self.with_filter(dev, |s| {
            if shadow {
                let mut sp = s.shadow_paint();
                if stroke {
                    s.apply_stroke(&mut sp);
                }
                let sm = Matrix::concat(&Matrix::translate((s.st.sh_x * s.k, s.st.sh_y * s.k)), &m);
                c.set_matrix(&M44::from(sm));
                c.draw_text_blob(&blob, (0, 0), &sp);
            }
            c.set_matrix(&M44::from(m));
            c.draw_text_blob(&blob, (0, 0), &paint);
        });
    }

    fn image(&mut self, id: usize, frame: usize, v: [f32; 8]) {
        if v.iter().any(|x| !x.is_finite()) {
            return;
        }
        let [mut sx, mut sy, mut sw, mut sh, mut dx, mut dy, mut dw, mut dh] = v;
        if sw < 0. { sx += sw; sw = -sw; }
        if sh < 0. { sy += sh; sh = -sh; }
        if dw < 0. { dx += dw; dw = -dw; }
        if dh < 0. { dy += dh; dh = -dh; }
        if sw == 0. || sh == 0. || dw == 0. || dh == 0. {
            return;
        }
        let dst = Rect::from_xywh(dx, dy, dw, dh);
        let ctm = self.st.ctm;
        let (dev, _) = ctm.map_rect(dst);
        if self.ana.is_some() {
            let ch = self.assets.content_hash(id, frame);
            self.record(DRAW_IMAGE, ch, &[sx, sy, sw, sh, dx, dy, dw, dh], ch, dev);
            return;
        }
        let Some(c) = self.canvas else { return };
        // nothing on screen: skip the decode wait too
        let mut vis = dev;
        if !vis.intersect(self.st.clip) {
            return;
        }
        let t_pic = std::time::Instant::now();
        let img = match self.assets.picture(id, frame) {
            Ok(i) => i,
            Err(e) => {
                self.warn(format!("image {id}: {e}"));
                return;
            }
        };
        if prof_on() {
            let kind = self.assets.kind_code(id);
            let mut g = PROF.lock().unwrap();
            let e = g.get_or_insert_with(Default::default).entry(1000 + kind).or_default();
            e.0 += 1;
            e.1 += t_pic.elapsed().as_nanos() as u64;
        }
        // a picture decoded smaller than its native size (draft video): source rects are in native px
        let (nw, nh) = self.assets.native_size(id);
        let (kx, ky) = if nw > 0 && nh > 0 { (img.width() as f32 / nw as f32, img.height() as f32 / nh as f32) } else { (1., 1.) };
        let src = Rect::from_xywh(sx * kx, sy * ky, sw * kx, sh * ky);
        let scale = (dev.width() / src.width()).min(dev.height() / src.height());
        let sampling = self.sampling(scale);
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_alpha_f(self.st.alpha);
        paint.set_blend_mode(blend(self.st.comp));
        let shadow = self.shadow_on();
        let (k, shx, shy, shb, shc, alpha) = (self.k, self.st.sh_x, self.st.sh_y, self.st.sh_blur, self.st.sh_col, self.st.alpha);
        self.with_filter(dev, |_s| {
            if shadow {
                // the image's alpha, blurred and tinted, in device space
                let mut lp = Paint::default();
                let sigma = shb * k / 2.;
                if sigma > 0.01 {
                    if let Some(bl) = image_filters::blur((sigma, sigma), None, None, None) {
                        lp.set_image_filter(bl);
                    }
                }
                let mut col = shc;
                col.a *= alpha;
                if let Some(cf) = color_filters::blend(col.to_color(), BlendMode::SrcIn) { lp.set_color_filter(cf); }
                c.save();
                c.reset_matrix();
                let mut lb = dev.with_offset((shx * k, shy * k));
                lb.outset((sigma * 3. + 2., sigma * 3. + 2.));
                c.save_layer(&SaveLayerRec::default().bounds(&lb).paint(&lp));
                c.set_matrix(&M44::from(Matrix::concat(&Matrix::translate((shx * k, shy * k)), &ctm)));
                c.draw_image_rect_with_sampling_options(&img, Some((&src, SrcRectConstraint::Fast)), dst, sampling, &Paint::default());
                c.restore();
                c.restore();
            }
            c.set_matrix(&M44::from(ctm));
            c.draw_image_rect_with_sampling_options(&img, Some((&src, SrcRectConstraint::Fast)), dst, sampling, &paint);
        });
    }

    fn begin_layer(&mut self, alpha: f32, comp: u8, fs: &str, fh: u64) {
        let flt = parse_filter(fs, fh);
        let mut h = Xxh3::new();
        h.update(&self.st.layer_h.to_le_bytes());
        hash_f32s(&mut h, &[alpha]);
        h.update(&[comp]);
        h.update(&fh.to_le_bytes());
        self.st.layer_h = h.digest();
        if let Some(f) = &flt {
            self.st.layer_out += f.outset;
        }
        self.st.fp_k = [None; 6];
        self.layers.push(self.stack.len());
        if let Some(c) = self.canvas {
            let mut lp = Paint::default();
            lp.set_alpha_f(alpha.clamp(0., 1.));
            lp.set_blend_mode(blend(comp));
            if let Some(f) = &flt {
                if let Some(imf) = build_filter(f, self.k) {
                    lp.set_image_filter(imf);
                }
            }
            // device-space layer (filters measure in device px); END_LAYER's restore closes it
            c.reset_matrix();
            c.save_layer(&SaveLayerRec::default().paint(&lp));
        }
    }
}

fn hash_style(h: &mut Xxh3, s: &Style) {
    match s {
        Style::Color(c) => hash_f32s(h, &[c.r, c.g, c.b, c.a]),
        Style::Grad(g) => h.update(&g.hash.to_le_bytes()),
        Style::Pat(p) => h.update(&p.hash.to_le_bytes()),
    }
}

#[allow(deprecated)]
fn grad_shader(g: &Grad, local: Option<&Matrix>) -> Option<Shader> {
    if g.stops.is_empty() {
        return None;
    }
    let mut stops = g.stops.clone();
    if stops.len() == 1 {
        stops.push(stops[0]);
    }
    let colors: Vec<skia_safe::Color> = stops.iter().map(|s| s.1.to_color()).collect();
    let pos: Vec<f32> = stops.iter().map(|s| s.0).collect();
    let c = g.c;
    match g.kind {
        0 => Shader::linear_gradient(((c[0], c[1]), (c[2], c[3])), colors.as_slice(), pos.as_slice(), TileMode::Clamp, None, local),
        1 => Shader::two_point_conical_gradient((c[0], c[1]), c[2], (c[3], c[4]), c[5], colors.as_slice(), pos.as_slice(), TileMode::Clamp, None, local),
        _ => {
            // conic: angle from +x, clockwise; Skia's sweep starts at +x too
            let deg = c[0].to_degrees();
            let m = Matrix::rotate_deg_pivot(deg, (c[1], c[2]));
            let lm = match local {
                Some(l) => Matrix::concat(l, &m),
                None => m,
            };
            Shader::sweep_gradient((c[1], c[2]), colors.as_slice(), pos.as_slice(), TileMode::Clamp, None, None, Some(&lm))
        }
    }
}

/// CSS filter functions: blur(px) brightness() contrast() saturate() grayscale() sepia() invert() opacity()
/// hue-rotate(deg) drop-shadow(x y blur color). `none` clears.
fn parse_filter(s: &str, h: u64) -> Option<Rc<Filter>> {
    let s = s.trim();
    if s.is_empty() || s == "none" {
        return None;
    }
    let mut ops = Vec::new();
    let mut outset = 0f32;
    let mut rest = s;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().to_ascii_lowercase();
        let Some(close) = rest[open..].find(')') else { break };
        // drop-shadow can contain rgba(...): take to the matching parenthesis
        let mut depth = 0;
        let mut end = open;
        for (j, ch) in rest[open..].char_indices() {
            if ch == '(' { depth += 1; }
            if ch == ')' { depth -= 1; if depth == 0 { end = open + j; break; } }
        }
        let _ = close;
        let arg = rest[open + 1..end].trim().to_string();
        rest = &rest[end + 1..];
        let amount = |a: &str, def: f32| -> f32 {
            let a = a.trim();
            if a.is_empty() { return def; }
            if let Some(p) = a.strip_suffix('%') { p.parse::<f32>().map(|v| v / 100.).unwrap_or(def) } else { a.parse::<f32>().unwrap_or(def) }
        };
        let px = |a: &str| -> f32 { a.trim().trim_end_matches("px").parse::<f32>().unwrap_or(0.) };
        match name.as_str() {
            "blur" => {
                let r = px(&arg);
                if r > 0. {
                    ops.push(FOp::Blur(r));
                    outset += r * 3.;
                }
            }
            "brightness" => { let a = amount(&arg, 1.); ops.push(FOp::Matrix(cm_scale(a))); }
            "contrast" => { let a = amount(&arg, 1.); ops.push(FOp::Matrix(cm_contrast(a))); }
            "saturate" => { let a = amount(&arg, 1.); ops.push(FOp::Matrix(cm_saturate(a))); }
            "grayscale" => { let a = amount(&arg, 1.).clamp(0., 1.); ops.push(FOp::Matrix(cm_saturate(1. - a))); }
            "sepia" => { let a = amount(&arg, 1.).clamp(0., 1.); ops.push(FOp::Matrix(cm_sepia(a))); }
            "invert" => { let a = amount(&arg, 1.).clamp(0., 1.); ops.push(FOp::Matrix(cm_invert(a))); }
            "opacity" => { let a = amount(&arg, 1.).clamp(0., 1.); ops.push(FOp::Opacity(a)); }
            "hue-rotate" => {
                let d = arg.trim().trim_end_matches("deg").parse::<f32>().unwrap_or(0.);
                ops.push(FOp::Matrix(cm_hue(d)));
            }
            "drop-shadow" => {
                // x y [blur] [color] — the color may come first or last
                let mut nums = Vec::new();
                let mut col = Color4f::new(0., 0., 0., 1.);
                let mut parts = arg.as_str();
                if let Some(ci) = parts.find(|c: char| c == '#' || c.is_ascii_alphabetic()) {
                    let (a2, b2) = parts.split_at(ci);
                    if let Some(c2) = color::parse(b2.trim()) {
                        col = c2;
                        parts = a2;
                    } else if let Some(c2) = color::parse(a2.trim()) {
                        col = c2;
                        parts = b2;
                    }
                }
                for t in parts.split_whitespace() {
                    if let Ok(v) = t.trim_end_matches("px").parse::<f32>() {
                        nums.push(v);
                    }
                }
                let (x, y, b) = (nums.first().copied().unwrap_or(0.), nums.get(1).copied().unwrap_or(0.), nums.get(2).copied().unwrap_or(0.));
                ops.push(FOp::Shadow(x, y, b, col));
                outset += x.abs().max(y.abs()) + b * 3.;
            }
            _ => {}
        }
    }
    if ops.is_empty() {
        return None;
    }
    Some(Rc::new(Filter { ops, hash: h, outset }))
}

fn build_filter(f: &Filter, k: f32) -> Option<ImageFilter> {
    let mut cur: Option<ImageFilter> = None;
    for op in &f.ops {
        cur = match op {
            FOp::Blur(r) => image_filters::blur((r * k, r * k), TileMode::Decal, cur, None),
            FOp::Matrix(m) => image_filters::color_filter(color_filters::matrix(m, None), cur, None),
            FOp::Opacity(a) => {
                let mut m = ColorMatrix::default();
                m.set_scale(1., 1., 1., Some(*a));
                image_filters::color_filter(color_filters::matrix(&m, None), cur, None)
            }
            FOp::Shadow(x, y, b, c) => image_filters::drop_shadow((x * k, y * k), (b * k / 2., b * k / 2.), *c, None, cur, None),
        };
    }
    cur
}

fn cm_scale(a: f32) -> ColorMatrix {
    let mut m = ColorMatrix::default();
    m.set_scale(a, a, a, None);
    m
}

fn cm_contrast(a: f32) -> ColorMatrix {
    let t = 0.5 * (1. - a);
    ColorMatrix::new(a, 0., 0., 0., t, 0., a, 0., 0., t, 0., 0., a, 0., t, 0., 0., 0., 1., 0.)
}

fn cm_saturate(s: f32) -> ColorMatrix {
    let (r, g, b) = (0.2126, 0.7152, 0.0722);
    ColorMatrix::new(
        r + (1. - r) * s, g - g * s, b - b * s, 0., 0.,
        r - r * s, g + (1. - g) * s, b - b * s, 0., 0.,
        r - r * s, g - g * s, b + (1. - b) * s, 0., 0.,
        0., 0., 0., 1., 0.,
    )
}

fn cm_sepia(a: f32) -> ColorMatrix {
    let i = 1. - a;
    ColorMatrix::new(
        0.393 + 0.607 * i, 0.769 - 0.769 * i, 0.189 - 0.189 * i, 0., 0.,
        0.349 - 0.349 * i, 0.686 + 0.314 * i, 0.168 - 0.168 * i, 0., 0.,
        0.272 - 0.272 * i, 0.534 - 0.534 * i, 0.131 + 0.869 * i, 0., 0.,
        0., 0., 0., 1., 0.,
    )
}

fn cm_invert(a: f32) -> ColorMatrix {
    let s = 1. - 2. * a;
    ColorMatrix::new(s, 0., 0., 0., a, 0., s, 0., 0., a, 0., 0., s, 0., a, 0., 0., 0., 1., 0.)
}

fn cm_hue(deg: f32) -> ColorMatrix {
    let (c, s) = (deg.to_radians().cos(), deg.to_radians().sin());
    ColorMatrix::new(
        0.213 + c * 0.787 - s * 0.213, 0.715 - c * 0.715 - s * 0.715, 0.072 - c * 0.072 + s * 0.928, 0., 0.,
        0.213 - c * 0.213 + s * 0.143, 0.715 + c * 0.285 + s * 0.140, 0.072 - c * 0.072 - s * 0.283, 0., 0.,
        0.213 - c * 0.213 - s * 0.787, 0.715 - c * 0.715 + s * 0.715, 0.072 + c * 0.928 + s * 0.072, 0., 0.,
        0., 0., 0., 1., 0.,
    )
}
