//! The chunk cache: a video is kept as 1-second chunks named by what is in them. Before a render every frame is
//! hashed (its op record, cheap: no painting); a chunk whose frames all hash the same as last time is reused as
//! encoded, so after an edit only the seconds that changed are painted and encoded again, and the chunks are
//! joined without re-encoding. The sound mix is cached the same way (by its inputs).

use crate::encode::{EncOpts, Encoder};
use crate::job::Job;
use crate::render::{Pool, Settings, Want};
use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;
use xxhash_rust::xxh3::Xxh3;

pub const CHUNK: usize = 30;

pub struct CacheReport {
    pub frames: usize,
    pub chunks: usize,
    pub reused: usize,
    pub painted: usize,
    pub hash_s: f64,
    pub render_s: f64,
    pub join_s: f64,
}

/// The engine binary's identity: a rebuilt engine never reuses chunks made by an older one.
pub fn exe_id() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| format!("{}:{:?}", m.len(), m.modified().ok()))
        .unwrap_or_default()
}

fn settings_key(s: &Settings, e: &EncOpts) -> u64 {
    let mut h = Xxh3::new();
    h.update(exe_id().as_bytes());
    h.update(format!("{}x{}|{}|{}|{:?}|{}|{}|{}", s.out_w, s.out_h, s.k, s.fps, s.blur.map(|b| (b.shutter, b.gap, b.smax)), e.codec, e.quality, e.preset).as_bytes());
    h.digest()
}

pub fn cache_dir(job: &Job) -> PathBuf {
    let d = crate::job::cache_dir(&job.dir).join("chunks");
    let _ = std::fs::create_dir_all(&d);
    d
}

struct Chunk {
    lo: usize, // positions in `frames`
    hi: usize,
    path: PathBuf,
}

/// Renders `frames` (ascending, contiguous) to `out` through the chunk cache. With `sound`, the join also muxes
/// that file in (one ffmpeg call, one write, one `+faststart`). `sound` is called after the picture is done, so a
/// mix that runs at the same time can finish first.
pub fn video(pool: &Pool, frames: &[usize], enc: &EncOpts, dir: &Path, out: &Path, progress: bool, sound: impl FnOnce() -> Result<Option<PathBuf>>) -> Result<CacheReport> {
    let s = &pool.settings;
    let t0 = Instant::now();
    let hashes = pool.hashes(frames)?;
    let hash_s = t0.elapsed().as_secs_f64();
    let skey = settings_key(s, enc);
    // group by absolute chunk index
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut lo = 0;
    while lo < frames.len() {
        let ci = frames[lo] / CHUNK;
        let mut hi = lo;
        while hi < frames.len() && frames[hi] / CHUNK == ci && (hi == lo || frames[hi] == frames[hi - 1] + 1) {
            hi += 1;
        }
        let mut h = Xxh3::new();
        h.update(&skey.to_le_bytes());
        for k in lo..hi {
            h.update(&(frames[k] as u64).to_le_bytes());
            h.update(&hashes[k].to_le_bytes());
        }
        chunks.push(Chunk { lo, hi, path: dir.join(format!("{:016x}.mp4", h.digest())) });
        lo = hi;
    }
    let missing: Vec<usize> = (0..chunks.len()).filter(|&i| !chunks[i].path.exists()).collect();
    let reused = chunks.len() - missing.len();
    let t1 = Instant::now();
    let mut painted = 0;
    // runs of neighbouring missing chunks share one encoder
    let mut i = 0;
    let mut run_no = 0;
    while i < missing.len() {
        let mut j = i + 1;
        while j < missing.len() && missing[j] == missing[j - 1] + 1 && chunks[missing[j]].lo == chunks[missing[j - 1]].hi {
            j += 1;
        }
        let run = &missing[i..j];
        let (rlo, rhi) = (chunks[run[0]].lo, chunks[*run.last().unwrap()].hi);
        let run_frames = &frames[rlo..rhi];
        let mut splits = Vec::new();
        for &c in &run[1..] {
            splits.push((chunks[c].lo - rlo) as f64 / s.fps - 1e-4);
        }
        let tmp = dir.join(format!("tmp-{}-{run_no}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp)?;
        let mut e = Encoder::segments(&tmp.join("%05d.mp4"), s.out_w, s.out_h, s.fps, enc, &splits)?;
        let r = pool.render(run_frames, |_| Want::Yuv, |_, b| e.write(&b), progress);
        if let Err(x) = r {
            drop(e);
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(x);
        }
        if let Err(x) = e.finish() {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(x);
        }
        for (k, &c) in run.iter().enumerate() {
            let seg = tmp.join(format!("{k:05}.mp4"));
            if !seg.exists() {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(anyhow!("chunk encoder made fewer segments than chunks ({} missing)", seg.display()));
            }
            std::fs::rename(&seg, &chunks[c].path)?;
        }
        let _ = std::fs::remove_dir_all(&tmp);
        painted += rhi - rlo;
        i = j;
        run_no += 1;
    }
    let render_s = t1.elapsed().as_secs_f64();
    // join (+ sound)
    let list = dir.join(format!("list-{}.txt", std::process::id()));
    let mut txt = String::new();
    for c in &chunks {
        txt.push_str(&format!("file '{}'\n", c.path.display()));
    }
    std::fs::write(&list, txt)?;
    let snd = match sound() {
        Ok(x) => x,
        Err(e) => {
            let _ = std::fs::remove_file(&list);
            return Err(e);
        }
    };
    let t2 = Instant::now();
    let mut c = Command::new("ffmpeg");
    c.args(["-v", "error", "-y", "-nostdin", "-f", "concat", "-safe", "0", "-i"]).arg(&list);
    match &snd {
        Some(w) => {
            c.arg("-i").arg(w).args(["-map", "0:v:0", "-map", "1:a:0", "-c:v", "copy"]);
            c.args(if w.extension().map(|e| e == "m4a").unwrap_or(false) { ["-c:a", "copy"] } else { ["-c:a", "aac"] });
            if w.extension().map(|e| e != "m4a").unwrap_or(true) { c.args(["-b:a", "256k"]); }
            c.arg("-shortest");
        }
        None => { c.args(["-c", "copy"]); }
    }
    let o = c.args(["-movflags", "+faststart"]).arg(out).output()?;
    let _ = std::fs::remove_file(&list);
    if !o.status.success() {
        return Err(anyhow!("joining chunks: {}", String::from_utf8_lossy(&o.stderr)));
    }
    Ok(CacheReport { frames: frames.len(), chunks: chunks.len(), reused, painted, hash_s, render_s, join_s: t2.elapsed().as_secs_f64() })
}

/// The cached sound mix for this job and range, or None (then mix and store it here).
pub fn audio_path(job: &Job, t0: f64, dur: f64, extra: &[serde_json::Value]) -> PathBuf {
    let mut h = Xxh3::new();
    h.update(exe_id().as_bytes());
    h.update(serde_json::to_string(extra).unwrap_or_default().as_bytes());
    let c = &job.cfg;
    for k in ["voice", "clip", "song", "sfx", "sfx_bed", "sfx_bed_gain_db", "loudness", "dur", "mute"] {
        h.update(k.as_bytes());
        h.update(c[k].to_string().as_bytes());
    }
    // the files themselves (size + mtime)
    let mut files: Vec<String> = Vec::new();
    for k in ["voice", "clip", "sfx_bed"] {
        if let Some(f) = c[k].as_str() { files.push(f.into()); }
    }
    if let Some(f) = c["song"]["file"].as_str() { files.push(f.into()); }
    if let Some(l) = c["sfx"].as_array() {
        for e in l { if let Some(f) = e["file"].as_str() { files.push(f.into()); } }
    }
    for e in extra { if let Some(f) = e["file"].as_str() { files.push(f.into()); } }
    for f in files {
        // resolved like the mixer does (job dir, AIOS3 root, AIOS2), so a changed library sound re-mixes
        let Some(p) = crate::audio::find(job, &f) else { continue };
        if let Ok(m) = std::fs::metadata(&p) {
            h.update(format!("{f}|{}|{:?}", m.len(), m.modified().ok()).as_bytes());
        }
    }
    h.update(format!("{t0:.6}|{dur:.6}").as_bytes());
    let d = crate::job::cache_dir(&job.dir).join("audio");
    let _ = std::fs::create_dir_all(&d);
    d.join(format!("{:016x}.m4a", h.digest()))
}

/// Removes cache chunks not used for `days` days.
pub fn prune(dir: &Path, days: u64) -> usize {
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if let Ok(m) = e.metadata() {
                let old = m.accessed().or_else(|_| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(|d| d.as_secs() > days * 86400).unwrap_or(false);
                if old && std::fs::remove_file(e.path()).is_ok() {
                    n += 1;
                }
            }
        }
    }
    n
}
