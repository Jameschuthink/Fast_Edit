//! A job as ONE text file, jobs/JOB/edit.txt: the raw clips and one line per thing (user design, 2026-10-12).
//! The engine compiles the file into the same `cfg` a job.json gives, so render, cache, lint, sound and the page
//! work as before; lib `edit` (engine/js/lib/edit.js) draws it.
//!
//!   size   1920x1080 fps=30 caps=1.6                            caps= the captions' size x (all tiers; 1 = v3)
//!   size   1080x1920 loud=-11 tp=-1.5                           the master loudness (LUFS, default -14) and true-peak
//!                                                               ceiling (dBTP) for drafts, the page and the final
//!   font   MontBold "fonts/Montserrat-Bold.ttf"
//!   take   A "raw.mp4" rotate=90 crop=1866x1050+24+16          a take (raw recording) and its prep (cached: .cache/prep)
//!   take   B "take.mp4" voice relight frame=locked              slow work on a take, in the background (.cache/proc):
//!                                                               Studio Voice, Relighting, 9:16 reframe (locked|follow)
//!   take   A "take.mp4" voice relight frame=locked exposure=0.2  exposure= stops, a step after those (brighter/darker)
//!   words  B "B.words.json"                                     the clip's word times (raw seconds; `aios take` makes it)
//!   say    s1 B 0.00-4.80 "Business owner, business owner."     a sentence that plays (like `use`, with its words);
//!                                                               "#" on it = that sentence is cut
//!   style  small font=MontBold size=84 color=#ffffff shadow pop=0.08
//!   style  card font=MontBold size=200 in=0.3 grow=1 from=up bg=#000000   moves in from above, on a black card
//!   use    u1 A 0.00-10.93                                      the part of a clip that plays, in reel order
//!   skip   k1 A 4.10-4.40                                       a part removed from inside a use
//!   text   t1 A 0.88-2.96 "upside" style=small at=560,560       words on screen
//!   note   n1 A 6.20 "smaller" covers=t9 status=new             the user's note; the AI's reply goes on it
//!   pic    p1 A 23.20-24.06 "meme.jpg" fill                     a picture: fill = the whole width on black (a cut-away;
//!                                                               the voice and the captions go on)
//!   fx     x1 A 5.82-6.14 "shake" amp=18                     a named effect (the skill library): engine/js/fx/NAME.js,
//!                                                               or the job's own jobs/JOB/fx/NAME.js (made by a note)
//!   faces  B "B.faces.json"                                     the face per frame of the clip's picture (face_src.py)
//!   cap    c1 B 0.00-4.80 "Business | *owner,* | ^business^"     captions (approved v3, lib captions): "|" splits the
//!                                                               groups, *w* = EMPHASIS, ^w^ = HERO; word times from
//!                                                               the clip's words, placed off the face from its faces
//!   moment m03 "It's finally | my time | where I can *focus*"   a moment by its spoken words (no times): its captions,
//!                                                               cut-away or card come from them (see `scenes` below)
//!     big    g1 "STORYTELLING"                                  a big word on the moment it is under
//!     broll  b1 "media/sea.mp4" from=1                          the moment's cut-away (= broll="…" on the moment line)
//!   frame  f1 window                                            the reel in one rounded window on black, its shape per
//!                                                               moment (win=), morphing; *emph* words big, own line, accent=
//!
//! Moment blocks (FLOW_SPEC step 1): the indented lines under a moment line belong to it (say, skip, big, sfx, broll,
//! text …), so a note on m03 changes only the m03 block. A moment ID never changes when others move; a new moment takes
//! the next free number. The reader still reads the old words `clip` (= take) and `scene` (= moment); the writer
//! writes the new ones.
//!
//! Rules: one line = one thing with a short id. Times are the RAW clip's seconds ("A 0.88-2.96"), so a skip never
//! moves another line. "#" at the start = off. A change by note n3 keeps the old line as "#n3< …"; undo turns the
//! new one into "#n3> …" and the old one back on, in the new one's place. Any other "#" line is off by hand or a
//! comment (a "#" line that does not start with a known kind).
//! The note lines and the old versions live in jobs/JOB/notes.txt, so edit.txt holds only what plays (`read` joins
//! the two, `tidy` splits them; an older job's notes move out on its first read).
//!
//! The job folder (FLOW_SPEC step 2, the `project` template): the takes in media/, what the user reviews in review/ (run
//! numbers), and the agent's files in _ai/: notes.txt, notes/, words/, faces/, matte/, fx/, cache/ (the ".cache" and
//! "jobs/JOB/…" paths below). An older job with no _ai/ keeps them in its own folder and opens as before.
//!
//! Notes: a chat on "check notes" (the page's notes agent was deleted 2026-10-24).
//! `aios note JOB` lists the new notes with the lines near each one, the page second and the words at it, the word
//! boxes, and ONE frame sheet per note (notes/ID.sheet.png: the note's frame on a pixel grid with the drawing, and
//! frames around it); the line keys, FX and looks go to notes/reference.txt (`--full` prints them); the agent looks, then
//! `aios note JOB ID --apply FILE` writes its answer: whole lines by id, `off ID`, `on ID`, `reply: one sentence`.
//! A question for the user goes in the chat, not in the file. Slow work runs by itself: the prep and the cut
//! are ffmpeg files in .cache, the person matte (`behind` on a style) runs nv.py in the background.

use crate::job::{ai_dir, cache_dir};
use crate::layout::{Key, Layout};
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;
use xxhash_rust::xxh3::Xxh3;

pub const FILE: &str = "edit.txt";
const KINDS: &[&str] = &["size", "font", "take", "words", "faces", "style", "use", "say", "skip", "text", "cap", "pic", "fx", "note", "song", "sfx", "moment", "big", "broll", "frame"];
const TOP: &[&str] = &["size", "font", "take", "words", "faces", "style", "song"];
/// the old words the reader still reads (old files): (old, new)
const OLD_WORDS: &[(&str, &str)] = &[("clip", "take"), ("scene", "moment")];
/// a note that names no place gets the lines this far around it (raw seconds)
const NEAR: f64 = 3.0;

#[derive(Clone, Debug, Default)]
pub struct Line {
    pub n: usize,
    pub raw: String,
    pub off: bool,
    /// "" | "n3<" (the line before note n3 changed it) | "n3>" (the change note n3 made, undone)
    pub mark: String,
    pub kind: String,
    pub id: String,
    pub clip: Option<String>,
    pub t0: Option<f64>,
    pub t1: Option<f64>,
    pub text: Option<String>,
    pub kv: Vec<(String, String)>,
    /// the moment block the line is in: (the moment line's n, its id). Set by `parse_all`, for an indented line under a
    /// moment line
    pub moment: Option<(usize, String)>,
}

impl Line {
    pub fn get(&self, k: &str) -> Option<&str> {
        self.kv.iter().find(|(a, _)| a == k).map(|(_, b)| b.as_str())
    }
    fn num(&self, k: &str) -> Option<f64> {
        self.get(k).and_then(|v| v.parse().ok())
    }
    fn set(&mut self, k: &str, v: &str) {
        match self.kv.iter_mut().find(|(a, _)| a == k) {
            Some(p) => p.1 = v.to_string(),
            None => self.kv.push((k.to_string(), v.to_string())),
        }
    }
    /// the line without its "#" and mark
    pub fn body(&self) -> String {
        if !self.off {
            return self.raw.trim().to_string();
        }
        let s = self.raw.trim().trim_start_matches('#').trim_start();
        let s = if !self.mark.is_empty() { s.strip_prefix(self.mark.as_str()).unwrap_or(s).trim_start() } else { s };
        s.to_string()
    }
    fn top(&self) -> bool {
        TOP.contains(&self.kind.as_str())
    }
    /// the note's number (n3 -> 3)
    fn note_no(&self) -> u64 {
        self.id.trim_start_matches('n').parse().unwrap_or(0)
    }
}

/// Words split on spaces; "…" keeps spaces (\" inside). Each token: (text, it started with a quote).
fn tokenize(s: &str) -> Vec<(String, bool)> {
    let mut out = vec![];
    let (mut cur, mut quoted, mut inq, mut started) = (String::new(), false, false, false);
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if inq {
            if c == '\\' && it.peek() == Some(&'"') {
                cur.push('"');
                it.next();
            } else if c == '"' {
                inq = false;
            } else {
                cur.push(c);
            }
        } else if c.is_whitespace() {
            if started {
                out.push((std::mem::take(&mut cur), quoted));
                started = false;
                quoted = false;
            }
        } else if c == '"' {
            if !started {
                quoted = true;
            }
            started = true;
            inq = true;
        } else {
            started = true;
            cur.push(c);
        }
    }
    if started {
        out.push((cur, quoted));
    }
    out
}

fn range(s: &str) -> Option<(f64, Option<f64>)> {
    if let Ok(a) = s.parse::<f64>() {
        return Some((a, None));
    }
    let (a, b) = s.split_once('-')?;
    Some((a.parse().ok()?, Some(b.parse().ok()?)))
}

pub fn parse_line(n: usize, raw: &str) -> Option<Line> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let (off, mark, body) = match s.strip_prefix('#') {
        Some(r) => {
            let r = r.trim_start();
            let first = r.split_whitespace().next().unwrap_or("");
            let is_mark = first.len() >= 3 && first.starts_with('n') && (first.ends_with('<') || first.ends_with('>'))
                && first[1..first.len() - 1].chars().all(|c| c.is_ascii_digit());
            if is_mark { (true, first.to_string(), r[first.len()..].trim_start()) } else { (true, String::new(), r) }
        }
        None => (false, String::new(), s),
    };
    let toks = tokenize(body);
    let (kind, q) = toks.first()?;
    let kind = OLD_WORDS.iter().find(|(old, _)| old == kind).map(|(_, new)| new.to_string()).unwrap_or_else(|| kind.clone());
    if *q || !KINDS.contains(&kind.as_str()) {
        return None; // a comment
    }
    let mut l = Line { n, raw: raw.to_string(), off, mark, kind, ..Default::default() };
    let mut i = 1;
    if let Some((id, false)) = toks.get(i) {
        if !id.contains('=') {
            l.id = id.clone();
            i += 1;
        }
    }
    if l.kind != "take" {
        if let Some((c, false)) = toks.get(i) {
            if c.len() == 1 && c.chars().all(|x| x.is_ascii_uppercase()) {
                l.clip = Some(c.clone());
                i += 1;
            }
        }
    }
    if let Some((r, false)) = toks.get(i) {
        if let Some((a, b)) = range(r) {
            l.t0 = Some(a);
            l.t1 = b;
            i += 1;
        }
    }
    if let Some((t, true)) = toks.get(i) {
        l.text = Some(t.clone());
        i += 1;
    }
    for (t, _) in &toks[i..] {
        match t.split_once('=') {
            Some((k, v)) => l.kv.push((k.to_string(), v.to_string())),
            None => l.kv.push((t.clone(), "1".into())),
        }
    }
    Some(l)
}

/// All lines, with their moment blocks: a `moment` line opens a block, and every indented line after it belongs to it,
/// up to the next line that is not indented (blank lines do not end a block). An old or undone version of a moment line
/// ("#n3< moment …") opens none: the lines under it stay with the version that is on.
pub fn parse_all(text: &str) -> Vec<Line> {
    let mut cur: Option<(usize, String)> = None;
    text.lines().enumerate().filter_map(|(n, r)| {
        let indented = r.starts_with(char::is_whitespace);
        if !indented && !r.trim().is_empty() && !is_old_moment(r) {
            cur = None;
        }
        let mut l = parse_line(n, r)?;
        if l.kind == "moment" && l.mark.is_empty() {
            cur = Some((n, l.id.clone()));
        } else if indented && l.kind != "moment" {
            l.moment = cur.clone();
        }
        Some(l)
    }).collect()
}

fn is_old_moment(r: &str) -> bool {
    parse_line(0, r).is_some_and(|l| l.kind == "moment" && !l.mark.is_empty())
}

/// the spaces in front of a row
fn indent_of(row: &str) -> &str {
    &row[..row.len() - row.trim_start().len()]
}

/// a new row in the place of `old`, with old's indent (so a line in a moment block stays in it)
fn keep_indent(old: &str, new: &str) -> String {
    format!("{}{}", indent_of(old), new.trim_start())
}

/// the indent for a line put in at `at`: the block's when the row before it is a moment line or in a block
fn indent_at(rows: &[String], at: usize, l: &Line) -> String {
    let Some(prev) = at.checked_sub(1).and_then(|i| rows.get(i)) else { return String::new() };
    if l.top() || l.kind == "moment" || l.kind == "note" {
        return String::new();
    }
    match parse_line(0, prev) {
        Some(p) if p.kind == "moment" && indent_of(prev).is_empty() => "  ".into(),
        _ if !prev.trim().is_empty() => indent_of(prev).to_string(),
        _ => String::new(),
    }
}

fn quote(v: &str) -> String {
    if v.is_empty() || v.chars().any(|c| c.is_whitespace() || c == '"' || c == '=') {
        format!("\"{}\"", v.replace('"', "\\\""))
    } else {
        v.to_string()
    }
}

fn fmt_time(t: f64) -> String {
    format!("{:.2}", t)
}

/// a line written back from its fields (used for notes, whose status and reply change)
fn fmt_line(l: &Line) -> String {
    let mut s = String::new();
    if l.off {
        s.push('#');
        if !l.mark.is_empty() {
            s.push_str(&l.mark);
        }
        s.push(' ');
    }
    s.push_str(&format!("{:<5} {}", l.kind, l.id));
    if let Some(c) = &l.clip {
        s.push_str(&format!(" {c}"));
    }
    if let Some(a) = l.t0 {
        s.push(' ');
        s.push_str(&fmt_time(a));
        if let Some(b) = l.t1 {
            s.push('-');
            s.push_str(&fmt_time(b));
        }
    }
    if let Some(t) = &l.text {
        s.push_str(&format!(" \"{}\"", t.replace('"', "\\\"")));
    }
    for (k, v) in &l.kv {
        if v == "1" && ["shadow", "behind", "fill", "voice", "relight"].contains(&k.as_str()) {   // flags
            s.push_str(&format!(" {k}"));
        } else {
            s.push_str(&format!(" {k}={}", quote(v)));
        }
    }
    s
}

// ───────────── compile ─────────────

struct Clip {
    path: PathBuf,
    dur: f64,
    rotate: i64,
    crop: Option<(u32, u32, u32, u32)>,
    audio: bool,
    voice: bool,
    relight: bool,
    /// "locked" | "follow": reframe to the job's shape (reframe skill, verthor)
    frame: Option<String>,
    /// exposure=STOPS: a brightness step after relight and reframe (ffmpeg-skill color.py --correct --exposure)
    exposure: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct Seg {
    pub clip: String,
    pub a: f64,
    pub b: f64,
    pub o: f64,
}

/// raw (clip, t) -> output seconds. A time inside a skip goes to the next kept second (a start) or the last one (an end).
fn to_out(segs: &[Seg], clip: &str, t: f64, end: bool) -> Option<f64> {
    const E: f64 = 1e-6;
    for s in segs.iter().filter(|s| s.clip == clip) {
        if t >= s.a - E && t <= s.b + E {
            return Some(s.o + (t - s.a).clamp(0., s.b - s.a));
        }
    }
    if end {
        segs.iter().filter(|s| s.clip == clip && s.b <= t).last().map(|s| s.o + (s.b - s.a))
    } else {
        segs.iter().find(|s| s.clip == clip && s.a >= t).map(|s| s.o)
    }
}

/// output seconds -> raw (clip, t)
fn to_raw(segs: &[Seg], t: f64) -> Option<(String, f64)> {
    let mut last = None;
    for s in segs {
        if t >= s.o && t < s.o + (s.b - s.a) {
            return Some((s.clip.clone(), s.a + t - s.o));
        }
        last = Some((s.clip.clone(), s.b));
    }
    last
}

fn crop_of(s: &str) -> Option<(u32, u32, u32, u32)> {
    // WxH+X+Y
    let (wh, xy) = s.split_once('+')?;
    let (w, h) = wh.split_once('x')?;
    let (x, y) = xy.split_once('+')?;
    Some((w.parse().ok()?, h.parse().ok()?, x.parse().ok()?, y.parse().ok()?))
}

fn has_audio(p: &Path) -> bool {
    Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "a:0", "-show_entries", "stream=index", "-of", "csv=p=0"])
        .arg(p)
        .output()
        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false)
}

/// ffmpeg into `out`, one maker at a time: two processes (the page server and a lint) that need the same file at
/// once once wrote one temp file together and left a broken prep (2026-10-12). The lock (OUT.lock) holds the maker's
/// process id; a second caller waits for `out`, and a lock whose process is gone is taken over. The temp file is
/// per process, so a broken run never leaves half a file under the final name.
fn run_ffmpeg(args: &[String], out: &Path) -> Result<()> {
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d)?;
    }
    let lock = out.with_extension("lock");
    let me = std::process::id();
    let t0 = Instant::now();
    loop {
        if out.exists() {
            return Ok(());
        }
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&lock) {
            Ok(mut f) => {
                use std::io::Write;
                let _ = write!(f, "{me}");
                break;
            }
            Err(_) if fresh_lock(&lock) => {
                if t0.elapsed().as_secs() > 600 {
                    return Err(anyhow!("waited 10 min for {} (lock {})", out.display(), lock.display()));
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            Err(_) => {
                let _ = std::fs::remove_file(&lock);   // its process is gone
            }
        }
    }
    let res = (|| {
        if out.exists() {
            return Ok(());
        }
        let tmp = out.with_extension(format!("{me}.tmp.mp4"));
        let o = Command::new("ffmpeg").args(["-v", "error", "-y"]).args(args).arg(&tmp).output()?;
        if !o.status.success() {
            let _ = std::fs::remove_file(&tmp);
            return Err(anyhow!("ffmpeg: {}", String::from_utf8_lossy(&o.stderr).trim()));
        }
        std::fs::rename(&tmp, out)?;
        Ok(())
    })();
    let _ = std::fs::remove_file(&lock);
    res
}


fn file_key(p: &Path, extra: &str) -> Result<u64> {
    let mut hs = Xxh3::new();
    let meta = std::fs::metadata(p)?;
    let real = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    hs.update(format!("{}|{}|{:?}|{extra}", real.display(), meta.len(), meta.modified().ok()).as_bytes());
    Ok(hs.digest())
}

fn turn_crop(rotate: i64, crop: Option<(u32, u32, u32, u32)>) -> Vec<String> {
    let mut vf: Vec<String> = vec![];
    match rotate.rem_euclid(360) {
        90 => vf.push("transpose=1".into()),
        270 => vf.push("transpose=2".into()),
        180 => vf.push("transpose=1,transpose=1".into()),
        _ => {}
    }
    if let Some((cw, ch, x, y)) = crop {
        vf.push(format!("crop={cw}:{ch}:{x}:{y}"));
    }
    vf
}

/// The picture turned and cropped to the world size: .cache/prep/HASH.mp4 (made once, no sound: the sound is cut
/// apart, see voice_cut). `src` is the raw clip, or its processed picture (then already turned and cropped).
fn prep(dir: &Path, src: &Path, rotate: i64, crop: Option<(u32, u32, u32, u32)>, w: u32, h: u32, fps: f64) -> Result<(PathBuf, u64)> {
    let c = Clip { path: src.to_path_buf(), dur: 0., rotate, crop, audio: false, voice: false, relight: false, frame: None, exposure: None };
    let key = file_key(&c.path, &format!("{:?}|{w}x{h}|{fps}|v3", (c.rotate, c.crop)))?;
    let out = cache_dir(dir).join("prep").join(format!("{key:016x}.mp4"));
    if out.exists() {
        return Ok((out, key));
    }
    let mut vf = turn_crop(c.rotate, c.crop);
    let (cw, ch) = match c.crop {
        Some((cw, ch, _, _)) => (cw as f64, ch as f64),
        None => (0., 0.),
    };
    // the same aspect (within 1 %): scale straight (what the approved a test job clip did); else fill and centre
    let same = ch > 0. && ((cw / ch) / (w as f64 / h as f64) - 1.).abs() < 0.01;
    if same {
        vf.push(format!("scale={w}:{h}:flags=lanczos,setsar=1"));
    } else {
        vf.push(format!("scale={w}:{h}:force_original_aspect_ratio=increase:flags=lanczos,crop={w}:{h},setsar=1"));
    }
    let t0 = Instant::now();
    // ultrafast crf 14: 6 s for 47 s of 1080x1920 (veryfast crf 12: 37 s), 47.8 dB against it; the CPU, as the GPU
    // may be busy with a reframe. A keyframe every 10 frames: no seek proxy needed (video.rs makes one above 0.5 s)
    // even frames at the job's rate, by timestamp: a phone take is variable (29.885 avg at a 29.97 tag), and a frame
    // read by its number drifted against the sound and the matte (a hand showed twice, 2026-10-12)
    vf.push(format!("fps={fps}"));
    let mut args: Vec<String> = vec!["-i".into(), c.path.to_string_lossy().into(), "-vf".into(), vf.join(","), "-an".into()];
    args.extend(["-c:v", "libx264", "-preset", "ultrafast", "-crf", "14", "-g", "10", "-pix_fmt", "yuv420p", "-movflags", "+faststart"].iter().map(|s| s.to_string()));
    run_ffmpeg(&args, &out).with_context(|| format!("prep {}", c.path.display()))?;
    eprintln!("edit: prepared {} ({:.1} s)", c.path.file_name().unwrap_or_default().to_string_lossy(), t0.elapsed().as_secs_f64());
    Ok((out, key))
}

/// The sound of the kept parts joined: .cache/cut/HASH.wav, 30 ms fades at each join (user, voice cuts). The
/// picture is not cut: lib edit reads each frame from the clip's prep at its raw second, so a skip turned on or
/// off costs one short sound cut, not a new video. `srcs`: (clip, the file whose sound plays, a key for it).
fn voice_cut(dir: &Path, segs: &[Seg], srcs: &[(String, PathBuf, u64)]) -> Result<Option<PathBuf>> {
    if segs.iter().any(|s| !srcs.iter().any(|p| p.0 == s.clip)) {
        return Ok(None);
    }
    let mut hs = Xxh3::new();
    for s in segs {
        let p = srcs.iter().find(|p| p.0 == s.clip).unwrap();
        hs.update(format!("{}|{:.4}|{:.4};", p.2, s.a, s.b).as_bytes());
    }
    let out = cache_dir(dir).join("cut").join(format!("{:016x}.wav", hs.digest()));
    if out.exists() {
        return Ok(Some(out));
    }
    let used: Vec<&(String, PathBuf, u64)> = srcs.iter().filter(|p| segs.iter().any(|s| s.clip == p.0)).collect();
    let mut args: Vec<String> = vec![];
    for p in &used {
        args.extend(["-i".into(), p.1.to_string_lossy().to_string()]);
    }
    let mut g: Vec<String> = vec![];
    let mut cat = String::new();
    for (k, s) in segs.iter().enumerate() {
        let i = used.iter().position(|p| p.0 == s.clip).unwrap();
        let mut f = format!("[{i}:a]atrim=start={:.4}:end={:.4},asetpts=PTS-STARTPTS,aresample=48000", s.a, s.b);
        if k > 0 {
            f.push_str(",afade=t=in:d=0.03");
        }
        if k + 1 < segs.len() {
            f.push_str(&format!(",afade=t=out:st={:.4}:d=0.03", (s.b - s.a - 0.03).max(0.)));
        }
        g.push(format!("{f}[a{k}]"));
        cat.push_str(&format!("[a{k}]"));
    }
    g.push(format!("{cat}concat=n={}:v=0:a=1[a]", segs.len()));
    args.extend(["-filter_complex".into(), g.join(";"), "-map".into(), "[a]".into(), "-c:a".into(), "pcm_s16le".into()]);
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d)?;
    }
    // its own temp name: the live page and a CLI command cut the same sound at once after an edit, and the second
    // rename found nothing ("No such file or directory", 2026-10-17)
    let tmp = out.with_extension(format!("{}.tmp.wav", std::process::id()));
    let t0 = Instant::now();
    let o = Command::new("ffmpeg").args(["-v", "error", "-y"]).args(&args).arg(&tmp).output()?;
    if !o.status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow!("sound cut: {}", String::from_utf8_lossy(&o.stderr).trim()));
    }
    std::fs::rename(&tmp, &out)?;
    eprintln!("edit: sound cut {} parts ({:.1} s)", segs.len(), t0.elapsed().as_secs_f64());
    Ok(Some(out))
}

/// a background step is running: its lock names a process that is still alive (a killed step never blocks)
fn fresh_lock(lock: &Path) -> bool {
    let Some(pid) = std::fs::read_to_string(lock).ok().and_then(|s| s.trim().parse::<libc::pid_t>().ok()).filter(|p| *p > 0) else { return false };
    // SAFETY: signal 0 sends nothing; it only asks whether the process exists (Linux and macOS; pid 0 is filtered out)
    unsafe { libc::kill(pid, 0) == 0 }
}

fn sh_q(p: &Path) -> String {
    format!("'{}'", p.display().to_string().replace('\'', "'\\''"))
}

/// The slow work a clip line asks for (voice / relight / frame), in the background like the matte, cached in
/// .cache/proc/KEY.*. Returns the processed picture when it is ready (else None: the raw clip shows until then),
/// the clean voice when ready, and the files to watch (the page reloads when one appears).
fn proc(dir: &Path, id: &str, c: &Clip, start: bool, layout: &Layout) -> Result<(Option<PathBuf>, Option<PathBuf>, Vec<(String, PathBuf)>)> {
    // absolute paths: the reframe step runs in another folder
    let base = cache_dir(&std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf())).join("proc");
    let src = std::fs::canonicalize(&c.path).unwrap_or_else(|_| c.path.clone());
    let key = file_key(&c.path, &format!("{:?}", (c.rotate, c.crop)))?;
    let skills = layout.dir(Key::Skills);
    let nv = skills.join("nvidia/scripts/nv.py");
    let mut watch = vec![];
    let spawn = |script: String, lock: &Path, log: &Path, what: &str| {
        let _ = std::fs::create_dir_all(&base);
        let sh = format!("( ( set -e; {script} ) > {} 2>&1; rm -f {} ) >/dev/null 2>&1 & echo $! > {}", sh_q(log), sh_q(lock), sh_q(lock));
        let _ = Command::new("sh").arg("-c").arg(sh).stdin(Stdio::null()).status();
        eprintln!("edit: {what} started for clip {id} (background; the page reloads when it is ready)");
    };
    // the clean voice (Studio Voice; nv.py removes its 143 ms delay, so raw times stay)
    let mut voice = None;
    if c.voice {
        let out = base.join(format!("{key:016x}.voice.wav"));
        if out.exists() {
            voice = Some(out);
        } else {
            let lock = out.with_extension("lock");
            if start && !fresh_lock(&lock) {
                let tmp = out.with_extension("part.wav");
                spawn(format!("python3 {} voice {} -o {} --overwrite && mv {} {}", sh_q(&nv), sh_q(&src), sh_q(&tmp), sh_q(&tmp), sh_q(&out)),
                    &lock, &out.with_extension("log"), "Studio Voice");
            }
            watch.push((format!("voice {id}"), out));
        }
    }
    // the picture: turn/crop (only when a later step needs it), relight, reframe, exposure; each step cached on its own
    // (exposure last: a new value re-runs only that step, ~1 s per second of take: color.py float RGB, x264 medium)
    let mut steps: Vec<(String, String)> = vec![];
    if c.relight || c.frame.is_some() || c.exposure.is_some() {
        let mut tag = String::new();
        let mut cur = src.clone();
        if c.rotate.rem_euclid(360) != 0 || c.crop.is_some() {
            tag.push_str("tc");
            let out = base.join(format!("{key:016x}.{tag}.mp4"));
            steps.push((out.display().to_string(), format!("ffmpeg -v error -y -i {} -vf {} -c:v libx264 -preset veryfast -crf 12 -c:a copy {{TMP}}",
                sh_q(&cur), sh_q(Path::new(&turn_crop(c.rotate, c.crop).join(","))))));
            cur = out;
        }
        if c.relight {
            tag.push_str(if tag.is_empty() { "rl" } else { "-rl" });
            let out = base.join(format!("{key:016x}.{tag}.mp4"));
            steps.push((out.display().to_string(), format!("python3 {} relight {} -o {{TMP}} --overwrite", sh_q(&nv), sh_q(&cur))));
            cur = out;
        }
        if let Some(m) = &c.frame {
            tag.push_str(&format!("{}fr{m}", if tag.is_empty() { "" } else { "-" }));
            let out = base.join(format!("{key:016x}.{tag}.mp4"));
            let vt = skills.join("reframe/.venv/bin/verthor");
            // in the skill's vendor folder: verthor fetches its yolo11n.pt into the working folder when missing
            let vend = skills.join("reframe/vendor");
            steps.push((out.display().to_string(), format!("( cd {} && {} {} {{TMP}} --preset talking_head --device cuda --camera-mode {m} )", sh_q(&vend), sh_q(&vt), sh_q(&cur))));
            cur = out;
        }
        if let Some(e) = c.exposure {
            tag.push_str(&format!("{}ex{e}", if tag.is_empty() { "" } else { "-" }));
            let out = base.join(format!("{key:016x}.{tag}.mp4"));
            let color = skills.join("ffmpeg-skill/scripts/color.py");
            steps.push((out.display().to_string(), format!("python3 {} {} --correct --exposure {e} -o {{TMP}} --overwrite", sh_q(&color), sh_q(&cur))));
            cur = out;
        }
        if cur.exists() {
            return Ok((Some(cur), voice, watch));
        }
        let lock = cur.with_extension("lock");
        if start && !fresh_lock(&lock) {
            let script: Vec<String> = steps.iter().map(|(out, cmd)| {
                let o = Path::new(out);
                let tmp = o.with_extension("part.mp4");
                format!("[ -f {o} ] || {{ {} && mv {t} {o}; }}", cmd.replace("{TMP}", &sh_q(&tmp)), o = sh_q(o), t = sh_q(&tmp))
            }).collect();
            let what: Vec<&str> = [(c.relight, "relight"), (c.frame.is_some(), "reframe"), (c.exposure.is_some(), "exposure")].iter().filter(|x| x.0).map(|x| x.1).collect();
            spawn(script.join("; "), &lock, &cur.with_extension("log"), &what.join(" + "));
        }
        watch.push((format!("picture {id}"), cur));
    }
    Ok((None, voice, watch))
}

/// output seconds of a raw second that plays (None when it is skipped)
fn out_exact(segs: &[Seg], clip: &str, t: f64) -> Option<f64> {
    segs.iter().find(|s| s.clip == clip && t >= s.a && t < s.b).map(|s| s.o + t - s.a)
}

/// The person matte for a span of a prepared clip (raw seconds): .cache/matte/HASH/ (nv.py, NVIDIA; ~30 s), run in
/// the background. Returns the folder when it is ready, else None (the words draw in front until then).
fn matte(dir: &Path, prep: &Path, key: u64, (a, b): (f64, f64), fps: f64, start: bool, layout: &Layout) -> (Option<PathBuf>, PathBuf) {
    let mut hs = Xxh3::new();
    hs.update(format!("{key}|{a:.3}|{b:.3}|{fps}").as_bytes());
    let base = if dir.join("_ai").is_dir() { dir.join("_ai").join("matte") } else { dir.join(".cache").join("matte") };
    let out = base.join(format!("{:016x}", hs.digest()));
    if out.join("matte.json").exists() {
        return (Some(out.clone()), out);
    }
    let lock = out.with_extension("lock");
    let fresh = std::fs::metadata(&lock).and_then(|m| m.modified()).ok().and_then(|m| m.elapsed().ok()).map(|e| e.as_secs() < 600).unwrap_or(false);
    if start && !fresh {
        let _ = std::fs::create_dir_all(&base);
        let _ = std::fs::write(&lock, "");
        let nv = layout.dir(Key::Skills).join("nvidia/scripts/nv.py");
        let tmp = out.with_extension("tmp");
        // from 0.4 frame early: ffmpeg's seek then starts on the frame at `a`, not the next one (a float a hair late)
        let sh = format!(
            "(python3 '{nv}' matte '{p}' -o '{t}' --overwrite --from {a0:.6} --to {b:.6} --fps {fps} > '{log}' 2>&1 && rm -rf '{o}' && mv '{t}' '{o}'; rm -f '{l}') >/dev/null 2>&1 &",
            nv = nv.display(), a0 = a - 0.4 / fps, p = prep.display(), t = tmp.display(), o = out.display(), l = lock.display(), log = out.with_extension("log").display()
        );
        let _ = Command::new("sh").arg("-c").arg(sh).stdin(Stdio::null()).status();
        eprintln!("edit: person matte started for {a:.2}-{b:.2} s (about 30 s; the words draw in front until then)");
    }
    (None, out)
}

fn num_or_str(v: &str) -> Value {
    match v.parse::<f64>() {
        Ok(f) if !v.contains(',') => json!(f),
        _ => json!(v),
    }
}

fn pair(v: &str) -> Option<Value> {
    let (a, b) = v.split_once(',')?;
    Some(json!([a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?]))
}

pub struct Compiled {
    pub cfg: Value,
    /// files whose change must reload the job (edit.txt, a matte that is being made)
    pub watch: Vec<(String, PathBuf)>,
    pub segs: Vec<Seg>,
}

/// an error on one line of the file
fn line_err(l: &Line, m: String) -> anyhow::Error {
    anyhow!("{FILE} line {} ({} {}): {m}", l.n + 1, l.kind, l.id)
}

/// edit.txt -> the job's cfg. `prepare` makes the cached prep / cut files and starts the matte; without it only
/// the lines are checked (a note's change is checked this way before it is written). One step per line kind, in this
/// order (the first error wins).
pub fn compile(dir: &Path, text: &str, prepare: bool, layout: &Layout) -> Result<Compiled> {
    let lines = parse_all(text);
    let read = blocks(&lines)?;
    let act: Vec<&Line> = read.iter().filter(|l| !l.off).collect();
    check_ids(&act)?;
    let (w, h, fps, capk) = size_line(&act)?;
    let mut fonts = font_lines(dir, &act)?;
    let clips = take_lines(dir, &act, prepare)?;
    let segs = play_lines(&act, &clips, fps)?;
    let o = segs.last().map(|s| s.o + (s.b - s.a)).unwrap_or(0.);
    let dur = ((o * fps).round() / fps).max(1. / fps);
    // moment lines (word-anchored) -> text lines, caption groups and cut-aways; the rest of the compile sees them as lines
    let sc = scenes(dir, &act, &segs, dur, (w as f64, h as f64), &line_err)?;
    let act: Vec<&Line> = act.iter().copied().chain(sc.lines.iter()).collect();
    let (look_lines, look_files) = looks_for(&act, &mut fonts);
    let styles = style_lines(&act, &look_lines, &fonts)?;
    let mut behind: Vec<(String, f64, f64)> = vec![];
    let items = text_lines(&act, &clips, &segs, &styles, fps, &mut behind)?;
    let mut captions = caps(dir, &act, &segs, h as f64, &line_err, sc.groups)?;
    for c in captions.iter_mut() {   // capcolor= on a moment: its captions' fill (dark words on a white b-roll)
        let t = c["start"].as_f64().unwrap_or(-1.);
        if let Some((_, col)) = sc.capcolor.iter().find(|(t0, _)| (t0 - t).abs() < 1e-6) { c["color"] = json!(col); }
        // its moment: the caption's element id, so a frame drag nudges the moment's captions (nudge= on the moment line)
        let m = sc.spans.iter().filter(|s| s.1 <= t + 1e-6 && t < s.2).filter_map(|s| lines.iter().find(|l| l.n == s.0 && l.kind == "moment")).next();
        if let Some(l) = m { c["moment"] = json!(l.id); }
    }
    // nudge=dx,dy,k on any line: its element moves by dx,dy and scales k about the frame centre (the frame tab's drag);
    // nudge.PART=dx,dy,k on an fx line: only that part (f.part in lib/edit.js), element id "ID.PART"
    let nudge: Map<String, Value> = act.iter().filter(|l| !l.id.is_empty()).flat_map(|l| l.kv.iter().filter_map(move |(k, x)| {
        let key = if k == "nudge" { l.id.clone() } else { format!("{}.{}", l.id, k.strip_prefix("nudge.")?) };
        let v: Vec<f64> = x.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        (v.len() == 3).then(|| (key, json!(v)))
    })).collect();
    let pics = pic_lines(dir, &act, &clips, &segs, fps)?;
    let mut fx_files: Vec<PathBuf> = vec![];
    let mut fx = fx_lines(dir, &act, &clips, &segs, fps, &mut behind, &mut fx_files)?;
    for (v, f) in sc.fx {
        fx.push(v);
        add_file(&mut fx_files, f);
    }
    // a frame line: the reel in one rounded window (last, so its black lies over the b-roll; the words stay on top)
    let frame = act.iter().find(|l| l.kind == "frame").copied();
    if let Some(l) = frame {
        let (v, f) = frame_line(dir, l, &act, &sc.frames, dur)?;
        fx.push(v);
        add_file(&mut fx_files, f);
    }
    // notes: their times and the takes they name
    for l in act.iter().filter(|l| l.kind == "note") {
        clip_of(&clips, l)?;
    }
    let mut watch = vec![("edit".to_string(), dir.join(FILE)), ("notes".to_string(), ai_dir(dir).join(NOTES))];
    for f in &look_files {
        watch.push((format!("look {}", f.file_stem().unwrap_or_default().to_string_lossy()), f.clone()));
    }
    for f in &fx_files {
        watch.push((format!("fx {}", f.file_stem().unwrap_or_default().to_string_lossy()), f.clone()));
    }
    let files = if prepare { prepare_files(dir, &clips, &segs, &behind, (w, h, fps), &mut watch, layout)? } else { Prepared::default() };
    let view = view_of(text, &lines, &segs, &sc.spans);
    let words = words_lane(dir, &act, &segs, &items)?;
    let seg_json: Vec<Value> = segs.iter().map(|s| json!({ "clip": s.clip, "a": s.a, "b": s.b, "o": s.o })).collect();
    let mut cfg = json!({
        "size": [w, h], "fps": fps, "dur": dur, "words": words, "lib": ["motion", "edit"], "comp": fx_files.iter().map(|f| f.to_string_lossy().to_string()).collect::<Vec<_>>(), "fonts": fonts,
        "edit": { "items": items, "pics": pics, "fx": fx, "styles": styles, "segs": seg_json, "mattes": files.mattes, "lines": view, "videos": files.videos, "nudge": nudge },
    });
    if !captions.is_empty() {
        cfg["lib"] = json!(["motion", "edit", "captions"]);
        cfg["captions"] = json!({ "captions": captions });
    }
    // in a frame window: an *emphasis* word on its own line, bigger, in the accent colour (story ref)
    if let Some(l) = frame {
        cfg["captionStyle"] = json!({ "emphLine": true, "emphScale": l.num("emph").unwrap_or(1.9), "emphColor": l.get("accent").unwrap_or("#F8E479"),
            "safeW": 900. });
    }
    // caps=K on the size line: every caption tier K x bigger (the user, geo 2026-10-19: big captions over the person)
    if let Some(k) = capk {
        if !cfg["captionStyle"].is_object() { cfg["captionStyle"] = json!({}); }
        cfg["captionStyle"]["scale"] = json!(k);
        cfg["captionStyle"]["lineW"] = json!(960.);
    }
    // loud=LUFS tp=DB on the size line: the master target (audio.rs master; a test job 2026-10-24: the user wanted -11)
    if let Some(l) = act.iter().find(|l| l.kind == "size").filter(|l| l.num("loud").is_some()) {
        cfg["loudness"] = json!({ "lufs": l.num("loud"), "tp": l.num("tp").unwrap_or(-1.5) });
    }
    if let Some(l) = act.iter().find(|l| l.kind == "song") {
        cfg["song"] = song_line(dir, l, layout.root())?;
    }
    if prepare {
        cfg["clip"] = json!(files.clip);
        if !files.voice.is_empty() {
            cfg["voice"] = json!(files.voice);
        }
    }
    let skill = layout.dir(Key::Skills).join("sfx");
    let sfx = sfx_lines(&skill, &act, &clips, &segs)?;
    if prepare && !sfx.is_empty() {
        sound_fx(dir, &skill, sfx, &files.voice, &mut cfg, layout)?;
    }
    Ok(Compiled { cfg, watch, segs })
}

/// Moment blocks -> the lines the compile reads. A `broll` line gives its moment broll="FILE" and its own keys (as a
/// broll= key on the moment line does); a big or broll line under a moment that is off is off too. A top line in a
/// block, or a broll line outside one, is an error.
fn blocks(lines: &[Line]) -> Result<Vec<Line>> {
    let mut out = lines.to_vec();
    for (i, l) in lines.iter().enumerate().filter(|(_, l)| !l.off) {
        let Some((mn, _)) = &l.moment else {
            if l.kind == "broll" {
                return Err(line_err(l, "a broll line goes under a moment line (indented)".into()));
            }
            continue;
        };
        if l.top() {
            return Err(line_err(l, format!("a {} line is not part of a moment (take its indent away)", l.kind)));
        }
        let m = lines.iter().position(|x| x.n == *mn).unwrap();
        if lines[m].off && (l.kind == "big" || l.kind == "broll") {
            out[i].off = true;
        } else if l.kind == "broll" {
            let f = l.text.clone().ok_or_else(|| line_err(l, "broll needs a \"file\"".into()))?;
            out[m].set("broll", &f);
            if !l.id.is_empty() {
                out[m].set("brollid", &l.id);   // the b-roll's element id = its own line (a drag nudges it, not the moment)
            }
            for (k, v) in l.kv.iter().filter(|(k, _)| k != "nudge") {
                out[m].set(k, v);
            }
        }
    }
    Ok(out)
}

/// ids are unique among the lines that are on
fn check_ids(act: &[&Line]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for l in act {
        if l.id.is_empty() {
            return Err(line_err(l, "no id".into()));
        }
        if !seen.insert((l.top(), l.kind.clone(), l.id.clone())) || (!l.top() && act.iter().any(|o| o.n != l.n && !o.top() && o.id == l.id)) {
            return Err(line_err(l, format!("the id {} is used twice", l.id)));
        }
    }
    Ok(())
}

/// the size line: (width, height, fps, caps=)
fn size_line(act: &[&Line]) -> Result<(u32, u32, f64, Option<f64>)> {
    let Some(l) = act.iter().find(|l| l.kind == "size") else { return Ok((1080, 1920, 30., None)) };
    let (a, b) = l.id.split_once('x').ok_or_else(|| line_err(l, "size is WxH, e.g. 1920x1080".into()))?;
    let w = a.parse().map_err(|_| line_err(l, "bad width".into()))?;
    let h = b.parse().map_err(|_| line_err(l, "bad height".into()))?;
    Ok((w, h, l.num("fps").unwrap_or(30.), l.num("caps")))
}

fn font_lines(dir: &Path, act: &[&Line]) -> Result<Map<String, Value>> {
    let mut fonts = Map::new();
    for l in act.iter().filter(|l| l.kind == "font") {
        let f = l.text.clone().ok_or_else(|| line_err(l, "font needs a \"file\"".into()))?;
        if !dir.join(&f).exists() && !Path::new(&f).exists() {
            return Err(line_err(l, format!("no font file {f}")));
        }
        fonts.insert(l.id.clone(), json!(f));
    }
    Ok(fonts)
}

/// the take lines (a raw recording and its prep keys)
fn take_lines(dir: &Path, act: &[&Line], prepare: bool) -> Result<Vec<(String, Clip)>> {
    let mut clips = vec![];
    for l in act.iter().filter(|l| l.kind == "take") {
        let f = l.text.clone().ok_or_else(|| line_err(l, "take needs a \"file\"".into()))?;
        let p = if Path::new(&f).is_absolute() { PathBuf::from(&f) } else { dir.join(&f) };
        if !p.exists() {
            return Err(line_err(l, format!("no file {}", p.display())));
        }
        let pr = crate::video::probe(&p.to_string_lossy())?;
        let crop = match l.get("crop") {
            Some(c) => Some(crop_of(c).ok_or_else(|| line_err(l, "crop is WxH+X+Y".into()))?),
            None => None,
        };
        let rotate = l.num("rotate").unwrap_or(0.) as i64;
        let frame = l.get("frame").map(|v| if v == "1" { "locked".to_string() } else { v.to_string() });
        if let Some(m) = &frame {
            if m != "locked" && m != "follow" {
                return Err(line_err(l, format!("frame={m}: use frame=locked or frame=follow")));
            }
        }
        clips.push((l.id.clone(), Clip { audio: prepare && has_audio(&p), path: p, dur: pr.duration, rotate, crop,
            voice: l.get("voice").is_some(), relight: l.get("relight").is_some(), frame, exposure: l.num("exposure").filter(|v| *v != 0.) }));
    }
    Ok(clips)
}

/// the take a line names by its letter
fn clip_of<'a>(clips: &'a [(String, Clip)], l: &Line) -> Result<&'a Clip> {
    let c = l.clip.as_ref().ok_or_else(|| line_err(l, "no take letter (e.g. A)".into()))?;
    clips.iter().find(|x| &x.0 == c).map(|x| &x.1).ok_or_else(|| line_err(l, format!("no take {c}")))
}

/// a line's raw span is inside its take
fn check_t(l: &Line, c: &Clip) -> Result<()> {
    let (a, b) = (l.t0.unwrap_or(0.), l.t1.unwrap_or(l.t0.unwrap_or(0.)));
    if a < -0.001 || b > c.dur + 0.05 || b < a {
        return Err(line_err(l, format!("time {a:.2}-{b:.2} is outside the take (0-{:.2} s)", c.dur)));
    }
    Ok(())
}

/// the parts that play: each use / say line minus the skips inside it, in file order, with their output seconds
fn play_lines(act: &[&Line], clips: &[(String, Clip)], fps: f64) -> Result<Vec<Seg>> {
    let mut segs: Vec<Seg> = vec![];
    let skips: Vec<&&Line> = act.iter().filter(|l| l.kind == "skip").collect();
    for l in &skips {
        check_t(l, clip_of(clips, l)?)?;
        if l.t1.is_none() {
            return Err(line_err(l, "a skip needs a span: A 4.10-4.40".into()));
        }
    }
    for l in act.iter().filter(|l| l.kind == "use" || l.kind == "say") {
        let c = clip_of(clips, l)?;
        if l.t1.is_none() {
            return Err(line_err(l, format!("a {} needs a span: A 0.00-4.80", l.kind)));
        }
        check_t(l, c)?;
        let cl = l.clip.clone().unwrap();
        let mut parts = vec![(l.t0.unwrap_or(0.), l.t1.unwrap_or(c.dur).min(c.dur))];
        let mut ks: Vec<(f64, f64)> = skips.iter().filter(|k| k.clip == l.clip).map(|k| (k.t0.unwrap(), k.t1.unwrap())).collect();
        ks.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (ka, kb) in ks {
            parts = parts.into_iter().flat_map(|(a, b)| {
                if kb <= a || ka >= b { vec![(a, b)] } else { vec![(a, ka), (kb, b)] }
            }).filter(|(a, b)| b - a >= 1. / fps).collect();
        }
        for (a, b) in parts {
            segs.push(Seg { clip: cl.clone(), a, b, o: 0. });
        }
    }
    if segs.is_empty() {
        return Err(anyhow!("{FILE}: nothing plays (add a line: use u1 A 0.00-10.00, or turn a say line on)"));
    }
    let mut o = 0.;
    for s in segs.iter_mut() {
        s.o = o;
        o += s.b - s.a;
    }
    Ok(segs)
}

/// Looks: a text line's style that no style line here defines comes from the library (engine/looks/NAME.txt).
/// Returns the looks' style and font lines and their files; the fonts they need go into `fonts`.
fn looks_for(act: &[&Line], fonts: &mut Map<String, Value>) -> (Vec<Line>, Vec<PathBuf>) {
    let mut look_lines: Vec<Line> = vec![];
    let mut look_files: Vec<PathBuf> = vec![];
    for l in act.iter().filter(|l| l.kind == "text") {
        let Some(st) = l.get("style") else { continue };
        if act.iter().any(|o| o.kind == "style" && o.id == st) || look_lines.iter().any(|o| o.kind == "style" && o.id == st) {
            continue;
        }
        if let Some((f, ls)) = look(st) {
            look_lines.extend(ls.into_iter().filter(|o| o.kind == "style" || o.kind == "font"));
            look_files.push(f);
        }
    }
    for l in look_lines.iter().filter(|l| l.kind == "font") {
        if !fonts.contains_key(&l.id) {
            if let Some(f) = &l.text {
                fonts.insert(l.id.clone(), json!(looks_dir().join(f).to_string_lossy()));
            }
        }
    }
    // a style line written here with a library font (the agent copied a look): that font, when the job has none
    for l in act.iter().filter(|l| l.kind == "style") {
        if let Some(f) = l.get("font") {
            if !fonts.contains_key(f) {
                if let Some(p) = look_font(f) {
                    fonts.insert(f.to_string(), json!(p.to_string_lossy()));
                }
            }
        }
    }
    (look_lines, look_files)
}

fn style_lines(act: &[&Line], look_lines: &[Line], fonts: &Map<String, Value>) -> Result<Map<String, Value>> {
    let mut styles = Map::new();
    for l in act.iter().copied().chain(look_lines.iter()).filter(|l| l.kind == "style") {
        let mut m = Map::new();
        for (k, v) in &l.kv {
            if k == "about" {
                continue;
            }
            m.insert(k.clone(), num_or_str(v));
        }
        if let Some(f) = l.get("font") {
            if !fonts.contains_key(f) {
                return Err(line_err(l, format!("no font {f} (add: font {f} \"fonts/file.ttf\")")));
            }
        }
        styles.insert(l.id.clone(), Value::Object(m));
    }
    Ok(styles)
}

/// a span of a take that needs the person matte (a style with `behind`, an fx that draws the person)
fn add_behind(behind: &mut Vec<(String, f64, f64)>, cl: &str, a: f64, b: f64) {
    match behind.iter_mut().find(|x| x.0 == cl) {
        Some(x) => { x.1 = x.1.min(a); x.2 = x.2.max(b); }
        None => behind.push((cl.to_string(), a, b)),
    }
}

fn add_file(files: &mut Vec<PathBuf>, f: PathBuf) {
    if !files.contains(&f) {
        files.push(f);
    }
}

/// a line's raw span -> output seconds; None when all of it is cut
fn out_span(segs: &[Seg], cl: &str, a: f64, b: f64, fps: f64) -> Option<(f64, f64)> {
    let (t0, t1) = (to_out(segs, cl, a, false)?, to_out(segs, cl, b, true)?);
    (t1 - t0 >= 0.5 / fps).then_some((t0, t1))
}

/// the text lines (and the moments' words as text lines) -> the items lib edit draws
fn text_lines(act: &[&Line], clips: &[(String, Clip)], segs: &[Seg], styles: &Map<String, Value>, fps: f64, behind: &mut Vec<(String, f64, f64)>) -> Result<Vec<Value>> {
    let mut items = vec![];
    for l in act.iter().filter(|l| l.kind == "text") {
        check_t(l, clip_of(clips, l)?)?;
        let st = l.get("style").ok_or_else(|| line_err(l, "text needs style=NAME".into()))?;
        let s = styles.get(st).ok_or_else(|| line_err(l, format!("no style {st}")))?;
        let txt = l.text.clone().ok_or_else(|| line_err(l, "text needs the words in \"quotes\"".into()))?;
        let cl = l.clip.clone().unwrap();
        let (a, b) = (l.t0.unwrap(), l.t1.ok_or_else(|| line_err(l, "text needs a span: A 0.88-2.96".into()))?);
        let Some((t0, t1)) = out_span(segs, &cl, a, b, fps) else { continue };
        let mut it = json!({ "id": l.id, "text": txt, "t0": t0, "t1": t1, "style": st, "clip": cl, "raw": [a, b] });
        for (k, v) in &l.kv {
            if k == "style" {
                continue;
            }
            it[k] = pair(v).unwrap_or_else(|| num_or_str(v));
        }
        if s.get("behind").is_some() {
            add_behind(behind, &cl, a, b);
        }
        items.push(it);
    }
    Ok(items)
}

fn pic_lines(dir: &Path, act: &[&Line], clips: &[(String, Clip)], segs: &[Seg], fps: f64) -> Result<Vec<Value>> {
    let mut pics = vec![];
    for l in act.iter().filter(|l| l.kind == "pic") {
        clip_of(clips, l)?;
        let f = l.text.clone().ok_or_else(|| line_err(l, "pic needs a \"file\"".into()))?;
        let p = if Path::new(&f).is_absolute() { PathBuf::from(&f) } else { dir.join(&f) };
        if !p.exists() {
            return Err(line_err(l, format!("no file {}", p.display())));
        }
        let cl = l.clip.clone().unwrap();
        let (a, b) = (l.t0.unwrap(), l.t1.ok_or_else(|| line_err(l, "pic needs a span: A 23.20-24.06".into()))?);
        if let Some((t0, t1)) = out_span(segs, &cl, a, b, fps) {
            pics.push(json!({ "id": l.id, "src": p.to_string_lossy(), "t0": t0, "t1": t1, "fill": l.get("fill").is_some() }));
        }
    }
    Ok(pics)
}

/// effects: each names a file of the skill library (the job's own fx/ first, then engine/js/fx/)
fn fx_lines(dir: &Path, act: &[&Line], clips: &[(String, Clip)], segs: &[Seg], fps: f64, behind: &mut Vec<(String, f64, f64)>, fx_files: &mut Vec<PathBuf>) -> Result<Vec<Value>> {
    let mut fx = vec![];
    for l in act.iter().filter(|l| l.kind == "fx") {
        clip_of(clips, l)?;
        let name = l.text.clone().ok_or_else(|| line_err(l, "fx needs a \"name\" (one of the FX list)".into()))?;
        let f = fx_file(dir, &name).ok_or_else(|| line_err(l, format!("no fx {name} (the list: {})", fx_names(dir).join(", "))))?;
        let cl = l.clip.clone().unwrap();
        let (a, b) = (l.t0.unwrap(), l.t1.ok_or_else(|| line_err(l, "fx needs a span: A 5.80-6.20".into()))?);
        let Some((t0, t1)) = out_span(segs, &cl, a, b, fps) else { continue };
        let keys: Map<String, Value> = l.kv.iter().map(|(k, v)| (k.clone(), pair(v).unwrap_or_else(|| num_or_str(v)))).collect();
        fx.push(json!({ "id": l.id, "name": name, "t0": t0, "t1": t1, "keys": keys }));
        // an effect that draws the person (`// needs: matte` in its file) gets the matte over its span, as `behind` does
        if std::fs::read_to_string(&f).map(|s| s.contains("// needs: matte")).unwrap_or(false) {
            add_behind(behind, &cl, a, b);
        }
        add_file(fx_files, f);
    }
    Ok(fx)
}

/// a take's faces file: its faces line, else _ai/faces/TAKE.json (an older job: TAKE.faces.json)
fn faces_file(dir: &Path, act: &[&Line], take: &str) -> PathBuf {
    match act.iter().find(|l| l.kind == "faces" && l.clip.clone().unwrap_or_else(|| l.id.clone()) == take).and_then(|l| l.text.clone()) {
        Some(f) => dir.join(f),
        None if dir.join("_ai").is_dir() => dir.join("_ai").join("faces").join(format!("{take}.json")),
        None => dir.join(format!("{take}.faces.json")),
    }
}

/// the frame line -> the frame fx over the whole reel: each moment's window and each take's face
fn frame_line(dir: &Path, l: &Line, act: &[&Line], frames: &[(f64, [f64; 5])], dur: f64) -> Result<(Value, PathBuf)> {
    let f = fx_file(dir, "frame").ok_or_else(|| line_err(l, "no fx frame in the library".into()))?;
    let mut keys: Map<String, Value> = l.kv.iter().filter(|(k, _)| ["morph", "fill"].contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.parse::<f64>().map(|n| json!(n)).unwrap_or(json!(v)))).collect();
    keys.insert("boxes".into(), json!(frames.iter().map(|(t, b)| json!([t, b[0], b[1], b[2], b[3], b[4]])).collect::<Vec<_>>()));
    // each take's face (median head top, chin; world px of its prep) from its faces file
    let mut faces = Map::new();
    for c in act.iter().filter(|c| c.kind == "take") {
        let Ok(v) = std::fs::read_to_string(faces_file(dir, act, &c.id)).map(|s| serde_json::from_str::<Value>(&s).unwrap_or(Value::Null)) else { continue };
        let med = |k: &str| -> Option<f64> {
            let mut x: Vec<f64> = v["frames"].as_object()?.values().filter_map(|f| f[k].as_f64()).collect();
            x.sort_by(|a, b| a.total_cmp(b));
            x.get(x.len() / 2).copied()
        };
        if let (Some(a), Some(b)) = (med("head_top"), med("chin")) {
            faces.insert(c.id.clone(), json!([a, b]));
        }
    }
    keys.insert("faces".into(), Value::Object(faces));
    Ok((json!({ "id": l.id, "name": "frame", "t0": 0., "t1": dur, "keys": keys }), f))
}

/// the files a render needs: processed takes, prep, sound cut, matte
#[derive(Default)]
struct Prepared {
    clip: String,
    voice: String,
    videos: Map<String, Value>,
    mattes: Vec<Value>,
}

fn prepare_files(dir: &Path, clips: &[(String, Clip)], segs: &[Seg], behind: &[(String, f64, f64)], (w, h, fps): (u32, u32, f64), watch: &mut Vec<(String, PathBuf)>, layout: &Layout) -> Result<Prepared> {
    let mut out = Prepared::default();
    let mut preps: Vec<(String, PathBuf, u64)> = vec![];
    let mut sounds: Vec<(String, PathBuf, u64)> = vec![];
    for (id, c) in clips {
        if segs.iter().any(|s| &s.clip == id) || behind.iter().any(|b| &b.0 == id) {
            let (pic, voice, w2) = proc(dir, id, c, true, layout)?;
            watch.extend(w2);
            let (p, k) = match &pic {
                Some(f) => prep(dir, f, 0, None, w, h, fps)?,
                None => prep(dir, &c.path, c.rotate, c.crop, w, h, fps)?,
            };
            out.videos.insert(id.clone(), json!(p.to_string_lossy()));
            // the page reloads when the prep is made again or removed (it held a deleted, broken prep, 2026-10-12)
            watch.push((format!("prep {id}"), p.clone()));
            preps.push((id.clone(), p, k));
            match voice {
                Some(v) => sounds.push((id.clone(), v.clone(), file_key(&v, "voice")?)),
                None if c.audio => sounds.push((id.clone(), c.path.clone(), file_key(&c.path, "raw")?)),
                None => {}
            }
        }
    }
    out.clip = preps.iter().find(|p| p.0 == segs[0].clip).unwrap().1.to_string_lossy().to_string();
    if let Some(v) = voice_cut(dir, segs, &sounds)? {
        out.voice = v.to_string_lossy().to_string();
    }
    for (cl, a, b) in behind {
        let p = preps.iter().find(|p| &p.0 == cl).unwrap();
        // on the prep's own frame grid (it may be 29.97 in a 30 fps job): one matte frame = one video frame. A matte
        // at the job's 30 fps drifted a frame against the 29.97 video, and the hand showed twice (2026-10-12)
        let vfps = crate::video::probe(&p.1.to_string_lossy()).map(|v| v.fps).unwrap_or(fps);
        let (a, b) = (&((*a * vfps + 1e-6).floor() / vfps), &((*b * vfps - 1e-6).ceil() / vfps));
        let fps = vfps;
        let (ready, folder) = matte(dir, &p.1, p.2, (*a, *b), fps, true, layout);
        match ready {
            Some(m) => {
                let n = std::fs::read_to_string(m.join("matte.json")).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).and_then(|v| v["n"].as_u64()).unwrap_or(0);
                out.mattes.push(json!({ "clip": cl, "from": a, "fps": fps, "n": n, "pattern": m.join("%05d.webp").to_string_lossy() }));
            }
            None => watch.push((format!("matte {cl}"), folder)),
        }
    }
    Ok(out)
}

/// the page's view of the file: every line with its output times (a broll line: its moment's), and its moment
fn view_of(text: &str, lines: &[Line], segs: &[Seg], spans: &[(usize, f64, f64)]) -> Vec<Value> {
    text.lines().enumerate().map(|(n, r)| {
        match lines.iter().find(|l| l.n == n) {
            None => json!({ "n": n, "raw": r }),
            Some(l) => {
                let (mut o0, mut o1) = (Value::Null, Value::Null);
                if let (Some(c), Some(a)) = (&l.clip, l.t0) {
                    o0 = to_out(segs, c, a, false).map(|x| json!(x)).unwrap_or(Value::Null);
                    o1 = to_out(segs, c, l.t1.unwrap_or(a), true).map(|x| json!(x)).unwrap_or(Value::Null);
                }
                let sn = match (&l.moment, l.kind.as_str()) { (Some(m), "broll") => m.0, _ => n };
                if let Some(x) = spans.iter().find(|x| x.0 == sn && !l.off) {
                    (o0, o1) = (json!(x.1), json!(x.2));
                }
                let mut v = json!({ "n": n, "raw": r, "off": l.off, "mark": l.mark, "kind": l.kind, "id": l.id, "t0": o0, "t1": o1 });
                if let Some((_, m)) = &l.moment {
                    v["moment"] = json!(m);
                }
                if l.kind != "note" {   // the fields, for the page's controls
                    v["clip"] = json!(l.clip);
                    v["a"] = json!(l.t0);
                    v["b"] = json!(l.t1);
                    v["text"] = json!(l.text);
                    v["kv"] = Value::Object(l.kv.iter().map(|(k, x)| (k.clone(), json!(x))).collect());
                }
                if l.kind == "note" {
                    for k in ["status", "reply", "changed", "covers", "moment", "file", "sec", "pt", "ptby"] {
                        if let Some(x) = l.get(k) {
                            v[k] = json!(x);
                        }
                    }
                    v["text"] = json!(l.text);
                }
                v
            }
        }
    }).collect()
}

/// the words lane: the take's word times (a `words` line) in output seconds, else the text lines
fn words_lane(dir: &Path, act: &[&Line], segs: &[Seg], items: &[Value]) -> Result<Vec<Value>> {
    let mut words: Vec<Value> = vec![];
    for l in act.iter().filter(|l| l.kind == "words") {
        let f = l.text.clone().ok_or_else(|| line_err(l, "words needs a \"file\"".into()))?;
        let cl = l.clip.clone().unwrap_or_else(|| l.id.clone());
        let v: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(&f)).map_err(|e| line_err(l, format!("{f}: {e}")))?)?;
        for wd in v.as_array().or_else(|| v["words"].as_array()).cloned().unwrap_or_default() {
            let (a, b) = (wd["start"].as_f64().unwrap_or(0.), wd["end"].as_f64().unwrap_or(0.));
            if out_exact(segs, &cl, (a + b) / 2.).is_none() {
                continue;
            }
            if let (Some(t0), Some(t1)) = (to_out(segs, &cl, a, false), to_out(segs, &cl, b, true)) {
                words.push(json!({ "word": wd["word"], "start": t0, "end": t1.max(t0) }));
            }
        }
    }
    if words.is_empty() {
        words = items.iter().map(|i| json!({ "word": i["text"], "start": i["t0"], "end": i["t1"] })).collect();
    }
    words.sort_by(|a, b| a["start"].as_f64().unwrap_or(0.).total_cmp(&b["start"].as_f64().unwrap_or(0.)));
    Ok(words)
}

/// the song under the voice (audio.rs: ducked, faded out at the end); times are output seconds of the song file
fn song_line(dir: &Path, l: &Line, root: &Path) -> Result<Value> {
    let f = l.text.clone().ok_or_else(|| line_err(l, "song needs a \"file\"".into()))?;
    if !dir.join(&f).exists() && !root.join(&f).exists() && !Path::new(&f).exists() {
        return Err(line_err(l, format!("no song file {f}")));
    }
    let mut s = json!({ "file": f, "from": l.num("from").unwrap_or(0.), "gain_db": l.num("gain").unwrap_or(-14.),
        "duck_db": l.num("duck").unwrap_or(12.), "fade_out": l.num("fade").unwrap_or(1.5) });
    if let Some(a) = l.num("in") {
        s["holes"] = json!([[0., a]]);
    }
    Ok(s)
}

/// sound effects: each `sfx` line's moment in output seconds (a moment inside a skip goes to the next kept second)
fn sfx_lines(skill: &Path, act: &[&Line], clips: &[(String, Clip)], segs: &[Seg]) -> Result<Vec<Value>> {
    let book: Value = serde_json::from_str(&std::fs::read_to_string(skill.join("recipes.json")).unwrap_or_default()).unwrap_or_default();
    let names: Vec<&String> = book.as_object().map(|o| o.keys().filter(|k| !k.starts_with('_')).collect()).unwrap_or_default();
    let mut sfx = vec![];
    for l in act.iter().filter(|l| l.kind == "sfx") {
        let r = l.text.clone().ok_or_else(|| line_err(l, "sfx needs a recipe in \"quotes\", e.g. \"impact\"".into()))?;
        if !names.contains(&&r) {
            return Err(line_err(l, format!("no sfx recipe {r}; one of: {}", names.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" "))));
        }
        check_t(l, clip_of(clips, l)?)?;
        let Some(t) = to_out(segs, l.clip.as_deref().unwrap_or(""), l.t0.unwrap_or(0.), false) else { continue }; // all cut
        sfx.push(json!({ "id": l.id, "recipe": r, "t": (t * 1000.).round() / 1000., "rel": l.num("rel").unwrap_or(0.) }));
    }
    Ok(sfx)
}

/// The `sfx` lines (`sfx x1 A 13.39 "whoosh-impact" rel=-2`, here as {id, recipe, t: output s, rel}): a recipe of the
/// sfx library -> cfg "sfx" (the files, the point of each that lands on the moment, the level against the voice) and
/// the song's holes. `sfx.py resolve` decides, as `sfx.py stack` does for a job.json; cached by its input (.cache/sfx/).
fn sound_fx(dir: &Path, skill: &Path, q: Vec<Value>, voice: &str, cfg: &mut Value, layout: &Layout) -> Result<()> {
    let input = json!({ "voice": voice, "cache": cache_dir(dir), "words": cfg["words"], "song": cfg["song"].is_object(), "lines": q }).to_string();
    let mut hs = Xxh3::new();
    hs.update(input.as_bytes());
    for f in [skill.join("recipes.json"), layout.dir(Key::Sfx).join("index.json"), PathBuf::from(voice)] {
        hs.update(&file_key(&f, "sfx").unwrap_or(0).to_le_bytes());
    }
    let cache = cache_dir(dir).join("sfx").join(format!("{:016x}.json", hs.digest()));
    let out: Value = match std::fs::read_to_string(&cache).ok().and_then(|s| serde_json::from_str(&s).ok()) {
        Some(v) => v,
        None => {
            let mut c = Command::new("python3").arg(skill.join("scripts/sfx.py")).arg("resolve").current_dir(layout.root())
                .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
            std::io::Write::write_all(c.stdin.as_mut().unwrap(), input.as_bytes())?;
            let o = c.wait_with_output()?;
            if !o.status.success() {
                return Err(anyhow!("sfx lines: {}", String::from_utf8_lossy(&o.stderr).trim().lines().last().unwrap_or("sfx.py resolve failed")));
            }
            let v: Value = serde_json::from_slice(&o.stdout)?;
            std::fs::create_dir_all(cache.parent().unwrap())?;
            std::fs::write(&cache, v.to_string())?;
            v
        }
    };
    cfg["sfx"] = out["sfx"].clone();
    if let (Some(h), true) = (out["holes"].as_array(), cfg["song"].is_object()) {
        if !h.is_empty() {
            let mut all = cfg["song"]["holes"].as_array().cloned().unwrap_or_default();
            all.extend(h.iter().cloned());
            cfg["song"]["holes"] = json!(all);
        }
    }
    Ok(())
}

// ───────────── scenes (word-anchored) ─────────────
// A scene line names its spoken words, not times. The engine finds them in the words that play (in order, each scene
// after the one before), so a skip or a new cut never moves a caption, a cut-away or a card:
//   scene sc1 "It's finally | my time | where I can *focus*"            the face; captions low (y=1494, capy= to move)
//   scene sc2 "more on how to do a storytelling" broll="refs/videos/sea.mp4"   the clip centred on black (w=912), its
//             speed fitted so it never freezes (from= rate= w= end = its last frame on the cut); captions under it
//   scene sc4 "AI is gonna replace creativity now." card="AI | does not replace | ^creativity^"   a black card, the
//             card words in the caption tiers, each on its spoken word; no captions
//   big   g1 "STORYTELLING"            a big word on the scene line above it (scene=sc2 to pick), from that word to the
//             scene's end, out of the captions; on the face: look behind_head; on a clip: under it (style= at= size=);
//             on a card: it restyles that card word (its keys over the card's: color= size= style= at=)
// Caption words: "|" splits groups (else one word at a time, small words join the next), *w* EMPHASIS, ^w^ HERO,
// ~w~ = spoken, not shown; a marker can span words (*does not replace*). A scene runs from its first word to the next scene's first word (the first from 0).

/// a caption group: (words (text, tier, out t0, out t1), clip, raw start, raw end, y, latest end)
type Group = (Vec<(String, &'static str, f64, f64)>, String, f64, f64, Option<f64>, f64);

struct Scenes {
    lines: Vec<Line>,
    groups: Vec<Group>,
    fx: Vec<(Value, PathBuf)>,
    /// (line n, out t0, out t1) for the page's view
    spans: Vec<(usize, f64, f64)>,
    /// a frame line's window per scene: (out t0, [x, y, w, h, r])
    frames: Vec<(f64, [f64; 5])>,
    /// a moment's capcolor=: (its first caption word's out t, the colour)
    capcolor: Vec<(f64, String)>,
}
/// the window shapes of a frame line, on one grid (1080x1920, 20 px margins); "dot" = closed (a card)
fn win_box(name: &str) -> Option<[f64; 5]> {
    Some(match name {
        "sq" => [20., 407., 1040., 1040., 80.],
        "tall" => [70., 250., 940., 1300., 80.],
        "full" => [0., 0., 1080., 1920., 0.],
        "wide" => [20., 600., 1040., 600., 64.],
        "small" => [150., 560., 780., 780., 64.],
        "dot" => [540., 960., 0., 0., 0.],
        _ => return None,
    })
}

struct Tok {
    w: String,
    tier: &'static str,
    hidden: bool,
    group: usize,
}

const SMALL: &[&str] = &["a", "an", "the", "to", "of", "in", "on", "at", "by", "for", "and", "or", "but", "so", "if", "is",
    "am", "are", "was", "be", "it", "its", "it's", "i", "i'm", "my", "me", "we", "you", "your", "that", "this", "with", "as",
    "not", "no", "do", "just", "like", "can"];

fn norm(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == '\'').collect()
}

/// a scene's or card's words: markers read, groups by "|" (none: one word at a time, a small word joins the next)
fn scene_toks(s: &str) -> Vec<Tok> {
    let explicit = s.contains('|');
    let mut out: Vec<Tok> = vec![];
    let mut g = 0;
    let mut open: Option<char> = None;   // a marker over several words: *does not replace*
    for part in s.split('|') {
        for tok in part.split_whitespace() {
            let mut w = tok;
            let mut m = open;
            if let Some(c) = ['^', '*', '~'].into_iter().find(|&c| w.len() > 1 && w.starts_with(c)) {
                w = &w[1..];
                m = Some(c);
                open = Some(c);
            }
            // the closing marker, before or after the end punctuation: *say*. = *say.*
            let core = w.trim_end_matches(|c: char| ",.!?;:".contains(c));
            let mut ww = w.to_string();
            if let Some(c) = m.filter(|&c| core.len() > 1 && core.ends_with(c)) {
                ww = format!("{}{}", &core[..core.len() - c.len_utf8()], &w[core.len()..]);
                open = None;
            }
            let w = ww.as_str();
            let (tier, hidden) = match m { Some('^') => ("HERO", false), Some('*') => ("EMPHASIS", false), Some('~') => ("BASE", true), _ => ("BASE", false) };
            out.push(Tok { w: w.to_string(), tier, hidden, group: g });
            let small = SMALL.contains(&norm(w).as_str()) && !w.ends_with(|c: char| ",.!?;:".contains(c));
            if !explicit && !(small && !hidden) {
                g += 1;
            }
        }
        if explicit {
            g += 1;
        }
    }
    // small words at the end join the group before them ("I'm not saying that")
    if !explicit {
        if let Some(last) = out.iter().filter(|t| !t.hidden).map(|t| t.group).max() {
            let only_small = out.iter().filter(|t| t.group == last && !t.hidden).all(|t| SMALL.contains(&norm(&t.w).as_str()));
            if let Some(prev) = out.iter().filter(|t| t.group < last).map(|t| t.group).max().filter(|_| only_small) {
                for t in out.iter_mut().filter(|t| t.group == last) {
                    t.group = prev;
                }
            }
        }
    }
    out
}

/// an output span -> its raw parts, one per clip run: (clip, raw start, raw end)
fn raw_runs(segs: &[Seg], o0: f64, o1: f64) -> Vec<(String, f64, f64)> {
    let mut out: Vec<(String, f64, f64)> = vec![];
    for s in segs {
        let (a, b) = (o0.max(s.o), o1.min(s.o + s.b - s.a));
        if b - a < 1e-6 {
            continue;
        }
        let (ra, rb) = (s.a + a - s.o, s.a + b - s.o);
        match out.last_mut() {
            Some(x) if x.0 == s.clip && ra >= x.2 - 1e-6 => x.2 = rb,
            _ => out.push((s.clip.clone(), ra, rb)),
        }
    }
    out
}

fn scenes(dir: &Path, act: &[&Line], segs: &[Seg], dur: f64, world: (f64, f64), err: &dyn Fn(&Line, String) -> anyhow::Error) -> Result<Scenes> {
    let mut sc = Scenes { lines: vec![], groups: vec![], fx: vec![], spans: vec![], frames: vec![], capcolor: vec![] };
    let scs: Vec<&&Line> = act.iter().filter(|l| l.kind == "moment").collect();
    let bigs: Vec<&&Line> = act.iter().filter(|l| l.kind == "big").collect();
    if scs.is_empty() {
        if let Some(b) = bigs.first() {
            return Err(err(b, "a big word needs moment lines".into()));
        }
        return Ok(sc);
    }
    // the words that play, in output order: (word, clip, raw start, raw end, out start, out end)
    let mut ow: Vec<(String, String, f64, f64, f64, f64)> = vec![];
    for l in act.iter().filter(|l| l.kind == "words") {
        let f = l.text.clone().ok_or_else(|| err(l, "words needs a \"file\"".into()))?;
        let cl = l.clip.clone().unwrap_or_else(|| l.id.clone());
        let v: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(&f)).map_err(|e| err(l, format!("{f}: {e}")))?)?;
        for wd in v.as_array().or_else(|| v["words"].as_array()).cloned().unwrap_or_default() {
            let (a, b) = (wd["start"].as_f64().unwrap_or(0.), wd["end"].as_f64().unwrap_or(0.));
            if out_exact(segs, &cl, (a + b) / 2.).is_none() {
                continue;
            }
            if let (Some(t0), Some(t1)) = (to_out(segs, &cl, a, false), to_out(segs, &cl, b, true)) {
                ow.push((wd["word"].as_str().unwrap_or("").trim().to_string(), cl.clone(), a, b, t0, t1.max(t0)));
            }
        }
    }
    ow.sort_by(|a, b| a.4.total_cmp(&b.4));
    // each scene's words -> the spoken words (a look-ahead of 8; ASR spelling can miss a word, not a quarter of them)
    let mut cur = 0;
    let mut found: Vec<(Vec<Tok>, Vec<Option<usize>>)> = vec![];
    for l in &scs {
        let txt = l.text.clone().ok_or_else(|| err(l, "moment needs its spoken words in \"quotes\"".into()))?;
        let toks = scene_toks(&txt);
        let mut hit = vec![];
        let mut k = cur;
        for t in &toks {
            match (k..(k + 8).min(ow.len())).find(|&j| norm(&ow[j].0) == norm(&t.w)) {
                Some(j) => { hit.push(Some(j)); k = j + 1; }
                None => hit.push(None),
            }
        }
        let miss: Vec<&str> = toks.iter().zip(&hit).filter(|x| x.1.is_none()).map(|x| x.0.w.as_str()).collect();
        if miss.len() == toks.len() || (miss.len() > 1 && miss.len() * 4 > toks.len()) {
            let at = ow.get(cur).map(|w| w.4).unwrap_or(dur);
            let next: Vec<&str> = ow.iter().skip(cur).take(8).map(|w| w.0.as_str()).collect();
            return Err(err(l, format!("words not spoken here: {} (after {at:.2} s the words are: \"{}\")", miss.join(" "), next.join(" "))));
        }
        cur = k;
        found.push((toks, hit));
    }
    let starts: Vec<f64> = found.iter().enumerate().map(|(i, (_, h))| if i == 0 { 0. } else { ow[h.iter().flatten().next().copied().unwrap()].4 }).collect();
    let span = |i: usize| (starts[i], starts.get(i + 1).copied().unwrap_or(dur));
    for (i, l) in scs.iter().enumerate() {
        let (o0, o1) = span(i);
        if o1 - o0 < 0.1 {
            return Err(err(l, format!("this moment is {:.2} s long (it starts where the next one does)", o1 - o0)));
        }
        sc.spans.push((l.n, o0, o1));
    }
    // a text line over an output span (one per clip run)
    let text_line = |l: &Line, id: &str, txt: &str, o0: f64, o1: f64, kv: Vec<(String, String)>| -> Vec<Line> {
        raw_runs(segs, o0, o1).into_iter().enumerate().map(|(k, (cl, a, b))| Line {
            n: l.n, kind: "text".into(), id: if k == 0 { id.to_string() } else { format!("{id}.{k}") }, clip: Some(cl),
            t0: Some(a), t1: Some(b), text: Some(txt.to_string()), kv: kv.clone(), ..Default::default()
        }).collect()
    };
    let fxf = |name: &str, l: &Line| fx_file(dir, name).ok_or_else(|| err(l, format!("no fx {name} in the library")));
    // a frame line: each scene's window shape (win=NAME, else: a card closes it, a clip by its shape, the face in turn
    // sq -> tall -> full -> sq -> small -> tall, never the same shape twice in a row); b-roll covers it, captions in it
    let framed = act.iter().any(|l| l.kind == "frame");
    let (mut prev_win, mut face_n) = (String::new(), 0usize);
    // big words: their scene and words (out of that scene's captions)
    let mut cut: Vec<(usize, usize)> = vec![];   // (scene, token) not captioned
    let mut big_at: Vec<(usize, &Line, f64)> = vec![];
    let mut card_over: Vec<(usize, usize, &Line)> = vec![];   // (scene, card group, big line): a big on a card restyles that card word
    for b in &bigs {
        let words: Vec<String> = b.text.clone().ok_or_else(|| err(b, "big needs its word in \"quotes\"".into()))?.split_whitespace().map(norm).collect();
        let pick = |i: usize| -> Option<usize> {
            let t = &found[i].0;
            (0..t.len()).find(|&s| s + words.len() <= t.len() && (0..words.len()).all(|k| norm(&t[s + k].w) == words[k]))
        };
        // on a card: the card group that holds the word (the card's own words, spoken or not)
        let card_pick = |i: usize| -> Option<usize> {
            let t = scene_toks(scs[i].get("card")?);
            (0..t.len()).find(|&s| s + words.len() <= t.len() && (0..words.len()).all(|k| norm(&t[s + k].w) == words[k])).map(|s| t[s].group)
        };
        let says = |i: usize| pick(i).is_some() || card_pick(i).is_some();
        // moment=ID (old: scene=ID), else the moment block it is in
        let (i, s) = match b.get("moment").or_else(|| b.get("scene")).or(b.moment.as_ref().map(|m| m.1.as_str())) {
            Some(id) => {
                let i = scs.iter().position(|l| l.id == id).ok_or_else(|| err(b, format!("no moment {id}")))?;
                (i, pick(i))
            }
            // the moment line above it, else the first moment that says it
            None => match scs.iter().rposition(|l| l.n < b.n).filter(|&i| says(i)) {
                Some(i) => (i, pick(i)),
                None => (0..scs.len()).find(|&i| says(i)).map(|i| (i, pick(i))).ok_or_else(|| err(b, "no moment says this word (add moment=ID)".into()))?,
            },
        };
        if scs[i].get("card").is_some() {
            let g = card_pick(i).ok_or_else(|| err(b, "this card has no such word; a big on a card restyles one of its card=\"…\" words".into()))?;
            card_over.push((i, g, b));
            continue;
        }
        let (o0, _) = span(i);
        let t = s.and_then(|s| found[i].1[s]).map(|j| ow[j].4).unwrap_or(o0);
        if let Some(s) = s {
            cut.extend((s..s + words.len()).map(|k| (i, k)));
        }
        big_at.push((i, b, t));
    }
    for (i, l) in scs.iter().enumerate() {
        let (o0, o1) = span(i);
        let (toks, hit) = &found[i];
        let fbox = if !framed { None } else {
            let name = match l.get("win") {
                Some(w) => w.to_string(),
                None if l.get("card").is_some() => "dot".into(),
                None => match l.get("broll") {
                    Some(src) => {
                        let p = if Path::new(src).is_absolute() { PathBuf::from(src) } else { dir.join(src) };
                        let a = crate::video::probe(&p.to_string_lossy()).map(|pr| pr.width as f64 / pr.height.max(1) as f64).unwrap_or(1.);
                        let n = if a > 1.25 { "wide" } else if a < 0.8 { "tall" } else { "sq" };
                        (if n == prev_win { if n == "sq" { "small" } else { "sq" } } else { n }).into()   // two clips in a row: another shape
                    }
                    None => {
                        let cyc = ["sq", "tall", "full", "sq", "small", "tall"];
                        let mut n = cyc[face_n % cyc.len()];
                        face_n += 1;
                        if n == prev_win { n = cyc[face_n % cyc.len()]; face_n += 1; }
                        n.into()
                    }
                },
            };
            let b = win_box(&name).ok_or_else(|| err(l, format!("win={name}: sq tall full wide small dot")))?;
            sc.frames.push((o0, b));
            prev_win = name;
            Some(b)
        };
        let fcap = |b: [f64; 5]| if b[3] >= 1900. { 1494. } else if b[2] > b[3] * 1.3 { b[1] + b[3] + 100. } else { b[1] + b[3] - 170. };
        let mut capy = l.num("capy").unwrap_or(fbox.map(fcap).unwrap_or(1494.));
        if let Some(card) = l.get("card") {
            // a black card; each group of the card words on the first of its words that is spoken, else 0.35 s after
            let bf = fxf("blackout", l)?;
            sc.fx.push((json!({ "id": format!("{}.ground", l.id), "name": "blackout", "t0": o0, "t1": o1, "keys": {} }), bf));
            let ct = scene_toks(card);
            let ng = ct.iter().map(|t| t.group + 1).max().unwrap_or(0);
            let spoken: Vec<usize> = hit.iter().flatten().copied().collect();
            let size = |tier: &str| match tier { "HERO" => 380., "EMPHASIS" => 170., _ => 200. };
            let mut rows = vec![];
            let (mut t, mut used, mut page) = (o0, 0, 0);
            for g in 0..ng {
                let ws: Vec<&Tok> = ct.iter().filter(|x| x.group == g).collect();
                if ws.is_empty() {
                    page += 1;   // "||": a new page of the card; the page before fades out when it comes
                    continue;
                }
                let tier = if ws.iter().any(|x| x.tier == "HERO") { "HERO" } else if ws.iter().any(|x| x.tier == "EMPHASIS") { "EMPHASIS" } else { "BASE" };
                let at = ws.iter().find_map(|x| spoken.iter().skip(used).position(|&j| norm(&ow[j].0) == norm(&x.w)).map(|p| (p, ow[spoken[used + p]].4)));
                // each word of the row on its own spoken word (the next 6 spoken words), else 0.12 s after the one before
                let mut u = used;
                let mut wt: Vec<f64> = vec![];
                for x in &ws {
                    let tt = match spoken.iter().skip(u).take(6).position(|&j| norm(&ow[j].0) == norm(&x.w)) {
                        Some(p) => { u += p + 1; Some(ow[spoken[u - 1]].4) }
                        None => None,
                    };
                    wt.push(tt.unwrap_or(wt.last().map(|v| v + 0.12).unwrap_or(f64::NAN)));
                }
                t = match at { Some((p, tt)) => { used += p + 1; tt.max(t) } None if g == 0 => o0, None => t + 0.35 };
                used = used.max(u);
                let t = t.min(o1 - 0.2);
                let wt: Vec<String> = wt.iter().map(|v| format!("{:.2}", if v.is_nan() { 0. } else { (v - t).clamp(0., (o1 - 0.2 - t).max(0.)) })).collect();
                rows.push((ws.iter().map(|x| x.w.as_str()).collect::<Vec<_>>().join(" "), tier, t, g, wt.join(";"), page));
            }
            let hs: Vec<f64> = rows.iter().map(|r| size(r.1) * if r.1 == "HERO" { 0.8 } else { 1.15 }).collect();
            // each page centred on its own; its rows end where the next page starts
            let page_h = |p: usize| rows.iter().zip(&hs).filter(|(r, _)| r.5 == p).map(|(_, h)| h).sum::<f64>();
            let page_end = |p: usize| rows.iter().find(|r| r.5 > p).map(|r| r.2).unwrap_or(o1);
            let mut y = 0.;
            for (k, (txt, tier, t, g, wt, pg)) in rows.iter().enumerate() {
                if k == 0 || rows[k - 1].5 != *pg {
                    y = world.1 / 2. - page_h(*pg) / 2.;
                }
                let t1 = page_end(*pg);
                let st = match *tier { "HERO" => "scene_hero", "EMPHASIS" => "scene_emph", _ => "scene_base" };
                let mut kv = vec![("style".to_string(), st.to_string()), ("at".into(), format!("{},{}", (world.0 / 2.).round(), (y + hs[k] / 2.).round()))];
                if wt.contains(';') {
                    kv.push(("wt".into(), wt.clone()));   // per word: seconds after the row's start (lib edit: each word fades up on its own)
                }
                if t1 < o1 {
                    kv.extend([("exit".to_string(), "fade".to_string()), ("exitdur".into(), "0.2".into())]);
                }
                match card_over.iter().rev().find(|c| c.0 == i && c.1 == *g) {
                    // a big line on this card word: its keys over the card's (the line keeps the big's id, so a drag writes it)
                    Some((_, _, b)) => {
                        for (bk, bv) in b.kv.iter().filter(|(bk, _)| bk != "scene" && bk != "moment") {
                            match kv.iter_mut().find(|(k2, _)| k2 == bk) {
                                Some(x) => x.1 = bv.clone(),
                                None => kv.push((bk.clone(), bv.clone())),
                            }
                        }
                        sc.lines.extend(text_line(b, &b.id, txt, *t, t1, kv));
                        sc.spans.push((b.n, *t, t1));
                    }
                    None => sc.lines.extend(text_line(l, &format!("{}.w{}", l.id, k + 1), txt, *t, t1, kv)),
                }
                y += hs[k];
            }
            continue;   // no captions on a card
        }
        let mut bottom = None;
        if let Some(src) = l.get("broll") {
            let p = if Path::new(src).is_absolute() { PathBuf::from(src) } else { dir.join(src) };
            let pr = crate::video::probe(&p.to_string_lossy()).map_err(|e| err(l, format!("broll {src}: {e}")))?;
            let w = l.num("w").unwrap_or(912.);
            let (s, mut rate) = (o1 - o0, l.num("rate").unwrap_or(1.));
            let mut from = l.num("from").unwrap_or(0.);
            if l.get("end").is_some() {
                from = (pr.duration - s * rate).max(0.);
            }
            if pr.duration - from < s * rate {
                rate = ((pr.duration - from) / s).max(0.05);   // slower, never a frozen last frame
            }
            let h = w * pr.height as f64 / pr.width.max(1) as f64;
            let mut keys = json!({ "src": p.to_string_lossy(), "w": w, "from": from, "rate": (rate * 1000.).round() / 1000. });
            match fbox {
                Some(b) => keys["box"] = json!(format!("{},{},{},{}", b[0], b[1], b[2], b[3])),   // in the window (the frame fx moves it)
                None => {
                    bottom = Some(world.1 / 2. + h / 2.);
                    if l.get("capy").is_none() && world.1 / 2. + h / 2. + 110. < capy {
                        capy = world.1 / 2. + h / 2. + 110.;
                    }
                }
            }
            sc.fx.push((json!({ "id": l.get("brollid").unwrap_or(&l.id), "name": "broll", "t0": o0, "t1": o1, "keys": keys }), fxf("broll", l)?));
        }
        // big words on this scene
        for (_, b, t) in big_at.iter().filter(|x| x.0 == i) {
            let mut kv: Vec<(String, String)> = b.kv.iter().filter(|(k, _)| k != "scene" && k != "moment").cloned().collect();
            if !kv.iter().any(|(k, _)| k == "style") {
                kv.push(("style".into(), if bottom.is_some() { "scene_big" } else { "behind_head" }.into()));
            }
            if let (Some(bt), false) = (bottom, kv.iter().any(|(k, _)| k == "at" || k == "y")) {
                let sz = b.num("size").unwrap_or(200.);
                kv.push(("at".into(), format!("{},{}", (world.0 / 2.).round(), (bt + sz * 0.6 + 30.).round())));
                capy = capy.max(bt + sz * 1.2 + 110.).min(world.1 - 330.);
            }
            sc.lines.extend(text_line(b, &b.id, b.text.as_deref().unwrap_or(""), *t, o1, kv));
            sc.spans.push((b.n, *t, o1));
        }
        // captions: the scene's words by group, at their spoken times
        let ng = toks.iter().map(|t| t.group + 1).max().unwrap_or(0);
        for g in 0..ng {
            let mut ws: Vec<(String, &'static str, f64, f64)> = vec![];
            let (mut cl, mut ra, mut rb) = (None::<String>, f64::MAX, 0f64);
            for (k, t) in toks.iter().enumerate().filter(|(_, t)| t.group == g) {
                let j = hit[k].or_else(|| hit[..k].iter().rev().flatten().next().copied()).or_else(|| hit[k..].iter().flatten().next().copied());
                let Some(j) = j else { continue };
                if t.hidden || cut.contains(&(i, k)) {
                    continue;
                }
                let w = &ow[j];
                if cl.is_none() { cl = Some(w.1.clone()); }
                if cl.as_deref() == Some(w.1.as_str()) { ra = ra.min(w.2); rb = rb.max(w.3); }
                ws.push((t.w.clone(), t.tier, w.4, w.5));
            }
            if let (false, Some(cl)) = (ws.is_empty(), cl) {
                if let Some(c) = l.get("capcolor") { sc.capcolor.push((ws[0].2, c.to_string())); }
                sc.groups.push((ws, cl, ra, rb, Some(capy), o1));
            }
        }
    }
    Ok(sc)
}

/// The `cap` lines as lib captions' list, in output seconds. Each word gets its time from the clip's `words` file (the
/// next spoken word with the same letters, else the next word); a word that is cut drops out, a group with none left
/// too. Timing as captions/build.py (approved v3): a group stays until the next one when the gap is <= 0.6 s, else
/// 0.3 s after its last word; a HERO group at least 0.5 s after its word. The face (worst case over the group, world px)
/// comes from the clip's `faces` file; lib captions then puts the group low, above the head, or low with the HERO shrunk.
fn caps(dir: &Path, act: &[&Line], segs: &[Seg], world_h: f64, err: &dyn Fn(&Line, String) -> anyhow::Error, extra: Vec<Group>) -> Result<Vec<Value>> {
    let lines: Vec<&&Line> = act.iter().filter(|l| l.kind == "cap").collect();
    if lines.is_empty() && extra.is_empty() {
        return Ok(vec![]);
    }
    let load = |kind: &str, cl: &str| -> Result<Option<Value>> {
        let Some(l) = act.iter().find(|l| l.kind == kind && l.clip.clone().unwrap_or_else(|| l.id.clone()) == cl) else { return Ok(None) };
        let f = l.text.clone().ok_or_else(|| err(l, format!("{kind} needs a \"file\"")))?;
        let txt = std::fs::read_to_string(dir.join(&f)).map_err(|e| err(l, format!("{f}: {e}")))?;
        Ok(Some(serde_json::from_str(&txt).map_err(|e| err(l, format!("{f}: {e}")))?))
    };
    let norm = |s: &str| s.to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == '\'').collect::<String>();
    let mut groups: Vec<Group> = vec![];
    let mut cache: std::collections::HashMap<String, (Vec<(String, f64, f64)>, Option<Value>)> = Default::default();
    for l in &lines {
        let cl = l.clip.clone().ok_or_else(|| err(l, "no clip letter (e.g. A)".into()))?;
        let (a, b) = (l.t0.unwrap_or(0.), l.t1.ok_or_else(|| err(l, "cap needs a span: A 0.30-3.30".into()))?);
        let txt = l.text.clone().ok_or_else(|| err(l, "cap needs the words in \"quotes\"".into()))?;
        if !cache.contains_key(&cl) {
            let w = load("words", &cl)?.ok_or_else(|| err(l, format!("cap needs a words line for clip {cl}")))?;
            let ws = w.as_array().or_else(|| w["words"].as_array()).cloned().unwrap_or_default().iter()
                .map(|x| (x["word"].as_str().unwrap_or("").to_string(), x["start"].as_f64().unwrap_or(0.), x["end"].as_f64().unwrap_or(0.))).collect();
            cache.insert(cl.clone(), (ws, load("faces", &cl)?));
        }
        let cand: Vec<&(String, f64, f64)> = cache[&cl].0.iter().filter(|x| x.1 >= a - 0.05 && x.1 < b + 0.05).collect();
        let mut j = 0;
        for g in txt.split('|') {
            let mut ws = vec![];
            let (mut ra, mut rb) = (f64::MAX, 0f64);
            for tok in g.split_whitespace() {
                let (w, tier) = if tok.len() > 2 && tok.starts_with('^') && tok.ends_with('^') { (&tok[1..tok.len() - 1], "HERO") }
                    else if tok.len() > 2 && tok.starts_with('*') && tok.ends_with('*') { (&tok[1..tok.len() - 1], "EMPHASIS") }
                    else { (tok, "BASE") };
                if let Some(k) = (j..cand.len().min(j + 5)).find(|&k| norm(&cand[k].0) == norm(w)) {
                    j = k;
                }
                let c = cand.get(j).ok_or_else(|| err(l, format!("\"{w}\": more words than are spoken in {cl} {a:.2}-{b:.2}")))?;
                j += 1;
                if out_exact(segs, &cl, (c.1 + c.2) / 2.).is_none() {
                    continue; // cut
                }
                let (Some(t0), Some(t1)) = (to_out(segs, &cl, c.1, false), to_out(segs, &cl, c.2, true)) else { continue };
                ws.push((w.to_string(), tier, t0, t1.max(t0)));
                ra = ra.min(c.1);
                rb = rb.max(c.2);
            }
            if !ws.is_empty() {
                groups.push((ws, cl.clone(), ra, rb, l.num("y"), f64::MAX));
            }
        }
    }
    // a scene's groups (word times known): the faces of their clips
    for g in &extra {
        if !cache.contains_key(&g.1) {
            cache.insert(g.1.clone(), (vec![], load("faces", &g.1)?));
        }
    }
    groups.extend(extra);
    groups.sort_by(|x, y| x.0[0].2.total_cmp(&y.0[0].2));
    let mut out = vec![];
    for (n, (ws, cl, ra, rb, cy, lim)) in groups.iter().enumerate() {
        let start = ws[0].2;
        let endw = ws.last().unwrap().3;
        let nxt = groups.get(n + 1).map(|g| g.0[0].2);
        let hero = ws.iter().any(|w| w.1 == "HERO");
        let mut end = match nxt { Some(x) if x - endw <= 0.6 => x, _ => endw + 0.3 };
        if hero {
            end = (end.max(endw + 0.5)).min(nxt.unwrap_or(f64::MAX));
        }
        end = end.min(*lim);   // a scene's caption leaves with its scene
        // lines: a HERO word on its own line
        let mut ls: Vec<Vec<Value>> = vec![vec![]];
        for w in ws {
            let v = json!({ "w": w.0, "tier": w.1, "t": w.2 });
            if w.1 == "HERO" {
                if !ls.last().unwrap().is_empty() { ls.push(vec![]); }
                ls.last_mut().unwrap().push(v);
                ls.push(vec![]);
            } else {
                ls.last_mut().unwrap().push(v);
            }
        }
        ls.retain(|l| !l.is_empty());
        let mut c = json!({ "start": start, "end": end, "single": ws.len() == 1, "center": hero, "lines": ls });
        if let Some(y) = cy { c["y"] = json!(y); }
        if let Some(f) = cache[cl].1.as_ref().and_then(|v| v["frames"].as_object()) {
            let k = world_h / cache[cl].1.as_ref().unwrap()["size"][1].as_f64().unwrap_or(world_h);
            let fr: Vec<(f64, &Value)> = f.iter().filter_map(|(i, v)| i.parse::<f64>().ok().map(|i| (i / 30., v))).collect();
            let mut near: Vec<&Value> = fr.iter().filter(|(t, _)| *t >= ra - 0.1 && *t <= rb + 0.4).map(|x| x.1).collect();
            if near.is_empty() {
                if let Some(x) = fr.iter().min_by(|x, y| (x.0 - ra).abs().total_cmp(&(y.0 - ra).abs())) { near.push(x.1); }
            }
            if !near.is_empty() {
                let num = |v: &Value, key: &str| v[key].as_f64();
                let mb = near.iter().filter_map(|v| num(v, "mouth_bottom")).fold(f64::MIN, f64::max);
                // the head top as lib captions reads it: the hair; never > 60 px over the face (a raised hand)
                let ht = near.iter().filter_map(|v| num(v, "head_top").map(|h| num(v, "hair_top").unwrap_or(f64::MIN).max(h - 60.).max(90.))).fold(f64::MAX, f64::min);
                c["mouth_bottom"] = json!((mb * k).round());
                c["head_top"] = json!((ht * k).round());
            }
        }
        out.push(c);
    }
    Ok(out)
}

// ───────────── the skill library (fx) ─────────────
// An effect is one JS file: line 1 `// fx NAME: what it does. keys: k=default …` (the FX list shows it), then
// FX.def(NAME, {keys}, {video, under, over}) (lib edit draws it). The library is engine/js/fx/; a note's new effect
// goes to jobs/JOB/fx/ first and joins the library once the user keeps it (engine/tools/promote.py, the next `aios note`).

fn fx_dirs(dir: &Path) -> [PathBuf; 2] {
    [ai_dir(dir).join("fx"), crate::job::engine_dir().join("js").join("fx")]
}

pub fn fx_file(dir: &Path, name: &str) -> Option<PathBuf> {
    fx_dirs(dir).into_iter().map(|d| d.join(format!("{name}.js"))).find(|p| p.exists())
}

fn fx_names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fx_dirs(dir).iter().flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten())
        .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".js").map(String::from)).collect();
    v.sort();
    v.dedup();
    v
}

// Looks: saved styles (engine/looks/NAME.txt). Line 1 "# look NAME: what it looks like, where it sits", then its
// font lines (paths from engine/looks/) and ONE style line named NAME. A text line uses one by name (style=NAME) with no
// style line in the job; a style line in the job with that name wins. engine/tools/promote.py adds a job's style to the
// library once the user kept the note that made it.
pub fn looks_dir() -> PathBuf {
    std::env::var("AIOS_LOOKS").map(PathBuf::from).unwrap_or_else(|_| crate::job::engine_dir().join("looks"))
}

fn look(name: &str) -> Option<(PathBuf, Vec<Line>)> {
    let f = looks_dir().join(format!("{name}.txt"));
    let text = std::fs::read_to_string(&f).ok()?;
    Some((f, parse_all(&text).into_iter().filter(|l| !l.off).collect()))
}

fn look_files() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(looks_dir()).into_iter().flatten().flatten().map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "txt")).collect();
    v.sort();
    v
}

/// a font id from any look (a style line copied from a look keeps its font= name)
fn look_font(id: &str) -> Option<PathBuf> {
    look_files().iter().find_map(|f| {
        let l = parse_all(&std::fs::read_to_string(f).ok()?).into_iter().find(|l| !l.off && l.kind == "font" && l.id == id)?;
        Some(looks_dir().join(l.text?))
    })
}

/// The LOOKS list for the agent: per look, what it is and its style keys.
pub fn looks_list() -> String {
    let rows: Vec<String> = look_files().iter().filter_map(|f| {
        let text = std::fs::read_to_string(f).ok()?;
        let name = f.file_stem()?.to_string_lossy().to_string();
        let about = text.lines().next().unwrap_or("").trim_start_matches('#').trim().strip_prefix(&format!("look {name}:")).unwrap_or("").trim().to_string();
        let st = parse_all(&text).into_iter().find(|l| !l.off && l.kind == "style" && l.id == name)?;
        let keys: Vec<String> = st.kv.iter().filter(|(k, _)| k != "about").map(|(k, v)| if v == "1" { k.clone() } else { format!("{k}={}", quote(v)) }).collect();
        Some(format!("{name}: {about} [{}]", keys.join(" ")))
    }).collect();
    format!("LOOKS (saved styles: a text line takes style=NAME with no style line; to change one here, write its style line whole, same name): {}\n",
        if rows.is_empty() { "none yet".into() } else { rows.join(" | ") })
}

/// The FX list for the agent: one line per effect, from each file's first line.
pub fn fx_list(dir: &Path) -> String {
    let rows: Vec<String> = fx_names(dir).iter().filter_map(|n| {
        let f = fx_file(dir, n)?;
        let head = std::fs::read_to_string(&f).ok()?.lines().next().unwrap_or("").trim_start_matches('/').trim().to_string();
        let own = if f.starts_with(dir) { " (this job's)" } else { "" };
        Some(format!("{}{own}", head.strip_prefix("fx ").unwrap_or(&head)))
    }).collect();
    format!("FX (named effects: fx ID TAKE A-B \"NAME\" key=value): {}\n", if rows.is_empty() { "none yet".into() } else { rows.join(" | ") })
}

/// The FX and LOOKS names in one line: the brief packet names them, the reference file holds what each does.
pub fn names_list(dir: &Path) -> String {
    let looks: Vec<String> = look_files().iter().filter_map(|f| Some(f.file_stem()?.to_string_lossy().into_owned())).collect();
    format!("FX: {} | LOOKS: {}\n", fx_names(dir).join(" "), looks.join(" "))
}

/// A new effect from a note's answer runs before it is kept: every frame of each fx line that uses it, through the
/// comp (no paint). An error names the second.
fn fx_check(dir: &Path, names: &[String], layout: &Layout) -> Result<()> {
    let job = crate::job::Job::load(&dir.to_string_lossy(), layout)?;
    let assets = crate::assets::Assets::new(job.dir.clone(), job.fps, true, true, 1.);
    let shaper = std::rc::Rc::new(std::cell::RefCell::new(crate::text::Shaper::new()));
    let r = (|| -> Result<()> {
        let mut js = crate::js::Js::new(&job.src, assets.clone(), shaper)?;
        for f in job.cfg["edit"]["fx"].as_array().cloned().unwrap_or_default() {
            if !names.iter().any(|n| f["name"].as_str() == Some(n.as_str())) {
                continue;
            }
            let (a, b) = (f["t0"].as_f64().unwrap_or(0.), f["t1"].as_f64().unwrap_or(0.));
            let mut t = a;
            while t < b {
                js.record(t)?;
                t += 1. / job.fps;
            }
            js.record(b.min(job.dur - 0.5 / job.fps))?;   // just after: it must end cleanly
        }
        Ok(())
    })();
    assets.stop();
    r
}

pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// The notes and the old versions of lines live in notes.txt, so edit.txt is only what plays (fewer tokens to read).
/// In code the two are one text: edit.txt, the line SEP, notes.txt.
pub const NOTES: &str = "notes.txt";
const SEP: &str = "# ── notes.txt: notes and old versions of lines (undo) ──";

/// edit.txt + notes.txt as one text (tidied: an old edit.txt with notes in it reads the same)
pub fn read(dir: &Path) -> Result<String> {
    let e = std::fs::read_to_string(path(dir)).with_context(|| format!("{}", path(dir).display()))?;
    let n = std::fs::read_to_string(ai_dir(dir).join(NOTES)).unwrap_or_default();
    let t = tidy(&format!("{}\n{SEP}\n{n}", e.trim_end()));
    // an older job keeps its notes in edit.txt: moved out once, here
    if !t.starts_with(&format!("{}\n{SEP}\n", e.trim_end())) {
        let _ = write(dir, &t);
    }
    Ok(t)
}

/// Notes go to the notes part; an old version goes there once a newer line with its id is on. A line an undo or redo
/// brings back takes the place of its twin (the version it replaces), so say and scene lines keep their order.
/// A say, use, scene or big line a note turned off stays where it is (its place is its order); other lines come back
/// by their time (insert_at).
const ORDERED: &[&str] = &["say", "use", "moment", "big", "broll"];
fn tidy(text: &str) -> String {
    let rows: Vec<&str> = text.lines().collect();
    let cut = rows.iter().position(|r| r.trim() == SEP).unwrap_or(rows.len());
    let mut edit: Vec<String> = rows[..cut].iter().map(|s| s.to_string()).collect();
    let mut notes: Vec<String> = rows.get(cut + 1..).unwrap_or(&[]).iter().filter(|r| !r.trim().is_empty()).map(|s| s.to_string()).collect();
    let mut back = vec![];
    notes.retain(|r| match parse_line(0, r) {
        Some(l) if l.kind != "note" && l.mark.is_empty() => { back.push(r.clone()); false }
        _ => true,
    });
    for r in back {
        let l = parse_line(0, &r).unwrap();
        let twin = edit.iter().position(|e| parse_line(0, e).map(|x| x.kind == l.kind && x.id == l.id && !x.mark.is_empty()).unwrap_or(false));
        // in its twin's place with its indent, else by its time with the indent of the block it lands in
        let (at, row) = match twin {
            Some(at) => (at, keep_indent(&edit[at], &r)),
            None => { let at = insert_at(&edit, &l); (at, format!("{}{}", indent_at(&edit, at, &l), r.trim_start())) }
        };
        edit.insert(at, row);
    }
    let on: std::collections::HashSet<(String, String)> = edit.iter().filter_map(|r| parse_line(0, r)).filter(|l| !l.off).map(|l| (l.kind, l.id)).collect();
    // a line in a moment block a note turned off stays in its block, as an ordered line does
    let parsed: std::collections::HashMap<usize, Line> = parse_all(&edit.join("\n")).into_iter().map(|l| (l.n, l)).collect();
    let mut keep = vec![];
    for (n, r) in edit.into_iter().enumerate() {
        match parsed.get(&n) {
            Some(l) if l.kind == "note" || (!l.mark.is_empty() && (on.contains(&(l.kind.clone(), l.id.clone())) || (!ORDERED.contains(&l.kind.as_str()) && l.moment.is_none()))) => notes.push(r),
            _ => keep.push(r),
        }
    }
    while keep.last().map(|r| r.trim().is_empty()).unwrap_or(false) {
        keep.pop();
    }
    format!("{}\n{SEP}\n{}", keep.join("\n"), notes.join("\n"))
}

fn write_file(p: &Path, text: &str) -> Result<()> {
    let tmp = p.with_extension("txt.tmp");
    let t = format!("{}\n", text.trim_end_matches('\n'));
    std::fs::write(&tmp, t)?;
    std::fs::rename(&tmp, p)?;
    Ok(())
}

fn write(dir: &Path, text: &str) -> Result<()> {
    let t = tidy(text);
    let (e, n) = t.split_once(&format!("\n{SEP}\n")).unwrap_or((&t, ""));
    write_file(&path(dir), e)?;
    if !n.trim().is_empty() || ai_dir(dir).join(NOTES).exists() {
        write_file(&ai_dir(dir).join(NOTES), n)?;
    }
    Ok(())
}

/// tidy, check that it compiles, write. Returns the text as written.
fn save(dir: &Path, text: &str, layout: &Layout) -> Result<String> {
    let t = tidy(text);
    compile(dir, &t, false, layout)?;
    write(dir, &t)?;
    Ok(t)
}

// ───────────── changes from the page ─────────────

/// `#` on or off for one line (the page's toggle). `raw` must still be the line's text (no race with another edit).
pub fn toggle(dir: &Path, n: usize, raw: &str, off: bool, layout: &Layout) -> Result<Value> {
    let text = read(dir)?;
    let mut rows: Vec<String> = text.lines().map(String::from).collect();
    let cur = rows.get(n).ok_or_else(|| anyhow!("no line {}", n + 1))?;
    if cur.trim() != raw.trim() {
        return Err(anyhow!("line {} changed meanwhile; try again", n + 1));
    }
    let l = parse_line(n, cur).ok_or_else(|| anyhow!("line {} is a comment", n + 1))?;
    if l.kind == "note" {
        return Err(anyhow!("a note is not turned off; undo it instead"));
    }
    if l.off == off {
        return Ok(json!({ "ok": true }));
    }
    let before = cur.clone();
    rows[n] = keep_indent(cur, &if off { format!("# {}", cur.trim()) } else { l.body() });
    let new = rows.join("\n");
    save(dir, &new, layout)?;
    push_undo(dir, n, &before, &rows[n], &format!("{} {} {}", if off { "off" } else { "on" }, l.kind, l.id))?;
    Ok(json!({ "ok": true, "line": rows[n] }))
}

/// One line changed by the page's controls. `set`: {"a": raw start, "b": raw end, "text": "…", KEY: value}; a value
/// null, false or "" takes the key away, true writes the flag, [x, y] writes "x,y". `raw` must still be the line's text.
pub fn set_line(dir: &Path, n: usize, raw: &str, set: &Map<String, Value>, layout: &Layout) -> Result<Value> {
    let text = read(dir)?;
    let mut rows: Vec<String> = text.lines().map(String::from).collect();
    let cur = rows.get(n).ok_or_else(|| anyhow!("no line {}", n + 1))?.clone();
    if cur.trim() != raw.trim() {
        return Err(anyhow!("line {} changed meanwhile; try again", n + 1));
    }
    let mut l = parse_line(n, &cur).ok_or_else(|| anyhow!("line {} is a comment", n + 1))?;
    if l.kind == "note" || !l.mark.is_empty() {
        return Err(anyhow!("line {} is a note or an old version; it is not changed here", n + 1));
    }
    let num = |x: f64| { let s = format!("{:.3}", x); s.trim_end_matches('0').trim_end_matches('.').to_string() };
    for (k, v) in set {
        match k.as_str() {
            "a" => l.t0 = Some(v.as_f64().ok_or_else(|| anyhow!("a: a number of seconds"))?),
            "b" => l.t1 = Some(v.as_f64().ok_or_else(|| anyhow!("b: a number of seconds"))?),
            "text" => l.text = v.as_str().map(String::from),
            _ => {
                if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') {
                    return Err(anyhow!("no key {k:?}"));
                }
                let val = match v {
                    Value::Null | Value::Bool(false) => None,
                    Value::Bool(true) => Some("1".to_string()),
                    Value::Number(x) => Some(num(x.as_f64().unwrap_or(0.))),
                    Value::String(x) => Some(x.trim().to_string()).filter(|x| !x.is_empty()),
                    Value::Array(a) => Some(a.iter().map(|x| x.as_f64().map(num).unwrap_or_default()).collect::<Vec<_>>().join(",")),
                    Value::Object(_) => return Err(anyhow!("{k}: not a value")),
                };
                match val {
                    Some(x) => l.set(k, &x),
                    None => l.kv.retain(|(a, _)| a != k),
                }
            }
        }
    }
    if let (Some(a), Some(b)) = (l.t0, l.t1) {
        if b <= a {
            return Err(anyhow!("the end must come after the start"));
        }
    }
    rows[n] = keep_indent(&cur, &fmt_line(&l));
    if rows[n].trim() == cur.trim() {
        return Ok(json!({ "ok": true, "line": rows[n] }));
    }
    let new = rows.join("\n");
    save(dir, &new, layout)?;
    push_undo(dir, n, &cur, &rows[n], &format!("{} {}", l.kind, l.id))?;
    Ok(json!({ "ok": true, "line": rows[n] }))
}

// The page's own changes (a toggle, a control) are undone line by line: .cache/lines/STAMP.json = {n, before, after,
// what}. An undo puts `before` back only while the line is still `after`, so a note applied since is never lost.
fn lines_dir(dir: &Path) -> PathBuf {
    cache_dir(dir).join("lines")
}

fn stamps_in(d: &Path) -> Vec<u128> {
    let mut v: Vec<u128> = std::fs::read_dir(d).map(|r| r.flatten().filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".json").and_then(|x| x.parse().ok())).collect()).unwrap_or_default();
    v.sort();
    v
}

fn push_to(d: &Path, rec: &Value) -> Result<()> {
    std::fs::create_dir_all(d)?;
    let mut s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|x| x.as_millis()).unwrap_or(0);
    while d.join(format!("{s}.json")).exists() {
        s += 1;
    }
    std::fs::write(d.join(format!("{s}.json")), rec.to_string())?;
    Ok(())
}

fn push_undo(dir: &Path, n: usize, before: &str, after: &str, what: &str) -> Result<()> {
    let d = lines_dir(dir);
    let _ = std::fs::remove_dir_all(d.join("redo"));
    push_to(&d, &json!({ "n": n, "before": before, "after": after, "what": what }))
}

/// The page's last change back (`redo` = again). Returns what it was ("text t1").
pub fn undo_line(dir: &Path, redo: bool, layout: &Layout) -> Result<String> {
    let d = lines_dir(dir);
    let (from, to) = if redo { (d.join("redo"), d.clone()) } else { (d.clone(), d.join("redo")) };
    let Some(last) = stamps_in(&from).pop() else { return Err(anyhow!("nothing to {}", if redo { "redo" } else { "undo" })) };
    let f = from.join(format!("{last}.json"));
    let rec: Value = serde_json::from_str(&std::fs::read_to_string(&f)?)?;
    let (want, put) = if redo { (&rec["before"], &rec["after"]) } else { (&rec["after"], &rec["before"]) };
    let (want, put) = (want.as_str().unwrap_or(""), put.as_str().unwrap_or(""));
    let text = read(dir)?;
    let mut rows: Vec<String> = text.lines().map(String::from).collect();
    // the line may have moved (a note line added above it): find it by its text
    let n = rec["n"].as_u64().unwrap_or(0) as usize;
    let at = if rows.get(n).map(|r| r.trim() == want.trim()).unwrap_or(false) { Some(n) } else { rows.iter().position(|r| r.trim() == want.trim()) };
    let what = rec["what"].as_str().unwrap_or("").to_string();
    let Some(at) = at else {
        std::fs::remove_file(&f)?;
        return Err(anyhow!("{what} changed since (a note?); nothing undone"));
    };
    rows[at] = put.to_string();
    let new = rows.join("\n");
    save(dir, &new, layout)?;
    std::fs::remove_file(&f)?;
    push_to(&to, &rec)?;
    Ok(what)
}

/// where a line goes: after the last line of the same clip that starts at or before it (top lines: after the top)
fn insert_at(rows: &[String], l: &Line) -> usize {
    let parsed: Vec<Line> = rows.iter().enumerate().filter_map(|(n, r)| parse_line(n, r)).collect();
    if l.top() {
        return parsed.iter().filter(|p| p.top()).map(|p| p.n + 1).max().unwrap_or(0);
    }
    let t = l.t0.unwrap_or(0.);
    let before = parsed.iter().filter(|p| !p.top() && p.clip == l.clip && p.t0.map(|x| x <= t + 1e-6).unwrap_or(false)).map(|p| p.n + 1).max();
    before.or_else(|| parsed.iter().filter(|p| !p.top()).map(|p| p.n).min()).unwrap_or(rows.len())
}

/// A new note from the page at output second `t` -> its line in edit.txt (status new). Returns the note id.
/// `at`: where the user wrote it on the moment page (moment=ID, file=, sec= the playhead second in that file).
pub fn add_note(dir: &Path, t: f64, text: &str, covers: &[String], at: &[(&str, String)], layout: &Layout) -> Result<String> {
    let src = read(dir)?;
    let c = compile(dir, &src, false, layout)?;
    let (clip, rt) = to_raw(&c.segs, t).ok_or_else(|| anyhow!("no clip at {t:.2} s"))?;
    let parsed = parse_all(&src);
    let no = parsed.iter().filter(|l| l.kind == "note").map(|l| l.note_no()).max().unwrap_or(0) + 1;
    let mut l = Line { kind: "note".into(), id: format!("n{no}"), clip: Some(clip), t0: Some((rt * 100.).round() / 100.), text: Some(text.trim().to_string()), ..Default::default() };
    if !covers.is_empty() {
        l.set("covers", &covers.join(","));
    }
    for (k, v) in at.iter().filter(|(_, v)| !v.is_empty()) {
        l.set(k, v);
    }
    l.set("status", "new");
    let mut rows: Vec<String> = src.lines().map(String::from).collect();
    let at = insert_at(&rows, &l);
    rows.insert(at, fmt_line(&l));
    write(dir, &rows.join("\n"))?;
    Ok(l.id)
}

/// Undo what note `id` changed: its new lines go off ("#nK>"), its old lines come back.
pub fn undo_note(dir: &Path, id: &str, layout: &Layout) -> Result<Value> {
    let text = read(dir)?;
    let mut rows: Vec<String> = text.lines().map(String::from).collect();
    let lines = parse_all(&text);
    let note = lines.iter().find(|l| l.kind == "note" && l.id == id && !l.off).ok_or_else(|| anyhow!("no note {id}"))?;
    let no = note.note_no();
    let changed: Vec<String> = note.get("changed").unwrap_or("").split(',').filter(|s| !s.is_empty()).map(String::from).collect();
    if changed.is_empty() || note.get("status") != Some("done") {
        return Err(anyhow!("note {id} has no change to undo"));
    }
    // ids repeat across kinds (fx m1, song m1): the kind of this note's old or undone version decides
    let kind = |x: &str| lines.iter().find(|l| l.id == x && (l.mark == format!("n{no}<") || l.mark == format!("n{no}>"))).map(|l| l.kind.clone());
    let is = |l: &Line, x: &str| l.id == x && !l.off && l.kind != "note" && kind(x).map(|k| k == l.kind).unwrap_or(true);
    let active = |x: &str| lines.iter().find(|l| is(l, x) && !l.top()).or_else(|| lines.iter().find(|l| is(l, x) && l.top()));
    let old = |x: &str| lines.iter().find(|l| l.off && l.mark == format!("n{no}<") && l.id == x);
    for c in &changed {
        let (op, x) = match c.chars().next() {
            Some('+') | Some('-') | Some('^') => (c.chars().next().unwrap(), &c[1..]),
            _ => ('=', c.as_str()),
        };
        // a later note changed the same line: undo that one first
        if let Some(later) = lines.iter().find(|l| l.id == x && l.mark.ends_with('<') && l.mark[1..l.mark.len() - 1].parse::<u64>().unwrap_or(0) > no) {
            return Err(anyhow!("note {} changed {x} later: undo it first", &later.mark[..later.mark.len() - 1]));
        }
        if op == '=' || op == '+' || op == '^' {
            if let Some(a) = active(x) {
                rows[a.n] = keep_indent(&a.raw, &if op == '^' { format!("# {}", a.body()) } else { format!("#n{no}> {}", a.body()) });
            }
        }
        if op == '=' || op == '-' {
            if let Some(o) = old(x) {
                rows[o.n] = keep_indent(&o.raw, &o.body());
            }
        }
    }
    let mut n2 = note.clone();
    n2.set("status", "undone");
    rows[note.n] = fmt_line(&n2);
    let new = rows.join("\n");
    save(dir, &new, layout)?;
    Ok(json!({ "undone": id, "lines": changed }))
}

// ───────────── the agent's answer for a note -> the changed lines ─────────────

enum Op {
    Put(Line),
    Off(String),
    On(String),
}

/// Put back what undo took away (the note was undone by mistake): the "#nN>" lines go on again, the lines the
/// undo put back go under "#nN<" again.
pub fn redo_note(dir: &Path, id: &str, layout: &Layout) -> Result<Value> {
    let text = read(dir)?;
    let mut rows: Vec<String> = text.lines().map(String::from).collect();
    let lines = parse_all(&text);
    let note = lines.iter().find(|l| l.kind == "note" && l.id == id && !l.off).ok_or_else(|| anyhow!("no note {id}"))?;
    let no = note.note_no();
    let changed: Vec<String> = note.get("changed").unwrap_or("").split(',').filter(|s| !s.is_empty()).map(String::from).collect();
    if changed.is_empty() || note.get("status") != Some("undone") {
        return Err(anyhow!("note {id} is not undone"));
    }
    // ids repeat across kinds (fx m1, song m1): the kind of this note's old or undone version decides
    let kind = |x: &str| lines.iter().find(|l| l.id == x && (l.mark == format!("n{no}<") || l.mark == format!("n{no}>"))).map(|l| l.kind.clone());
    let is = |l: &Line, x: &str| l.id == x && !l.off && l.kind != "note" && kind(x).map(|k| k == l.kind).unwrap_or(true);
    let active = |x: &str| lines.iter().find(|l| is(l, x) && !l.top()).or_else(|| lines.iter().find(|l| is(l, x) && l.top()));
    let gone = |x: &str| lines.iter().find(|l| l.off && l.mark == format!("n{no}>") && l.id == x);
    for c in &changed {
        let (op, x) = match c.chars().next() {
            Some('+') | Some('-') | Some('^') => (c.chars().next().unwrap(), &c[1..]),
            _ => ('=', c.as_str()),
        };
        // a later note changed the same line meanwhile: its change would be lost
        if let Some(later) = lines.iter().find(|l| l.id == x && l.mark.ends_with('<') && l.mark[1..l.mark.len() - 1].parse::<u64>().unwrap_or(0) > no) {
            return Err(anyhow!("note {} changed {x} after the undo: undo it first", &later.mark[..later.mark.len() - 1]));
        }
        if op == '=' || op == '-' {
            if let Some(a) = active(x) {
                rows[a.n] = keep_indent(&a.raw, &format!("#n{no}< {}", a.body()));
            }
        }
        if op == '=' || op == '+' {
            if let Some(g) = gone(x) {
                rows[g.n] = keep_indent(&g.raw, &g.body());
            }
        }
        if op == '^' {
            if let Some(o) = lines.iter().find(|l| l.off && l.mark.is_empty() && l.id == x) {
                rows[o.n] = keep_indent(&o.raw, &o.body());
            }
        }
    }
    let mut n2 = note.clone();
    n2.set("status", "done");
    rows[note.n] = fmt_line(&n2);
    let new = rows.join("\n");
    save(dir, &new, layout)?;
    Ok(json!({ "redone": id, "lines": changed }))
}

struct Reply {
    ops: Vec<Op>,
    /// `js NAME` … `end js`: a new effect for the skill library (written to jobs/JOB/fx/NAME.js)
    js: Vec<(String, String)>,
    reply: Option<String>,
    /// `ask: 1) … 2) …`: the note is not clear; it gets status=ask and the options as its reply, nothing changes
    ask: bool,
    /// `main: what I lack`: the notes agent cannot do it right (data it needs is missing or looks wrong); status=main,
    /// nothing changes, the main chat takes it (user pick 2026-10-12: route up, then teach the agent)
    main: bool,
}

fn read_reply(s: &str) -> Result<Reply> {
    let mut r = Reply { ops: vec![], js: vec![], reply: None, ask: false, main: false };
    let mut code: Option<(String, Vec<String>)> = None;
    for raw in s.lines() {
        let t = raw.trim();
        if let Some((name, body)) = code.as_mut() {
            if t == "end js" {
                r.js.push((name.clone(), body.join("\n")));
                code = None;
            } else if !t.starts_with("```") {
                body.push(raw.to_string());
            }
            continue;
        }
        if let Some(name) = t.strip_prefix("js ") {
            let name = name.trim().to_string();
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
                return Err(anyhow!("js NAME: a-z, 0-9 and _ only ({name:?})"));
            }
            code = Some((name, vec![]));
            continue;
        }
        // a list bullet in front ("- reply: …") is not part of the line
        let t = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).unwrap_or(t).trim();
        if t.is_empty() || t.starts_with("```") {
            continue;
        }
        if let Some(x) = t.strip_prefix("reply:") {
            r.reply = Some(x.trim().to_string());
        } else if let Some(x) = t.strip_prefix("ask:") {
            r.reply = Some(x.trim().to_string());
            r.ask = true;
        } else if let Some(x) = t.strip_prefix("main:") {
            r.reply = Some(format!("for the main chat: {}", x.trim()));
            r.ask = true;
            r.main = true;
        } else if let Some(x) = t.strip_prefix("off ") {
            r.ops.push(Op::Off(x.trim().to_string()));
        } else if let Some(x) = t.strip_prefix("on ") {
            r.ops.push(Op::On(x.trim().to_string()));
        } else if let Some(mut l) = parse_line(0, t) {
            if l.off || l.kind == "note" {
                return Err(anyhow!("not a line to write: {t}"));
            }
            l.raw = t.to_string();
            r.ops.push(Op::Put(l));
        } else {
            return Err(anyhow!("line not understood: {t}"));
        }
    }
    if let Some((name, _)) = code {
        return Err(anyhow!("js {name}: no `end js` line"));
    }
    if r.ops.is_empty() && r.reply.is_none() {
        return Err(anyhow!("an empty answer"));
    }
    if r.ask && !r.ops.is_empty() {
        return Err(anyhow!("an ask changes nothing: give the options only"));
    }
    Ok(r)
}

/// The ops written into the file text; the note gets status, reply and what it changed. Errors leave nothing written.
fn apply_ops(dir: &Path, text: &str, id: &str, r: &Reply, layout: &Layout) -> Result<(String, Vec<String>)> {
    let mut rows: Vec<String> = text.lines().map(String::from).collect();
    let note = parse_all(text).into_iter().find(|l| l.kind == "note" && l.id == id && !l.off).ok_or_else(|| anyhow!("no note {id}"))?;
    let no = note.note_no();
    let mut changed = vec![];
    for op in &r.ops {
        let lines: Vec<Line> = rows.iter().enumerate().filter_map(|(n, x)| parse_line(n, x)).collect();
        let active = |x: &str, kind: Option<&str>| lines.iter().find(|l| !l.off && l.kind != "note" && l.id == x && kind.map(|k| k == l.kind).unwrap_or(true)).cloned();
        match op {
            Op::Put(l) => match active(&l.id, Some(&l.kind)).or_else(|| if l.top() { None } else { active(&l.id, None) }) {
                Some(old) => {
                    if old.body() == l.raw.trim() {
                        continue;
                    }
                    if old.kind != l.kind {
                        return Err(anyhow!("{} is a {} line, not a {}", l.id, old.kind, l.kind));
                    }
                    rows[old.n] = keep_indent(&old.raw, &format!("#n{no}< {}", old.body()));
                    rows.insert(old.n + 1, keep_indent(&old.raw, l.raw.trim()));
                    changed.push(l.id.clone());
                }
                None => {
                    let at = insert_at(&rows, l);
                    rows.insert(at, format!("{}{}", indent_at(&rows, at, l), l.raw.trim()));
                    changed.push(format!("+{}", l.id));
                }
            },
            Op::Off(x) => {
                let old = active(x, None).ok_or_else(|| anyhow!("off {x}: no such line is on"))?;
                rows[old.n] = keep_indent(&old.raw, &format!("#n{no}< {}", old.body()));
                changed.push(format!("-{x}"));
            }
            Op::On(x) => {
                let old = lines.iter().find(|l| l.off && l.mark.is_empty() && l.id == *x).ok_or_else(|| anyhow!("on {x}: no line {x} is off"))?;
                rows[old.n] = keep_indent(&old.raw, &old.body());
                changed.push(format!("^{x}"));
            }
        }
    }
    // the note: status, reply, what it changed
    let l = rows.iter().enumerate().filter_map(|(n, x)| parse_line(n, x)).find(|l| l.kind == "note" && l.id == id && !l.off).unwrap();
    let mut l2 = l.clone();
    l2.set("status", if r.main { "main" } else if r.ask { "ask" } else { "done" });
    l2.set("reply", r.reply.as_deref().unwrap_or("done"));
    if !changed.is_empty() {
        l2.set("changed", &changed.join(","));
    }
    rows[l.n] = fmt_line(&l2);
    let new = tidy(&rows.join("\n"));
    compile(dir, &new, false, layout)?;
    Ok((new, changed))
}

fn near(l: &Line, clip: &str, t: f64) -> bool {
    if l.clip.as_deref() != Some(clip) {
        return false;
    }
    let (a, b) = (l.t0.unwrap_or(0.), l.t1.unwrap_or(l.t0.unwrap_or(0.)));
    b >= t - NEAR && a <= t + NEAR
}

/// The words around output second `t`, three each side: the word spoken at `t` in [brackets], a `|` where `t` falls
/// in a pause. None after the last word. `words` = the compile's output words, in time order.
fn words_at(words: &[Value], t: f64) -> Option<String> {
    const AROUND: usize = 3;
    let span = |w: &Value| (w["start"].as_f64().unwrap_or(0.), w["end"].as_f64().unwrap_or(0.));
    let next = words.iter().position(|w| span(w).1 > t)?;
    let inside = span(&words[next]).0 <= t + 1e-6;
    let mut out: Vec<String> = vec![];
    for (i, w) in words.iter().enumerate().take((next + AROUND + usize::from(inside)).min(words.len())).skip(next.saturating_sub(AROUND)) {
        let w = w["word"].as_str().unwrap_or("").trim();
        match (i == next, inside) {
            (true, true) => out.push(format!("[{w}]")),
            (true, false) => out.extend(["|".to_string(), w.to_string()]),
            _ => out.push(w.to_string()),
        }
    }
    Some(out.join(" "))
}

/// What the agent reads for a note: the top lines, the lines near the note (all lines for "all / every …"), the note
/// with its page second (`out`) and the words spoken there.
fn packet(text: &str, note: &Line, near_scenes: &[usize], out: Option<f64>, words: &[Value]) -> String {
    let lines = parse_all(text);
    let clip = note.clip.clone().unwrap_or_default();
    let t = note.t0.unwrap_or(0.);
    let note_text = note.text.clone().unwrap_or_default();
    let all = note_text.split(|c: char| !c.is_alphanumeric()).any(|w| ["all", "every", "each", "whole", "everything", "always", "everywhere"].contains(&w.to_lowercase().as_str()));
    let covers: Vec<&str> = note.get("covers").unwrap_or("").split(',').filter(|s| !s.is_empty()).collect();
    let mut p = match (note.get("status"), note.get("reply")) {
        (Some("main"), Some(r)) => format!("THE NOTES AGENT ROUTED IT UP ({r}). Do it, then teach (HOW TO APPLY, last point).\n"),
        _ => String::new(),
    };
    p.push_str("TOP LINES:\n");
    for l in lines.iter().filter(|l| l.top() && !l.off) {
        p.push_str(l.raw.trim());
        p.push('\n');
    }
    // every id in the file, so a new line never takes an id that is in use (n7 wrote over t1, 2026-10-12)
    let mut used: Vec<String> = vec![];
    for k in ["moment", "big", "broll", "text", "pic", "fx", "cap", "skip", "use", "say", "style", "font"] {
        let ids: Vec<&str> = lines.iter().filter(|l| l.kind == k).map(|l| l.id.as_str()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        if !ids.is_empty() && !["cap", "say", "skip"].contains(&k) {
            used.push(format!("{k} {}", ids.join(" ")));
        }
    }
    p.push_str(&format!("IDS IN USE (a new line takes a new id): {}\n", used.join(" | ")));
    if all {
        p.push_str("\nALL LINES:\n");
    } else {
        p.push_str(&format!("\nLINES NEAR {clip} {:.2} (±{NEAR:.0} s):\n", t));
    }
    // a moment near the note brings its whole block (indented, as in the file)
    for l in lines.iter().filter(|l| !l.top() && l.kind != "note" && l.mark.is_empty()) {
        let in_near = l.moment.as_ref().is_some_and(|m| near_scenes.contains(&m.0));
        if all || near(l, &clip, t) || covers.contains(&l.id.as_str()) || near_scenes.contains(&l.n) || in_near {
            p.push_str(if l.moment.is_some() { l.raw.trim_end() } else { l.raw.trim() });
            p.push('\n');
        }
    }
    p.push_str("\nON SCREEN AT THE NOTE:");
    let on: Vec<String> = lines.iter().filter(|l| !l.off && l.kind == "text" && l.clip.as_deref() == Some(clip.as_str())
        && l.t0.unwrap_or(0.) <= t + 1e-6 && t < l.t1.unwrap_or(0.)).map(|l| format!(" {} \"{}\"", l.id, l.text.clone().unwrap_or_default())).collect();
    p.push_str(&if on.is_empty() { " (no text)".to_string() } else { on.join(",") });
    if let Some(sl) = lines.iter().find(|l| !l.off && l.kind == "say" && l.clip.as_deref() == Some(clip.as_str()) && l.t0.unwrap_or(0.) <= t + 1e-6 && t < l.t1.unwrap_or(0.)) {
        p.push_str(&format!("\nSAID AT THE NOTE: {} \"{}\"", sl.id, sl.text.clone().unwrap_or_default()));
    }
    if !covers.is_empty() {
        p.push_str(&format!("\nTHE USER'S DRAWING COVERS: {}", covers.join(", ")));
    }
    let on_page = match (note.get("moment"), note.get("file")) {
        (Some(m), Some(f)) => format!(", on {m}, written on {f}{}", note.get("sec").map(|x| format!(" at {x} s")).unwrap_or_default()),
        (Some(m), None) => format!(", on {m}"),
        _ => String::new(),
    };
    let page = out.map(|o| format!(" = page {o:.2} s")).unwrap_or_default();
    p.push_str(&format!("\n\nTHE NOTE ({} at {clip} {:.2}{page}{on_page}): \"{}\"\n", note.id, t, note_text));
    if let Some(w) = out.and_then(|o| words_at(words, o)) {
        p.push_str(&format!("WORDS AT THE NOTE: {w}\n"));
    }
    p
}

/// `aios note JOB ID --apply FILE`: the agent's answer for one note, written into the file. The answer is the
/// same shape a note's reply always had: whole lines by id, `off ID`, `on ID`, `reply: …`.
/// All of it goes in or nothing does; each changed line keeps its old self as "#nN< …" (one-click undo).
pub fn apply_answer(dir: &Path, id: &str, answer: &str, layout: &Layout) -> Result<Value> {
    let text = read(dir)?;
    let note = parse_all(&text).into_iter().find(|l| l.kind == "note" && l.id == id && !l.off).ok_or_else(|| anyhow!("no note {id}"))?;
    if !matches!(note.get("status"), Some("new") | Some("ask") | Some("main")) {
        return Err(anyhow!("note {id} is already applied (undo it first)"));
    }
    let mut r = read_reply(answer)?;
    // a main note answered with words only (a list, a question back) still waits for the user: it stays main
    // (user pick 2026-10-12, after n31 went "done" on a song list)
    if note.get("status") == Some("main") && r.ops.is_empty() && r.js.is_empty() && !r.ask {
        r.ask = true;
        r.main = true;
    }
    if r.ask && !r.js.is_empty() {
        return Err(anyhow!("an ask changes nothing: no js"));
    }
    // a new effect: its file first (the compile looks for it), the old one kept to put back on an error
    let mut olds: Vec<(PathBuf, Option<String>)> = vec![];
    for (name, body) in &r.js {
        if crate::job::engine_dir().join("js").join("fx").join(format!("{name}.js")).exists() {
            return Err(anyhow!("fx {name} is in the library: use it, or give the new one another name"));
        }
        let f = ai_dir(dir).join("fx").join(format!("{name}.js"));
        std::fs::create_dir_all(f.parent().unwrap())?;
        olds.push((f.clone(), std::fs::read_to_string(&f).ok()));
        let mut body = body.trim_matches('\n').to_string();
        if !body.starts_with("// fx ") {
            body = format!("// fx {name}: (no description) keys: -\n{body}");
        }
        let (head, rest) = body.split_once('\n').unwrap_or((&body, ""));
        std::fs::write(&f, format!("{head}\n// made by note {id} ({})\n{rest}\n", dir.file_name().unwrap_or_default().to_string_lossy()))?;
    }
    let put_back = |olds: &[(PathBuf, Option<String>)]| for (f, o) in olds {
        let _ = match o { Some(x) => std::fs::write(f, x), None => std::fs::remove_file(f) };
    };
    let (new, changed) = match apply_ops(dir, &text, id, &r, layout) {
        Ok(x) => x,
        Err(e) => { put_back(&olds); return Err(e); }
    };
    write(dir, &new)?;
    if !r.js.is_empty() {
        let names: Vec<String> = r.js.iter().map(|x| x.0.clone()).collect();
        if let Err(e) = fx_check(dir, &names, layout) {
            write(dir, &text)?;
            put_back(&olds);
            return Err(anyhow!("the new fx failed when it ran, nothing kept: {e:#}"));
        }
    }
    Ok(json!({ "note": id, "status": if r.main { "main" } else if r.ask { "ask" } else { "done" }, "changed": changed, "fx": r.js.iter().map(|x| x.0.clone()).collect::<Vec<_>>() }))
}

const FILLERS: &[&str] = &["um", "uh", "er", "erm", "ah", "hmm", "mm", "mhm"];

/// `aios take JOB CLIP`: a new edit-file job from one raw take, made from its word times (no cut file, nothing
/// thrown away): one `say` line per sentence (the lines tile the take, so "#" on one cuts that sentence and its
/// pause), and `skip` lines for long pauses (a word gap of 0.45 s or more, 0.12 s kept each side), the start and end
/// silences and filler words. All on; the user turns any one off on the page to get it back.
/// Word gaps, not the sound level: a noisy room has no quiet stretch (a test job: silence.py found 0 pauses).
/// `add`: the take joins a job that has one already, as the next clip letter; its lines go after the others.
/// `script`: the script file; nv.py asr prints where the take differs (CrisperWhisper takes no hotwords: user, 2026-10-29).
pub fn take(dir: &Path, clip: &Path, words: Option<&Path>, script: Option<&Path>, size: (u32, u32), plain: bool, relight: bool, add: bool, layout: &Layout) -> Result<String> {
    let have = if path(dir).exists() {
        if !add {
            return Err(anyhow!("{} exists already (a take starts a new job; --add puts this take after the others)", path(dir).display()));
        }
        parse_all(&read(dir)?)
    } else {
        vec![]
    };
    let letter = ('A'..='Z').find(|c| !have.iter().any(|l| l.kind == "take" && l.id == c.to_string())).ok_or_else(|| anyhow!("no free take letter"))?;
    // ids go on from the job's last say / skip number
    let last = |p: char| have.iter().filter(|l| l.id.starts_with(p)).filter_map(|l| l.id[1..].parse::<usize>().ok()).max().unwrap_or(0);
    let (sn0, k0) = (last('s'), last('k'));
    let mut k = k0;
    let size = have.iter().find(|l| l.kind == "size").and_then(|l| l.id.split_once('x')).and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))).unwrap_or(size);
    // a new job is in the new layout (_ai/); a take added to an older job keeps its layout
    if have.is_empty() {
        std::fs::create_dir_all(dir.join("_ai"))?;
    }
    let wrel = if dir.join("_ai").is_dir() { format!("_ai/words/{letter}.json") } else { format!("{letter}.words.json") };
    let wf = dir.join(&wrel);
    std::fs::create_dir_all(wf.parent().unwrap())?;
    match words {
        Some(w) => { std::fs::copy(w, &wf).with_context(|| format!("{}", w.display()))?; }
        None => {
            let nv = layout.dir(Key::Skills).join("nvidia/scripts/nv.py");
            let mut c = Command::new("python3");
            c.arg(&nv).arg("asr").arg(clip).arg("-o").arg(&wf).arg("--overwrite");
            if let Some(s) = script {
                c.arg("--script").arg(s);
            }
            let st = c.status()?;
            if !st.success() {
                return Err(anyhow!("nv.py asr failed on {}", clip.display()));
            }
        }
    }
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&wf)?)?;
    let ws: Vec<(String, f64, f64)> = v.as_array().or_else(|| v["words"].as_array()).cloned().unwrap_or_default().iter()
        .map(|w| (w["word"].as_str().unwrap_or("").trim().to_string(), w["start"].as_f64().unwrap_or(0.), w["end"].as_f64().unwrap_or(0.)))
        .filter(|w| !w.0.is_empty()).collect();
    if ws.is_empty() {
        return Err(anyhow!("no words in {}", wf.display()));
    }
    let pr = crate::video::probe(&clip.to_string_lossy())?;
    let dur = pr.duration;
    let up = |t: f64| (t * 100.).ceil() / 100.;
    let down = |t: f64| (t * 100.).floor() / 100.;
    // sentences: a word that ends with . ? ! closes one
    let mut sents: Vec<(usize, usize)> = vec![];
    let mut s0 = 0;
    for (i, w) in ws.iter().enumerate() {
        let end = w.0.trim_end_matches(['"', '\'', ')', ']']);
        if end.ends_with(['.', '?', '!']) || i + 1 == ws.len() {
            sents.push((s0, i));
            s0 = i + 1;
        }
    }
    // skips
    let mut skips: Vec<(f64, f64)> = vec![];
    if ws[0].1 - 0.12 >= 0.25 {
        skips.push((0., down(ws[0].1 - 0.12)));
    }
    for p in ws.windows(2) {
        let (a, b) = (up(p[0].2 + 0.12), down(p[1].1 - 0.12));
        if p[1].1 - p[0].2 >= 0.45 && b - a >= 0.2 {
            skips.push((a, b));
        }
    }
    for w in &ws {
        let n: String = w.0.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect();
        if FILLERS.contains(&n.as_str()) {
            skips.push((down((w.1 - 0.03).max(0.)), up(w.2 + 0.03)));
        }
    }
    let last = ws.last().unwrap().2;
    if dur - (last + 0.25) >= 0.25 {
        skips.push((up(last + 0.25), down(dur)));
    }
    skips.sort_by(|a, b| a.0.total_cmp(&b.0));
    // the file
    let real = std::fs::canonicalize(clip)?;
    let root = std::fs::canonicalize(layout.root())?;
    let jd = std::fs::canonicalize(dir)?;
    // a take inside the job (media/) by its path in the job, so the job folder can move
    let rel = match (real.strip_prefix(&jd), real.strip_prefix(&root), jd.strip_prefix(&root)) {
        (Ok(r), _, _) => r.display().to_string(),
        (_, Ok(r), Ok(j)) => format!("{}{}", "../".repeat(j.components().count()), r.display()),
        _ => real.display().to_string(),
    };
    let mut keys = String::new();
    if !plain {
        keys.push_str(if relight { " voice relight" } else { " voice" });
        let (cw, ch) = (pr.width as f64, pr.height as f64);
        if ((cw / ch) / (size.0 as f64 / size.1 as f64) - 1.).abs() > 0.01 {
            keys.push_str(" frame=locked");
        }
    }
    let top = vec![
        format!("take   {letter} \"{}\"{keys}", rel.replace('"', "\\\"")),
        format!("words  {letter} \"{wrel}\""),
    ];
    let mut out = vec![];
    for (n, (i, j)) in sents.iter().enumerate() {
        let a = if n == 0 { 0. } else { down(ws[*i].1) };
        let b = if n + 1 == sents.len() { down(dur) } else { down(ws[sents[n + 1].0].1) };
        let text: Vec<&str> = ws[*i..=*j].iter().map(|w| w.0.as_str()).collect();
        out.push(format!("say    s{} {letter} {}-{} \"{}\"", sn0 + n + 1, fmt_time(a), fmt_time(b), text.join(" ").replace('"', "'")));
        for (ka, kb) in skips.iter().filter(|x| x.0 >= a && x.0 < b) {
            k += 1;
            out.push(format!("skip   k{k} {letter} {}-{}", fmt_time(*ka), fmt_time(*kb)));
        }
    }
    let k_new = k - k0;
    // the brief's bundle: window = the reel in the morphing window (engine/looks/window.md)
    let window = std::fs::read_to_string(dir.join("BRIEF.md")).unwrap_or_default().lines()
        .filter_map(|r| r.strip_prefix("- Bundle").and_then(|r| r.split_once(':')).map(|x| x.1.trim().to_lowercase()))
        .any(|v| !v.contains('|') && v.starts_with("window"));
    let text = if have.is_empty() {
        let fr = if window { vec!["frame  f1 window".to_string()] } else { vec![] };
        [vec![format!("size   {}x{} fps=30", size.0, size.1)], top, fr, out].concat().join("\n")
    } else {
        // the clip and words lines after the job's top lines, the say and skip lines after its last line that plays
        let t = read(dir)?;
        let (e, n) = t.split_once(&format!("\n{SEP}\n")).unwrap_or((&t, ""));
        let mut rows: Vec<String> = e.lines().map(String::from).collect();
        let at = rows.iter().enumerate().filter_map(|(i, r)| parse_line(i, r)).filter(|l| l.top()).map(|l| l.n + 1).max().unwrap_or(0);
        rows.splice(at..at, top);
        let end = rows.iter().enumerate().filter_map(|(i, r)| parse_line(i, r)).filter(|l| l.kind == "say" || l.kind == "use" || l.kind == "skip").map(|l| l.n + 1).max().unwrap_or(rows.len());
        rows.splice(end..end, out);
        format!("{}\n{SEP}\n{n}", rows.join("\n"))
    };
    write(dir, &text)?;
    let c = compile(dir, &read(dir)?, true, layout)?;
    let len = c.cfg["dur"].as_f64().unwrap_or(0.);
    Ok(format!("{}: take {letter}, {} sentences (say), {k_new} skips (pauses, fillers, start/end): {:.1} s; the reel is {:.1} s{}",
        path(dir).display(), sents.len(), dur, len, if plain { "" } else if relight { "; voice, relight and reframe run in the background" } else { "; voice and reframe run in the background" }))
}

/// The words of the file, so the agent needs no other file to write a line.
pub const KEYS: &str = "KEYS: moment ID \"spoken words | next group | *emph* | ~said, not shown~\" (^hero^ = Aston Script: old files only, never in a new line; user 2026-10-24) (one storyboard frame; no times: the moment runs from its first word to the next moment line; its captions, in file order; the lines indented under it belong to it, and a note on it changes only its block; a new moment takes the next free number m01 m02 …; old files say scene) | broll ID \"FILE\" from= rate= w= end (indented under a moment: a clip centred on black, sped to fill the moment, captions under it; old files: broll=\"FILE\" on the moment line) | moment keys: capcolor=#111111 (its captions in this colour, no shadow: a white b-roll) card=\"words | *emph* || next page\" (black card, the words in caption tiers, each word fading up on its spoken word; || = a new page, the one before fades out; no captions) capy=PX | big ID \"WORD\" (a big word on the moment it is indented under, else the moment line above it, moment=ID to pick; from that word to the moment end; its words leave the captions; style= at= size=; on a card moment: restyles that card word, its keys over the card's: color= size= style= at=) | text ID TAKE T0-T1 \"words\" style=NAME at=x,y size= color= y= fit= | style NAME font= size= color= y= shadow pop=SEC (small words) in=SEC grow=K (big words) fit=PX behind (the person over the words; makes a matte) fade=F (the bottom F of the word fades to 0) from=up|down dist=PX (moves in from above / below while it fades in) bg=#000000 (a full-screen colour under the word while it is on: a title card over the video and the captions) enter=soft|rise|blur|wipe|pop|slam|blurpop exit=fade|blur|drift (named moves in and out; enterdur= exitdur= SEC) steps=15 (the move on a 15 fps step clock) about=\"what it looks like, where it sits\" (a new style: the LOOKS list shows it in later reels) | frame ID window accent=#F8E479 emph=1.9 morph=0.45 (the reel in one rounded window on black that changes shape per scene and morphs between them; the face and every b-roll in it, captions in it, an *emph* word on its own line, bigger, in the accent colour; a moment takes win=sq|tall|full|wide|small|dot) | say ID TAKE A-B \"sentence\" (plays; off = that sentence is cut) | cap ID TAKE A-B \"w w | *emph*\" (captions v3 for that sentence: | = next group; the engine times and places them; y=PX puts every caption of the line with its middle on that world y) | pic ID TAKE A-B \"file\" fill (the picture fills the width on black) | fx ID TAKE A-B \"NAME\" key=value (a named effect from the FX list) | use ID TAKE A-B | skip ID TAKE A-B (a part removed; off = it plays again) | sfx ID TAKE T \"RECIPE\" rel=DB (a library sound at raw second T, its hit point on T; RECIPE: whoosh (a graphic moves in) whoosh-heavy whoosh-move whip riser build-drop deep-tension smash-silence whoosh-impact (a whoosh into a hit) impact (payoff, a title lands) hit (a smaller deep hit, a number) pop (a light hit for teaching reels: a title, a number, a box lands) whoosh-pop (a whoosh into a light hit) typing (laptop keys, one per letter at 15 a second, T = the first letter) click (a mouse tap) click-run (a camera shutter, one per photo) click-whoosh; sparse, professional) | song ID \"FILE.mp3\" (root-relative, in the aios.toml songs folder) from=SEC (song second at the reel start) gain=DB duck=DB fade=SEC in=SEC (silent until this reel second) | size WxH fps= caps=K (every caption K x bigger) loud=-11 tp=-1.5 (master LUFS and true-peak ceiling; default -14) | take ID \"FILE\" voice relight frame=locked|follow exposure=0.2 (stops; + brighter, - darker) (a take = one raw recording, ID = its letter A B …; old files say clip) | world px, x = the middle of the text, y = its middle; times are RAW take seconds (moment, big and broll lines have none)\n";

/// `aios note JOB`: each waiting note (id, raw clip, raw t) with what the agent needs to apply it: the lines near
/// it, what is on screen, what the drawing covers. The frame sheet is made by the caller (it needs a render).
/// Also the notes the old page notes agent routed up (status=main).
pub fn waiting(dir: &Path, layout: &Layout) -> Result<Vec<(String, String)>> {
    let text = read(dir)?;
    // moment and big lines have no raw time: their output spans from the compile (each note gets the ones near it)
    let c = compile(dir, &text, false, layout).ok();
    let spans: Vec<(usize, f64, f64)> = c.as_ref().and_then(|c| c.cfg["edit"]["lines"].as_array().cloned()).unwrap_or_default().iter()
        .filter(|v| v["kind"] == "moment" || v["kind"] == "big")
        .filter_map(|v| Some((v["n"].as_u64()? as usize, v["t0"].as_f64()?, v["t1"].as_f64()?))).collect();
    let words: Vec<Value> = c.as_ref().and_then(|c| c.cfg["words"].as_array().cloned()).unwrap_or_default();
    Ok(parse_all(&text).into_iter().filter(|l| l.kind == "note" && !l.off && matches!(l.get("status"), Some("new") | Some("main")))
        .map(|l| {
            let o = c.as_ref().and_then(|c| to_out(&c.segs, l.clip.as_deref().unwrap_or(""), l.t0.unwrap_or(0.), false));
            let mut near: Vec<usize> = o.map(|o| spans.iter().filter(|s| s.1 <= o + NEAR && s.2 >= o - NEAR).map(|s| s.0).collect()).unwrap_or_default();
            // a note written on a moment (the moment page) brings that moment's block
            if let Some(m) = l.get("moment").and_then(|m| parse_all(&text).into_iter().find(|x| x.kind == "moment" && x.id == m && x.mark.is_empty())) {
                near.push(m.n);
            }
            (l.id.clone(), packet(&text, &l, &near, o, &words))
        }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_around_a_note() {
        let ws: Vec<Value> = ["a", "b", "c", "d", "e", "f", "g", "h"].iter().enumerate()
            .map(|(i, w)| json!({ "word": w, "start": i as f64, "end": i as f64 + 0.8 })).collect();
        assert_eq!(words_at(&ws, 4.2).as_deref(), Some("b c d [e] f g h"), "inside a word");
        assert_eq!(words_at(&ws, 4.9).as_deref(), Some("c d e | f g h"), "in the pause before f");
        assert_eq!(words_at(&ws, 0.1).as_deref(), Some("[a] b c d"), "at the start");
        assert_eq!(words_at(&ws, 9.0), None, "after the last word");
    }

    #[test]
    fn lines() {
        let l = parse_line(0, r#"text t1 A 0.88-2.96 "upside down" style=small at=560,560"#).unwrap();
        assert_eq!((l.kind.as_str(), l.id.as_str(), l.clip.as_deref(), l.t0, l.t1), ("text", "t1", Some("A"), Some(0.88), Some(2.96)));
        assert_eq!(l.text.as_deref(), Some("upside down"));
        assert_eq!(l.get("at"), Some("560,560"));
        let c = parse_line(0, r#"clip A "a b/raw.mp4" rotate=90"#).unwrap();
        assert_eq!((c.id.as_str(), c.clip, c.text.as_deref()), ("A", None, Some("a b/raw.mp4")));
        let o = parse_line(3, "#n4< text t9 A 5.95-7.28 \"out\" style=big").unwrap();
        assert!(o.off);
        assert_eq!(o.mark, "n4<");
        assert_eq!(o.body(), "text t9 A 5.95-7.28 \"out\" style=big");
        assert!(parse_line(0, "# just a comment").is_none());
        let n = parse_line(0, r#"note n1 A 6.20 "make it \"smaller\"" reply="size 560 -> 480" status=done"#).unwrap();
        assert_eq!(n.text.as_deref(), Some("make it \"smaller\""));
        assert_eq!(n.get("reply"), Some("size 560 -> 480"));
        assert_eq!(parse_line(0, &fmt_line(&n)).unwrap().get("reply"), Some("size 560 -> 480"));
    }

    #[test]
    fn times() {
        let s = vec![Seg { clip: "A".into(), a: 0., b: 4.1, o: 0. }, Seg { clip: "A".into(), a: 4.4, b: 10., o: 4.1 }];
        assert_eq!(to_out(&s, "A", 5.0, false).map(|x| (x * 100.).round() / 100.), Some(4.7));
        assert_eq!(to_out(&s, "A", 4.2, false), Some(4.1));
        assert_eq!(to_out(&s, "A", 4.2, true), Some(4.1));
        assert_eq!(to_raw(&s, 4.7).map(|x| (x.0, (x.1 * 100.).round() / 100.)), Some(("A".to_string(), 5.0)));
    }

    // ── compile before / after: a small job (tests/fixtures/edit_small) compiles to the cfg saved in cfg_*.json ──
    // UPDATE_GOLDEN=1 cargo test writes the files again (only after a change that is meant to change the cfg).

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/edit_small")
    }

    /// the cfg as text, with the job folder and the AIOS root as $JOB and $ROOT (the same on every machine)
    /// the workspace above engine/ (cargo test runs in engine/, so the walk up finds its aios.toml)
    fn layout() -> Layout {
        Layout::load(None).unwrap_or_else(|e| panic!("layout: {e:#}"))
    }

    fn cfg_text(dir: &Path, text: &str) -> String {
        let layout = layout();
        let c = compile(dir, text, false, &layout).unwrap_or_else(|e| panic!("compile: {e:#}"));
        let root = layout.root().canonicalize().unwrap_or_else(|_| layout.root().to_path_buf());
        serde_json::to_string_pretty(&c.cfg).unwrap()
            .replace(&dir.to_string_lossy().to_string(), "$JOB")
            .replace(&root.to_string_lossy().to_string(), "$ROOT")
    }

    fn golden(name: &str, got: &str) {
        let f = fixture().join(name);
        if std::env::var("UPDATE_GOLDEN").is_ok() {
            std::fs::write(&f, got).unwrap();
            return;
        }
        let want = std::fs::read_to_string(&f).unwrap_or_else(|_| panic!("{} is missing (UPDATE_GOLDEN=1 makes it)", f.display()));
        if got != want {
            let out = std::env::temp_dir().join(format!("got_{name}"));
            std::fs::write(&out, got).unwrap();
            panic!("{name}: the cfg changed (diff {} {})", f.display(), out.display());
        }
    }

    #[test]
    fn compile_same_cfg() {
        let dir = fixture().canonicalize().unwrap();
        let old = std::fs::read_to_string(dir.join("old.txt")).unwrap();
        golden("cfg_old.json", &cfg_text(&dir, &old));
        let framed = old.replacen("words  A", "frame  f1 window accent=#F8E479\nwords  A", 1);
        golden("cfg_frame.json", &cfg_text(&dir, &framed));
    }

    /// the same job in moment blocks (take, moment, broll lines, lines indented under their moment) compiles to the
    /// same cfg as the old words; only the page's view of the lines differs
    #[test]
    fn moments_same_cfg() {
        let dir = fixture().canonicalize().unwrap();
        let cfg = |name: &str| {
            let mut c = compile(&dir, &std::fs::read_to_string(dir.join(name)).unwrap(), false, &layout()).unwrap_or_else(|e| panic!("{name}: {e:#}")).cfg;
            c["edit"]["lines"] = Value::Null;
            c
        };
        assert_eq!(cfg("moments.txt"), cfg("old.txt"));
        let lines = parse_all(&std::fs::read_to_string(dir.join("moments.txt")).unwrap());
        let owner = |id: &str| lines.iter().find(|l| l.id == id).and_then(|l| l.moment.clone()).map(|m| m.1);
        assert_eq!((owner("s1"), owner("g1"), owner("b1"), owner("t1"), owner("m02")), (Some("m01".into()), Some("m02".into()), Some("m01".into()), Some("m04".into()), None));
        assert_eq!(lines.iter().find(|l| l.id == "A").map(|l| l.kind.as_str()), Some("take"));
    }

    #[test]
    fn blocks_rules() {
        let text = "moment m1 \"a b\"\n  say s1 A 0.00-1.00\n\n  big g1 \"A\"\nsay s2 A 1.00-2.00\n#n3< moment m2 \"c\"\nmoment m2 \"c d\"\n  sfx x1 A 1.50 \"pop\"\n#n4< moment m2 \"c\"\n  big g2 \"D\"";
        let ls = parse_all(text);
        let owner = |id: &str| ls.iter().filter(|l| l.id == id).last().and_then(|l| l.moment.clone()).map(|m| m.1);
        assert_eq!((owner("s1"), owner("g1"), owner("s2"), owner("x1"), owner("g2")), (Some("m1".into()), Some("m1".into()), None, Some("m2".into()), Some("m2".into())));
        // a top line in a block, a broll line outside one
        assert!(blocks(&parse_all("moment m1 \"a\"\n  song m1 \"x.mp3\"")).is_err());
        assert!(blocks(&parse_all("broll b1 \"x.mp4\"")).is_err());
        // a broll line -> its moment's keys; a big under a moment that is off is off
        let b = blocks(&parse_all("moment m1 \"a\"\n  broll b1 \"x.mp4\" from=2\n# moment m2 \"b\"\n  big g1 \"B\"")).unwrap();
        assert_eq!((b[0].get("broll"), b[0].get("from")), (Some("x.mp4"), Some("2")));
        assert!(b.iter().find(|l| l.id == "g1").unwrap().off);
        // a new line put in after a moment line gets the block's indent; an indented line keeps its indent
        let rows: Vec<String> = ["moment m1 \"a\"", "  say s1 A 0.00-1.00"].iter().map(|s| s.to_string()).collect();
        let sfx = parse_line(0, "sfx x1 A 0.5 \"pop\"").unwrap();
        assert_eq!((indent_at(&rows, 1, &sfx), indent_at(&rows, 2, &sfx), indent_at(&rows, 0, &sfx)), ("  ".into(), "  ".into(), String::new()));
        assert_eq!(keep_indent("  say s1 A 0.00-1.00", "# say s1 A 0.00-1.00"), "  # say s1 A 0.00-1.00");
    }

    #[test]
    fn lock_alive() {
        let dir = std::env::temp_dir().join(format!("aios_lock_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let lock = dir.join("x.lock");
        let alive = |pid: String| { std::fs::write(&lock, pid).unwrap(); fresh_lock(&lock) };
        let mut c = std::process::Command::new("true").spawn().unwrap();
        let dead = c.id();
        c.wait().unwrap();
        let got = (alive(std::process::id().to_string()), alive(dead.to_string()), alive("0".into()), alive("x".into()));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(got, (true, false, false, false), "this process / a finished child / pid 0 / not a number");
    }

    /// `AIOS_DUMP_JOB=jobs/JOB AIOS_DUMP_OUT=FILE cargo test dump_job -- --ignored`: a job's cfg (no prep) to a file,
    /// for a before / after check on a copy of a real job
    #[test]
    #[ignore]
    fn dump_job() {
        let dir = PathBuf::from(std::env::var("AIOS_DUMP_JOB").unwrap()).canonicalize().unwrap();
        let text = read(&dir).unwrap();
        std::fs::write(std::env::var("AIOS_DUMP_OUT").unwrap(), cfg_text(&dir, &text)).unwrap();
    }
}
