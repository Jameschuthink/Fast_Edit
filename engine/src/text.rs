//! Text: CSS font strings, the font registry (files → typefaces, variable fonts pinned per weight), and a per-worker
//! shaper that turns (font, letter spacing, text) into a positioned TextBlob plus the metrics Canvas2D needs.
//!
//! Shaping goes through SkParagraph (HarfBuzz, kerning, ligatures, fallback fonts for missing glyphs); drawing uses
//! the glyph runs it produced as a TextBlob, so a text draw takes any paint (alpha, gradient, stroke, shadow) and
//! the expensive part (shaping) is cached per string.

use skia_safe::{
    font_arguments::{variation_position::Coordinate, VariationPosition},
    font_style::{Slant, Weight, Width},
    textlayout::{
        FontCollection, ParagraphBuilder, ParagraphStyle, TextAlign, TextStyle, TypefaceFontProvider,
    },
    Data, Font, FontArguments, FontMgr, FontStyle, FourByteTag, Rect, TextBlob, TextBlobBuilder, Typeface,
};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};

// ───────────────────────────── CSS font strings ─────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct FontSpec {
    pub families: Vec<String>,
    pub size: f32,
    pub weight: i32,
    pub italic: bool,
}

impl Default for FontSpec {
    fn default() -> Self {
        FontSpec { families: vec!["sans-serif".into()], size: 10., weight: 400, italic: false }
    }
}

/// Parses the CSS `font` shorthand the way Canvas2D does: `[style] [variant] [weight] [stretch] size[/lh] family, …`.
/// Returns None for a string the browser would reject (the caller keeps the previous font).
pub fn parse_font(s: &str) -> Option<FontSpec> {
    let s = s.trim();
    let mut weight = 400;
    let mut italic = false;
    let mut rest = s;
    loop {
        let tok_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let tok = &rest[..tok_end];
        if tok.is_empty() {
            return None;
        }
        let lower = tok.to_ascii_lowercase();
        // the size token: a number with a unit, optionally /line-height
        let size_part = lower.split('/').next().unwrap_or("");
        if let Some(px) = size_px(size_part) {
            let family_str = rest[tok_end..].trim();
            if family_str.is_empty() {
                return None;
            }
            let families = split_families(family_str);
            return Some(FontSpec { families, size: px, weight, italic });
        }
        match lower.as_str() {
            "normal" | "small-caps" | "ultra-condensed" | "extra-condensed" | "condensed" | "semi-condensed"
            | "semi-expanded" | "expanded" | "extra-expanded" | "ultra-expanded" => {}
            "italic" | "oblique" => italic = true,
            "bold" | "bolder" => weight = 700,
            "lighter" => weight = 300,
            _ => {
                if let Ok(w) = lower.parse::<f32>() {
                    if (1.0..=1000.0).contains(&w) {
                        weight = w.round() as i32;
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            }
        }
        rest = rest[tok_end..].trim_start();
    }
}

fn size_px(t: &str) -> Option<f32> {
    let (num, mul) = if let Some(n) = t.strip_suffix("px") {
        (n, 1.0)
    } else if let Some(n) = t.strip_suffix("pt") {
        (n, 4.0 / 3.0)
    } else if let Some(n) = t.strip_suffix("rem") {
        (n, 16.0)
    } else if let Some(n) = t.strip_suffix("em") {
        (n, 10.0)
    } else if let Some(n) = t.strip_suffix('%') {
        (n, 0.1)
    } else {
        return None;
    };
    let v: f32 = num.parse().ok()?;
    if v.is_finite() && v >= 0. { Some(v * mul) } else { None }
}

fn split_families(s: &str) -> Vec<String> {
    s.split(',')
        .map(|f| f.trim().trim_matches(|c| c == '"' || c == '\'').trim().to_string())
        .filter(|f| !f.is_empty())
        .collect()
}

// ───────────────────────────── the font registry (shared by all workers) ─────────────────────────────

struct FontFile {
    path: String,
    typeface: Typeface,
    wght_axis: Option<(f32, f32)>, // (min, max) when the face is variable in weight
    pins: Vec<(FourByteTag, f32)>, // other axes held at a value (e.g. slnt 0, wdth 100)
    weight: i32,                   // static weight (or the default instance's)
    italic: bool,
}

#[derive(Default)]
pub struct Registry {
    families: HashMap<String, Vec<FontFile>>,
    instances: HashMap<(String, i32, bool), (Typeface, String)>,
    pub generation: u64,
}

pub fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Registry::default()))
}

thread_local! {
    static FONT_MGR: FontMgr = FontMgr::new();
}

fn tag(s: &str) -> FourByteTag {
    let b = s.as_bytes();
    let g = |i: usize| *b.get(i).unwrap_or(&b' ');
    FourByteTag::from_chars(g(0) as char, g(1) as char, g(2) as char, g(3) as char)
}

/// Registers a font file under a family name (case-insensitive). `opts` may give {weight, style:'italic', pins:{slnt:0}}.
/// Registering the same file again is a no-op.
pub fn register_font(family: &str, path: &str, weight: Option<i32>, italic: Option<bool>, pins: &[(String, f32)]) -> anyhow::Result<()> {
    let key = family.trim().to_ascii_lowercase();
    {
        let r = registry().lock().unwrap();
        if let Some(v) = r.families.get(&key) {
            if v.iter().any(|f| f.path == path) {
                return Ok(());
            }
        }
    }
    let bytes = std::fs::read(path).map_err(|e| anyhow::anyhow!("font {path}: {e}"))?;
    let data = Data::new_copy(&bytes);
    let tf = FONT_MGR
        .with(|m| m.new_from_data(data.clone(), None))
        .ok_or_else(|| anyhow::anyhow!("font {path}: not a font Skia can read (use .ttf/.otf)"))?;
    let mut wght_axis = None;
    let mut pins_v = Vec::new();
    if let Some(axes) = tf.variation_design_parameters() {
        for a in &axes {
            if a.tag == tag("wght") {
                wght_axis = Some((a.min, a.max));
            }
        }
    }
    for (k, v) in pins {
        pins_v.push((tag(k), *v));
    }
    let st = tf.font_style();
    let ff = FontFile {
        path: path.to_string(),
        weight: weight.unwrap_or(*st.weight()),
        italic: italic.unwrap_or(st.slant() != Slant::Upright),
        typeface: tf,
        wght_axis,
        pins: pins_v,
    };
    let mut r = registry().lock().unwrap();
    let v = r.families.entry(key).or_default();
    if v.iter().any(|f| f.path == path) {
        return Ok(()); // another worker registered it while this one read the file
    }
    v.push(ff);
    r.generation += 1;
    Ok(())
}

/// The typeface for (family, weight, italic) and the unique alias it is registered under in every worker, or None
/// when the family is not registered (then the system font manager resolves it).
fn instance(family: &str, weight: i32, italic: bool) -> Option<(Typeface, String)> {
    let key = family.to_ascii_lowercase();
    let mut r = registry().lock().unwrap();
    if let Some(hit) = r.instances.get(&(key.clone(), weight, italic)) {
        return Some(hit.clone());
    }
    let files = r.families.get(&key)?;
    // best file: italic match first, then a variable face covering the weight, then the nearest static weight
    let score = |f: &FontFile| {
        let it = if f.italic == italic { 0 } else { 10_000 };
        let w = match f.wght_axis {
            Some((lo, hi)) if (weight as f32) >= lo && (weight as f32) <= hi => 0,
            _ => (f.weight - weight).abs(),
        };
        it + w
    };
    let f = files.iter().min_by_key(|f| score(f))?;
    let mut coords: Vec<Coordinate> = f.pins.iter().map(|(t, v)| Coordinate { axis: *t, value: *v }).collect();
    let tf = if let Some((lo, hi)) = f.wght_axis {
        coords.push(Coordinate { axis: tag("wght"), value: (weight as f32).clamp(lo, hi) });
        let args = FontArguments::new().set_variation_design_position(VariationPosition { coordinates: &coords });
        f.typeface.clone_with_arguments(&args).unwrap_or_else(|| f.typeface.clone())
    } else if !coords.is_empty() {
        let args = FontArguments::new().set_variation_design_position(VariationPosition { coordinates: &coords });
        f.typeface.clone_with_arguments(&args).unwrap_or_else(|| f.typeface.clone())
    } else {
        f.typeface.clone()
    };
    let alias = format!("{}@{}{}", key, weight, if italic { "i" } else { "" });
    r.instances.insert((key, weight, italic), (tf.clone(), alias.clone()));
    Some((tf, alias))
}

fn generic(f: &str) -> &str {
    match f.to_ascii_lowercase().as_str() {
        "system-ui" | "-apple-system" | "blinkmacsystemfont" | "ui-sans-serif" | "helvetica" | "arial" => "sans-serif",
        "ui-monospace" | "menlo" | "monaco" | "consolas" | "sfmono-regular" => "monospace",
        "ui-serif" | "georgia" | "times new roman" => "serif",
        _ => f,
    }
}

// ───────────────────────────── per-worker shaper ─────────────────────────────

pub struct Shaped {
    pub blob: Option<TextBlob>,
    pub width: f32,
    /// tight glyph bounds relative to the pen origin (x = start of the text, y = baseline)
    pub ink: Rect,
    /// font ascent / descent (positive, rounded like the browser) of the first face
    pub ascent: f32,
    pub descent: f32,
    pub height: f32,
    pub lines: usize,
}

#[derive(Hash, PartialEq, Eq, Clone)]
struct ShapeKey {
    font: String,
    ls: u32,
    ws: u32,
    text: String,
    box_w: u32,
    align: u8,
    lh: u32,
}

pub struct Shaper {
    provider: TypefaceFontProvider,
    collection: FontCollection,
    registered: HashMap<String, ()>,
    specs: HashMap<String, Option<Rc<FontSpec>>>,
    cache: HashMap<ShapeKey, Rc<Shaped>>,
    generation: u64,
}

impl Shaper {
    pub fn new() -> Self {
        let provider = TypefaceFontProvider::new();
        let mut collection = FontCollection::new();
        collection.set_asset_font_manager(Some(provider.clone().into()));
        FONT_MGR.with(|m| collection.set_default_font_manager(m.clone(), None));
        collection.enable_font_fallback();
        Shaper { provider, collection, registered: HashMap::new(), specs: HashMap::new(), cache: HashMap::new(), generation: 0 }
    }

    pub fn spec(&mut self, font: &str) -> Option<Rc<FontSpec>> {
        if let Some(s) = self.specs.get(font) {
            return s.clone();
        }
        let s = parse_font(font).map(Rc::new);
        self.specs.insert(font.to_string(), s.clone());
        s
    }

    /// The family list for SkParagraph: registered families become their per-weight alias.
    fn families(&mut self, spec: &FontSpec) -> Vec<String> {
        let gn = registry().lock().unwrap().generation;
        if gn != self.generation {
            self.generation = gn;
            self.cache.clear();
        }
        let mut out = Vec::new();
        let mut added = false;
        for f in &spec.families {
            if let Some((tf, alias)) = instance(f, spec.weight, spec.italic) {
                if !self.registered.contains_key(&alias) {
                    self.provider.register_typeface(tf, Some(alias.as_str()));
                    self.registered.insert(alias.clone(), ());
                    added = true;
                }
                out.push(alias);
            } else {
                out.push(generic(f).to_string());
            }
        }
        if added {
            self.collection.clear_caches();
        }
        if out.is_empty() {
            out.push("sans-serif".into());
        }
        out
    }

    /// Shapes one line (box_w = None) or a wrapped box. Canvas2D replaces every whitespace character with a space.
    pub fn shape(&mut self, font: &str, letter_spacing: f32, word_spacing: f32, text: &str, layout: Option<(f32, u8, f32)>) -> Rc<Shaped> {
        let (box_w, align, lh) = layout.unwrap_or((f32::INFINITY, 0, 0.));
        let key = ShapeKey {
            font: font.to_string(),
            ls: letter_spacing.to_bits(),
            ws: word_spacing.to_bits(),
            text: text.to_string(),
            box_w: box_w.to_bits(),
            align,
            lh: lh.to_bits(),
        };
        if let Some(s) = self.cache.get(&key) {
            return s.clone();
        }
        let spec = self.spec(font).unwrap_or_else(|| Rc::new(FontSpec::default()));
        let fams = self.families(&spec);
        let clean: String = if layout.is_none() {
            text.chars().map(|c| if c.is_whitespace() { ' ' } else { c }).collect()
        } else {
            text.to_string()
        };
        let mut ts = TextStyle::new();
        ts.set_font_families(&fams);
        ts.set_font_size(spec.size.max(0.01));
        ts.set_font_style(FontStyle::new(
            Weight::from(spec.weight),
            Width::NORMAL,
            if spec.italic { Slant::Italic } else { Slant::Upright },
        ));
        if letter_spacing != 0. {
            ts.set_letter_spacing(letter_spacing);
        }
        if word_spacing != 0. {
            ts.set_word_spacing(word_spacing);
        }
        if lh > 0. {
            ts.set_height(lh);
            ts.set_height_override(true);
        }
        let mut ps = ParagraphStyle::new();
        ps.set_text_style(&ts);
        ps.set_text_align(match align {
            1 => TextAlign::Center,
            2 => TextAlign::Right,
            _ => TextAlign::Left,
        });
        let mut b = ParagraphBuilder::new(&ps, &self.collection);
        b.push_style(&ts);
        b.add_text(&clean);
        let mut p = b.build();
        p.layout(if box_w.is_finite() { box_w.max(1.) } else { 1e7 });
        let width = if box_w.is_finite() { p.longest_line() } else { p.max_intrinsic_width() };
        let height = p.height();
        let lines = p.line_number();
        let mut builder = TextBlobBuilder::new();
        let mut ink = Rect::new_empty();
        let mut first_font: Option<Font> = None;
        let mut any = false;
        let baseline0 = if box_w.is_finite() { 0. } else { p.alphabetic_baseline() };
        p.visit(|_, info| {
            let Some(info) = info else { return };
            let n = info.count();
            if n == 0 {
                return;
            }
            let font = info.font();
            if first_font.is_none() {
                first_font = Some(font.clone());
            }
            let o = info.origin();
            let (glyphs, pos) = builder.alloc_run_pos(font, n, None);
            glyphs.copy_from_slice(&info.glyphs()[..n]);
            for i in 0..n {
                let pp = info.positions()[i];
                pos[i] = skia_safe::Point::new(o.x + pp.x, o.y + pp.y - baseline0);
            }
            let mut bounds = vec![Rect::default(); n];
            font.get_bounds(&info.glyphs()[..n], &mut bounds, None);
            for i in 0..n {
                let pp = info.positions()[i];
                let r = bounds[i].with_offset((o.x + pp.x, o.y + pp.y - baseline0));
                if !r.is_empty() {
                    ink.join(r);
                }
            }
            any = true;
        });
        let blob = if any { builder.make() } else { None };
        let (ascent, descent) = match &first_font {
            Some(f) => {
                let (_, m) = f.metrics();
                ((-m.ascent).round(), m.descent.round())
            }
            None => {
                // empty text: metrics of the primary face
                let tf = fams.first().and_then(|f| self.provider_typeface(f));
                match tf {
                    Some(tf) => {
                        let f = Font::from_typeface(tf, spec.size);
                        let (_, m) = f.metrics();
                        ((-m.ascent).round(), m.descent.round())
                    }
                    None => ((spec.size * 0.8).round(), (spec.size * 0.2).round()),
                }
            }
        };
        let s = Rc::new(Shaped { blob, width, ink, ascent, descent, height, lines });
        if self.cache.len() > 50_000 {
            self.cache.clear();
        }
        self.cache.insert(key, s.clone());
        s
    }

    fn provider_typeface(&self, alias: &str) -> Option<Typeface> {
        let r = registry().lock().unwrap();
        r.instances.values().find(|(_, a)| a == alias).map(|(t, _)| t.clone())
    }
}

/// Every .ttf/.otf under a folder, registered by its file stem (e.g. fonts/Inter.ttf → "Inter").
pub fn register_dir(dir: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext == "ttf" || ext == "otf" {
            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                let _ = register_font(stem, &p.to_string_lossy(), None, None, &[]);
            }
        }
    }
}

pub type SharedShaped = Arc<Shaped>;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fonts() {
        let f = parse_font("700 72px CapInter").unwrap();
        assert_eq!((f.weight, f.size, f.families[0].as_str()), (700, 72., "CapInter"));
        let f = parse_font("italic 600 48px \"Inter Tight\", sans-serif").unwrap();
        assert!(f.italic && f.families == vec!["Inter Tight".to_string(), "sans-serif".into()]);
        let f = parse_font("600 96px 'LookGeist', system-ui, sans-serif").unwrap();
        assert_eq!(f.families[0], "LookGeist");
        assert!(parse_font("bogus").is_none());
        let f = parse_font("91px CapGaramond").unwrap();
        assert_eq!(f.weight, 400);
    }
}
