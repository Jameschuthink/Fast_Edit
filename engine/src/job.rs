//! A job folder: jobs/NAME/job.json + comp.js (+ assets). job.json is the one place for size, fps, length, the
//! clip, the words, the song and the fonts; everything in it reaches the composition as the global `JOB`.
//!
//! {
//!   "comp": "comp.js",                 composition script(s), relative to the job folder
//!   "lib": ["motion", "look"],         engine libraries loaded first (engine/js/lib/NAME.js)
//!   "size": [1080, 1920], "fps": 30,   world size (what the comp draws in) and frame rate
//!   "dur": 42.7,                       seconds (default: the clip's length)
//!   "clip": "cut.mp4",                 the main video (optional); JOB.clip = {path, w, h, fps, dur}
//!   "words": "cut.words.json",         word times (optional); JOB.words = [{word, start, end}, …]
//!   "fonts": {"Family": "fonts/x.ttf" | {"file": …, "weight": 700, "pins": {"slnt": 0}}},
//!   "song": {…}, "sfx": […]            sound (see audio.rs)
//! }

use crate::js::CompSrc;
use crate::layout::{Key, Layout};
use crate::text::register_font;
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub fn engine_dir() -> PathBuf {
    std::env::var("AIOS_ENGINE").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

pub struct Job {
    pub name: String,
    pub dir: PathBuf,
    pub cfg: Value,
    pub sources: std::collections::HashMap<String, PathBuf>,
    pub world: (u32, u32),
    pub fps: f64,
    pub dur: f64,
    pub src: CompSrc,
    /// the workspace folders (aios.toml); a clone of the one `main` read, so a job needs no other handle
    pub layout: Layout,
}

/// NAME in the `jobs` folder, a path to a job folder, or a path to a .js comp (a job without job.json).
pub fn resolve_job(arg: &str, layout: &Layout) -> Result<PathBuf> {
    let p = PathBuf::from(arg);
    let is_job = |d: &Path| d.join("job.json").exists() || d.join(crate::editfile::FILE).exists();
    if is_job(&p) || p.extension().map(|e| e == "js").unwrap_or(false) && p.exists() {
        return Ok(std::fs::canonicalize(&p)?);
    }
    let j = layout.dir(Key::Jobs).join(arg);
    if is_job(&j) {
        return Ok(j);
    }
    Err(anyhow!("no job '{arg}' (looked for {arg}/job.json or edit.txt and {}/job.json; aios.toml key jobs)", j.display()))
}

impl Job {
    pub fn load(arg: &str, layout: &Layout) -> Result<Job> {
        let path = resolve_job(arg, layout)?;
        let mut edit_watch = vec![];
        let (dir, mut cfg) = if path.extension().map(|e| e == "js").unwrap_or(false) {
            (path.parent().unwrap().to_path_buf(), json!({ "comp": path.file_name().unwrap().to_string_lossy() }))
        } else if !path.join("job.json").exists() {
            // a job made from one edit file (editfile.rs): compiled into the same cfg
            let txt = crate::editfile::read(&path)?;
            let c = crate::editfile::compile(&path, &txt, true, layout)?;
            edit_watch = c.watch;
            (path.clone(), c.cfg)
        } else {
            let txt = std::fs::read_to_string(path.join("job.json")).with_context(|| format!("{}/job.json", path.display()))?;
            let v: Value = serde_json::from_str(&txt).map_err(|e| anyhow!("{}/job.json: {e}", path.display()))?;
            (path.clone(), v)
        };
        let name = dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let rel = |p: &str| -> PathBuf { let pb = PathBuf::from(p); if pb.is_absolute() { pb } else { dir.join(pb) } };
        let size = cfg["size"].as_array().filter(|a| a.len() >= 2).map(|a| (a[0].as_u64().unwrap_or(1080) as u32, a[1].as_u64().unwrap_or(1920) as u32)).unwrap_or((1080, 1920));
        let fps = cfg["fps"].as_f64().unwrap_or(30.);
        // the clip
        let mut clip_info = Value::Null;
        if let Some(c) = cfg["clip"].as_str() {
            let cp = rel(c);
            if cp.exists() {
                let p = crate::video::probe(&cp.to_string_lossy())?;
                clip_info = json!({ "path": cp.to_string_lossy(), "w": p.width, "h": p.height, "fps": p.fps, "dur": p.duration });
            } else {
                return Err(anyhow!("job.json clip not found: {}", cp.display()));
            }
        }
        let dur = cfg["dur"].as_f64().or_else(|| clip_info["dur"].as_f64()).unwrap_or(10.);
        // words
        let mut words = Value::Array(vec![]);
        match &cfg["words"] {
            Value::String(w) => {
                let wp = rel(w);
                let v: Value = serde_json::from_str(&std::fs::read_to_string(&wp).with_context(|| format!("words {}", wp.display()))?)?;
                words = if v.is_array() { v } else { v["words"].clone() };
            }
            Value::Array(a) => words = Value::Array(a.clone()),
            _ => {}
        }
        // data files named in job.json are inlined, so the comp reads them as JOB.beats, JOB.captions, JOB.data.x
        let mut sources: std::collections::HashMap<String, PathBuf> = edit_watch.into_iter().collect();
        if let Value::String(w) = &cfg["words"] {
            sources.insert("words".to_string(), rel(w));
        }
        for key in ["beats", "captions", "faces", "poster"] {
            if let Value::String(f) = &cfg[key] {
                let fp = rel(f);
                sources.insert(key.to_string(), fp.clone());
                cfg[key] = read_json(&fp, &format!("job.json {key}"))?;
            }
        }
        if let Some(m) = cfg["data"].as_object().cloned() {
            let mut out = serde_json::Map::new();
            for (k, v) in m {
                if let Value::String(f) = &v {
                    if f.ends_with(".json") {
                        let fp = rel(f);
                        out.insert(k.clone(), read_json(&fp, &format!("job.json data.{k}"))?);
                        continue;
                    }
                }
                out.insert(k, v);
            }
            cfg["data"] = Value::Object(out);
        }
        // fonts
        if let Some(m) = cfg["fonts"].as_object() {
            for (fam, spec) in m {
                let (file, weight, italic, pins) = match spec {
                    Value::String(f) => (f.clone(), None, None, vec![]),
                    o => (
                        o["file"].as_str().unwrap_or("").to_string(),
                        o["weight"].as_i64().map(|w| w as i32),
                        o["style"].as_str().map(|s| s == "italic"),
                        o["pins"].as_object().map(|m| m.iter().filter_map(|(k, v)| v.as_f64().map(|f| (k.clone(), f as f32))).collect()).unwrap_or_default(),
                    ),
                };
                let fp = if file.starts_with("@engine/") { engine_dir().join(&file[8..]) } else { rel(&file) };
                register_font(fam, &fp.to_string_lossy(), weight, italic, &pins).with_context(|| format!("job.json fonts.{fam}"))?;
            }
        }
        let libs: Vec<String> = match &cfg["lib"] {
            Value::Array(a) => a.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
            Value::String(s) => vec![s.clone()],
            _ => vec!["motion".into()],
        };
        let comps: Vec<PathBuf> = match &cfg["comp"] {
            Value::Array(a) => a.iter().filter_map(|v| v.as_str()).map(rel).collect(),
            Value::String(s) => vec![rel(s)],
            _ => vec![dir.join("comp.js")],
        };
        for c in &comps {
            if !c.exists() {
                return Err(anyhow!("comp not found: {}", c.display()));
            }
        }
        if let Value::Object(m) = &mut cfg {
            m.insert("name".into(), json!(name));
            m.insert("dir".into(), json!(dir.to_string_lossy()));
            m.insert("size".into(), json!([size.0, size.1]));
            m.insert("fps".into(), json!(fps));
            m.insert("dur".into(), json!(dur));
            m.insert("words".into(), words);
            m.insert("clipInfo".into(), clip_info);
            m.insert("engine".into(), json!(engine_dir().to_string_lossy()));
            m.insert("root".into(), json!(layout.root().to_string_lossy()));
        }
        let src = CompSrc {
            engine_js: engine_dir().join("js"),
            libs,
            job_json: serde_json::to_string(&cfg)?,
            files: comps,
            world: size,
            fps,
        };
        Ok(Job { name, dir, cfg, sources, world: size, fps, dur, src, layout: layout.clone() })
    }

    /// The file a job.json data key came from (beats, captions, words), for watching.
    pub fn cfg_source(&self, key: &str) -> Option<PathBuf> {
        self.sources.get(key).cloned()
    }

    pub fn frames(&self) -> usize {
        ((self.dur * self.fps).round() as usize).max(1)
    }
}

/// The job's own review folder (jobs/JOB/review): what the user reviews sits with the job (user, 2026-10-19).
pub fn review_dir(job: &Job) -> PathBuf {
    let d = job.dir.join("review");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// A new file in review/ under the next run number: review/004_draft.mp4 (FLOW_SPEC step 2). A number is never used
/// twice, even after its file is deleted (review/.last keeps the last one), and no file is written over, so the newest
/// is always last in a listing.
pub fn review_new(job: &Job, name: &str) -> PathBuf {
    review_next(&review_dir(job), name)
}

pub fn review_next(d: &Path, name: &str) -> PathBuf {
    let last = std::fs::read_to_string(d.join(".last")).ok().and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(0);
    let used = std::fs::read_dir(d).into_iter().flatten().flatten()
        .filter_map(|e| e.file_name().to_string_lossy().split_once('_').and_then(|(n, _)| n.parse::<u32>().ok())).max().unwrap_or(0);
    let n = last.max(used) + 1;
    let _ = std::fs::write(d.join(".last"), n.to_string());
    d.join(format!("{n:03}_{name}"))
}

/// A section clip under the next run number, named by its span (review/005_clip_7-12.5.mp4)
pub fn clip_path(job: &Job, a: f64, b: f64) -> PathBuf {
    let n = |x: f64| { let s = format!("{x:.2}"); s.trim_end_matches('0').trim_end_matches('.').to_string() };
    review_new(job, &format!("clip_{}-{}.mp4", n(a), n(b)))
}

/// A final's default file, jobs/JOB/final/DATE_JOB.EXT: the published file stays with its job (icm K52, 2026-10-28).
/// The writer makes the folder and reports a failure (`with_sound`, the export arm), so this only names the file.
pub fn final_path(job: &Job, date: &str, ext: &str) -> PathBuf {
    job.dir.join("final").join(format!("{date}_{}.{ext}", job.name))
}

/// The agent's folder of a job: jobs/JOB/_ai (notes.txt, notes/, words/, faces/, matte/, fx/, cache/; FLOW_SPEC
/// step 2). An older job has none: it keeps these in the job folder itself, so it opens as before.
pub fn ai_dir(dir: &Path) -> PathBuf {
    let a = dir.join("_ai");
    if a.is_dir() { a } else { dir.to_path_buf() }
}

/// The job's cache (files the engine can make again): jobs/JOB/_ai/cache, in an older job jobs/JOB/.cache
pub fn cache_dir(dir: &Path) -> PathBuf {
    let a = dir.join("_ai");
    if a.is_dir() { a.join("cache") } else { dir.join(".cache") }
}

#[allow(dead_code)]
pub fn rel_to(base: &Path, p: &str) -> PathBuf {
    let pb = PathBuf::from(p);
    if pb.is_absolute() { pb } else { base.join(pb) }
}

/// A JSON file named in job.json; errors name the job.json key and the file.
fn read_json(fp: &std::path::Path, what: &str) -> Result<Value> {
    let txt = std::fs::read_to_string(fp).with_context(|| format!("{what}: {}", fp.display()))?;
    serde_json::from_str(&txt).map_err(|e| anyhow!("{}: {e}", fp.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_numbers() {
        let d = std::env::temp_dir().join(format!("aios_review_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(".keep"), "").unwrap();
        let a = review_next(&d, "stills.png");
        assert_eq!(a.file_name().unwrap(), "001_stills.png");
        std::fs::write(&a, "x").unwrap();
        let b = review_next(&d, "draft.mp4");
        assert_eq!(b.file_name().unwrap(), "002_draft.mp4");
        // never reused: 002 was taken (even with no file), a deleted 001 does not come back
        std::fs::remove_file(&a).unwrap();
        assert_eq!(review_next(&d, "draft.mp4").file_name().unwrap(), "003_draft.mp4");
        // a file put in by hand with a higher number: the next one goes after it
        std::fs::write(d.join("010_storyboard.png"), "x").unwrap();
        assert_eq!(review_next(&d, "clip_1-2.mp4").file_name().unwrap(), "011_clip_1-2.mp4");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
