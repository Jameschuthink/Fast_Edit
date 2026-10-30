//! `aios serve JOB`: the live page (engine/js/live.html). A warm pool of workers answers frame requests in tens of
//! milliseconds with the engine's own pixels (no preview/final mismatch); every job file is watched, and an edit
//! reloads the comp and repaints the frame on screen. 1 s after an edit the cached draft (with sound) is made again,
//! so Play is current. The page is a thin client: it only asks for frames, the draft and small JSON, so the engine
//! renders as fast as without it.
//!
//!   GET  /                 the moment page: engine/js/live.html (read on each request, so a page edit needs no
//!                          rebuild): tabs transcript, storyboard, assets, clips + draft, frame (FLOW_SPEC step 3)
//!   GET  /editor           the frame editor (engine/js/editor.html), the page's frame tab, loaded when it opens
//!   GET  /moments          the moments: words, span, still, clips, notes, assets; the stage status (moments.rs)
//!   GET  /files            the job folder as a tree; GET /job/PATH one of its files (byte ranges)
//!   GET|POST /status       _ai/status; POST {stage, state: open|approved} (the agent's stage marks)
//! The clip queue (jobs with _ai/): 2 s after an edit, each moment whose frames or sound changed gets a new still
//! and then a new clip in review/ (moments.rs). In such a job the auto-draft waits until
//! the frame tab asks for the draft.
//!   GET  /frame?t=12.4     the frame as JPEG (draft size)
//!   GET  /lift?t=&id=&part=plate|sprite|top   a frame drag's pictures, painted once at the press: plate = what is
//!                          drawn before element `id` (JPEG); sprite = the element alone; top = what is drawn after it
//!                          (the head matte, the captions). PNG, header X-Box: x,y,w,h in world units; 204 = nothing.
//!                          The page stacks plate, moved sprite, top; nothing renders until the release.
//!   GET  /boxes?t=12.4     the frame's elements in world units: [{id, kind, x, y, w, h}] (click a graphic to select it)
//!   GET  /info             {name, dur, fps, world, words, beats, raw, captions, layout, sound, version, error}
//!   GET  /version          {version, error, drafted, draft_busy, draft_held, draft_msg, notes, seek, queue, made}   (the page polls it)
//!   GET  /draft/go         a frame drag held the auto-draft (draft_held); Play lets it run
//!   GET  /peaks?src=voice  the waveform, one byte per 10 ms: voice = the voice (or the clip's sound), mix = the draft's
//!   POST /undo, /redo      the last change (page edit) back / again
//!   POST /draft            runs `aios draft JOB` (the page's draft; ?review=1: a new review/NNN_draft.mp4), returns its report
//!   GET  /draft.mp4        the page's last draft (_ai/cache/live_draft.mp4; byte ranges, so the page can seek in it)
//!   GET  /library          the pick lists (moves, looks, fx, sfx, songs; `library`), GET /lib?p=PATH one of their files
//!   GET  /notes            the note log, jobs/JOB/notes.json: [{id, t, t2?, word, text, status, reply}]
//!   POST /notes            {t, t2?, word, text} -> a new note (status "new") + the frame it was written on, notes/ID.jpg
//!   POST /notes/del        {id} -> removes a note the agent has not taken yet
//! An edit-file job (jobs/JOB/edit.txt, editfile.rs) has its own:
//!   POST /line             {n, raw, off} -> "#" on / off for line n
//!   POST /lineset          {n, raw, set: {a, b, text, KEY: value|null}} -> one line changed by a frame drag (move / resize a text)
//!   POST /undo, /redo      (edit-file job) the page's last line change back / again (.cache/lines; notes keep their own)
//!   POST /enote            {t, text, covers, png, moment, file, sec} -> a note line in edit.txt (+ notes/ID.jpg, ID.draw.png); the
//!                          agent applies it on "check notes" (`aios note JOB`)
//!   POST /enote/undo       {id} -> put back what the note changed; POST /enote/redo {id} -> apply it again
//! The agent reads the log on "check notes", applies the new notes as one batch and writes status "done" or "ask" + a
//! reply. `aios seek JOB T` moves the page to second T (jobs/JOB/.cache/seek.json), to show the user a moment.

use crate::assets::Assets;
use crate::job::{ai_dir, cache_dir, engine_dir, Job};
use crate::layout::{Key, Layout};
use crate::moments;
use crate::render::{Pool, Settings, Want};
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// The pages as built into the binary; the files in engine/js on disk win (edit one, reload the page).
const PAGE: &str = include_str!("../js/live.html");
const EDITOR: &str = include_str!("../js/editor.html");

struct Live {
    job: Job,
    assets: Arc<Assets>,
    settings: Settings,
    version: u64,
    error: Option<String>,
    stamp: Vec<(PathBuf, Option<SystemTime>)>,
    info: Value,
    /// auto-draft: the comp version the last draft was made from, and when the comp last changed
    drafted: u64,
    draft_busy: bool,
    draft_msg: String,
    changed_at: std::time::Instant,
    /// the clip queue: its state for the page, and a count of the stills and clips it wrote (the page reloads on it)
    queue_msg: String,
    made: u64,
    /// the frame tab asked for the draft (a job with _ai/ drafts only then)
    draft_wanted: bool,
    /// a frame drag (a nudge-only /lineset) holds the auto-draft until Play asks for it (GET /draft/go)
    draft_held: bool,
}

/// The clip queue's view of the server (moments::Host): the pool's hashes and stills, `aios draft` for a clip.
struct Queue {
    live: Arc<Mutex<Live>>,
    pool: Arc<Mutex<Option<Pool>>>,
    lock: Arc<Mutex<()>>,
    v: u64,
    name: String,
}

impl moments::Host for Queue {
    fn hashes(&self, frames: &[usize]) -> Result<Vec<u64>> {
        self.pool.lock().unwrap().as_ref().ok_or_else(|| anyhow::anyhow!("no pool"))?.hashes(frames)
    }
    fn jpeg(&self, frame: usize) -> Result<Vec<u8>> {
        let mut out = None;
        self.pool.lock().unwrap().as_ref().ok_or_else(|| anyhow::anyhow!("no pool"))?
            .render(&[frame], |_| Want::Jpeg, |_, b| { out = Some(b); Ok(()) }, false)?;
        out.ok_or_else(|| anyhow::anyhow!("no frame {frame}"))
    }
    fn clip(&self, a: f64, b: f64, out: &Path) -> Result<()> {
        let _g = self.lock.lock().unwrap();
        let o = std::process::Command::new(aios_exe()).args(["draft", &self.name, "--from", &format!("{a:.3}"), "--to", &format!("{b:.3}"), "--out"]).arg(out).output()?;
        let txt = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
        if txt.contains("wrote ") { Ok(()) } else { Err(anyhow::anyhow!("aios draft: {}", txt.trim())) }
    }
    fn current(&self) -> bool {
        let l = self.live.lock().unwrap();
        l.version == self.v && l.error.is_none()
    }
    fn say(&self, msg: String, made: bool) {
        let mut l = self.live.lock().unwrap();
        l.queue_msg = msg;
        if made {
            l.made += 1;
        }
    }
}

/// this binary; after a rebuild (the running file is gone) the new one in target/release
fn aios_exe() -> PathBuf {
    std::env::current_exe().ok().filter(|p| p.exists()).unwrap_or_else(|| engine_dir().join("target/release/aios"))
}

/// The page's own draft (Play): in the cache, not in review/, so an edit does not add a review file
pub fn live_draft(dir: &Path) -> PathBuf {
    cache_dir(dir).join("live_draft.mp4")
}

/// `aios draft JOB` in a child process (its own pool and the chunk cache); one at a time.
fn run_draft(name: &str, dir: &Path, lock: &Mutex<()>) -> String {
    let _g = lock.lock().unwrap();
    let exe = aios_exe();
    let out = live_draft(dir);
    let _ = std::fs::create_dir_all(out.parent().unwrap());
    match std::process::Command::new(exe).args(["draft", name, "--out"]).arg(&out).output() {
        Ok(o) => format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)),
        Err(e) => format!("{e}"),
    }
}

fn watched(job: &Job) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = job.src.files.clone();
    v.push(job.dir.join("job.json"));
    for lib in &job.src.libs {
        v.push(job.src.engine_js.join("lib").join(format!("{lib}.js")));
    }
    v.push(job.src.engine_js.join("prelude.js"));
    for k in ["beats", "captions", "faces", "words", "poster"] {
        if let Some(p) = job.cfg_source(k) {
            v.push(p);
        }
    }
    // an edit-file job: edit.txt, a matte that is still being made (its folder appears when it is ready), and each
    // clip's prep (made again or removed: reload, so the pool never decodes a deleted file)
    for (k, p) in &job.sources {
        if k == "edit" || k == "notes" || k.starts_with("matte") || k.starts_with("prep ") {
            v.push(p.clone());
        }
    }
    v
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn stamps(files: &[PathBuf]) -> Vec<(PathBuf, Option<SystemTime>)> {
    files.iter().map(|p| (p.clone(), mtime(p))).collect()
}

type Built = (Job, Pool, Settings, Value, Arc<Assets>);

/// `prev`: the assets in use: their open videos are taken over when the files did not change (a warm first frame)
fn build(name: &str, width: u32, workers: usize, prev: Option<&Assets>, layout: &Layout) -> Result<Built> {
    let job = Job::load(name, layout)?;
    let k = width as f32 / job.world.0 as f32;
    let s = Settings {
        out_w: width,
        out_h: ((job.world.1 as f32 * k / 2.).round() as u32 * 2).max(2),
        k,
        fps: job.fps,
        dur: job.dur,
        blur: Some(crate::render::Blur { shutter: 180., gap: 10. * k.max(0.25), smax: 8 }),
        workers,
        gpu: 0,
    };
    let assets = Assets::renew(prev, job.dir.clone(), job.fps, true, true, k);
    // the page's timeline: captions, the grid, the comp's sounds (REEL beats: older comps only)
    let shaper = std::rc::Rc::new(std::cell::RefCell::new(crate::text::Shaper::new()));
    let js = crate::js::Js::new(&job.src, assets.clone(), shaper)?;
    let info = js
        .eval_json("({beats: (globalThis.REEL && REEL.state) ? REEL.state.beats.map(b => ({i: b.i, type: b.type, t0: b.t0, t1: Math.min(b.t1, JOB.dur), layer: b.layer || null, \
                text: b.text || (b.items && b.items.join(' / ')) || (b.lines && b.lines.join(' / ')) || (b.left !== undefined ? b.left + ' → ' + b.right : '') || b.src || b.video || b.seq || b.fn || ''})) : [], \
            captions: ((JOB.captions && JOB.captions.captions) || []).map(c => ({s: c.start, e: c.end, lines: c.lines.map(l => l.map(w => [w.w, w.tier]))})), \
            layout: globalThis.LAYOUT ? LAYOUT.info() : null, \
            sfx: (globalThis.REEL && REEL.state) ? REEL.sfx() : []})")
        .unwrap_or_default();
    drop(js);
    let pool = Pool::new(&job.src, assets.clone(), s.clone(), workers);
    Ok((job, pool, s, info, assets))
}

/// The waveform of a file's sound: the loudest sample of every 10 ms, -50..0 dB as 0..255.
fn peaks(path: &Path) -> Result<Vec<u8>> {
    let o = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", "4000", "-f", "s16le", "-"])
        .output()?;
    if !o.status.success() {
        return Err(anyhow::anyhow!("ffmpeg: {}", String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(o.stdout
        .chunks(80)
        .map(|c| {
            let m = c.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs()).max().unwrap_or(0) as f64 / 32768.;
            let db = 20. * m.max(1e-6).log10();
            (((db + 50.) / 50.).clamp(0., 1.) * 255.).round() as u8
        })
        .collect())
}

/// A file named in job.json: absolute, or in the job folder.
fn job_file(job: &Job, p: &str) -> PathBuf {
    let pb = PathBuf::from(p);
    if pb.is_absolute() { pb } else { job.dir.join(pb) }
}

/// "bytes=a-b" (or "a-", "-n") -> the first and last byte
fn byte_range(h: &str, len: usize) -> Option<(usize, usize)> {
    let r = h.trim().strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, z) = r.split_once('-')?;
    if len == 0 {
        return None;
    }
    let (a, z) = if a.is_empty() {
        let n: usize = z.parse().ok()?;
        (len.saturating_sub(n), len - 1)
    } else {
        let a: usize = a.parse().ok()?;
        (a, if z.is_empty() { len - 1 } else { z.parse::<usize>().ok()?.min(len - 1) })
    };
    (a <= z && a < len).then_some((a, z))
}

pub fn serve(name: &str, port: u16, width: u32, layout: &Layout) -> Result<()> {
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).saturating_sub(2).clamp(2, 8);
    // one handle for the watcher and the request threads
    let layout = Arc::new(layout.clone());
    let (job, pool, settings, info, assets) = build(name, width, workers, None, &layout)?;
    let stamp = stamps(&watched(&job));
    let live = Arc::new(Mutex::new(Live {
        job, assets, settings, version: 1, error: None, stamp, info,
        drafted: 0, draft_busy: false, draft_msg: String::new(), changed_at: std::time::Instant::now(),
        queue_msg: String::new(), made: 0, draft_wanted: false, draft_held: false,
    }));
    // the workers have their own lock: a frame being painted never holds up /version, /info or a note
    let pool: Arc<Mutex<Option<Pool>>> = Arc::new(Mutex::new(Some(pool)));
    let draft_lock = Arc::new(Mutex::new(()));
    // auto-draft: 1 s after the last good edit (and once at start), the cached draft is made again,
    // so "Play" always plays the current version (1-3 s after an edit: only changed seconds repaint)
    {
        let live = live.clone();
        let lock = draft_lock.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(300));
            let (v, name) = {
                let mut l = live.lock().unwrap();
                if l.draft_busy || l.draft_held || l.error.is_some() || l.drafted >= l.version || l.changed_at.elapsed() < Duration::from_secs(1) {
                    continue;
                }
                // a job with _ai/: the draft renders only for the frame tab (it asks once it opens)
                if l.job.dir.join("_ai").is_dir() && !l.draft_wanted {
                    continue;
                }
                if l.job.frames() <= 1 {
                    // a still (poster, cover): no video draft
                    l.drafted = l.version;
                    l.draft_msg = format!("a still: no video draft (print size: aios export {})", l.job.name);
                    continue;
                }
                l.draft_busy = true;
                (l.version, (l.job.name.clone(), l.job.dir.clone()))
            };
            let t0 = std::time::Instant::now();
            let out = run_draft(&name.0, &name.1, &lock);
            let ok = out.contains("wrote ");
            let mut l = live.lock().unwrap();
            l.drafted = v;
            l.draft_busy = false;
            l.draft_msg = if ok { format!("draft up to date ({:.1} s)", t0.elapsed().as_secs_f64()) } else { format!("draft failed: {}", out.trim()) };
            eprintln!("{}", l.draft_msg);
        });
    }
    // the clip queue (a job with _ai/): 2 s after the last edit, a pass makes the stills and clips that changed
    {
        let (live, pool, lock) = (live.clone(), pool.clone(), draft_lock.clone());
        let name = name.to_string();
        std::thread::spawn(move || {
            let mut done = 0u64;
            loop {
                std::thread::sleep(Duration::from_millis(500));
                let w = {
                    let l = live.lock().unwrap();
                    if l.error.is_some() || l.version == done || l.changed_at.elapsed() < Duration::from_secs(2) || !l.job.dir.join("_ai").is_dir() || l.job.frames() <= 1 {
                        continue;
                    }
                    moments::Work { v: l.version, dir: l.job.dir.clone(), cfg: l.job.cfg.clone(), fps: l.job.fps, frames: l.job.frames(), dur: l.job.dur }
                };
                let q = Queue { live: live.clone(), pool: pool.clone(), lock: lock.clone(), v: w.v, name: name.clone() };
                match moments::pass(&w, &q) {
                    Ok(true) => done = w.v,
                    Ok(false) => {}
                    Err(e) => {
                        eprintln!("clip queue: {e:#}");
                        moments::Host::say(&q, format!("clips failed: {e:#}"), false);
                        done = w.v;
                    }
                }
            }
        });
    }
    // the watcher: an edit reloads the comp (polled every 100 ms: a page edit shows in ~0.3 s)
    {
        let (live, pool, layout) = (live.clone(), pool.clone(), layout.clone());
        let name = name.to_string();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(100));
            let before = {
                let l = live.lock().unwrap();
                let files: Vec<PathBuf> = l.stamp.iter().map(|(p, _)| p.clone()).collect();
                let now = stamps(&files);
                if now == l.stamp {
                    continue;
                }
                now
            };
            let t0 = std::time::Instant::now();
            let prev = live.lock().unwrap().assets.clone();
            match build(&name, width, workers, Some(&prev), &layout) {
                Ok((job, new_pool, settings, info, assets)) => {
                    let old = pool.lock().unwrap().replace(new_pool);
                    if let Some(old) = old {
                        let _ = old.close();
                    }
                    let mut l = live.lock().unwrap();
                    // the times seen before the build: a file written during it is seen again next time
                    l.stamp = watched(&job).into_iter().map(|p| before.iter().find(|(q, _)| *q == p).cloned().unwrap_or_else(|| (p.clone(), mtime(&p)))).collect();
                    l.job = job;
                    l.assets = assets;
                    l.settings = settings;
                    l.info = info;
                    l.error = None;
                    l.version += 1;
                    l.changed_at = std::time::Instant::now();
                    eprintln!("reloaded ({:.0} ms)", t0.elapsed().as_secs_f64() * 1000.);
                }
                Err(e) => {
                    let mut l = live.lock().unwrap();
                    l.stamp = before;
                    l.error = Some(format!("{e:#}"));
                    l.version += 1;
                    eprintln!("reload failed: {e:#}");
                }
            }
        });
    }
    let server = tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| anyhow::anyhow!("port {port}: {e}"))?;
    println!("live page: http://127.0.0.1:{port}/   (job {name}; edits reload by themselves; Ctrl-C to stop)");
    // one thread per request: a draft render does not hold up the page's frames and polling.
    // /beat, /edit, /undo, /redo and /notes write job files, so they take `files` one at a time
    let files = Arc::new(Mutex::new(()));
    let waves: Arc<Mutex<HashMap<String, (PathBuf, Option<SystemTime>, Arc<Vec<u8>>)>>> = Arc::new(Mutex::new(HashMap::new()));
    for req in server.incoming_requests() {
        let (live, pool, draft_lock, files, waves, layout) = (live.clone(), pool.clone(), draft_lock.clone(), files.clone(), waves.clone(), layout.clone());
        std::thread::spawn(move || {
            let mut req = req;
            let url = req.url().to_string();
            let (path, query) = url.split_once('?').unwrap_or((&url, ""));
            let q = |k: &str| query.split('&').find_map(|kv| kv.split_once('=').filter(|(a, _)| *a == k).map(|(_, b)| b.to_string()));
            let ctype = |c: &str| tiny_http::Header::from_bytes(&b"Content-Type"[..], c.as_bytes()).unwrap();
            let nocache = tiny_http::Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap();
            let json_resp = |v: Value| tiny_http::Response::from_string(v.to_string()).with_header(ctype("application/json")).with_header(nocache.clone());
            let mut body = String::new();
            if *req.method() == tiny_http::Method::Post {
                let _ = req.as_reader().read_to_string(&mut body);
            }
            // the frame index for ?t=
            let frame_at = |l: &Live| {
                let t: f64 = q("t").and_then(|v| v.parse().ok()).unwrap_or(0.);
                ((t * l.job.fps).round().max(0.) as usize).min(l.job.frames().saturating_sub(1))
            };
            // a job file (the moment page's stills, clips, takes, the files panel): streamed, with byte ranges
            if let Some(rel) = path.strip_prefix("/job/") {
                let dir = live.lock().unwrap().job.dir.clone();
                let resp = serve_job_file(&dir, &pct(rel), &req);
                let _ = req.respond(resp);
                return;
            }
            let resp = match path {
                "/" | "/editor" => {
                    let (f, built) = if path == "/" { ("live.html", PAGE) } else { ("editor.html", EDITOR) };
                    let page = std::fs::read_to_string(engine_dir().join("js").join(f)).unwrap_or_else(|_| built.to_string());
                    tiny_http::Response::from_string(page).with_header(ctype("text/html; charset=utf-8")).with_header(nocache.clone())
                }
                "/moments" => {
                    let (dir, cfg) = { let l = live.lock().unwrap(); (l.job.dir.clone(), l.job.cfg.clone()) };
                    let mut v = moments::list(&dir, &cfg);
                    let l = live.lock().unwrap();
                    v["name"] = json!(l.job.name);
                    v["error"] = json!(l.error);
                    v["edit"] = json!(cfg["edit"].is_object());
                    json_resp(v)
                }
                "/files" => json_resp(moments::tree(&live.lock().unwrap().job.dir)),
                "/status" => {
                    let dir = live.lock().unwrap().job.dir.clone();
                    json_resp(if *req.method() == tiny_http::Method::Post {
                        let v: Value = serde_json::from_str(&body).unwrap_or_default();
                        moments::set_status(&dir, v["stage"].as_str().unwrap_or(""), v["state"].as_str().unwrap_or(""), chrono_now().get(..10).unwrap_or(""))
                            .unwrap_or_else(|e| json!({ "error": format!("{e:#}") }))
                    } else {
                        moments::status(&dir)
                    })
                }
                "/info" => {
                    let l = live.lock().unwrap();
                    let c = &l.job.cfg;
                    json_resp(json!({
                        "name": l.job.name, "dur": l.job.dur, "fps": l.job.fps, "world": [l.job.world.0, l.job.world.1],
                        "words": c["words"], "beats": l.info["beats"], "raw": c["beats"], "captions": l.info["captions"], "layout": l.info["layout"],
                        "sound": { "song": c["song"], "sfx": c["sfx"], "beat_sfx": l.info["sfx"], "sfx_bed": c["sfx_bed"], "mute": c["mute"] },
                        "version": l.version, "error": l.error,
                        "edit": if c["edit"].is_object() { json!({ "lines": c["edit"]["lines"], "segs": c["edit"]["segs"], "styles": c["edit"]["styles"] }) } else { Value::Null },
                    }))
                }
                "/version" => {
                    let l = live.lock().unwrap();
                    let notes = mtime(&l.job.dir.join("notes.json")).and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0);
                    let seek: Value = std::fs::read_to_string(cache_dir(&l.job.dir).join("seek.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
                    json_resp(json!({"version": l.version, "error": l.error, "drafted": l.drafted,
                        "draft_busy": l.draft_busy || (l.drafted < l.version && !l.draft_held), "draft_held": l.draft_held, "draft_msg": l.draft_msg, "notes": notes, "seek": seek,
                        "queue": l.queue_msg, "made": l.made}))
                }
                "/frame" => {
                    let (f, err0) = { let l = live.lock().unwrap(); (frame_at(&l), l.error.clone()) };
                    let mut out = None;
                    let mut err = None;
                    if let Some(p) = pool.lock().unwrap().as_ref() {
                        let r = p.render(&[f], |_| Want::Jpeg, |_, b| { out = Some(b); Ok(()) }, false);
                        if let Err(e) = r { err = Some(format!("{e:#}")); }
                    }
                    match out {
                        Some(b) => tiny_http::Response::from_data(b).with_header(ctype("image/jpeg")).with_header(nocache.clone()),
                        None => tiny_http::Response::from_string(err.or(err0).unwrap_or_else(|| "no frame".into())).with_status_code(500),
                    }
                }
                "/lift" => {
                    let (f, k) = { let l = live.lock().unwrap(); (frame_at(&l), l.settings.k.max(1e-6)) };
                    let id = q("id").map(|v| pct(&v)).unwrap_or_default();
                    let part = match q("part").as_deref() { Some("sprite") => crate::canvas::Lift::Solo, Some("top") => crate::canvas::Lift::Above, _ => crate::canvas::Lift::Below };
                    let solo = part != crate::canvas::Lift::Below;
                    let r = pool.lock().unwrap().as_ref().map(|p| p.lift(f, &id, part));
                    match r {
                        Some(Ok((b, _))) if b.is_empty() => tiny_http::Response::from_data(Vec::new()).with_status_code(204),
                        Some(Ok((b, bx))) => {
                            let wb = format!("{},{},{},{}", bx[0] as f32 / k, bx[1] as f32 / k, bx[2] as f32 / k, bx[3] as f32 / k);
                            tiny_http::Response::from_data(b).with_header(ctype(if solo { "image/png" } else { "image/jpeg" })).with_header(nocache.clone())
                                .with_header(tiny_http::Header::from_bytes(&b"X-Box"[..], wb.as_bytes()).unwrap())
                        }
                        Some(Err(e)) => tiny_http::Response::from_data(format!("{e:#}").into_bytes()).with_status_code(500),
                        None => tiny_http::Response::from_data(b"no pool".to_vec()).with_status_code(500),
                    }
                }
                "/boxes" => {
                    let (f, k) = { let l = live.lock().unwrap(); (frame_at(&l), l.settings.k.max(1e-6)) };
                    let r = pool.lock().unwrap().as_ref().map(|p| p.layout(&[f]));
                    match r {
                        Some(Ok(v)) => {
                            let list: Vec<Value> = v.into_iter().next().map(|(elems, _)| elems).unwrap_or_default().iter().map(|e| {
                                let lab: Value = serde_json::from_str(&e.label).unwrap_or(Value::Null);
                                let id = lab["id"].as_str().map(String::from).unwrap_or_else(|| if e.label.is_empty() { "text".into() } else { e.label.clone() });
                                json!({ "id": id, "kind": lab["kind"], "x": e.b.left / k, "y": e.b.top / k, "w": (e.b.right - e.b.left) / k, "h": (e.b.bottom - e.b.top) / k })
                            }).collect();
                            json_resp(Value::Array(list))
                        }
                        Some(Err(e)) => json_resp(json!({ "error": format!("{e:#}") })),
                        None => json_resp(json!([])),
                    }
                }
                "/peaks" => {
                    let src = q("src").unwrap_or_else(|| "voice".into());
                    let file = {
                        let l = live.lock().unwrap();
                        let c = &l.job.cfg;
                        match src.as_str() {
                            "mix" => Some(live_draft(&l.job.dir)),
                            _ => c["voice"].as_str().map(|v| job_file(&l.job, v)).or_else(|| c["clipInfo"]["path"].as_str().map(PathBuf::from)),
                        }
                    };
                    match file.filter(|p| p.exists()) {
                        None => tiny_http::Response::from_string("no sound").with_status_code(404),
                        Some(p) => {
                            let m = mtime(&p);
                            let hit = waves.lock().unwrap().get(&src).filter(|(q, t, _)| *q == p && *t == m).map(|x| x.2.clone());
                            let data = match hit {
                                Some(d) => Ok(d),
                                None => peaks(&p).map(|d| {
                                    let d = Arc::new(d);
                                    waves.lock().unwrap().insert(src.clone(), (p.clone(), m, d.clone()));
                                    d
                                }),
                            };
                            match data {
                                Ok(d) => tiny_http::Response::from_data(d.as_ref().clone()).with_header(ctype("application/octet-stream")).with_header(nocache.clone()),
                                Err(e) => tiny_http::Response::from_string(format!("{e:#}")).with_status_code(500),
                            }
                        }
                    }
                }
                "/undo" | "/redo" => {
                    let _f = files.lock().unwrap();
                    let name = live.lock().unwrap().job.name.clone();
                    let r = Job::load(&name, &layout).and_then(|j| {
                        if crate::editfile::path(&j.dir).exists() { crate::editfile::undo_line(&j.dir, path == "/redo", &layout) }   // an edit-file job
                        else { Err(anyhow::anyhow!("undo: this job has no edit.txt")) }
                    });
                    json_resp(match r {
                        Ok(n) => {
                            let mut m = serde_json::Map::new();
                            m.insert(if path == "/undo" { "undone" } else { "redone" }.into(), json!(n));
                            Value::Object(m)
                        }
                        Err(e) => json!({ "error": format!("{e:#}") }),
                    })
                }
                "/draft" if q("review").is_some() => {
                    // a draft for review (the clips + draft tab): review/NNN_draft.mp4
                    let name = live.lock().unwrap().job.name.clone();
                    let _g = draft_lock.lock().unwrap();
                    let txt = match std::process::Command::new(aios_exe()).args(["draft", &name]).output() {
                        Ok(o) => format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)),
                        Err(e) => format!("{e}"),
                    };
                    tiny_http::Response::from_string(txt).with_header(ctype("text/plain; charset=utf-8")).with_header(nocache.clone())
                }
                "/draft" => {
                    let (name, dir, v) = { let l = live.lock().unwrap(); (l.job.name.clone(), l.job.dir.clone(), l.version) };
                    let txt = run_draft(&name, &dir, &draft_lock);
                    if txt.contains("wrote ") { let mut l = live.lock().unwrap(); l.drafted = l.drafted.max(v); }
                    tiny_http::Response::from_string(txt).with_header(ctype("text/plain; charset=utf-8")).with_header(nocache.clone())
                }
                "/notes" | "/notes/del" => {
                    let _f = files.lock().unwrap();
                    let (dir, fps, frames) = { let l = live.lock().unwrap(); (l.job.dir.clone(), l.job.fps, l.job.frames()) };
                    let file = dir.join("notes.json");
                    let mut notes: Vec<Value> = std::fs::read_to_string(&file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
                    if *req.method() == tiny_http::Method::Post {
                        let v: Value = serde_json::from_str(&body).unwrap_or_default();
                        if path == "/notes/del" {
                            let before = notes.len();
                            notes.retain(|n| !(n["id"] == v["id"] && n["status"] == "new"));
                            if notes.len() < before {
                                let _ = std::fs::remove_file(dir.join("notes").join(format!("{}.jpg", v["id"].as_u64().unwrap_or(0))));
                            }
                        } else if let Some(text) = v["text"].as_str().map(str::trim).filter(|x| !x.is_empty()) {
                            let id = notes.iter().filter_map(|n| n["id"].as_u64()).max().unwrap_or(0) + 1;
                            let t = (v["t"].as_f64().unwrap_or(0.) * 100.).round() / 100.;
                            // the frame the note was written on, so the agent sees what the user saw
                            let f = ((t * fps).round() as usize).min(frames.saturating_sub(1));
                            let shot = dir.join("notes").join(format!("{id}.jpg"));
                            if let Some(p) = pool.lock().unwrap().as_ref() {
                                let _ = std::fs::create_dir_all(dir.join("notes"));
                                let _ = p.render(&[f], |_| Want::Jpeg, |_, b| { let _ = std::fs::write(&shot, b); Ok(()) }, false);
                            }
                            let mut n = json!({
                                "id": id, "t": t, "word": v["word"], "text": text, "status": "new",
                                "frame": format!("notes/{id}.jpg"), "added": chrono_now(),
                            });
                            // a note on a span (shift-drag on the timeline): it ends at t2
                            if let Some(t2) = v["t2"].as_f64().filter(|t2| *t2 > t + 0.01) {
                                n["t2"] = json!((t2 * 100.).round() / 100.);
                            }
                            notes.push(n);
                        }
                        let _ = std::fs::write(&file, serde_json::to_string_pretty(&notes).unwrap());
                    }
                    json_resp(Value::Array(notes))
                }
                "/line" | "/lineset" | "/enote" | "/enote/undo" | "/enote/redo" => {
                    let _f = files.lock().unwrap();
                    let (dir, fps, frames, world) = { let l = live.lock().unwrap(); (l.job.dir.clone(), l.job.fps, l.job.frames(), l.job.world) };
                    let v: Value = serde_json::from_str(&body).unwrap_or_default();
                    let id = v["id"].as_str().unwrap_or("").to_string();
                    let r: anyhow::Result<Value> = match path {
                        "/line" => crate::editfile::toggle(&dir, v["n"].as_u64().unwrap_or(0) as usize, v["raw"].as_str().unwrap_or(""), v["off"].as_bool().unwrap_or(false), &layout),
                        "/lineset" => match v["set"].as_object() {
                            Some(set) => crate::editfile::set_line(&dir, v["n"].as_u64().unwrap_or(0) as usize, v["raw"].as_str().unwrap_or(""), set, &layout),
                            None => Err(anyhow::anyhow!("lineset needs {{n, raw, set}}")),
                        },
                        "/enote/undo" => crate::editfile::undo_note(&dir, &id, &layout),
                        "/enote/redo" => crate::editfile::redo_note(&dir, &id, &layout),
                        _ => {
                            let t = v["t"].as_f64().unwrap_or(0.);
                            let covers: Vec<String> = v["covers"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
                            match v["text"].as_str().map(str::trim).filter(|x| !x.is_empty()) {
                                None => Err(anyhow::anyhow!("an empty note")),
                                Some(text) => {
                                    // where it was written on the moment page: moment ID, file, the playhead second in it
                                    let s = |k: &str| v[k].as_str().map(String::from).or_else(|| v[k].as_f64().map(|x| format!("{:.2}", x))).unwrap_or_default();
                                    // the point "this" means: [x, y] as 0-1 of the picture -> world px
                                    let pt = v["pt"].as_array().and_then(|p| Some((p.first()?.as_f64()?, p.get(1)?.as_f64()?)))
                                        .map(|(x, y)| format!("{:.0},{:.0}", x.clamp(0., 1.) * world.0 as f64, y.clamp(0., 1.) * world.1 as f64))
                                        .unwrap_or_default();
                                    let ptby = match (pt.is_empty(), v["ptby"].as_str()) { (true, _) => "", (false, Some("dot")) => "dot", _ => "rest" };   // one of two words, never page text
                                    let at = [("moment", s("moment")), ("file", s("file")), ("sec", s("sec")), ("pt", pt), ("ptby", ptby.to_string())];
                                    crate::editfile::add_note(&dir, t, text, &covers, &at, &layout).map(|id| {
                                    // what the user saw, and what they drew on it
                                    let nd = ai_dir(&dir).join("notes");
                                    let _ = std::fs::create_dir_all(&nd);
                                    let f = ((t * fps).round() as usize).min(frames.saturating_sub(1));
                                    if let Some(p) = pool.lock().unwrap().as_ref() {
                                        let shot = nd.join(format!("{id}.jpg"));
                                        let _ = p.render(&[f], |_| Want::Jpeg, |_, b| { let _ = std::fs::write(&shot, b); Ok(()) }, false);
                                    }
                                    if let Some(png) = v["png"].as_str().and_then(|u| u.split_once(",")).and_then(|(_, b)| unbase64(b)) {
                                        let _ = std::fs::write(nd.join(format!("{id}.draw.png")), png);
                                    }
                                    json!({ "id": id })
                                })
                                }
                            }
                        }
                    };
                    // a frame drag (only nudge keys) that changed the line: the draft waits for Play; any other page
                    // change to a line lets it run again
                    let changed = r.as_ref().ok().and_then(|x| x["line"].as_str()).map_or(false, |x| x.trim() != v["raw"].as_str().unwrap_or("").trim());
                    if changed && (path == "/lineset" || path == "/line") {
                        let moves = path == "/lineset" && v["set"].as_object().map_or(false, |m| !m.is_empty() && m.keys().all(|k| k.starts_with("nudge")));
                        live.lock().unwrap().draft_held = moves;
                    }
                    json_resp(r.unwrap_or_else(|e| json!({ "error": format!("{e:#}") })))
                }
                "/draft/go" => {
                    let mut l = live.lock().unwrap();
                    l.draft_held = false;
                    l.draft_wanted = true;
                    json_resp(json!({ "ok": true }))
                }
                "/draft.mp4" => {
                    let file = { let mut l = live.lock().unwrap(); l.draft_wanted = true; live_draft(&l.job.dir) };
                    match std::fs::read(file) {
                        Ok(b) => ranged(b, "video/mp4", &req).with_header(nocache.clone()),
                        Err(_) => tiny_http::Response::from_string("no draft yet").with_status_code(404),
                    }
                }
                "/library" => json_resp(library(&layout)),
                "/lib" => {
                    let p = pct(&q("p").unwrap_or_default());
                    let ok = !p.contains("..") && !p.starts_with('/') && lib_dirs(&layout).iter().any(|d| p.starts_with(d.as_str()));
                    let ct = match p.rsplit('.').next().unwrap_or("") {
                        "mp4" => "video/mp4", "mp3" => "audio/mpeg", "wav" => "audio/wav", _ => "text/plain; charset=utf-8",
                    };
                    match std::fs::read(layout.root().join(&p)) {
                        Ok(b) if ok => ranged(b, ct, &req),
                        _ => tiny_http::Response::from_string("not in the library").with_status_code(404),
                    }
                }
                _ => tiny_http::Response::from_string("not found").with_status_code(404),
            };
            let _ = req.respond(resp);
        });
    }
    Ok(())
}

/// GET /job/PATH: a file of the job folder, read as it is sent (a take can be large), with byte ranges.
fn serve_job_file(dir: &Path, rel: &str, req: &tiny_http::Request) -> tiny_http::ResponseBox {
    let none = || tiny_http::Response::from_string("not in the job").with_status_code(404).boxed();
    if rel.starts_with('/') || rel.split('/').any(|s| s == "..") {
        return none();
    }
    let p = dir.join(rel);
    let Ok(mut f) = std::fs::File::open(&p) else { return none() };
    let len = f.metadata().map(|m| m.len() as usize).unwrap_or(0);
    let ct = match p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "mp4" | "m4v" => "video/mp4", "mov" => "video/quicktime", "webm" => "video/webm",
        "mp3" => "audio/mpeg", "wav" => "audio/wav", "m4a" => "audio/mp4",
        "jpg" | "jpeg" => "image/jpeg", "png" => "image/png", "webp" => "image/webp", "gif" => "image/gif", "svg" => "image/svg+xml",
        "json" => "application/json", "html" => "text/html; charset=utf-8", "pdf" => "application/pdf",
        _ => "text/plain; charset=utf-8",
    };
    let h = |k: &str, v: &str| tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap();
    let mut hs = vec![h("Content-Type", ct), h("Accept-Ranges", "bytes"), h("Cache-Control", "no-store")];
    match req.headers().iter().find(|x| x.field.equiv("Range")).and_then(|x| byte_range(x.value.as_str(), len)) {
        Some((a, z)) if f.seek(SeekFrom::Start(a as u64)).is_ok() => {
            hs.push(h("Content-Range", &format!("bytes {a}-{z}/{len}")));
            let body: Box<dyn Read + Send> = Box::new(f.take((z - a + 1) as u64));
            tiny_http::Response::new(tiny_http::StatusCode(206), hs, body, Some(z - a + 1), None)
        }
        _ => tiny_http::Response::new(tiny_http::StatusCode(200), hs, Box::new(f) as Box<dyn Read + Send>, Some(len), None),
    }
}

/// A file with byte ranges (the page's <video>/<audio> seek in it).
fn ranged(b: Vec<u8>, ct: &str, req: &tiny_http::Request) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let h = |k: &str, v: &str| tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap();
    match req.headers().iter().find(|x| x.field.equiv("Range")).and_then(|x| byte_range(x.value.as_str(), b.len())) {
        Some((a, z)) => {
            let cr = format!("bytes {a}-{z}/{}", b.len());
            tiny_http::Response::from_data(b[a..=z].to_vec()).with_status_code(206)
                .with_header(h("Content-Range", &cr)).with_header(h("Accept-Ranges", "bytes")).with_header(h("Content-Type", ct))
        }
        None => tiny_http::Response::from_data(b).with_header(h("Accept-Ranges", "bytes")).with_header(h("Content-Type", ct)),
    }
}

/// The folders GET /lib may read from, each as "DIR/" relative to the workspace root (the page's file paths are
/// root-relative). A folder outside the root, or the root itself, is left out: the page never reads the whole root.
fn lib_dirs(layout: &Layout) -> Vec<String> {
    let eng = engine_dir();
    let dirs = [layout.dir(Key::Refs), layout.dir(Key::Sfx), layout.dir(Key::Songs), &eng.join("looks"), &eng.join("js").join("fx")];
    dirs.iter().filter_map(|d| d.strip_prefix(layout.root()).ok())
        .filter(|r| !r.as_os_str().is_empty())
        .map(|r| format!("{}/", r.display())).collect()
}

/// GET /library: what the user can pick on the page (moves, looks, fx, sfx, songs), read from their own files each
/// time, so a new look or song shows at once. Lists only: a pick goes into the note box as text (AI edits first).
fn library(layout: &Layout) -> Value {
    let (root, eng) = (layout.root(), engine_dir());
    let rel = |p: &Path| p.strip_prefix(root).map(|r| r.to_string_lossy().to_string()).unwrap_or_default();
    let (sfx_dir, songs_dir) = (layout.dir(Key::Sfx), layout.dir(Key::Songs));
    let read = |p: PathBuf| -> Value { std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null) };
    let files = |d: PathBuf, ext: &str| {
        let mut v: Vec<PathBuf> = std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == ext)).collect();
        v.sort();
        v
    };
    // the first line of a file, after its "# look NAME:" / "// fx NAME:" head
    let about = |p: &Path, head: &str| {
        let l = std::fs::read_to_string(p).unwrap_or_default().lines().next().unwrap_or("").to_string();
        let name = p.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let l = l.trim_start_matches(['#', '/', ' ']);
        l.strip_prefix(&format!("{head} {name}:")).unwrap_or(l).trim().to_string()
    };
    let looks: Vec<Value> = files(eng.join("looks"), "txt").iter().map(|p| json!({
        "name": p.file_stem().unwrap_or_default().to_string_lossy(), "what": about(p, "look"), "file": rel(p) })).collect();
    let fx: Vec<Value> = files(eng.join("js").join("fx"), "js").iter().map(|p| json!({
        "name": p.file_stem().unwrap_or_default().to_string_lossy(), "what": about(p, "fx"), "file": rel(p) })).collect();
    // sfx: each recipe with the first library file of each of its layers' roles
    let index = read(sfx_dir.join("index.json"));
    let role_file = |r: &str| index.as_array().and_then(|a| a.iter().find(|x| x["role"] == r)).and_then(|x| x["file"].as_str()).map(|f| rel(&sfx_dir.join(f)));
    let book = read(layout.dir(Key::Skills).join("sfx/recipes.json"));
    let sfx: Vec<Value> = book.as_object().into_iter().flatten().filter(|(k, _)| !k.starts_with('_')).map(|(k, v)| {
        let fs: Vec<String> = v["layers"].as_array().into_iter().flatten().filter_map(|l| role_file(l["role"].as_str()?)).collect();
        json!({ "name": k, "what": v["what"], "files": fs })
    }).collect();
    let songs: Vec<Value> = read(songs_dir.join("songs.json"))["songs"].as_object().into_iter().flatten().map(|(id, v)| json!({
        "name": id, "title": v["title"], "mood": v["mood"], "when": v["use_when"], "avoid": v["avoid"],
        "file": v["file"].as_str().map(|f| rel(&songs_dir.join(f))) })).collect();
    json!({ "moves": read(eng.join("js").join("moves.json")), "looks": looks, "fx": fx, "sfx": sfx, "songs": songs })
}

/// `aios seek JOB T`: the live page jumps to second T (and selects beat N with --beat).
pub fn seek(job: &Job, t: f64, beat: Option<usize>) -> Result<()> {
    let id = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let d = cache_dir(&job.dir);
    std::fs::create_dir_all(&d)?;
    std::fs::write(d.join("seek.json"), json!({ "t": t, "beat": beat, "id": id }).to_string())?;
    Ok(())
}

/// base64 (a data URL's body) -> bytes
/// a query value, %-decoded (+ = space)
fn pct(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => match std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(v) => { out.push(v); i += 3; continue; }
                None => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn unbase64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c { b'A'..=b'Z' => c - b'A', b'a'..=b'z' => c - b'a' + 26, b'0'..=b'9' => c - b'0' + 52, b'+' => 62, b'/' => 63, _ => return None } as u32)
    };
    let b: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=').collect();
    let mut out = Vec::with_capacity(b.len() * 3 / 4);
    for ch in b.chunks(4) {
        let mut n = 0u32;
        for (i, c) in ch.iter().enumerate() {
            n |= val(*c)? << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if ch.len() > 2 { out.push((n >> 8) as u8); }
        if ch.len() > 3 { out.push(n as u8); }
    }
    Some(out)
}

/// local time as "YYYY-MM-DD HH:MM" (from `date`; no time crate)
fn chrono_now() -> String {
    std::process::Command::new("date").arg("+%Y-%m-%d %H:%M").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
}
