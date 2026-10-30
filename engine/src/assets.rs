//! Assets shared by every worker: still images (decoded once), videos (one decoder each), image sequences
//! (decoded on demand into a small shared cache). Ids are stable within one engine process; the content hash of
//! each asset (path, size, mtime) is what frame hashes use, so caches stay valid across runs.

use crate::video::{VideoOpts, VideoSource};
use anyhow::{anyhow, Result};
use skia_safe::{images, AlphaType, Color, ColorType, Data, Image, ImageInfo, Size};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

pub enum Kind {
    Still(Image),
    Video(Arc<VideoSource>),
    Seq { pattern: String, n: usize, first: usize },
}

pub struct Asset {
    pub kind: Kind,
    pub width: u32,
    pub height: u32,
    pub hash: u64,
    pub path: String,
}

pub struct Assets {
    pub root: PathBuf,
    pub fps: f64,
    pub hwaccel: bool,
    pub random: bool,
    pub scale: f32,
    keys: Mutex<HashMap<String, usize>>,
    list: RwLock<Vec<Arc<Asset>>>,
    seq_cache: Mutex<(HashMap<(usize, usize), Image>, std::collections::VecDeque<(usize, usize)>)>,
    /// videos open in the previous Assets of this process (`aios serve` after an edit), by key, with the file's
    /// (size, mtime) when opened: taken over as they are, so the first frame after an edit has a warm decoder
    carry: Mutex<HashMap<String, (Arc<Asset>, Stamp)>>,
    /// the (size, mtime) of each video here when it was opened, by key
    opened: Mutex<HashMap<String, Stamp>>,
}

type Stamp = (u64, Option<std::time::SystemTime>);

fn stamp(p: &Path) -> Option<Stamp> {
    std::fs::metadata(p).ok().map(|m| (m.len(), m.modified().ok()))
}

fn file_hash(path: &Path, tag: &str) -> u64 {
    let meta = std::fs::metadata(path).ok();
    let len = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = meta
        .and_then(|m| m.modified().ok())
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    xxhash_rust::xxh3::xxh3_64(format!("{tag}|{}|{len}|{mtime}", path.display()).as_bytes())
}

/// Decodes an image file into a raster image (png, jpg, webp, gif first frame, bmp, ico; svg rendered at 2x).
pub fn decode_file(path: &Path) -> Result<Image> {
    let bytes = std::fs::read(path).map_err(|e| anyhow!("image {}: {e}", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext == "svg" {
        return render_svg(&bytes, path);
    }
    let img = Image::from_encoded(Data::new_copy(&bytes)).ok_or_else(|| anyhow!("image {}: cannot decode", path.display()))?;
    img.make_raster_image(None, skia_safe::image::CachingHint::Allow)
        .ok_or_else(|| anyhow!("image {}: cannot decode", path.display()))
}

fn render_svg(bytes: &[u8], path: &Path) -> Result<Image> {
    let fm = skia_safe::FontMgr::new();
    let mut dom = skia_safe::svg::Dom::from_bytes(bytes, fm).map_err(|e| anyhow!("svg {}: {e:?}", path.display()))?;
    let root = dom.root();
    let mut size = root.intrinsic_size();
    if size.width <= 0. || size.height <= 0. {
        if let Some(vb) = root.view_box() {
            size = Size::new(vb.width(), vb.height());
        } else {
            size = Size::new(512., 512.);
        }
    }
    // rasterise big enough to stay sharp when drawn large: 2x, at least 1024 px on the long side
    let k = (1024. / size.width.max(size.height)).max(2.);
    let (w, h) = ((size.width * k).ceil() as i32, (size.height * k).ceil() as i32);
    dom.set_container_size(size);
    let mut surf = skia_safe::surfaces::raster_n32_premul((w, h)).ok_or_else(|| anyhow!("svg surface"))?;
    let c = surf.canvas();
    c.clear(Color::TRANSPARENT);
    c.scale((k, k));
    dom.render(c);
    Ok(surf.image_snapshot())
}

impl Assets {
    pub fn new(root: PathBuf, fps: f64, hwaccel: bool, random: bool, scale: f32) -> Arc<Assets> {
        Self::renew(None, root, fps, hwaccel, random, scale)
    }

    /// New Assets that take over `prev`'s open videos whose files did not change (same root, fps and scale only).
    pub fn renew(prev: Option<&Assets>, root: PathBuf, fps: f64, hwaccel: bool, random: bool, scale: f32) -> Arc<Assets> {
        let mut carry = HashMap::new();
        if let Some(p) = prev.filter(|p| p.root == root && p.fps == fps && p.hwaccel == hwaccel && p.random == random && p.scale == scale) {
            let opened = p.opened.lock().unwrap();
            let list = p.list.read().unwrap();
            for (key, &id) in p.keys.lock().unwrap().iter() {
                if let (Some(a), Some(st)) = (list.get(id), opened.get(key)) {
                    if matches!(a.kind, Kind::Video(_)) && stamp(Path::new(&a.path)).as_ref() == Some(st) {
                        carry.insert(key.clone(), (a.clone(), *st));
                    }
                }
            }
            // taken over by prev but not used there: still good for the next one
            for (key, (a, st)) in p.carry.lock().unwrap().iter() {
                if stamp(Path::new(&a.path)).as_ref() == Some(st) {
                    carry.entry(key.clone()).or_insert_with(|| (a.clone(), *st));
                }
            }
        }
        Arc::new(Assets {
            root,
            fps,
            hwaccel,
            random,
            scale,
            keys: Mutex::new(HashMap::new()),
            list: RwLock::new(Vec::new()),
            seq_cache: Mutex::new((HashMap::new(), std::collections::VecDeque::new())),
            carry: Mutex::new(carry),
            opened: Mutex::new(HashMap::new()),
        })
    }

    pub fn resolve(&self, p: &str) -> PathBuf {
        let pb = PathBuf::from(p);
        if pb.is_absolute() { pb } else { self.root.join(pb) }
    }

    fn insert(&self, key: String, a: Asset) -> usize {
        self.insert_arc(key, Arc::new(a))
    }

    fn insert_arc(&self, key: String, a: Arc<Asset>) -> usize {
        let mut keys = self.keys.lock().unwrap();
        if let Some(&id) = keys.get(&key) {
            return id;
        }
        let mut l = self.list.write().unwrap();
        l.push(a);
        let id = l.len() - 1;
        keys.insert(key, id);
        id
    }

    fn lookup(&self, key: &str) -> Option<usize> {
        self.keys.lock().unwrap().get(key).copied()
    }

    pub fn get(&self, id: usize) -> Option<Arc<Asset>> {
        self.list.read().unwrap().get(id).cloned()
    }

    pub fn image(&self, p: &str) -> Result<(usize, u32, u32)> {
        let key = format!("img|{p}");
        if let Some(id) = self.lookup(&key) {
            let a = self.get(id).unwrap();
            return Ok((id, a.width, a.height));
        }
        let (file, query) = p.split_once('?').unwrap_or((p, ""));
        let path = self.resolve(file);
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext == "html" || ext == "htm" {
            // an HTML/CSS layout, snapshotted once by headless Chromium at 2x (transparent background); JS sees CSS px
            let q = |k: &str| query.split('&').find_map(|kv| kv.split_once('=').filter(|(a, _)| *a == k).and_then(|(_, v)| v.parse::<u32>().ok()));
            let (w, h) = (q("w").unwrap_or(1080), q("h").unwrap_or(1920));
            let png = html_snapshot(&path, w, h)?;
            let img = decode_file(&png)?;
            let hash = file_hash(&path, &format!("html{w}x{h}"));
            let id = self.insert(key, Asset { kind: Kind::Still(img), width: w, height: h, hash, path: path.display().to_string() });
            return Ok((id, w, h));
        }
        let img = decode_file(&path)?;
        let (w, h) = (img.width() as u32, img.height() as u32);
        let id = self.insert(key, Asset { kind: Kind::Still(img), width: w, height: h, hash: file_hash(&path, "img"), path: path.display().to_string() });
        Ok((id, w, h))
    }

    pub fn video(&self, p: &str, opts_json: &str) -> Result<(usize, u32, u32, f64, usize)> {
        let key = format!("vid|{p}|{opts_json}");
        if let Some(id) = self.lookup(&key) {
            let a = self.get(id).unwrap();
            if let Kind::Video(v) = &a.kind {
                return Ok((id, a.width, a.height, v.fps, v.n));
            }
        }
        let carried = self.carry.lock().unwrap().remove(&key);
        if let Some((a, st)) = carried {
            if let Kind::Video(v) = &a.kind {
                let r = (a.width, a.height, v.fps, v.n);
                self.opened.lock().unwrap().insert(key.clone(), st);
                let id = self.insert_arc(key, a.clone());
                return Ok((id, r.0, r.1, r.2, r.3));
            }
        }
        let path = self.resolve(p);
        if !path.exists() {
            return Err(anyhow!("video not found: {}", path.display()));
        }
        let st = stamp(&path);
        let o: serde_json::Value = serde_json::from_str(opts_json).unwrap_or_default();
        let fps = o["fps"].as_f64().unwrap_or(self.fps);
        let native = crate::video::probe(&path.to_string_lossy())?;
        let mut width = o["width"].as_u64().map(|w| w as u32);
        if width.is_none() && self.scale < 0.99 {
            width = Some(((native.width as f32 * self.scale / 2.).round() as u32 * 2).max(16));
        }
        let opts = VideoOpts {
            fps,
            width,
            hwaccel: self.hwaccel && o["hwaccel"].as_bool().unwrap_or(true),
            random: self.random,
            draft: self.scale < 0.99,
            ahead: {
                let (nw, nh) = width.map_or((native.width, native.height), |w| (w, (native.height as u64 * w as u64 / native.width.max(1) as u64) as u32));
                crate::video::ahead_for(nw, nh)
            },
            keep: 40,
        };
        let v = VideoSource::open(&path.to_string_lossy(), opts)?;
        // JS sees the native size (source rects are in native px); the decoded picture may be smaller
        let (w, h) = if native.rotation.abs() == 90 || native.rotation.abs() == 270 { (native.height, native.width) } else { (native.width, native.height) };
        let n = v.n;
        let hash = v.hash;
        if let Some(st) = st { self.opened.lock().unwrap().insert(key.clone(), st); }
        let id = self.insert(key, Asset { kind: Kind::Video(v), width: w, height: h, hash, path: path.display().to_string() });
        Ok((id, w, h, fps, n))
    }

    /// An image sequence; n = 0 when the count is unknown (frames past the last file hold the last one).
    pub fn seq(&self, pattern: &str, n: usize) -> Result<(usize, u32, u32, usize, usize)> {
        let key = format!("seq|{pattern}|{n}");
        if let Some(id) = self.lookup(&key) {
            let a = self.get(id).unwrap();
            let (first, nn) = match a.kind { Kind::Seq { first, n, .. } => (first, n), _ => (0, 0) };
            return Ok((id, a.width, a.height, first, nn));
        }
        // files are numbered from 1 (AIOS2) unless file 0 exists
        let first = if self.resolve(&fmt_pattern(pattern, 0)).exists() { 0 } else { 1 };
        let p0 = self.resolve(&fmt_pattern(pattern, first));
        let img = decode_file(&p0)?;
        let (w, h) = (img.width() as u32, img.height() as u32);
        let hash = file_hash(&p0, &format!("seq{n}"));
        let n = if n == 0 { count_files(&self.root, pattern, first) } else { n };
        let id = self.insert(key, Asset { kind: Kind::Seq { pattern: pattern.to_string(), n, first }, width: w, height: h, hash, path: pattern.to_string() });
        Ok((id, w, h, first, n))
    }

    /// The picture for (asset, frame): the still, the decoded video frame, or the sequence file.
    pub fn picture(&self, id: usize, frame: usize) -> Result<Image> {
        let a = self.get(id).ok_or_else(|| anyhow!("no asset {id}"))?;
        match &a.kind {
            Kind::Still(img) => Ok(img.clone()),
            Kind::Video(v) => v.get(frame),
            Kind::Seq { pattern, n, first } => {
                let f = frame.min(n.saturating_sub(1));
                {
                    let c = self.seq_cache.lock().unwrap();
                    if let Some(img) = c.0.get(&(id, f)) {
                        return Ok(img.clone());
                    }
                }
                let img = decode_file(&self.resolve(&fmt_pattern(pattern, f + first)))?;
                let mut c = self.seq_cache.lock().unwrap();
                c.0.insert((id, f), img.clone());
                c.1.push_back((id, f));
                while c.1.len() > 160 {
                    if let Some(k) = c.1.pop_front() {
                        c.0.remove(&k);
                    }
                }
                Ok(img)
            }
        }
    }

    /// 0 still, 1 video, 2 sequence (for the profile)
    pub fn kind_code(&self, id: usize) -> u32 {
        match self.get(id).map(|a| match a.kind { Kind::Still(_) => 0, Kind::Video(_) => 1, Kind::Seq { .. } => 2 }) { Some(k) => k, None => 9 }
    }

    /// The size JS sees for an asset (native pixels).
    pub fn native_size(&self, id: usize) -> (u32, u32) {
        self.get(id).map(|a| (a.width, a.height)).unwrap_or((0, 0))
    }

    /// Content hash of (asset, frame) for frame hashes.
    pub fn content_hash(&self, id: usize, frame: usize) -> u64 {
        match self.get(id) {
            Some(a) => match a.kind {
                Kind::Still(_) => a.hash,
                _ => a.hash ^ (frame as u64).wrapping_mul(0x9E3779B97F4A7C15),
            },
            None => 0,
        }
    }

    /// Stops the videos only this Assets holds (one taken over by newer Assets keeps running there).
    pub fn stop(&self) {
        let carry: Vec<Arc<Asset>> = self.carry.lock().unwrap().values().map(|(a, _)| a.clone()).collect();
        for a in self.list.read().unwrap().iter().chain(carry.iter()) {
            // held here once (list or carry; `carry` above adds one for its own entries)
            let mine = Arc::strong_count(a) <= if carry.iter().any(|c| Arc::ptr_eq(c, a)) { 2 } else { 1 };
            if !mine { continue; }
            if let Kind::Video(v) = &a.kind {
                v.stop();
                if std::env::var("AIOS_PROF").is_ok() {
                    use std::sync::atomic::Ordering::Relaxed;
                    eprintln!("  video {}: workers waited {:.0} ms, converted {:.0} ms, decoder read {:.0} ms",
                        v.path, v.wait_ns.load(Relaxed) as f64 / 1e6, v.conv_ns.load(Relaxed) as f64 / 1e6, v.read_ns.load(Relaxed) as f64 / 1e6);
                }
            }
        }
    }
}

/// headless Chromium (Playwright's chrome-headless-shell, or $AIOS_CHROME)
fn chrome() -> Option<PathBuf> {
    if let Ok(c) = std::env::var("AIOS_CHROME") {
        return Some(PathBuf::from(c));
    }
    let base = PathBuf::from(std::env::var("HOME").ok()?).join(".cache/ms-playwright");
    let mut found: Vec<PathBuf> = std::fs::read_dir(&base).ok()?.flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("chromium_headless_shell-"))
        .map(|e| e.path().join("chrome-headless-shell-linux64/chrome-headless-shell"))
        .filter(|p| p.exists())
        .collect();
    found.sort();
    found.pop()
}

/// An HTML file rendered to a transparent PNG at 2x, cached by its content and size.
fn html_snapshot(html: &Path, w: u32, h: u32) -> Result<PathBuf> {
    let src = std::fs::read(html).map_err(|e| anyhow!("{}: {e}", html.display()))?;
    let mut hh = xxhash_rust::xxh3::Xxh3::new();
    hh.update(&src);
    hh.update(format!("{w}x{h}|{}", html.display()).as_bytes());
    let dir = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into())).join(".cache/aios3/html");
    std::fs::create_dir_all(&dir)?;
    let out = dir.join(format!("{:016x}.png", hh.digest()));
    if out.exists() {
        return Ok(out);
    }
    let c = chrome().ok_or_else(|| anyhow!("HTML image needs headless Chromium (playwright install chromium-headless-shell, or set AIOS_CHROME)"))?;
    let o = std::process::Command::new(c)
        .args(["--disable-gpu", "--hide-scrollbars", "--force-device-scale-factor=2", "--default-background-color=00000000", "--allow-file-access-from-files"])
        .arg(format!("--window-size={w},{h}"))
        .arg(format!("--screenshot={}", out.display()))
        .arg(format!("file://{}", std::fs::canonicalize(html)?.display()))
        .output()?;
    if !out.exists() {
        return Err(anyhow!("HTML snapshot of {} failed: {}", html.display(), String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("")));
    }
    Ok(out)
}

/// How many consecutive files a pattern has from `first` (binary search on existence).
fn count_files(root: &Path, pattern: &str, first: usize) -> usize {
    let exists = |i: usize| {
        let p = PathBuf::from(fmt_pattern(pattern, i + first));
        if p.is_absolute() { p.exists() } else { root.join(p).exists() }
    };
    if !exists(0) {
        return 1;
    }
    let mut hi = 1usize;
    while exists(hi) && hi < 1 << 24 {
        hi *= 2;
    }
    let mut lo = hi / 2;
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if exists(mid) { lo = mid } else { hi = mid }
    }
    lo + 1
}

/// 'dir/%04d.jpg' with i → 'dir/0007.jpg'
pub fn fmt_pattern(p: &str, i: usize) -> String {
    if let Some(pos) = p.find('%') {
        let rest = &p[pos + 1..];
        let mut width = 0usize;
        let mut j = 0;
        let b = rest.as_bytes();
        while j < b.len() && b[j].is_ascii_digit() {
            width = width * 10 + (b[j] - b'0') as usize;
            j += 1;
        }
        if j < b.len() && b[j] == b'd' {
            return format!("{}{:0width$}{}", &p[..pos], i, &rest[j + 1..], width = width);
        }
    }
    p.to_string()
}

#[allow(dead_code)]
pub fn blank(w: i32, h: i32) -> Image {
    let info = ImageInfo::new((w, h), ColorType::BGRA8888, AlphaType::Premul, None);
    images::raster_from_data(&info, Data::new_zero_initialized((w * h * 4) as usize), (w * 4) as usize).unwrap()
}
