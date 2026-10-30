//! Video in. Frames are normalised to a constant rate (`fps`), so frame i is the picture at second i / fps — the
//! same grid AIOS2 used when it extracted JPEG frames with `-vf fps=30`.
//!
//! Two ways to read:
//! - sequential (renders): one decoder thread per source runs ffmpeg (NVDEC when the codec allows) ahead of the
//!   workers and keeps a window of decoded frames; a request outside the window restarts it there.
//! - random (stills, preview server): each request decodes its frame in the calling thread, in parallel. Sources
//!   with sparse keyframes (phone video, cuts: 7 s between keyframes) first get a seek proxy, made once and cached
//!   on disk (~/.cache/aios3/proxy): same picture, a keyframe every 10 frames, so any frame is ~10 decodes away.

use anyhow::{anyhow, Result};
use skia_safe::{images, AlphaType, ColorType, Data, Image, ImageInfo};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Probe {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: f64,
    pub pix_fmt: String,
    pub matrix: String,
    pub full_range: bool,
    pub codec: String,
    pub rotation: i32,
}

/// ffprobe of a video, kept per process by (path, size, mtime): `aios serve` loads the job again on each edit, and
/// a probe is ~0.1 s of the reload each time
pub fn probe(path: &str) -> Result<Probe> {
    type Seen = Mutex<HashMap<String, ((u64, Option<std::time::SystemTime>), Probe)>>;
    static SEEN: OnceLock<Seen> = OnceLock::new();
    let stamp = std::fs::metadata(path).map(|m| (m.len(), m.modified().ok())).ok();
    let seen = SEEN.get_or_init(Default::default);
    if let Some(st) = stamp {
        if let Some((s0, p)) = seen.lock().unwrap().get(path) {
            if *s0 == st { return Ok(p.clone()); }
        }
    }
    let p = probe_file(path)?;
    if let Some(st) = stamp { seen.lock().unwrap().insert(path.to_string(), (st, p.clone())); }
    Ok(p)
}

fn probe_file(path: &str) -> Result<Probe> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries",
            "stream=width,height,r_frame_rate,avg_frame_rate,pix_fmt,color_space,color_range,codec_name:stream_side_data=rotation:format=duration",
            "-of", "json", path])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!("ffprobe {path}: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let s = v["streams"].get(0).ok_or_else(|| anyhow!("{path}: no video stream"))?;
    let rate = |k: &str| -> f64 {
        let r = s[k].as_str().unwrap_or("0/1");
        let mut it = r.split('/');
        let a: f64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0.);
        let b: f64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(1.);
        if b > 0. { a / b } else { 0. }
    };
    let fps = { let r = rate("r_frame_rate"); if r > 0. && r < 1000. { r } else { rate("avg_frame_rate") } };
    let mut rotation = 0;
    if let Some(sd) = s["side_data_list"].as_array() {
        for d in sd {
            if let Some(r) = d["rotation"].as_i64() {
                rotation = r as i32;
            }
        }
    }
    Ok(Probe {
        width: s["width"].as_u64().unwrap_or(0) as u32,
        height: s["height"].as_u64().unwrap_or(0) as u32,
        fps,
        duration: v["format"]["duration"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.),
        pix_fmt: s["pix_fmt"].as_str().unwrap_or("").to_string(),
        matrix: s["color_space"].as_str().unwrap_or("").to_string(),
        full_range: s["color_range"].as_str() == Some("pc"),
        codec: s["codec_name"].as_str().unwrap_or("").to_string(),
        rotation,
    })
}

/// Largest gap between keyframes in the first 60 s (seconds).
fn keyframe_gap(path: &str) -> f64 {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-skip_frame", "nokey", "-read_intervals", "%+60", "-show_entries", "frame=pts_time", "-of", "csv=p=0", path])
        .output();
    let Ok(out) = out else { return 0. };
    let ts: Vec<f64> = String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().trim_end_matches(',').parse().ok()).collect();
    ts.windows(2).map(|w| w[1] - w[0]).fold(0., f64::max)
}

#[derive(Clone, Debug)]
pub struct VideoOpts {
    pub fps: f64,           // the grid frames are normalised to
    pub width: Option<u32>, // decode at this width (height keeps the aspect); None = native
    pub hwaccel: bool,
    pub random: bool,       // random access (stills, preview) instead of a sequential render
    pub draft: bool,        // drafts may read the seek proxy (fast jumps into the middle of a clip)
    pub ahead: usize,       // frames decoded ahead of the furthest request
    pub keep: usize,        // frames kept behind it
}

/// A decoded frame: the raw planes from ffmpeg, turned into an image by the first worker that needs it (so the
/// colour conversion is spread over the render workers instead of serialising in the decoder thread).
pub struct Frame {
    raw: Mutex<Option<Vec<u8>>>,
    img: OnceLock<Option<Image>>,
}

struct Shared {
    frames: BTreeMap<usize, Arc<Frame>>,
    want: usize,
    seek: Option<usize>,
    eof_at: Option<usize>,
    end: Option<usize>, // the first frame the file does not have (duration*fps can count one or two more); kept across seeks
    error: Option<String>,
    decoded_from: usize,
    next: usize,
    running: bool,
    started: bool,
    stop: bool,
}

pub struct VideoSource {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub n: usize,
    pub alpha: bool,
    pub hash: u64,
    st: Mutex<Shared>,
    cv: Condvar,
    opts: VideoOpts,
    probe: Probe,
    proxy: Mutex<Option<Option<String>>>,
    lru: Mutex<(HashMap<usize, Image>, VecDeque<usize>)>,
    pool: Mutex<Vec<Vec<u8>>>,
    hw_failed: Mutex<bool>,
    last_get: Mutex<std::time::Instant>, // the last sequential request: an idle source frees its frames
    pub wait_ns: AtomicU64,
    pub conv_ns: AtomicU64,
    pub read_ns: AtomicU64,
}

/// spare raw buffers kept for the decoder (it takes one per frame, the conversion gives one back)
const POOL: usize = 16;
/// a sequential source nobody asked for this long drops its frames and its ffmpeg: a b-roll whose scene has passed
/// held its whole window (4K: up to ~4 GB) until the end of the render (story final, 14 GB machine out of memory)
const IDLE: Duration = Duration::from_secs(2);

/// Frames a sequential source decodes ahead: 40, fewer for big frames so one source holds ~0.5 GB ahead at most
/// (1080x1920: 40, 4K: 15).
pub fn ahead_for(w: u32, h: u32) -> usize {
    ((512usize << 20) / (w as usize * h as usize * 4).max(1)).clamp(8, 40)
}

fn fmt_rate(f: f64) -> String {
    if (f - f.round()).abs() < 1e-9 { format!("{}", f.round() as i64) } else { format!("{f:.6}") }
}

impl VideoSource {
    pub fn open(path: &str, opts: VideoOpts) -> Result<Arc<VideoSource>> {
        let p = probe(path)?;
        let (mut w, mut h) = (p.width, p.height);
        if p.rotation.abs() == 90 || p.rotation.abs() == 270 {
            std::mem::swap(&mut w, &mut h);
        }
        if let Some(tw) = opts.width {
            if tw < w {
                h = ((h as f64 * tw as f64 / w as f64) / 2.).round() as u32 * 2;
                w = tw;
            }
        }
        let pf = p.pix_fmt.as_str();
        let alpha = pf.starts_with("yuva") || pf.contains("rgba") || pf.contains("bgra") || pf.contains("argb") || pf.contains("abgr") || pf.starts_with("gbrap") || pf == "ya8";
        let n = ((p.duration * opts.fps).round() as usize).max(1);
        let meta = std::fs::metadata(path)?;
        let mtime = meta.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
        let hash = xxhash_rust::xxh3::xxh3_64(format!("video|{path}|{}|{mtime}|{w}x{h}|{}", meta.len(), opts.fps).as_bytes());
        let v = Arc::new(VideoSource {
            path: path.to_string(),
            width: w,
            height: h,
            fps: opts.fps,
            n,
            alpha,
            hash,
            st: Mutex::new(Shared { frames: BTreeMap::new(), want: 0, seek: None, eof_at: None, end: None, error: None, decoded_from: 0, next: 0, running: false, started: false, stop: false }),
            cv: Condvar::new(),
            opts,
            probe: p,
            proxy: Mutex::new(None),
            lru: Mutex::new((HashMap::new(), VecDeque::new())),
            pool: Mutex::new(Vec::new()),
            hw_failed: Mutex::new(false),
            last_get: Mutex::new(std::time::Instant::now()),
            wait_ns: AtomicU64::new(0),
            conv_ns: AtomicU64::new(0),
            read_ns: AtomicU64::new(0),
        });
        Ok(v)
    }

    /// The decoded frame i (clamped to the clip).
    pub fn get(self: &Arc<Self>, i: usize) -> Result<Image> {
        let i = i.min(self.n.saturating_sub(1));
        if self.opts.random {
            return self.get_random(i);
        }
        let t_wait = std::time::Instant::now();
        *self.last_get.lock().unwrap() = t_wait;
        let mut st = self.st.lock().unwrap();
        if !st.started {
            st.started = true;
            let me = self.clone();
            std::thread::Builder::new().name("decode".into()).spawn(move || me.run()).unwrap();
        }
        loop {
            if let Some(e) = &st.error {
                return Err(anyhow!("{}: {e}", self.path));
            }
            // past the real end: ask for the last frame the file has (a seek to a missing frame finds nothing, and
            // asking again would undo the decoder's step back, so the pair looped forever)
            let i = match st.end { Some(e) if i >= e && e > 0 => e - 1, _ => i };
            if let Some(f) = st.frames.get(&i) {
                let f = f.clone();
                drop(st);
                let w = t_wait.elapsed().as_nanos() as u64;
                self.wait_ns.fetch_add(w, Ordering::Relaxed);
                crate::trace::add(crate::trace::C::VideoWait, w);
                if w > 200_000 { crate::trace::span("video", t_wait, || format!("wait video frame {i}")); }
                let t_c = std::time::Instant::now();
                let img = self.image(&f);
                crate::trace::span("video", t_c, || "convert".into());
                return img;
            }
            if let Some(eof) = st.eof_at {
                if i >= eof && eof > st.decoded_from {
                    // the file ends before duration*fps said: hold the last frame
                    if let Some((_, f)) = st.frames.range(..eof).next_back() {
                        let f = f.clone();
                        drop(st);
                        return self.image(&f);
                    }
                }
            }
            let ahead = self.opts.ahead.max(8);
            let in_run = st.running && i >= st.decoded_from && i < st.next + ahead * 2;
            let lost = i < st.next && i >= st.decoded_from && !st.frames.contains_key(&i);
            let past_eof = st.eof_at.map(|e| i >= e).unwrap_or(false) && st.eof_at.unwrap() > st.decoded_from;
            // a pending seek a little before i brings i too: replacing it would undo another thread's seek
            let covered = st.seek.map_or(false, |s| s <= i && i < s + ahead * 2);
            // the run still owes frames a worker asked for: a seek now would kill it before its first frame, and two
            // workers far apart (a 4-frame part between two jumps, story 46.45 s) killed each other's seeks forever
            let owed = st.running && st.seek.is_none() && st.next <= st.want;
            if (!in_run || lost) && !past_eof && !covered && !owed {
                st.seek = Some(i);
                self.cv.notify_all();
            } else if i > st.want {
                st.want = i;
                self.cv.notify_all();
            }
            st = self.cv.wait_timeout(st, Duration::from_millis(100)).unwrap().0;
            *self.last_get.lock().unwrap() = std::time::Instant::now(); // still waiting: not idle
        }
    }

    fn image(&self, f: &Frame) -> Result<Image> {
        let (w, h) = (self.width as usize, self.height as usize);
        f.img
            .get_or_init(|| {
                let t0 = std::time::Instant::now();
                let r = (|| {
                let raw = f.raw.lock().unwrap().take()?;
                let img = if self.alpha { bgra_image(&raw, w, h, AlphaType::Unpremul) } else { yuv_image(&raw, w, h, &self.probe) };
                let mut p = self.pool.lock().unwrap();
                if p.len() < POOL {
                    p.push(raw);
                }
                img
                })();
                self.conv_ns.fetch_add(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
                r
            })
            .clone()
            .ok_or_else(|| anyhow!("{}: frame conversion failed", self.path))
    }

    pub fn stop(&self) {
        let mut st = self.st.lock().unwrap();
        st.stop = true;
        self.cv.notify_all();
    }

    // ───────────────────────────── random access ─────────────────────────────

    fn proxy_file(&self) -> PathBuf {
        let dir = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into())).join(".cache/aios3/proxy");
        let _ = std::fs::create_dir_all(&dir);
        dir.join(format!("{:016x}.mp4", self.hash))
    }

    /// The proxy if it is already made (no waiting).
    fn existing_proxy(&self) -> Option<String> {
        let f = self.proxy_file();
        f.exists().then(|| f.to_string_lossy().to_string())
    }

    fn proxy_path(&self) -> Option<String> {
        let mut p = self.proxy.lock().unwrap();
        if let Some(x) = &*p {
            return x.clone();
        }
        if let Some(e) = self.existing_proxy() {
            *p = Some(Some(e.clone()));
            return Some(e);
        }
        let gap = if self.alpha { 0. } else { keyframe_gap(&self.path) };
        let r = if gap > 0.5 {
            let dir = self.proxy_file().parent().unwrap().to_path_buf();
            let out = self.proxy_file();
            if out.exists() {
                Some(out.to_string_lossy().to_string())
            } else {
                eprintln!("making a seek proxy for {} (keyframes {gap:.1} s apart; once)", self.path);
                let tmp = dir.join(format!("{:016x}.part.mp4", self.hash));
                let mut c = Command::new("ffmpeg");
                c.args(["-v", "error", "-y", "-nostdin"]);
                if self.opts.hwaccel { c.args(["-hwaccel", "cuda"]); }
                c.args(["-i", &self.path, "-map", "0:v:0", "-an", "-vf"]);
                let mut vf = format!("fps={}:start_time=0", fmt_rate(self.fps));
                if self.width != self.probe.width && self.probe.rotation == 0 {
                    vf.push_str(&format!(",scale={}:{}:flags=bicubic", self.width, self.height));
                }
                c.arg(vf);
                let enc: &[&str] = if crate::encode::nvenc_ok() {
                    &["-c:v", "h264_nvenc", "-preset", "p4", "-rc", "vbr", "-cq", "15", "-b:v", "0", "-g", "10", "-bf", "0"]
                } else {
                    &["-c:v", "libx264", "-preset", "veryfast", "-crf", "15", "-g", "10", "-bf", "0"]
                };
                c.args(enc).args(["-pix_fmt", "yuv420p"]).arg(&tmp);
                match c.status() {
                    Ok(s) if s.success() => {
                        let _ = std::fs::rename(&tmp, &out);
                        Some(out.to_string_lossy().to_string())
                    }
                    _ => None,
                }
            }
        } else {
            None
        };
        *p = Some(r.clone());
        r
    }

    fn get_random(&self, i: usize) -> Result<Image> {
        {
            let l = self.lru.lock().unwrap();
            if let Some(img) = l.0.get(&i) {
                return Ok(img.clone());
            }
        }
        // one ffmpeg call decodes the frame and the next few: scrubbing and neighbouring stills hit the cache
        let proxy = self.proxy_path();
        let src = proxy.clone().unwrap_or_else(|| self.path.clone());
        let window = if proxy.is_some() { 12 } else { 4 };
        let (mut child, pre) = self.spawn(&src, i, proxy.is_some(), false, Some(window))?;
        let (w, h) = (self.width as usize, self.height as usize);
        let frame_bytes = if self.alpha { w * h * 4 } else { w * h * 3 / 2 };
        let mut buf = vec![0u8; frame_bytes];
        let out = child.stdout.as_mut().unwrap();
        let mut got: Vec<(usize, Image)> = Vec::new();
        let mut k = 0usize;
        loop {
            match read_full(out, &mut buf) {
                Ok(true) => {
                    if k >= pre {
                        let idx = i + (k - pre);
                        let img = if self.alpha { bgra_image(&buf, w, h, AlphaType::Unpremul) } else { yuv_image(&buf, w, h, &self.probe) };
                        if let Some(img) = img { got.push((idx, img)); }
                    }
                    k += 1;
                }
                _ => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        if got.is_empty() {
            if i > 0 {
                // past the real end of the file: the last frame
                return self.get_random(i - 1);
            }
            return Err(anyhow!("{}: could not decode frame {i}", self.path));
        }
        let first = got[0].1.clone();
        let mut l = self.lru.lock().unwrap();
        for (idx, img) in got {
            if l.0.insert(idx, img).is_none() { l.1.push_back(idx); }
        }
        while l.1.len() > 160 {
            if let Some(k) = l.1.pop_front() { l.0.remove(&k); }
        }
        Ok(first)
    }

    // ───────────────────────────── ffmpeg ─────────────────────────────

    /// ffmpeg from frame `from`, started `pre` frames earlier so the fps grid has settled when `from` comes out;
    /// the caller discards those first `pre` frames. A proxy is already on the grid at the right size.
    fn spawn(&self, src: &str, from: usize, is_proxy: bool, hw: bool, frames: Option<usize>) -> Result<(Child, usize)> {
        let pre = if is_proxy { 0 } else { from.min(15) };
        let start = from - pre;
        let hw = hw && !self.alpha && !*self.hw_failed.lock().unwrap()
            && matches!(self.probe.codec.as_str(), "h264" | "hevc" | "av1" | "vp9" | "mpeg4" | "mpeg2video");
        let mut c = Command::new("ffmpeg");
        c.args(["-v", "error", "-nostdin"]);
        if hw {
            // decode, resize and convert on the GPU; only the finished yuv420p planes cross to the CPU
            c.args(["-hwaccel", "cuda", "-hwaccel_output_format", "cuda"]);
        }
        if start > 0 {
            c.args(["-ss", &format!("{:.6}", start as f64 / self.fps)]);
        }
        c.args(["-i", src, "-map", "0:v:0", "-an", "-sn"]);
        let resize = !is_proxy && self.width != self.probe.width && self.probe.rotation == 0;
        let pf = if self.alpha { "bgra" } else { "yuv420p" };
        let mut vf = if is_proxy { "null".to_string() } else { format!("fps={}:start_time=0", fmt_rate(self.fps)) };
        if hw {
            if resize {
                vf.push_str(&format!(",scale_cuda={}:{}:format=yuv420p", self.width, self.height));
            } else {
                vf.push_str(",scale_cuda=format=yuv420p");
            }
            vf.push_str(",hwdownload,format=yuv420p");
        } else {
            if resize {
                vf.push_str(&format!(",scale={}:{}:flags=bicubic", self.width, self.height));
            }
            vf.push_str(&format!(",format={pf}"));
        }
        if let Some(n) = frames {
            c.args(["-frames:v", &(n + pre).to_string()]);
        }
        c.args(["-vf", &vf, "-f", "rawvideo", "-pix_fmt", pf, "pipe:1"]);
        c.stdout(Stdio::piped()).stderr(Stdio::piped());
        let child = c.spawn()?;
        #[cfg(target_os = "linux")]
        if let Some(o) = &child.stdout {
            // 1 MB pipe: a frame arrives in 3 reads instead of 48 (Linux only)
            use std::os::fd::AsRawFd;
            unsafe { libc::fcntl(o.as_raw_fd(), libc::F_SETPIPE_SZ, 1 << 20) };
        }
        Ok((child, pre))
    }

    fn run(self: Arc<Self>) {
        let (w, h) = (self.width as usize, self.height as usize);
        let frame_bytes = if self.alpha { w * h * 4 } else { w * h * 3 / 2 };
        let _ = (w, h);
        let mut child: Option<Child> = None;
        let mut skip = 0usize;
        let mut buf = vec![0u8; frame_bytes];
        loop {
            let (seek, stop) = {
                let mut st = self.st.lock().unwrap();
                loop {
                    if st.stop {
                        break (None, true);
                    }
                    if st.seek.is_some() {
                        break (st.seek.take(), false);
                    }
                    if child.is_some() && st.next <= st.want + self.opts.ahead {
                        break (None, false);
                    }
                    if (child.is_some() || !st.frames.is_empty()) && self.last_get.lock().unwrap().elapsed() > IDLE {
                        // idle: free the window; the next request seeks back here
                        if let Some(mut c) = child.take() {
                            let _ = c.kill();
                            let _ = c.wait();
                        }
                        st.frames.clear();
                        st.running = false;
                        st.eof_at = None;
                        self.pool.lock().unwrap().clear();
                    }
                    st = self.cv.wait_timeout(st, Duration::from_millis(100)).unwrap().0;
                }
            };
            if stop {
                if let Some(mut c) = child.take() {
                    let _ = c.kill();
                    let _ = c.wait();
                }
                return;
            }
            if let Some(from) = seek {
                crate::trace::add(crate::trace::C::VideoSeeks, 1);
                let t_seek = std::time::Instant::now();
                let _seek_span = SeekSpan(from, t_seek);
                if let Some(mut c) = child.take() {
                    let _ = c.kill();
                    let _ = c.wait();
                }
                let proxy = if self.opts.draft && !self.alpha { self.existing_proxy() } else { None };
                if self.opts.draft && proxy.is_none() && !self.alpha && from == 0 {
                    // first draft of this clip: read the original now, make the proxy for the next time
                    let me = self.clone();
                    std::thread::spawn(move || { let _ = me.proxy_path(); });
                }
                let spawned = match &proxy {
                    // the proxy is small with a keyframe every 10 frames: the CPU starts decoding in 0.1 s, NVDEC in 0.7 s
                    Some(p) => self.spawn(p, from, true, false, None),
                    None => self.spawn(&self.path, from, false, self.opts.hwaccel, None),
                };
                match spawned {
                    Ok((c, pre)) => {
                        child = Some(c);
                        skip = pre;
                    }
                    Err(e) => {
                        let mut st = self.st.lock().unwrap();
                        st.error = Some(e.to_string());
                        self.cv.notify_all();
                        return;
                    }
                }
                let mut st = self.st.lock().unwrap();
                st.decoded_from = from;
                st.next = from;
                // from here on: after a jump back (a loop wrap) an old high `want` would let the decoder race ahead and
                // drop the frames the workers wait for; they raise it again as they ask
                st.want = from;
                st.eof_at = None;
                st.running = true;
                let keep_lo = from.saturating_sub(self.opts.keep);
                let keep_hi = from + self.opts.ahead * 4;
                st.frames.retain(|k, _| *k >= keep_lo && *k <= keep_hi);
                continue;
            }
            let c = child.as_mut().unwrap();
            let out = c.stdout.as_mut().unwrap();
            let t_read = std::time::Instant::now();
            let rr = read_full(out, &mut buf);
            self.read_ns.fetch_add(t_read.elapsed().as_nanos() as u64, Ordering::Relaxed);
            crate::trace::add_since(crate::trace::C::VideoRead, t_read);
            crate::trace::span("decode", t_read, || "read frame".into());
            match rr {
                Ok(true) if skip > 0 => skip -= 1,
                Ok(true) => {
                    let fresh = self.pool.lock().unwrap().pop().unwrap_or_else(|| vec![0u8; frame_bytes]);
                    let raw = std::mem::replace(&mut buf, fresh);
                    let fr = Arc::new(Frame { raw: Mutex::new(Some(raw)), img: OnceLock::new() });
                    let mut st = self.st.lock().unwrap();
                    let idx = st.next;
                    st.next += 1;
                    st.frames.insert(idx, fr);
                    let lo = st.want.saturating_sub(self.opts.keep + self.opts.ahead);
                    while let Some((&k, _)) = st.frames.iter().next() {
                        if k < lo && st.frames.len() > self.opts.keep { st.frames.remove(&k); } else { break; }
                    }
                    self.cv.notify_all();
                }
                Ok(false) => {
                    let mut err = String::new();
                    if let Some(mut e) = c.stderr.take() {
                        let _ = e.read_to_string(&mut err);
                    }
                    let mut st = self.st.lock().unwrap();
                    if st.next == st.decoded_from && !err.trim().is_empty() {
                        let mut hf = self.hw_failed.lock().unwrap();
                        if self.opts.hwaccel && !*hf {
                            // NVDEC could not take this file: decode on the CPU from here on
                            *hf = true;
                            eprintln!("{}: GPU decode failed, using the CPU ({})", self.path, err.lines().next().unwrap_or("").trim());
                            st.seek = Some(st.decoded_from);
                            st.running = false;
                            drop(hf);
                            self.cv.notify_all();
                            let _ = c.wait();
                            child = None;
                            continue;
                        }
                        st.error = Some(format!("ffmpeg could not decode: {}", err.trim()));
                    }
                    if st.next == st.decoded_from && st.decoded_from > 0 && st.error.is_none() {
                        // asked past the end: hold the last frame
                        let last = st.decoded_from - 1;
                        st.seek = Some(last.saturating_sub(1));
                    }
                    if (st.error.is_none() && st.decoded_from > 0) || st.next > st.decoded_from {
                        let e = st.next;
                        st.end = Some(st.end.map_or(e, |x| x.min(e)));
                    }
                    st.eof_at = Some(st.next);
                    st.running = false;
                    self.cv.notify_all();
                    let _ = c.wait();
                    child = None;
                }
                Err(e) => {
                    let mut st = self.st.lock().unwrap();
                    st.error = Some(e.to_string());
                    self.cv.notify_all();
                    return;
                }
            }
        }
    }
}

fn read_full(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<bool> {
    let mut got = 0;
    while got < buf.len() {
        match r.read(&mut buf[got..]) {
            Ok(0) => return Ok(false),
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

pub fn bgra_image(buf: &[u8], w: usize, h: usize, at: AlphaType) -> Option<Image> {
    let info = ImageInfo::new((w as i32, h as i32), ColorType::BGRA8888, at, None);
    images::raster_from_data(&info, Data::new_copy(buf), w * 4)
}

/// YUV 4:2:0 (planar) → BGRA, BT.709 or BT.601, limited or full range, split across 4 threads.
pub fn yuv_image(buf: &[u8], w: usize, h: usize, p: &Probe) -> Option<Image> {
    let bt601 = matches!(p.matrix.as_str(), "bt470bg" | "smpte170m" | "bt601") || (p.matrix.is_empty() && h < 720);
    let data = unsafe { Data::new_uninitialized(w * h * 4) };
    // SAFETY: the buffer is fully written by the converter before anyone reads it
    let out = unsafe { std::slice::from_raw_parts_mut(data.as_bytes().as_ptr() as *mut u8, w * h * 4) };
    yuv420_to_bgra(buf, w, h, out, bt601, p.full_range);
    let info = ImageInfo::new((w as i32, h as i32), ColorType::BGRA8888, AlphaType::Opaque, None);
    images::raster_from_data(&info, data, w * 4)
}

pub fn yuv420_to_bgra(src: &[u8], w: usize, h: usize, out: &mut [u8], bt601: bool, full: bool) {
    let cw = w / 2;
    let (ys, rest) = src.split_at(w * h);
    let (us, vs) = rest.split_at(cw * (h / 2));
    let (kr, kb) = if bt601 { (0.299f64, 0.114f64) } else { (0.2126f64, 0.0722f64) };
    let kg = 1. - kr - kb;
    let (ysc, csc, yoff) = if full { (1.0, 1.0, 0.0) } else { (255. / 219., 255. / 224., 16.0) };
    let f = |x: f64| (x * 65536.).round() as i32;
    let cy = f(ysc);
    let crv = f(csc * 2. * (1. - kr));
    let cbu = f(csc * 2. * (1. - kb));
    let cgu = f(csc * 2. * (1. - kb) * kb / kg);
    let cgv = f(csc * 2. * (1. - kr) * kr / kg);
    let yoff = yoff as i32;
    #[inline(always)]
    fn px(o: &mut [u8], yy: i32, ruv: i32, guv: i32, buv: i32) {
        o[0] = ((yy + buv) >> 16).clamp(0, 255) as u8;
        o[1] = ((yy + guv) >> 16).clamp(0, 255) as u8;
        o[2] = ((yy + ruv) >> 16).clamp(0, 255) as u8;
        o[3] = 255;
    }
    for y in 0..h {
        let yrow = &ys[y * w..y * w + w];
        let crow = (y / 2).min((h / 2).saturating_sub(1));
        let urow = &us[crow * cw..crow * cw + cw];
        let vrow = &vs[crow * cw..crow * cw + cw];
        let orow = &mut out[y * w * 4..y * w * 4 + w * 4];
        for (((yp, op), &u), &v) in yrow.chunks_exact(2).zip(orow.chunks_exact_mut(8)).zip(urow.iter()).zip(vrow.iter()) {
            let (u, v) = (u as i32 - 128, v as i32 - 128);
            let ruv = crv * v + 32768;
            let guv = -cgu * u - cgv * v + 32768;
            let buv = cbu * u + 32768;
            let (o0, o1) = op.split_at_mut(4);
            px(o0, (yp[0] as i32 - yoff) * cy, ruv, guv, buv);
            px(o1, (yp[1] as i32 - yoff) * cy, ruv, guv, buv);
        }
        if w % 2 == 1 {
            let x = w - 1;
            let (u, v) = (urow[cw.saturating_sub(1)] as i32 - 128, vrow[cw.saturating_sub(1)] as i32 - 128);
            px(&mut orow[x * 4..x * 4 + 4], (yrow[x] as i32 - yoff) * cy, crv * v + 32768, -cgu * u - cgv * v + 32768, cbu * u + 32768);
        }
    }
}



/// Trace span of a decoder restart (kill + spawn ffmpeg at `from`).
struct SeekSpan(usize, std::time::Instant);
impl Drop for SeekSpan {
    fn drop(&mut self) { let f = self.0; crate::trace::span("decode", self.1, || format!("seek to frame {f}")); }
}
