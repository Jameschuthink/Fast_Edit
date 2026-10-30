//! Timing that stays: every command appends one line to `logs/perf.jsonl` (date, commit, command, wall time, what
//! the command noted, and the wait counters below), so speed can be followed across weeks of work.
//! `AIOS_TRACE=1` also keeps every span and writes `logs/trace-CMD-TIME.json` (Chrome trace format: open it at
//! ui.perfetto.dev): one row per thread (render workers, the video decoder, the dispatcher that feeds the encoder),
//! so a wait shows who waited for whom.

use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Counters summed over the whole command (ns or counts). Cheap: one atomic add each.
pub enum C {
    /// render workers waiting for a decoded video frame
    VideoWait,
    /// the decoder thread reading frames from ffmpeg
    VideoRead,
    /// decoder restarts (a seek: a new ffmpeg, ~0.1-0.7 s)
    VideoSeeks,
    /// the dispatcher waiting for any worker to finish a frame
    DispatchIdle,
    /// the dispatcher handing finished frames on (the encoder pipe write)
    Consume,
}
const NC: usize = 5;
const NAMES: [&str; NC] = ["video_wait_ms", "video_read_ms", "video_seeks", "dispatch_idle_ms", "consume_ms"];
static COUNT: [AtomicU64; NC] = [const { AtomicU64::new(0) }; NC];

pub fn add(c: C, v: u64) {
    COUNT[c as usize].fetch_add(v, Relaxed);
}

pub fn add_since(c: C, t: Instant) {
    add(c, t.elapsed().as_nanos() as u64);
}

/// Starts the clock (call first thing in main, so wall_s is the whole command).
pub fn start() -> Instant {
    static T0: OnceLock<Instant> = OnceLock::new();
    *T0.get_or_init(Instant::now)
}

/// Fields a command adds to its perf line (frames, cache use, worker ms…).
static NOTES: Mutex<Option<Map<String, Value>>> = Mutex::new(None);

pub fn note(k: &str, v: impl Into<Value>) {
    NOTES.lock().unwrap().get_or_insert_with(Map::new).insert(k.into(), v.into());
}

pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("AIOS_TRACE").map(|v| !v.is_empty() && v != "0").unwrap_or(false))
}

struct Ev {
    cat: &'static str,
    name: String,
    ts: u64, // µs from start
    dur: Option<u64>,
    tid: u64,
}
static EVENTS: Mutex<Vec<Ev>> = Mutex::new(Vec::new());
static THREADS: Mutex<Vec<(u64, String)>> = Mutex::new(Vec::new());

fn tid() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    thread_local!(static ID: u64 = {
        let id = NEXT.fetch_add(1, Relaxed);
        let name = std::thread::current().name().unwrap_or("main").to_string();
        THREADS.lock().unwrap().push((id, format!("{name} {id}")));
        id
    });
    ID.with(|i| *i)
}

fn us(t: Instant) -> u64 {
    t.saturating_duration_since(start()).as_micros() as u64
}

/// A span from `t` to now (only kept with AIOS_TRACE). `name` is built only then.
pub fn span(cat: &'static str, t: Instant, name: impl FnOnce() -> String) {
    if !on() { return; }
    let ts = us(t);
    let ev = Ev { cat, name: name(), ts, dur: Some(us(Instant::now()).saturating_sub(ts)), tid: tid() };
    EVENTS.lock().unwrap().push(ev);
}

/// A moment (a seek, a cache decision).
pub fn mark(cat: &'static str, name: impl FnOnce() -> String) {
    if !on() { return; }
    let ev = Ev { cat, name: name(), ts: us(Instant::now()), dur: None, tid: tid() };
    EVENTS.lock().unwrap().push(ev);
}

/// Called once at exit: the perf line, and the trace file with AIOS_TRACE. Never fails the command.
/// `cmd`: the subcommand clap parsed (the first word may be a global flag such as `--root DIR`).
pub fn finish(ok: bool, cmd: &str, layout: &crate::layout::Layout) {
    if matches!(cmd, "serve" | "perf" | "config" | "") { return; }
    // the words after the subcommand
    let args: Vec<String> = std::env::args().skip(1).skip_while(|a| a != cmd).skip(1).collect();
    let root = layout.root();
    let dir = layout.dir(crate::layout::Key::Logs);
    let _ = std::fs::create_dir_all(dir);
    let commit = std::process::Command::new("git").args(["-C"]).arg(root).args(["rev-parse", "--short", "HEAD"]).output().ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let mut line = json!({
        "date": now_stamp(),
        "commit": commit,
        "cmd": cmd,
        "args": args.join(" "),
        "ok": ok,
        "wall_s": (start().elapsed().as_secs_f64() * 1000.).round() / 1000.,
    });
    for (i, n) in NAMES.iter().enumerate() {
        let v = COUNT[i].load(Relaxed);
        if v == 0 { continue; }
        line[*n] = if n.ends_with("_ms") { json!((v as f64 / 1e6).round()) } else { json!(v) };
    }
    if let Some(m) = NOTES.lock().unwrap().take() {
        for (k, v) in m { line[k] = v; }
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("perf.jsonl")) {
        let _ = writeln!(f, "{line}");
    }
    if !on() { return; }
    let evs = std::mem::take(&mut *EVENTS.lock().unwrap());
    let mut out: Vec<Value> = THREADS.lock().unwrap().iter()
        .map(|(id, n)| json!({"ph": "M", "name": "thread_name", "pid": 1, "tid": id, "args": {"name": n}}))
        .collect();
    out.extend(evs.into_iter().map(|e| match e.dur {
        Some(d) => json!({"ph": "X", "cat": e.cat, "name": e.name, "ts": e.ts, "dur": d, "pid": 1, "tid": e.tid}),
        None => json!({"ph": "i", "s": "t", "cat": e.cat, "name": e.name, "ts": e.ts, "pid": 1, "tid": e.tid}),
    }));
    let p = dir.join(format!("trace-{cmd}-{}.json", now_stamp().replace([':', ' '], "-")));
    if std::fs::write(&p, json!({"traceEvents": out}).to_string()).is_ok() {
        eprintln!("trace: {} (open at ui.perfetto.dev)", p.display());
    }
}

/// Local time "YYYY-MM-DD HH:MM:SS".
fn now_stamp() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as libc::time_t;
    // SAFETY: localtime_r writes only into `tm`
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
}
