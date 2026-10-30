//! The moment page's data (engine/js/live.html, FLOW_SPEC steps 3-4): the moments with their stills, clips and
//! notes, the job's files, the stage status, and the clip queue. After an edit, each moment whose frames or sound
//! changed gets a new still (_ai/stills/ID.jpg) and then a new clip (review/NNN_ID_clip.mp4: the moment plus 0.5 s
//! before and after, with sound, 540 px). Both show the moment ID in the top-left corner, so "m07" in the chat
//! matches the page. _ai/clips.json keeps what each moment's newest still and clip were made from.

use crate::job::{ai_dir, cache_dir, engine_dir, review_next};
use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use xxhash_rust::xxh3::Xxh3;

/// a clip starts this much before its moment and ends this much after it
pub const PAD: f64 = 0.5;

/// The stages in order (FLOW_SPEC step 5); `_ai/status` has one line per stage: NAME open|approved DATE
pub const STAGES: &[&str] = &["script", "cleaning", "storyboard", "assets", "clips", "draft", "final"];

/// What the queue works from: one comp version.
pub struct Work {
    pub v: u64,
    pub dir: PathBuf,
    pub cfg: Value,
    pub fps: f64,
    pub frames: usize,
    pub dur: f64,
}

/// What the queue needs from the server: frame hashes and stills from its pool, a clip from `aios draft`.
pub trait Host {
    fn hashes(&self, frames: &[usize]) -> Result<Vec<u64>>;
    fn jpeg(&self, frame: usize) -> Result<Vec<u8>>;
    fn clip(&self, a: f64, b: f64, out: &Path) -> Result<()>;
    /// false when the comp changed after `Work` was taken: the pass stops, the next one starts over
    fn current(&self) -> bool;
    /// the queue's state for the page; `made` = a still or clip was written
    fn say(&self, msg: String, made: bool);
}

/// the moments that play (on, with times), in reel order: (id, t0, t1) in output seconds
pub fn spans(cfg: &Value) -> Vec<(String, f64, f64)> {
    lines(cfg).iter()
        .filter(|l| l["kind"] == "moment" && is_on(l))
        .filter_map(|l| Some((l["id"].as_str()?.to_string(), l["t0"].as_f64()?, l["t1"].as_f64()?)))
        .collect()
}

/// a clip's span: the moment ±PAD inside the reel
pub fn clip_span(dur: f64, t0: f64, t1: f64) -> (f64, f64) {
    ((t0 - PAD).max(0.), (t1 + PAD).min(dur))
}

fn lines(cfg: &Value) -> Vec<Value> {
    cfg["edit"]["lines"].as_array().cloned().unwrap_or_default()
}

fn is_on(l: &Value) -> bool {
    !l["off"].as_bool().unwrap_or(false) && l["mark"].as_str().unwrap_or("").is_empty()
}

pub fn still_file(dir: &Path, id: &str) -> PathBuf {
    ai_dir(dir).join("stills").join(format!("{id}.jpg"))
}

fn state_file(dir: &Path) -> PathBuf {
    ai_dir(dir).join("clips.json")
}

fn read_state(dir: &Path) -> Value {
    std::fs::read_to_string(state_file(dir)).ok().and_then(|s| serde_json::from_str(&s).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}))
}

fn write_state(dir: &Path, st: &Value) -> Result<()> {
    std::fs::write(state_file(dir), serde_json::to_string_pretty(st)?)?;
    Ok(())
}

/// What a moment is made of: (its pixels = the hashes of its frames, the op records, no painting; its clip = the
/// pixels and its sound inputs). A sound change makes a new clip and keeps the still.
fn sig(cfg: &Value, hashes: &[u64], a: f64, b: f64) -> (String, String) {
    let mut h = Xxh3::new();
    for x in hashes {
        h.update(&x.to_le_bytes());
    }
    let pix = format!("{:016x}", h.digest());
    for k in ["voice", "clip", "song", "sfx_bed", "mute", "loudness"] {
        h.update(k.as_bytes());
        h.update(cfg[k].to_string().as_bytes());
    }
    // the placed sounds that land in the span
    for e in cfg["sfx"].as_array().into_iter().flatten() {
        let t = ["t", "at", "start"].iter().find_map(|k| e[*k].as_f64());
        if t.map_or(true, |t| t >= a - 1. && t <= b + 1.) {
            h.update(e.to_string().as_bytes());
        }
    }
    (pix, format!("{:016x}", h.digest()))
}

/// The moment ID in the top-left corner of a still or a clip (Inter, the captions font).
fn tag(src: &Path, dst: &Path, id: &str) -> Result<()> {
    let font = engine_dir().join("fonts/captions/Inter.ttf");
    let draw = format!(
        "drawtext=fontfile='{}':text='{id}':x=14:y=14:fontsize=34:fontcolor=0xF8E479:box=1:boxcolor=black@0.75:boxborderw=8",
        font.to_string_lossy().replace('\'', "")
    );
    let video = src.extension().is_some_and(|e| e == "mp4");
    let tmp = dst.with_extension(if video { "part.mp4" } else { "part.jpg" });
    let mut c = std::process::Command::new("ffmpeg");
    c.args(["-y", "-v", "error", "-i"]).arg(src).args(["-vf", &draw]);
    if video {
        c.args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p", "-c:a", "copy", "-movflags", "+faststart"]);
    } else {
        c.args(["-q:v", "3"]);
    }
    let o = c.arg(&tmp).output()?;
    if !o.status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow!("ffmpeg tag {id}: {}", String::from_utf8_lossy(&o.stderr).trim()));
    }
    std::fs::rename(&tmp, dst)?;
    Ok(())
}

/// One pass of the queue: stills first (cheap, the storyboard needs them), then the clips, each only for a moment
/// whose frames or sound changed. Ok(false) = the comp changed during the pass (the next pass starts over).
pub fn pass(w: &Work, h: &dyn Host) -> Result<bool> {
    let ms = spans(&w.cfg);
    if ms.is_empty() {
        return Ok(true);
    }
    let frame = |s: f64| ((s * w.fps).round().max(0.) as usize).min(w.frames.saturating_sub(1));
    let mut sigs = vec![];
    for (_, t0, t1) in &ms {
        if !h.current() {
            return Ok(false);
        }
        let (a, b) = clip_span(w.dur, *t0, *t1);
        let fr: Vec<usize> = (frame(a)..frame(b).max(frame(a) + 1)).collect();
        sigs.push(sig(&w.cfg, &h.hashes(&fr)?, a, b));
    }
    let mut st = read_state(&w.dir);
    let _ = std::fs::create_dir_all(ai_dir(&w.dir).join("stills"));
    for (i, (id, t0, t1)) in ms.iter().enumerate() {
        let file = still_file(&w.dir, id);
        if st[id]["still"] == json!(sigs[i].0) && file.exists() {
            continue;
        }
        if !h.current() {
            return Ok(false);
        }
        let raw = file.with_extension("raw.jpg");
        std::fs::write(&raw, h.jpeg(frame((t0 + t1) / 2.))?)?;
        let r = tag(&raw, &file, id);
        let _ = std::fs::remove_file(&raw);
        r?;
        entry(&mut st, id).insert("still".into(), json!(sigs[i].0));
        write_state(&w.dir, &st)?;
        h.say(format!("still {id}"), true);
    }
    let review = w.dir.join("review");
    let newest = |st: &Value, id: &str| st[id]["clips"].as_object().and_then(|m| m.keys().max().cloned());
    let todo: Vec<usize> = (0..ms.len())
        .filter(|&i| st[&ms[i].0]["sig"] != json!(sigs[i].1) || newest(&st, &ms[i].0).map_or(true, |f| !review.join(f).exists()))
        .collect();
    for (k, &i) in todo.iter().enumerate() {
        let (id, t0, t1) = &ms[i];
        if !h.current() {
            return Ok(false);
        }
        h.say(format!("clip {id} ({} of {})", k + 1, todo.len()), false);
        let (a, b) = clip_span(w.dur, *t0, *t1);
        let tmp = cache_dir(&w.dir).join("clip_tmp.mp4");
        let _ = std::fs::create_dir_all(cache_dir(&w.dir));
        h.clip(a, b, &tmp)?;
        if !h.current() {
            return Ok(false);   // made from an older version: the next pass makes it again (the chunk cache keeps it fast)
        }
        let _ = std::fs::create_dir_all(&review);
        let out = review_next(&review, &format!("{id}_clip.mp4"));
        tag(&tmp, &out, id)?;
        let _ = std::fs::remove_file(&tmp);
        let name = out.file_name().unwrap().to_string_lossy().to_string();
        let e = entry(&mut st, id);
        e.insert("sig".into(), json!(sigs[i].1));
        let clips = e.entry("clips").or_insert_with(|| json!({}));
        clips[&name] = json!([round2(a), round2(b)]);
        write_state(&w.dir, &st)?;
        h.say(format!("clip {id} done ({} of {})", k + 1, todo.len()), true);
    }
    h.say(format!("clips up to date ({} moments)", ms.len()), false);
    Ok(true)
}

fn entry<'a>(st: &'a mut Value, id: &str) -> &'a mut Map<String, Value> {
    let o = st.as_object_mut().unwrap();
    o.entry(id).or_insert_with(|| json!({}));
    o.get_mut(id).unwrap().as_object_mut().unwrap()
}

fn round2(x: f64) -> f64 {
    (x * 100.).round() / 100.
}

/// The spoken words of a moment line, without the caption marks (| * ^ ~).
fn plain(t: &str) -> String {
    t.replace(['|', '*', '^', '~'], " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// GET /moments: every moment in file order with its words, span, still, clips (newest last, each with the reel
/// span it shows), notes and assets; the stage status; the drafts in review/.
pub fn list(dir: &Path, cfg: &Value) -> Value {
    let ls = lines(cfg);
    let st = read_state(dir);
    let mut names: Vec<String> = std::fs::read_dir(dir.join("review")).into_iter().flatten().flatten()
        .map(|e| e.file_name().to_string_lossy().to_string()).collect();
    names.sort();
    let notes: Vec<&Value> = ls.iter().filter(|l| l["kind"] == "note" && is_on(l)).collect();
    let ms: Vec<&Value> = ls.iter().filter(|l| l["kind"] == "moment" && l["mark"].as_str().unwrap_or("").is_empty()).collect();
    let out: Vec<Value> = ms.iter().map(|m| {
        let id = m["id"].as_str().unwrap_or("");
        let (t0, t1) = (m["t0"].as_f64(), m["t1"].as_f64());
        let inside = |n: &Value| match (n["t0"].as_f64(), t0, t1) { (Some(t), Some(a), Some(b)) => t >= a && t < b, _ => false };
        let mine: Vec<Value> = notes.iter().filter(|n| n["moment"].as_str().map_or_else(|| inside(n), |x| x == id))
            .map(|n| json!({ "id": n["id"], "text": n["text"], "status": n["status"], "reply": n["reply"], "file": n["file"], "sec": n["sec"], "t": n["t0"] })).collect();
        let suffix = format!("_{id}_clip.mp4");
        let clips: Vec<Value> = names.iter().filter(|f| f.ends_with(&suffix) && f[..f.len() - suffix.len()].parse::<u32>().is_ok())
            .map(|f| json!({ "file": format!("review/{f}"), "span": st[id]["clips"][f] })).collect();
        // its broll lines (old files: broll="FILE" on the moment line)
        let broll: Vec<Value> = ls.iter().filter(|l| l["kind"] == "broll" && l["moment"] == id && is_on(l)).filter_map(|l| l["text"].as_str())
            .chain(m["kv"]["broll"].as_str())
            .map(|f| json!({ "file": f, "ok": dir.join(f).exists() || Path::new(f).exists() })).collect();
        let still = still_file(dir, id);
        let stamp = std::fs::metadata(&still).and_then(|x| x.modified()).ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64);
        json!({
            "id": id, "n": m["n"], "off": !is_on(m), "t0": t0, "t1": t1, "words": plain(m["text"].as_str().unwrap_or("")),
            "card": m["kv"]["card"].is_string(), "broll": broll, "notes": mine, "clips": clips,
            "still": stamp.map(|s| format!("/job/{}?v={s}", rel(dir, &still))),
        })
    }).collect();
    let drafts: Vec<String> = names.iter().filter(|f| f.ends_with("_draft.mp4")).map(|f| format!("review/{f}")).collect();
    json!({ "moments": out, "status": status(dir), "drafts": drafts, "pad": PAD, "dur": cfg["dur"] })
}

fn rel(dir: &Path, p: &Path) -> String {
    p.strip_prefix(dir).map(|r| r.to_string_lossy().to_string()).unwrap_or_default()
}

/// `_ai/status` as {stage: {state, date}}
pub fn status(dir: &Path) -> Value {
    let mut m = Map::new();
    for l in std::fs::read_to_string(ai_dir(dir).join("status")).unwrap_or_default().lines() {
        let mut w = l.split_whitespace();
        if let (Some(s), Some(v)) = (w.next(), w.next()) {
            if !s.starts_with('#') {
                m.insert(s.into(), json!({ "state": v, "date": w.next() }));
            }
        }
    }
    Value::Object(m)
}

/// POST /status {stage, state}: one stage's line in `_ai/status` (the page's approve button)
pub fn set_status(dir: &Path, stage: &str, state: &str, date: &str) -> Result<Value> {
    if !STAGES.contains(&stage) || !["open", "approved"].contains(&state) {
        return Err(anyhow!("no stage {stage} / state {state}"));
    }
    let mut cur = status(dir);
    cur[stage] = json!({ "state": state, "date": date });
    let mut txt = String::from("# stage state date (the page's approve button; FLOW_SPEC step 5)\n");
    for s in STAGES {
        if let Some(v) = cur.get(*s) {
            txt.push_str(&format!("{s} {} {}\n", v["state"].as_str().unwrap_or("open"), v["date"].as_str().unwrap_or("")));
        }
    }
    std::fs::write(ai_dir(dir).join("status"), txt)?;
    Ok(status(dir))
}

pub fn approved(dir: &Path, stage: &str) -> bool {
    status(dir)[stage]["state"] == "approved"
}

/// GET /files: the job folder as a tree (the engine's cache and the mattes left out; hidden files too)
pub fn tree(dir: &Path) -> Value {
    json!({ "name": dir.file_name().map(|n| n.to_string_lossy().to_string()), "kids": walk(dir, dir, 0) })
}

fn walk(root: &Path, d: &Path, depth: usize) -> Vec<Value> {
    let mut es: Vec<(bool, String, PathBuf)> = std::fs::read_dir(d).into_iter().flatten().flatten()
        .map(|e| (e.path().is_dir(), e.file_name().to_string_lossy().to_string(), e.path()))
        .filter(|(_, n, _)| !n.starts_with('.') && !n.ends_with(".part.mp4") && !n.ends_with(".tmp"))
        .collect();
    es.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    es.into_iter().filter_map(|(is_dir, name, p)| {
        let r = rel(root, &p);
        if is_dir {
            if depth >= 4 || r == "_ai/cache" || name.starts_with("matte") {   // matte frames: thousands of files, not for the user
                return None;
            }
            Some(json!({ "name": name, "path": r, "kids": walk(root, &p, depth + 1) }))
        } else {
            Some(json!({ "name": name, "path": r, "size": std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) }))
        }
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lines() {
        let d = std::env::temp_dir().join(format!("aios_status_{}", std::process::id()));
        std::fs::create_dir_all(d.join("_ai")).unwrap();
        set_status(&d, "storyboard", "approved", "2026-10-19").unwrap();
        set_status(&d, "clips", "open", "").unwrap();
        assert!(approved(&d, "storyboard") && !approved(&d, "clips"));
        assert!(set_status(&d, "nope", "approved", "").is_err());
        let txt = std::fs::read_to_string(d.join("_ai/status")).unwrap();
        assert!(txt.contains("storyboard approved 2026-10-19\n"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn spans_and_words() {
        let cfg = json!({ "dur": 10.0, "edit": { "lines": [
            { "n": 1, "kind": "moment", "id": "m01", "off": false, "mark": "", "t0": 0.0, "t1": 4.0, "text": "I *learned* | every", "kv": {} },
            { "n": 2, "kind": "moment", "id": "m02", "off": true, "mark": "", "t0": null, "t1": null, "text": "gone", "kv": {} },
            { "n": 3, "kind": "moment", "id": "m03", "off": false, "mark": "", "t0": 4.0, "t1": 10.0, "text": "end", "kv": {} },
            { "n": 4, "kind": "note", "id": "n1", "off": false, "mark": "", "t0": 5.0, "text": "a", "moment": null },
            { "n": 5, "kind": "note", "id": "n2", "off": false, "mark": "", "t0": 5.0, "text": "b", "moment": "m01" },
        ] } });
        let s = spans(&cfg);
        assert_eq!(s.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), ["m01", "m03"]);
        assert_eq!(clip_span(10., 0., 4.), (0., 4.5));
        assert_eq!(clip_span(10., 4., 10.), (3.5, 10.));
        let d = std::env::temp_dir().join(format!("aios_list_{}", std::process::id()));
        std::fs::create_dir_all(d.join("review")).unwrap();
        std::fs::write(d.join("review/003_m01_clip.mp4"), "").unwrap();
        std::fs::write(d.join("review/007_m01_clip.mp4"), "").unwrap();
        std::fs::write(d.join("review/004_m03_clip.mp4"), "").unwrap();
        let v = list(&d, &cfg);
        let m = &v["moments"];
        assert_eq!(m[0]["words"], "I learned every");
        assert_eq!(m[0]["clips"].as_array().unwrap().len(), 2);
        assert_eq!(m[0]["clips"][1]["file"], "review/007_m01_clip.mp4");
        assert_eq!(m[0]["notes"].as_array().unwrap().len(), 1);   // n2 by its moment=
        assert_eq!(m[2]["notes"][0]["id"], "n1");                 // n1 by its time
        assert_eq!(m[1]["off"], true);
        let _ = std::fs::remove_dir_all(d);
    }
}
