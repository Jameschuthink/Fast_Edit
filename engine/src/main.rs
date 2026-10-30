//! aios — the AIOS3 engine CLI. `aios help` lists the commands.

mod assets;
mod audio;
mod cache;
mod canvas;
mod color;
mod editfile;
mod encode;
#[cfg(target_os = "linux")]
mod gpu;
#[cfg(not(target_os = "linux"))]
#[path = "gpu_none.rs"]
mod gpu;
mod job;
mod js;
mod layout;
mod lint;
mod moments;
mod render;
mod serve;
mod sound;
mod text;
mod trace;
mod video;
#[cfg(target_os = "linux")]
mod yuv;
#[cfg(not(target_os = "linux"))]
#[path = "yuv_none.rs"]
mod yuv;

use anyhow::{anyhow, Result};
use clap::parser::ValueSource;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use encode::EncOpts;
use job::{review_new, Job};
use layout::{Found, Key, Layout};
use render::{render_once, Blur, Pool, Settings, Sink};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Parser)]
#[command(name = "aios", about = "AIOS3 engine: JS Canvas2D compositions rendered by Skia on every core, NVENC out")]
struct Cli {
    /// the workspace (the folder with aios.toml); default: the first aios.toml from here up
    #[arg(long, global = true, env = "AIOS_ROOT")]
    root: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Args, Clone)]
struct Look {
    /// output width in px (height follows the world aspect); default: the world size
    #[arg(long)]
    width: Option<u32>,
    /// 540-wide draft
    #[arg(long)]
    draft: bool,
    /// shutter angle for motion blur in degrees (0 = off)
    #[arg(long, default_value_t = 180.)]
    shutter: f64,
    /// px between copies of a moving edge (smaller = smoother blur, more samples)
    #[arg(long)]
    gap: Option<f32>,
    /// most samples per moving frame
    #[arg(long, default_value_t = 16)]
    smax: usize,
    /// render workers (default: cores - 2)
    #[arg(long)]
    workers: Option<usize>,
    /// decode videos on the CPU instead of NVDEC
    #[arg(long)]
    no_hw: bool,
    /// paint every frame again (skip the chunk cache)
    #[arg(long)]
    no_cache: bool,
    /// workers that paint on the GPU (Vulkan); 0 = all on the CPU. Default: 8 for full-size renders of 20 s or more
    /// (13.3 s vs 15.5 s on a test job), CPU otherwise (a GPU context takes ~0.1 s to start: short clips are faster on the CPU)
    #[arg(long)]
    gpu: Option<usize>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Render a job (or a part of it) to a video
    Render {
        job: String,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        from: Option<f64>,
        #[arg(long)]
        to: Option<f64>,
        /// h264_nvenc, hevc_nvenc, libx264 (default: x264 for drafts, NVENC otherwise)
        #[arg(long)]
        codec: Option<String>,
        /// cq / crf (lower = better)
        #[arg(long)]
        quality: Option<u32>,
        #[arg(long)]
        preset: Option<String>,
        /// mux this audio file
        #[arg(long)]
        audio: Option<String>,
        #[command(flatten)]
        look: Look,
    },
    /// A new edit-file job from one raw take: word times, one say line per sentence, skips for pauses and fillers
    Take {
        job: String,
        clip: PathBuf,
        /// word times made already (else nv.py asr runs, ~10 s a minute)
        #[arg(long)]
        words: Option<PathBuf>,
        #[arg(long, default_value = "1080x1920")]
        size: String,
        /// no Studio Voice, relight or reframe keys on the clip line
        #[arg(long)]
        plain: bool,
        /// add relight to the take line (off by default: user, 2026-10-26)
        #[arg(long)]
        relight: bool,
        /// add this take to the job as the next clip (B, C …): its say and skip lines go after the others
        #[arg(long)]
        add: bool,
        /// the script (jobs/JOB/script.md): a diff script vs heard (the spelling slips to fix)
        #[arg(long)]
        script: Option<PathBuf>,
    },
    /// Stills sheet at chosen seconds -> jobs/JOB/review/NNN_stills.png (the next run number)
    Still {
        job: String,
        /// seconds, comma separated
        #[arg(long, default_value = "0")]
        at: String,
        /// draw the layout grid over the stills (safe area, platform zones, columns, thirds, golden lines, slots)
        #[arg(long)]
        grid: bool,
        #[arg(long)]
        out: Option<PathBuf>,
        /// also write each still as its own PNG next to the sheet
        #[arg(long)]
        each: bool,
        #[command(flatten)]
        look: Look,
    },
    /// One move in one PNG: n frames from A to B blended, early frames faint, the last strongest
    /// -> jobs/JOB/review/NNN_onion.png. Read the path, the spacing (easing) and any overshoot from one image.
    Onion {
        job: String,
        #[arg(long)]
        from: f64,
        #[arg(long)]
        to: f64,
        /// frames in the blend (spread evenly from A to B)
        #[arg(short, long, default_value_t = 6)]
        n: usize,
        #[arg(long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        look: Look,
    },
    /// Write frames as PNG (or JPEG) files
    Frames {
        job: String,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        from: Option<f64>,
        #[arg(long)]
        to: Option<f64>,
        #[arg(long)]
        jpg: bool,
        #[command(flatten)]
        look: Look,
    },
    /// Sound only: voice + ducked song + effects, mastered -> jobs/JOB/review/NNN_mix.wav
    Mix {
        job: String,
        #[arg(long)]
        from: Option<f64>,
        #[arg(long)]
        to: Option<f64>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// The speed log (logs/perf.jsonl, one line per command): the last runs as a table. Filter by command
    /// and/or a word in the arguments (a job name). AIOS_TRACE=1 on any command also writes a per-frame timeline.
    Perf {
        /// e.g. final, draft, render, or a job name
        filter: Vec<String>,
        #[arg(short, long, default_value_t = 15)]
        n: usize,
    },
    /// The mix read without listening: level, peak and silence per beat, loudness + true peak, and a waveform
    /// with beat lines and sfx ticks -> jobs/JOB/review/NNN_sound.png (the mix comes from the cache when unchanged)
    Sound {
        job: String,
        #[arg(long)]
        from: Option<f64>,
        #[arg(long)]
        to: Option<f64>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Draft with sound: 540 px picture + mix -> jobs/JOB/review/NNN_draft.mp4 (--from/--to: one section, NNN_clip_A-B.mp4)
    Draft {
        job: String,
        #[arg(long)]
        from: Option<f64>,
        #[arg(long)]
        to: Option<f64>,
        #[arg(long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        look: Look,
    },
    /// Full-size final with sound -> jobs/JOB/final/DATE_JOB.mp4, then the Reels check
    Final {
        job: String,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value = "h264_nvenc")]
        codec: String,
        #[command(flatten)]
        look: Look,
    },
    /// A still graphic at print size -> jobs/JOB/final/DATE_JOB.png + .pdf. Size: job.json "print": {"mm": [w, h], "dpi": 150}, or --width
    Export {
        job: String,
        /// the second to export
        #[arg(long, default_value_t = 0.)]
        at: f64,
        /// output width in px (default: from "print", else the world width)
        #[arg(long)]
        width: Option<u32>,
        /// dots per inch (default: "print".dpi, else 150)
        #[arg(long)]
        dpi: Option<f64>,
        #[arg(long)]
        out: Option<PathBuf>,
        /// JPEG instead of PNG
        #[arg(long)]
        jpg: bool,
        /// no PDF
        #[arg(long)]
        no_pdf: bool,
    },
    /// Run every frame through the comp without painting: JS errors (with their second), missing assets, warnings;
    /// with lib "layout": every element against the grid (safe area, face, overlaps, contrast, type sizes)
    Lint {
        job: String,
    },
    /// The live page: scrub the job's frames (engine pixels), edits reload by themselves
    Serve {
        job: String,
        #[arg(long, default_value_t = 8765)]
        port: u16,
        /// frame width on the page
        #[arg(long, default_value_t = 540)]
        width: u32,
    },
    /// A new job from a template in the aios.toml `templates` folder: project (default; a reel in the FLOW_SPEC layout) or poster
    New {
        job: String,
        #[arg(long, default_value = "project")]
        template: String,
        #[arg(long)]
        force: bool,
    },
    /// An edit-file job (edit.txt): no ID = list the new notes; ID --apply FILE = write the agent's answer; ID --undo / --redo
    Note {
        job: String,
        id: Option<String>,
        /// the answer for note ID: whole lines by id, `off ID`, `on ID`, `reply: …` / `ask: …` ("-" = stdin)
        #[arg(long)]
        apply: Option<String>,
        #[arg(long)]
        undo: bool,
        #[arg(long)]
        redo: bool,
        /// the packet with the line keys, every FX and look written out (default: their names + the reference file)
        #[arg(long)]
        full: bool,
    },
    /// Move the live page to second T (to show the user a moment); --beat N also selects beat N
    Seek {
        job: String,
        t: f64,
        #[arg(long)]
        beat: Option<usize>,
    },
    /// Evaluate a JS expression in the job's context (after the comp loaded) and print it as JSON
    Eval {
        job: String,
        expr: String,
        /// run draw(g, t) at this second first
        #[arg(long)]
        at: Option<f64>,
    },
    /// Render without writing anything and print where the time goes
    Bench {
        job: String,
        #[arg(long)]
        from: Option<f64>,
        #[arg(long)]
        to: Option<f64>,
        /// also encode (to /dev/null) to include the encoder
        #[arg(long)]
        encode: bool,
        #[command(flatten)]
        look: Look,
    },
    /// Print the workspace layout: each aios.toml key, its folder, and where it came from
    Config,
}

fn settings(job: &Job, look: &Look) -> Settings {
    let out_w = if look.draft { 540 } else { look.width.unwrap_or(job.world.0) };
    let k = out_w as f32 / job.world.0 as f32;
    let out_h = ((job.world.1 as f32 * k / 2.).round() as u32 * 2).max(2);
    let out_w = (out_w / 2) * 2;
    let blur = if look.shutter > 0. {
        let gap = look.gap.unwrap_or(if look.draft { 10. } else { 6. });
        Some(Blur { shutter: look.shutter, gap: gap * k.max(0.25), smax: look.smax })
    } else {
        None
    };
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    Settings { out_w, out_h, k, fps: job.fps, dur: job.dur, blur, workers: look.workers.unwrap_or(cores.saturating_sub(2).max(1)), gpu: look.gpu.unwrap_or(0) }
}

fn frame_range(job: &Job, from: Option<f64>, to: Option<f64>) -> Vec<usize> {
    let n = job.frames();
    let a = from.map(|s| (s * job.fps).round() as usize).unwrap_or(0).min(n);
    let b = to.map(|s| (s * job.fps).round() as usize).unwrap_or(n).min(n);
    (a..b.max(a)).collect()
}

/// GPU painting pays off on long full-size renders only.
/// `aios lint` with captions + faces: the sampled frames where a shown caption covers the face (a warning; the user judges)
fn caption_check(job: &Job) {
    let assets = assets::Assets::new(job.dir.clone(), job.fps, true, true, 1.);
    let shaper = std::rc::Rc::new(std::cell::RefCell::new(text::Shaper::new()));
    let r = js::Js::new(&job.src, assets.clone(), shaper).and_then(|mut js| {
        js.record(0.)?;
        js.eval_json("globalThis.CAPTIONS && CAPTIONS.check ? CAPTIONS.check() : null")
    });
    assets.stop();
    match r {
        Ok(v) if !v.is_null() => {
            let over = v["over"].as_array().cloned().unwrap_or_default();
            let n = v["frames"].as_u64().unwrap_or(0);
            println!("captions: {} of {} sampled frames keep off the face", n.saturating_sub(over.len() as u64), n);
            for o in &over {
                let mv = if o["move"].as_bool().unwrap_or(false) { " (the clip moves to or from the box)" } else { "" };
                println!("  over the face: {:.1} s \"{}\"{}", o["t"].as_f64().unwrap_or(0.), o["text"].as_str().unwrap_or(""), mv);
            }
        }
        Ok(_) => {}
        Err(e) => println!("  warning: caption check: {e:#}"),
    }
}

fn auto_gpu(look: &Look, s: &mut Settings, frames: usize) {
    if cfg!(target_os = "linux") && look.gpu.is_none() && s.k >= 0.99 && frames >= 600 {
        s.gpu = 8;
    }
}

fn parts(job: &Job, look: &Look, random: bool) -> (Settings, std::sync::Arc<assets::Assets>) {
    let s = settings(job, look);
    // hardware decode = CUDA (video.rs): the NVIDIA card on Linux only
    let assets = assets::Assets::new(job.dir.clone(), job.fps, !look.no_hw && cfg!(target_os = "linux"), random, s.k);
    (s, assets)
}

/// After `aios note JOB ID --apply / --undo / --redo`: the new length, where each changed word sits on the frame
/// (world px, measured where it is fully in), and the live page moves there. One call instead of a still + ffprobe
/// + seek after each note.
/// times a note names in page seconds: "@ 41.06 s", "at 12", "12.5s", "3 sec" (a bare number is not a time)
/// A note about "each change" / "every photo" / "each time" (an "each" word and a change word): the packet gets the
/// frames where the picture jumps, measured in code (user pick 2026-10-12: n23 guessed a photo change every 0.15 s;
/// it was every 0.65 s, and the 8 frames of the sheet cannot show that).
fn wants_changes(text: &str) -> bool {
    let t = text.to_lowercase();
    let w: Vec<&str> = t.split(|c: char| !c.is_alphanumeric()).filter(|x| !x.is_empty()).collect();
    let each = w.iter().any(|x| matches!(*x, "each" | "every" | "all" | "whenever"));
    let what = w.iter().any(|x| ["change", "photo", "picture", "image", "cut", "switch", "frame", "slide", "time"].iter().any(|p| x.starts_with(p)));
    each && what
}

/// Prints `PICTURE CHANGES`: the element the note is about (the lines its drawing covers, else the graphics on screen
/// at the note), over that element's whole span (at most 15 s each side), in its box at the note's frame (else the
/// whole frame). A jump = a frame that differs from the one before at least twice as much as its neighbours do (a
/// new photo, a cut); smooth motion (a scroll, a slide-in) differs every frame and is not a jump.
#[allow(clippy::too_many_arguments)]
fn picture_changes(j: &Job, pool: &Pool, lines: &[serde_json::Value], covers: &[&str], t: f64, boxes: &[(String, [f64; 4])],
                   k: f64, (w, h): (u32, u32), raw: &dyn Fn(usize) -> String) -> Result<()> {
    let on: Vec<&serde_json::Value> = lines.iter().filter(|x| !x["off"].as_bool().unwrap_or(false) && x["mark"].as_str().unwrap_or("").is_empty())
        .filter(|x| if covers.is_empty() {
            matches!(x["kind"].as_str(), Some("fx" | "pic" | "text")) && x["t0"].as_f64().is_some_and(|a| a <= t + 1e-6) && x["t1"].as_f64().is_some_and(|b| t < b)
        } else {
            covers.contains(&x["id"].as_str().unwrap_or(""))
        }).collect();
    let ids: Vec<&str> = on.iter().filter_map(|x| x["id"].as_str()).collect();
    let (a, b) = if on.is_empty() { (t - 3., t + 3.) } else {
        (on.iter().filter_map(|x| x["t0"].as_f64()).fold(f64::MAX, f64::min).max(t - 15.), on.iter().filter_map(|x| x["t1"].as_f64()).fold(f64::MIN, f64::max).min(t + 15.))
    };
    let last = j.frames().saturating_sub(1);
    let (f0, f1) = (((a * j.fps).round().max(0.) as usize).min(last), (((b * j.fps).round() as usize).saturating_sub(1)).min(last));
    let frames: Vec<usize> = (f0..=f1).collect();
    if frames.len() < 3 {
        return Ok(());
    }
    // one region per element (a box around two elements holds the face between them: n25, 2026-10-12), else the whole frame
    let c = |v: f64, m: u32| ((v * k).round().max(0.) as usize).min(m as usize);
    let mut regions: Vec<(String, (usize, usize, usize, usize))> = ids.iter().filter_map(|id| {
        let rs: Vec<[f64; 4]> = boxes.iter().filter(|(b, _)| b == id).map(|(_, r)| *r).collect();
        (!rs.is_empty()).then(|| (id.to_string(), (c(rs.iter().map(|r| r[0]).fold(f64::MAX, f64::min), w), c(rs.iter().map(|r| r[1]).fold(f64::MAX, f64::min), h),
            c(rs.iter().map(|r| r[2]).fold(f64::MIN, f64::max), w), c(rs.iter().map(|r| r[3]).fold(f64::MIN, f64::max), h))))
    }).collect();
    if regions.is_empty() {
        regions.push(("the frame".to_string(), (0, 0, w as usize, h as usize)));
    }
    let mut imgs: Vec<Option<Vec<u8>>> = vec![None; frames.len()];
    pool.render(&frames, |_| render::Want::Bgra, |f, px| { if let Some(x) = imgs.get_mut(f.wrapping_sub(f0)) { *x = Some(px); } Ok(()) }, false)?;
    let imgs: Vec<Vec<u8>> = imgs.into_iter().map(|x| x.unwrap_or_default()).collect();
    let mut parts = vec![];
    for (id, (x0, y0, x1, y1)) in &regions {
        let diff = |p: &[u8], q: &[u8]| -> f64 {
            let (mut sum, mut n) = (0u64, 0u64);
            for y in (*y0..*y1).step_by(3) {
                for x in (*x0..*x1).step_by(3) {
                    let o = (y * w as usize + x) * 4;
                    for c in 0..3 { sum += (p[o + c] as i32 - q[o + c] as i32).unsigned_abs() as u64; }
                    n += 3;
                }
            }
            if n == 0 { 0. } else { sum as f64 / n as f64 }
        };
        let d: Vec<f64> = (0..imgs.len()).map(|i| if i == 0 || imgs[i].is_empty() || imgs[i - 1].is_empty() { 0. } else { diff(&imgs[i], &imgs[i - 1]) }).collect();
        if std::env::var("AIOS_DEBUG_CHANGES").is_ok() { for (i, v) in d.iter().enumerate() { eprintln!("{id} {} {v:.1}", raw(frames[i])); } }
        let jumps: Vec<String> = (1..d.len()).filter(|&i| {
            let nb = d.get(i - 1).copied().unwrap_or(0.).max(d.get(i + 1).copied().unwrap_or(0.));
            // a jump stands alone (a lift or a slide is a hump of high frames); the edge frames have no neighbour
            d[i] >= 4. && d[i] >= 3. * nb && i > 1 && i + 1 < d.len()
        }).map(|i| raw(frames[i])).collect();
        parts.push(if jumps.is_empty() { format!("{id}: none (moves smoothly, no jump)") } else {
            format!("{id}: {} jumps at {}", jumps.len(), jumps.join(", "))
        });
    }
    println!("PICTURE CHANGES ({a:.2}-{b:.2} page s), measured frame by frame per element, the first frame of each new picture (raw clip time): {}. \"On each change\" = one line on each time of the element the note means, no other times", parts.join("; "));
    Ok(())
}

fn named_times(text: &str) -> Vec<f64> {
    let c: Vec<char> = text.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < c.len() {
        if !c[i].is_ascii_digit() || (i > 0 && (c[i - 1].is_alphanumeric() || c[i - 1] == '.')) {
            i += 1;
            continue;
        }
        let st = i;
        while i < c.len() && (c[i].is_ascii_digit() || (c[i] == '.' && i + 1 < c.len() && c[i + 1].is_ascii_digit())) { i += 1; }
        let num: String = c[st..i].iter().collect();
        let before: String = c[..st].iter().collect::<String>().trim_end().to_lowercase();
        let mut k = i;
        while k < c.len() && c[k] == ' ' { k += 1; }
        let unit: String = c[k..].iter().take_while(|ch| ch.is_alphabetic()).collect::<String>().to_lowercase();
        let secs = matches!(unit.as_str(), "s" | "sec" | "secs" | "second" | "seconds");
        if secs || before.ends_with('@') || before.ends_with(" at") || before == "at" {
            if let Ok(v) = num.parse::<f64>() { out.push(v); }
        }
    }
    out
}

/// How far after a note `aios note` looks for elements coming in, how far before it for elements that just went
/// out, and in how many layout frames each way.
const AHEAD_S: f64 = 1.0;
const BEHIND_S: f64 = 0.5;
const NEAR_STEPS: usize = 4;

/// The ids laid out at `times` (nearest the note first) that are not in `now`, each once at its nearest time:
/// (id, time - t, box).
fn not_at_note(now: &[(String, [f64; 4])], times: &[f64], frames: &[Vec<(String, [f64; 4])>], t: f64) -> Vec<(String, f64, [f64; 4])> {
    let mut out: Vec<(String, f64, [f64; 4])> = vec![];
    for (x, bs) in times.iter().zip(frames) {
        for (bid, r) in bs {
            if !now.iter().any(|(b, _)| b == bid) && !out.iter().any(|(b, ..)| b == bid) {
                out.push((bid.clone(), x - t, *r));
            }
        }
    }
    out
}

/// The labelled elements of one laid-out frame (fx, text, pic, caption ids) with their boxes in world px.
fn id_boxes(els: &[canvas::Elem], k: f64) -> Vec<(String, [f64; 4])> {
    els.iter().filter(|e| e.alpha > 0.01).filter_map(|e| {
        let v: serde_json::Value = serde_json::from_str(&e.label).ok()?;
        Some((v["id"].as_str()?.to_string(), [e.b.left as f64 / k, e.b.top as f64 / k, e.b.right as f64 / k, e.b.bottom as f64 / k]))
    }).collect()
}

/// How much of the fixed reference (line keys, FX, looks) `aios note` prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Detail {
    /// the FX and look names + the reference file's path
    Brief,
    /// all of it inline (`--full`)
    Full,
}

/// `aios note JOB`: per new note the packet, the boxes of what is on screen (world px) and ONE sheet to look at:
/// the note's frame on a pixel grid with the user's drawing and the boxes, then frames around it. So a note about
/// the picture needs one look, not a frame per call (n14, 2026-10-12: 8 of the helper's 16 calls made frames).
/// `only`: one note (the page's "chat" button on a note routed up): the chat gets that note and no other open one.
fn note_list(job: &str, dir: &std::path::Path, only: Option<&str>, detail: Detail, layout: &Layout) -> Result<()> {
    // kept effects and looks join the libraries (engine/tools/promote.py); a failure is printed, never hidden
    match std::process::Command::new("python3").arg(job::engine_dir().join("tools/promote.py")).arg(job).output() {
        Ok(o) => {
            print!("{}", String::from_utf8_lossy(&o.stdout));
            if !o.status.success() { println!("promote.py failed: {}", String::from_utf8_lossy(&o.stderr).trim()); }
        }
        Err(e) => println!("promote.py failed: {e}"),
    }
    let notes: Vec<(String, String)> = editfile::waiting(dir, layout)?.into_iter().filter(|(id, _)| only.map_or(true, |o| o == id)).collect();
    if notes.is_empty() {
        println!("{}", only.map_or("no new notes".to_string(), |o| format!("note {o} is not waiting (new or main)")));
        return Ok(());
    }
    let reference = format!("{}{}{}", editfile::KEYS, editfile::fx_list(dir), editfile::looks_list());
    match detail {
        Detail::Full => print!("{reference}"),
        // the same ~17 KB every run: in a file, read once per chat (step 0, 2026-10-30: the agent cut the packet
        // with head/sed in 23 of 26 runs, then ran it again for the part it cut)
        Detail::Brief => {
            let nd = job::ai_dir(dir).join("notes");
            std::fs::create_dir_all(&nd)?;
            let f = nd.join("reference.txt");
            std::fs::write(&f, &reference)?;
            println!("REFERENCE: {} (the line keys, what each FX does and its keys, the looks): read it once per chat, for a new kind of line, FX or look", f.display());
            print!("{}", editfile::names_list(dir));
        }
    }
    let j = Job::load(job, layout)?;
    let lines: Vec<serde_json::Value> = j.cfg["edit"]["lines"].as_array().cloned().unwrap_or_default();
    let last = j.frames().saturating_sub(1);
    let fr = |t: f64| ((t * j.fps).round().max(0.) as usize).min(last);
    let (ww, wh) = (j.world.0 as f64, j.world.1 as f64);
    let tall = wh > ww;
    let look = Look { width: Some(j.world.0.min(1200)), draft: false, shutter: 0., gap: None, smax: 2, workers: Some(4), no_hw: false, no_cache: true, gpu: Some(0) };
    let (s, assets) = parts(&j, &look, true);
    let pool = Pool::new(&j.src, assets, s.clone(), 9);
    let k = s.k as f64;
    // the top lines and ids are the same for every note: printed once, before the first note
    let head = |p: &str| -> Option<(usize, usize)> {
        let a = p.find("TOP LINES:\n")?;
        let b = a + p[a..].find("\n\n")? + 1;
        Some((a, b))
    };
    if let Some((p, (a, b))) = notes.first().and_then(|(_, p)| Some((p, head(p)?))) {
        print!("{}", &p[a..b]);
    }
    for (id, packet) in &notes {
        println!("===== {id}");
        match head(packet) {
            Some((a, b)) => print!("{}{}", &packet[..a], &packet[b + 1..]),
            None => print!("{packet}"),
        }
        let l = lines.iter().find(|l| l["kind"] == "note" && l["id"].as_str() == Some(id.as_str()));
        let t = l.and_then(|l| l["t0"].as_f64()).unwrap_or(0.);
        // the caption on screen too: a new big word over a caption word shows it twice (n10 test, 2026-10-12)
        let cap: Vec<String> = j.cfg["captions"]["captions"].as_array().into_iter().flatten()
            .filter(|c| c["start"].as_f64().unwrap_or(0.) <= t + 1e-6 && t < c["end"].as_f64().unwrap_or(0.))
            .flat_map(|c| c["lines"].as_array().cloned().unwrap_or_default().into_iter().flat_map(|l| l.as_array().cloned().unwrap_or_default()))
            .map(|w| format!("{}{}", w["w"].as_str().unwrap_or(""), if w["tier"] == "BASE" { String::new() } else { format!(" ({})", w["tier"].as_str().unwrap_or("")) })).collect();
        if !cap.is_empty() {
            println!("CAPTION ON SCREEN AT THE NOTE: {} (a new word with the same text: take it out of the cap line)", cap.join(" "));
        }
        // the strip: 0.8 s around the note, widened to the lines the drawing covers (at most 1.5 s each side)
        let covers: Vec<&str> = l.and_then(|l| l["covers"].as_str()).unwrap_or("").split(',').filter(|x| !x.is_empty()).collect();
        let spans: Vec<(f64, f64)> = lines.iter().filter(|x| covers.contains(&x["id"].as_str().unwrap_or("")) && !x["off"].as_bool().unwrap_or(false))
            .filter_map(|x| Some((x["t0"].as_f64()?, x["t1"].as_f64()?))).collect();
        let (a, b) = if spans.is_empty() { (t - 0.4, t + 0.4) } else {
            (spans.iter().map(|x| x.0).fold(t - 0.4, f64::min).max(t - 1.5), spans.iter().map(|x| x.1 - 1. / j.fps).fold(t + 0.4, f64::max).min(t + 1.5))
        };
        let n = if tall { 4 } else { 8 };
        let mut frames = vec![fr(t)];
        for i in 0..n { frames.push(fr(a + (b - a) * i as f64 / (n - 1) as f64)); }
        // a time the note names ("reference @ 41.06 s"): that frame too, last on the sheet, with its boxes (n18,
        // 2026-10-12: without it the agent could only ask what 41.06 s was)
        let refs: Vec<f64> = named_times(l.and_then(|l| l["text"].as_str()).unwrap_or(""))
            .into_iter().filter(|x| (x - t).abs() > 0.5 && *x <= j.dur).take(2).collect();
        for x in &refs { frames.push(fr(*x)); }
        let lay = pool.layout(&frames)?;
        let mut imgs: Vec<Vec<u8>> = Vec::new();
        pool.render(&frames, |_| render::Want::Bgra, |_, px| { imgs.push(px); Ok(()) }, false)?;
        let boxes: Vec<Vec<(String, [f64; 4])>> = lay.iter().map(|(els, _)| id_boxes(els, k)).collect();
        println!("BOXES AT THE NOTE (world px):{}", if boxes[0].is_empty() { " none" } else { "" });
        for (bid, r) in &boxes[0] {
            println!("  {bid} x {:.0}-{:.0} y {:.0}-{:.0}", r[0], r[2], r[1], r[3]);
        }
        // "this" is often what comes in just after the pause (n19, 2026-10-29: the split frame 0.2 s later),
        // or what went out just before it (a late pause)
        let step = |i: usize, span: f64| span * i as f64 / NEAR_STEPS as f64;
        let ahead: Vec<f64> = (1..=NEAR_STEPS).map(|i| t + step(i, AHEAD_S)).filter(|x| *x < j.dur).collect();
        let behind: Vec<f64> = (1..=NEAR_STEPS).map(|i| t - step(i, BEHIND_S)).filter(|x| *x >= 0.).collect();
        let near: Vec<usize> = ahead.iter().chain(&behind).map(|x| fr(*x)).collect();
        let near_boxes: Vec<Vec<(String, [f64; 4])>> = pool.layout(&near)?.iter().map(|(els, _)| id_boxes(els, k)).collect();
        let (ahead_boxes, behind_boxes) = near_boxes.split_at(ahead.len());
        let incoming = not_at_note(&boxes[0], &ahead, ahead_boxes, t);
        let gone = not_at_note(&boxes[0], &behind, behind_boxes, t);
        for (list, head) in [
            (&incoming, format!("COMING IN WITHIN {AHEAD_S:.0} s (not on screen at the note; \"this\" can mean one of them):")),
            (&gone, format!("WENT OUT IN THE LAST {BEHIND_S:.1} s (gone at the note, a late pause; \"this\" can mean one of them):")),
        ] {
            if !list.is_empty() {
                println!("{head}");
                for (bid, dt, r) in list {
                    println!("  {bid} at {dt:+.2} s, x {:.0}-{:.0} y {:.0}-{:.0}", r[0], r[2], r[1], r[3]);
                }
            }
        }
        // the point the user meant on the page: a dot they clicked on the paused clip, else where the mouse rested
        if let Some((px, py)) = l.and_then(|l| l["pt"].as_str()).and_then(|p| p.split_once(',')).and_then(|(a, b)| Some((a.parse::<f64>().ok()?, b.parse::<f64>().ok()?))) {
            let hit = |r: &[f64; 4]| px >= r[0] && px <= r[2] && py >= r[1] && py <= r[3];
            let mut under: Vec<String> = boxes[0].iter().filter(|(_, r)| hit(r)).map(|(b, _)| b.clone()).collect();
            under.extend(incoming.iter().filter(|(_, _, r)| hit(r)).map(|(b, dt, _)| format!("{b} (in at +{dt:.2} s)")));
            let by = if l.and_then(|l| l["ptby"].as_str()) == Some("dot") { "a dot the user clicked" } else { "where the mouse rested" };
            println!("UNDER THE POINT (x {px:.0} y {py:.0} world px, {by}): {}", if under.is_empty() { "no box (the picture itself: the face, the b-roll)".to_string() } else { under.join(", ") });
        }
        // panel labels in raw clip time: the times the agent writes
        let raw = |f: usize| -> String {
            let o = f as f64 / j.fps;
            j.cfg["edit"]["segs"].as_array().and_then(|ss| ss.iter().find(|x| {
                let (so, sa, sb) = (x["o"].as_f64().unwrap_or(0.), x["a"].as_f64().unwrap_or(0.), x["b"].as_f64().unwrap_or(0.));
                o >= so - 1e-6 && o < so + sb - sa
            }).map(|x| format!("{} {:.2}", x["clip"].as_str().unwrap_or(""), x["a"].as_f64().unwrap_or(0.) + o - x["o"].as_f64().unwrap_or(0.)))).unwrap_or_else(|| format!("out {o:.2}"))
        };
        let mut labels: Vec<String> = frames.iter().map(|&f| raw(f)).collect();
        for (i, x) in refs.iter().enumerate() {
            let at = n + 1 + i;
            println!("THE NOTE NAMES {x:.2} s (page time) = {}: on screen there (world px; the last panel of the sheet). A note like \"reference / like / same as @ T\" = make the note's lines look like these (style, size, place): do it, do not ask:{}", labels[at], if boxes[at].is_empty() { " no graphic" } else { "" });
            for (bid, r) in &boxes[at] {
                println!("  {bid} x {:.0}-{:.0} y {:.0}-{:.0}", r[0], r[2], r[1], r[3]);
            }
            labels[at] = format!("named {x:.2}s = {}", labels[at]);
        }
        if wants_changes(l.and_then(|l| l["text"].as_str()).unwrap_or("")) {
            picture_changes(&j, &pool, &lines, &covers, t, &boxes[0], k, (s.out_w, s.out_h), &raw)?;
        }
        let draw = job::ai_dir(&dir).join("notes").join(format!("{id}.draw.png"));
        let out = job::ai_dir(&dir).join("notes").join(format!("{id}.sheet.png"));
        sheet::note(&out, &imgs, s.out_w, s.out_h, (ww, wh), &boxes, &labels, draw.exists().then_some(draw.as_path()))?;
        println!("look: {}  (top: the note's frame, grid every 100 px, labels = world px; below: {} frames {} to {})\n", out.display(), n, labels[1], labels[n]);
    }
    pool.close();
    // the apply rules, so "check notes JOB" in a new chat needs no skill (engine SKILL.md has the same, longer)
    println!("HOW TO APPLY (cost = calls x context: aim for 2-3 calls in all)
- A note about the picture (a hand, a face, a gap, a place, a moment): look at its sheet ONCE. Make no frames; do not
  read edit.js or edit.txt (grep edit.txt only when a line you need is not above).
- All notes in ONE Bash call, chained with &&:  printf '%s\\n' 'LINE' 'reply: …' | aios note {job} ID --apply -
  The answer: each changed line written WHOLE with its id, `off ID`, `on ID` (only a line turned off by hand),
  `reply: one sentence, old -> new`. Times are RAW clip seconds (scene, big: none); at=x,y = the middle of the text in world px.
- The apply prints the check (changed ids, length, the new box) and moves the page: no still, ffprobe or seek after it.
- No key and no FX fits: write a new effect in the same answer (`js NAME` … `end js`, the FX.def API at the top of
  engine/js/lib/edit.js); the apply runs its frames and keeps nothing on an error.
- An unclear note: do not apply it; ask in the chat with 2-3 numbered options and your pick.
- Reply: one line per note, `ID: what changed (old -> new)`, short simple words. Undo: aios note {job} ID --undo (or --redo).");
    if editfile::read(&dir).unwrap_or_default().lines().any(|l| l.starts_with("note") && l.contains("status=main")) {
        println!("- A status=main note (routed up by the old page notes agent, deleted 2026-10-24): its reply says what it
  lacked. Get that (measure, look), apply as above. A rule worth keeping: ONE numbered proposal with your pick; after a
  yes, one line in _shared/TASTE.md (its section; change a line instead of adding a near-duplicate).");
    }
    Ok(())
}

fn note_report(job: &str, id: &str, what: &str, changed: &[String], before: Option<f64>, layout: &Layout) -> Result<()> {
    let j = Job::load(job, layout)?;
    let dur = j.cfg["dur"].as_f64().unwrap_or(0.);
    let ids: Vec<&str> = changed.iter().map(|c| c.trim_start_matches(['+', '-', '^'])).collect();
    println!("{id} {what}: {}", if ids.is_empty() { "no line changed".to_string() } else { ids.join(", ") });
    match before {
        Some(b) if (b - dur).abs() > 0.5 / j.fps => println!("length {dur:.2} s (was {b:.2})"),
        _ => println!("length {dur:.2} s (same)"),
    }
    let items: Vec<serde_json::Value> = j.cfg["edit"]["items"].as_array().cloned().unwrap_or_default();
    let shown: Vec<(&serde_json::Value, f64)> = ids.iter().filter_map(|x| items.iter().find(|i| i["id"].as_str() == Some(*x))).map(|i| {
        let (a, b) = (i["t0"].as_f64().unwrap_or(0.), i["t1"].as_f64().unwrap_or(0.));
        (i, (a + 0.4).min((a + b) / 2.))
    }).collect();
    let mut boxes: Vec<Option<(f64, f64, f64, f64)>> = vec![None; shown.len()];
    if !shown.is_empty() {
        let look = Look { width: None, draft: false, shutter: 0., gap: None, smax: 2, workers: Some(shown.len().min(4)), no_hw: false, no_cache: true, gpu: Some(0) };
        let (s, assets) = parts(&j, &look, true);
        let frames: Vec<usize> = shown.iter().map(|(_, t)| ((t * j.fps).round() as usize).min(j.frames().saturating_sub(1))).collect();
        let pool = Pool::new(&j.src, assets, s.clone(), frames.len());
        let lay = pool.layout(&frames);
        pool.close();
        if let Ok(lay) = lay {
            for (n, (elems, _)) in lay.iter().enumerate() {
                let want = shown[n].0["id"].as_str();
                boxes[n] = elems.iter().find(|e| serde_json::from_str::<serde_json::Value>(&e.label).ok().and_then(|v| v["id"].as_str().map(String::from)).as_deref() == want)
                    .map(|e| { let k = s.k as f64; (e.b.left as f64 / k, e.b.right as f64 / k, e.b.top as f64 / k, e.b.bottom as f64 / k) });
            }
        }
    }
    for ((i, t), b) in shown.iter().zip(&boxes) {
        let bx = b.map(|(l, r, tp, bt)| format!("box x {l:.0}-{r:.0} y {tp:.0}-{bt:.0}")).unwrap_or_else(|| "no box".into());
        println!("{} \"{}\" {:.2}-{:.2} s, {bx} (at {t:.2} s)", i["id"].as_str().unwrap_or(""), i["text"].as_str().unwrap_or(""), i["t0"].as_f64().unwrap_or(0.), i["t1"].as_f64().unwrap_or(0.));
    }
    // the other changed timed lines: raw and page seconds, so a still or a seek after the apply aims right (gap G13)
    let view: Vec<serde_json::Value> = j.cfg["edit"]["lines"].as_array().cloned().unwrap_or_default();
    for x in ids.iter().filter(|x| !shown.iter().any(|(i, _)| i["id"].as_str() == Some(**x))) {
        let Some(l) = view.iter().find(|l| l["id"].as_str() == Some(*x) && l["kind"] != "note" && !l["off"].as_bool().unwrap_or(false)) else { continue };
        let raw = match (l["clip"].as_str(), l["a"].as_f64(), l["b"].as_f64()) {
            (Some(c), Some(a), Some(b)) => format!("{c} {a:.2}-{b:.2} = "),
            (Some(c), Some(a), None) => format!("{c} {a:.2} = "),
            _ => String::new(),
        };
        let kind = l["kind"].as_str().unwrap_or("");
        // a span that plays nothing maps to one page second: a skip's cut point, or a line that is all cut
        match (l["t0"].as_f64(), l["t1"].as_f64()) {
            (Some(p0), Some(p1)) if p1 - p0 >= 0.5 / j.fps => println!("{x} {kind}: {raw}page {p0:.2}-{p1:.2} s"),
            (Some(p0), _) if kind == "skip" => println!("{x} skip: {raw}cut at page {p0:.2} s"),
            (Some(p0), None) if l["b"].is_null() => println!("{x} {kind}: {raw}page {p0:.2} s"),
            _ if l["clip"].is_string() => println!("{x} {kind}: {raw}not on screen (all of it is cut)"),
            _ => {}
        }
    }
    // the page goes to the first changed word, else to the note
    let note_t = j.cfg["edit"]["lines"].as_array().and_then(|ls| ls.iter().find(|l| l["kind"] == "note" && l["id"].as_str() == Some(id))).and_then(|l| l["t0"].as_f64());
    if let Some(t) = shown.first().map(|s| s.1).or(note_t) {
        serve::seek(&j, t, None)?;
        println!("page: {t:.2} s");
    }
    Ok(())
}

/// Picture and sound at the same time (sound is independent of the picture), then one mux. Both are cached:
/// unchanged seconds of picture and an unchanged mix are reused.
fn with_sound(job: &Job, look: &Look, from: Option<f64>, to: Option<f64>, enc: EncOpts, out: &std::path::Path) -> Result<()> {
    let t0 = Instant::now();
    let (mut s, assets) = parts(job, look, false);
    let frames = frame_range(job, from, to);
    if frames.is_empty() {
        return Err(anyhow!("no frames in that range"));
    }
    auto_gpu(look, &mut s, frames.len());
    let a = frames[0] as f64 / job.fps;
    let dur = frames.len() as f64 / job.fps;
    if let Some(p) = out.parent() { std::fs::create_dir_all(p)?; }
    let pic = out.with_extension("picture.mp4");
    let extra = audio::comp_sfx(job);
    let wav = cache::audio_path(job, a, dur, &extra);
    let wav_cached = wav.exists();
    let pool = Pool::new(&job.src, assets, s.clone(), frames.len());
    let (pr, m) = std::thread::scope(|sc| {
        // mix into a temp file, then rename: a stopped mix never leaves a broken file in the cache
        let h = sc.spawn(|| -> Result<Option<audio::Mix>> {
            if wav_cached {
                return Ok(None);
            }
            let part = wav.with_extension(format!("{}.part.m4a", std::process::id()));   // per process: two mixes of one key at once
            let m = audio::mix(job, a, dur, &extra, &part)?;
            std::fs::rename(&part, &wav)?;
            Ok(Some(m))
        });
        if look.no_cache {
            return (render_once_on(&pool, &frames, &enc, &pic), h.join().unwrap());
        }
        // the cached path muxes the sound into the join: one write of the file, not two
        let mut m = Ok(None);
        let pr = cache::video(&pool, &frames, &enc, &cache::cache_dir(job), out, true, || {
            m = Ok(h.join().unwrap()?);
            Ok(Some(wav.clone()))
        });
        (pr, m)
    });
    let st = pool.close();
    let c = pr?;
    let m = m?;
    let mut mux = String::from("muxed in the join");
    if look.no_cache {
        let t_mux = Instant::now();
        audio::mux(&pic, &wav, out)?;
        let _ = std::fs::remove_file(&pic);
        mux = format!("mux {:.2} s", t_mux.elapsed().as_secs_f64());
    }
    print_cache(&c, &st, &s);
    match &m {
        Some(m) => {
            for n in &m.notes { println!("  sound note: {n}"); }
            println!("sound: mixed, {:.1} LUFS, {:.1} dBTP; {mux}", m.lufs, m.tp);
        }
        None => println!("sound: unchanged (cached); {mux}"),
    }
    println!("wrote {} ({:.2} s total for {:.1} s of video)", out.display(), t0.elapsed().as_secs_f64(), dur);
    Ok(())
}

/// The no-cache path on an existing pool (one encoder, every frame painted).
fn render_once_on(pool: &Pool, frames: &[usize], enc: &EncOpts, out: &std::path::Path) -> Result<cache::CacheReport> {
    let s = &pool.settings;
    let t0 = Instant::now();
    let mut e = encode::Encoder::new(out, s.out_w, s.out_h, s.fps, enc)?;
    pool.render(frames, |_| render::Want::Yuv, |_, b| e.write(&b), true)?;
    e.finish()?;
    Ok(cache::CacheReport { frames: frames.len(), chunks: 0, reused: 0, painted: frames.len(), hash_s: 0., render_s: t0.elapsed().as_secs_f64(), join_s: 0. })
}

fn print_cache(c: &cache::CacheReport, st: &render::Stats, s: &Settings) {
    canvas::prof_report();
    note_stats(st, s);
    trace::note("frames", c.frames);
    trace::note("painted", c.painted);
    trace::note("chunks_reused", format!("{}/{}", c.reused, c.chunks));
    trace::note("hash_s", (c.hash_s * 1000.).round() / 1000.);
    trace::note("render_s", (c.render_s * 1000.).round() / 1000.);
    trace::note("join_s", (c.join_s * 1000.).round() / 1000.);
    if c.chunks > 0 {
        println!("picture {}x{}: {} frames; cache: {}/{} one-second chunks reused, {} frames painted", s.out_w, s.out_h, c.frames, c.reused, c.chunks, c.painted);
        println!("  hash pass {:.2} s, paint+encode {:.2} s ({:.0} fps), join {:.2} s", c.hash_s, c.render_s, c.painted as f64 / c.render_s.max(1e-6), c.join_s);
    } else {
        println!("picture {}x{}: {} frames painted in {:.2} s ({:.0} fps)", s.out_w, s.out_h, c.painted, c.render_s, c.painted as f64 / c.render_s.max(1e-6));
    }
    println!(
        "  frames: {} still, {} with motion blur, {} paints | worker ms: js {:.0}, analyse {:.0}, paint {:.0}, post {:.0}",
        st.still, st.blurred, st.paints, st.js_ms, st.ana_ms, st.paint_ms, st.post_ms
    );
    for w in &st.warnings { println!("  warning: {w}"); }
}

fn chrono_date() -> String {
    let o = std::process::Command::new("date").arg("+%Y-%m-%d").output().ok();
    o.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_else(|| "today".into())
}

mod check {
    use anyhow::Result;
    use std::path::Path;
    use std::process::Command;

    /// Reels delivery check: 1080x1920, 30 fps or more, H.264 + AAC, about the job's LUFS target (`loud=`, default -14),
    /// true peak at or below -1 dBTP.
    pub fn report(p: &Path, target: f64) -> Result<bool> {
        let o = Command::new("ffprobe")
            .args(["-v", "error", "-show_entries", "stream=codec_type,codec_name,width,height,r_frame_rate:format=duration", "-of", "json"])
            .arg(p)
            .output()?;
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap_or_default();
        let mut ok = true;
        let mut lines = Vec::new();
        let streams = v["streams"].as_array().cloned().unwrap_or_default();
        let vs = streams.iter().find(|s| s["codec_type"] == "video");
        let au = streams.iter().find(|s| s["codec_type"] == "audio");
        if let Some(s) = vs {
            let (w, h) = (s["width"].as_u64().unwrap_or(0), s["height"].as_u64().unwrap_or(0));
            let size_ok = w == 1080 && h == 1920;
            ok &= size_ok;
            lines.push(format!("{} size {w}x{h}", if size_ok { "PASS" } else { "FAIL" }));
            let c = s["codec_name"].as_str().unwrap_or("");
            lines.push(format!("{} video {c}", if c == "h264" || c == "hevc" { "PASS" } else { "FAIL" }));
        }
        match au {
            Some(a) => lines.push(format!("PASS audio {}", a["codec_name"].as_str().unwrap_or(""))),
            None => { ok = false; lines.push("FAIL no audio".into()); }
        }
        let e = Command::new("ffmpeg").args(["-hide_banner", "-nostdin", "-i"]).arg(p)
            .args(["-af", "ebur128=peak=true", "-f", "null", "-"]).output()?;
        let err = String::from_utf8_lossy(&e.stderr);
        let grab = |key: &str| -> Option<f64> {
            let i = err.rfind(key)?;
            err[i + key.len()..].split_whitespace().next()?.parse().ok()
        };
        if let Some(i) = grab("I:") {
            let lo = (i - target).abs() <= 1.;
            ok &= lo;
            lines.push(format!("{} loudness {i:.1} LUFS (target {target})", if lo { "PASS" } else { "FAIL" }));
        }
        if let Some(tp) = grab("Peak:") {
            let tpo = tp <= -1.;
            ok &= tpo;
            lines.push(format!("{} true peak {tp:.1} dBTP (max -1)", if tpo { "PASS" } else { "FAIL" }));
        }
        println!("Reels check {}: {}", if ok { "PASS" } else { "FAIL" }, p.display());
        for l in lines { println!("  {l}"); }
        Ok(ok)
    }
}

/// The perf line's worker fields (trace.rs).
fn note_stats(st: &render::Stats, s: &Settings) {
    trace::note("size", format!("{}x{}", s.out_w, s.out_h));
    trace::note("workers", s.workers);
    trace::note("gpu", s.gpu);
    trace::note("paints", st.paints);
    trace::note("blurred", st.blurred);
    trace::note("worker_ms", serde_json::json!({ "js": st.js_ms.round(), "analyse": st.ana_ms.round(), "paint": st.paint_ms.round(), "post": st.post_ms.round() }));
}

fn print_report(r: &render::Report, what: &str) {
    canvas::prof_report();
    let st = &r.stats;
    trace::note("frames", r.frames);
    trace::note("render_s", (r.seconds * 1000.).round() / 1000.);
    trace::note("workers", r.workers);
    trace::note("paints", st.paints);
    trace::note("worker_ms", serde_json::json!({ "js": st.js_ms.round(), "analyse": st.ana_ms.round(), "paint": st.paint_ms.round(), "post": st.post_ms.round() }));
    let fps = r.frames as f64 / r.seconds.max(1e-6);
    println!("{what}: {} frames in {:.2} s = {:.0} fps on {} workers", r.frames, r.seconds, fps, r.workers);
    println!(
        "  frames: {} still, {} with motion blur, {} paints | worker ms: js {:.0}, analyse {:.0}, paint {:.0}, post {:.0}",
        st.still, st.blurred, st.paints, st.js_ms, st.ana_ms, st.paint_ms, st.post_ms
    );
    for w in &st.warnings {
        println!("  warning: {w}");
    }
}

fn run(cmd: Cmd, layout: &Layout) -> Result<()> {
    match cmd {
        Cmd::Config => print!("{}", layout.report()),
        Cmd::Render { job, out, from, to, codec, quality, preset, audio, look } => {
            let t0 = Instant::now();
            let job = Job::load(&job, layout)?;
            let (mut s, assets) = parts(&job, &look, false);
            let frames = frame_range(&job, from, to);
            auto_gpu(&look, &mut s, frames.len());
            let mut enc = if look.draft { EncOpts::draft() } else { EncOpts::final_() };
            if let Some(c) = codec { enc.codec = c; }
            if let Some(q) = quality { enc.quality = q; }
            if let Some(p) = preset { enc.preset = p; }
            let out = out.unwrap_or_else(|| review_new(&job, if look.draft { "draft.mp4" } else { "render.mp4" }));
            if look.no_cache {
                enc.audio = audio;
                let r = render_once(&job.src, assets, s.clone(), frames, Sink::Video { out: out.clone(), enc }, true)?;
                print_report(&r, &format!("render {}x{}", s.out_w, s.out_h));
            } else {
                // --audio: the picture goes through the cache, the sound file is muxed in the join (as with_sound does)
                if let Some(p) = out.parent() { std::fs::create_dir_all(p)?; }
                let pool = Pool::new(&job.src, assets, s.clone(), frames.len());
                let res = cache::video(&pool, &frames, &enc, &cache::cache_dir(&job), &out, true, || Ok(audio.as_ref().map(std::path::PathBuf::from)));
                let st = pool.close();
                let c = res?;
                print_cache(&c, &st, &s);
            }
            println!("wrote {} ({:.2} s total)", out.display(), t0.elapsed().as_secs_f64());
        }
        Cmd::Take { job, clip, words, size, plain, relight, add, script } => {
            let t0 = Instant::now();
            let (w, h) = size.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))).ok_or_else(|| anyhow!("--size is WxH"))?;
            let dir = layout.dir(Key::Jobs).join(&job);
            // voice (the NVIDIA clients, not in git) and frame= (CUDA reframe) run on the Linux machine only
            let plain = plain || !cfg!(target_os = "linux");
            if !cfg!(target_os = "linux") {
                eprintln!("take: no voice or frame= on this OS (Linux only): the take plays as filmed");
            }
            println!("{}", editfile::take(&dir, &clip, words.as_deref(), script.as_deref(), (w, h), plain, relight, add, layout)?);
            println!("({:.1} s) next: aios serve {job}", t0.elapsed().as_secs_f64());
        }
        Cmd::Still { job, at, out, each, grid, look } => {
            let t0 = Instant::now();
            let mut job = Job::load(&job, layout)?;
            if grid {
                let mut v: serde_json::Value = serde_json::from_str(&job.src.job_json)?;
                v["guides"] = true.into();
                job.src.job_json = serde_json::to_string(&v)?;
            }
            let mut look = look;
            // a still job (poster): one big still to read the type; a video: a 540 px sheet
            if look.width.is_none() && !look.draft { look.width = Some(if job.frames() <= 1 { 1080 } else { 540 }); }
            let (s, assets) = parts(&job, &look, true);
            let times: Vec<f64> = at.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            if times.is_empty() { return Err(anyhow!("--at needs seconds, e.g. --at 1.2,4,9.5")); }
            let frames: Vec<usize> = times.iter().map(|t| ((t * job.fps).round() as usize).min(job.frames() - 1)).collect();
            let r = render_once(&job.src, assets, s.clone(), frames.clone(), Sink::Memory, false)?;
            let out = out.unwrap_or_else(|| review_new(&job, "stills.png"));
            sheet::write(&out, &r.images, &times, s.out_w, s.out_h)?;
            // so a note can say "#3": the times of the last sheet
            let _ = std::fs::create_dir_all(job::cache_dir(&job.dir));
            let _ = std::fs::write(job::cache_dir(&job.dir).join("stills.json"), serde_json::to_string(&times)?);
            if each {
                for (i, (_, px)) in r.images.iter().enumerate() {
                    let p = out.with_file_name(format!("still_{}_{:.2}s.png", i + 1, times[i]));
                    render::write_image(&p, px, s.out_w, s.out_h, false)?;
                }
            }
            for w in &r.stats.warnings { println!("  warning: {w}"); }
            println!("stills: {} ({} frames, {:.2} s)", out.display(), times.len(), t0.elapsed().as_secs_f64());
        }
        Cmd::Onion { job, from, to, n, out, look } => {
            let t0 = Instant::now();
            let job = Job::load(&job, layout)?;
            if !(to > from) || n < 2 { return Err(anyhow!("onion needs --to after --from and -n 2 or more")); }
            let mut look = look;
            if look.width.is_none() && !look.draft { look.width = Some(540); }
            let (s, assets) = parts(&job, &look, true);
            let times: Vec<f64> = (0..n).map(|i| from + (to - from) * i as f64 / (n - 1) as f64).collect();
            let frames: Vec<usize> = times.iter().map(|t| ((t * job.fps).round() as usize).min(job.frames() - 1)).collect();
            let r = render_once(&job.src, assets, s.clone(), frames, Sink::Memory, false)?;
            let out = out.unwrap_or_else(|| review_new(&job, "onion.png"));
            sheet::onion(&out, &r.images, &times, s.out_w, s.out_h)?;
            for w in &r.stats.warnings { println!("  warning: {w}"); }
            println!("onion: {} ({n} frames {from:.2}-{to:.2} s, {:.2} s)", out.display(), t0.elapsed().as_secs_f64());
        }
        Cmd::Frames { job, out, from, to, jpg, look } => {
            let job = Job::load(&job, layout)?;
            let (s, assets) = parts(&job, &look, false);
            std::fs::create_dir_all(&out)?;
            let frames = frame_range(&job, from, to);
            let r = render_once(&job.src, assets, s, frames, Sink::Frames { dir: out.clone(), jpg }, true)?;
            print_report(&r, "frames");
            println!("wrote {}", out.display());
        }
        Cmd::Export { job, at, width, dpi, out, jpg, no_pdf } => {
            let t0 = Instant::now();
            let job = Job::load(&job, layout)?;
            let pr = &job.cfg["print"];
            let dpi = dpi.or(pr["dpi"].as_f64()).unwrap_or(150.);
            let w = width.or(pr["mm"][0].as_f64().map(|mm| (mm / 25.4 * dpi).round() as u32)).unwrap_or(job.world.0);
            // one worker: a print-size surface is large (8000 x 7000 px = 224 MB)
            let look = Look { width: Some(w), draft: false, shutter: 0., gap: None, smax: 2, workers: Some(1), no_hw: false, no_cache: true, gpu: Some(0) };
            let (s, assets) = parts(&job, &look, true);
            let f = ((at * job.fps).round() as usize).min(job.frames().saturating_sub(1));
            let r = render_once(&job.src, assets, s.clone(), vec![f], Sink::Memory, false)?;
            for w in &r.stats.warnings { println!("  warning: {w}"); }
            let (_, px) = r.images.first().ok_or_else(|| anyhow!("no frame painted"))?;
            let out = out.unwrap_or_else(|| job::final_path(&job, &chrono_date(), if jpg { "jpg" } else { "png" }));
            if let Some(p) = out.parent() { std::fs::create_dir_all(p)?; }
            render::write_image(&out, px, s.out_w, s.out_h, jpg)?;
            let mm = |v: u32| v as f64 / dpi * 25.4;
            println!("export: {} ({}x{} px = {:.0} x {:.0} mm at {} dpi, {:.1} s)", out.display(), s.out_w, s.out_h, mm(s.out_w), mm(s.out_h), dpi, t0.elapsed().as_secs_f64());
            if !no_pdf {
                let pdf = out.with_extension("pdf");
                let st = std::process::Command::new("magick")
                    .arg(&out)
                    .args(["-units", "PixelsPerInch", "-density", &format!("{dpi}"), "-compress", "jpeg", "-quality", "92"])
                    .arg(&pdf)
                    .status()
                    .map_err(|e| anyhow!("magick (ImageMagick 7) for the PDF: {e}"))?;
                if !st.success() { return Err(anyhow!("magick failed on {}", pdf.display())); }
                println!("export: {} ({:.1} s)", pdf.display(), t0.elapsed().as_secs_f64());
            }
        }
        Cmd::Lint { job } => {
            let t0 = Instant::now();
            let job = Job::load(&job, layout)?;
            let mut look = Look { width: Some(270), draft: false, shutter: 0., gap: None, smax: 2, workers: None, no_hw: false, no_cache: true, gpu: Some(0) };
            look.draft = false;
            let (s, assets) = parts(&job, &look, true);
            let frames: Vec<usize> = (0..job.frames()).collect();
            let pool = Pool::new(&job.src, assets, s.clone(), frames.len());
            let r = pool.hashes(&frames);
            // jobs on the layout grid: every element of every frame (boxes, text sizes and colours)
            let lay = if r.is_ok() && lint::wants(&job) { Some(pool.layout(&frames)) } else { None };
            let st = pool.close();
            match r {
                Ok(h) => {
                    let mut distinct = h.clone();
                    distinct.sort();
                    distinct.dedup();
                    println!("lint PASS: {} frames ran ({} distinct pictures), {:.2} s", frames.len(), distinct.len(), t0.elapsed().as_secs_f64());
                    if job.cfg.get("captions").is_some() && job.cfg.get("faces").is_some() { caption_check(&job); }
                    match lay {
                        Some(Ok(d)) => { if let Err(e) = lint::report(&job, &frames, d, s.k) { println!("  warning: layout check: {e:#}"); } }
                        Some(Err(e)) => println!("  warning: layout check: {e:#}"),
                        None => {}
                    }
                }
                Err(e) => {
                    println!("lint FAIL: {e:#}");
                    std::process::exit(1);
                }
            }
            for w in &st.warnings { println!("  warning: {w}"); }
        }
        Cmd::Serve { job, port, width } => {
            serve::serve(&job, port, width, layout)?;
        }
        Cmd::New { job, template, force } => {
            let dir = layout.dir(Key::Jobs).join(&job);
            if (dir.join("job.json").exists() || dir.join(editfile::FILE).exists()) && !force {
                return Err(anyhow!("{} exists (pass --force to overwrite its files)", dir.display()));
            }
            let tpl = layout.dir(Key::Templates).join(&template);
            if !tpl.is_dir() {
                return Err(anyhow!("no template {} (project = a reel, poster = a poster)", tpl.display()));
            }
            // the template's whole tree (project: BRIEF.md, scripts/, media/refs/, review/, _ai/), not its README
            fn copy_tree(from: &std::path::Path, to: &std::path::Path, top: bool) -> Result<()> {
                std::fs::create_dir_all(to)?;
                for e in std::fs::read_dir(from)?.flatten() {
                    let n = e.file_name();
                    if e.path().is_dir() { copy_tree(&e.path(), &to.join(&n), false)?; }
                    else if !(top && n == "README.md") { std::fs::copy(e.path(), to.join(&n))?; }
                }
                Ok(())
            }
            copy_tree(&tpl, &dir, true)?;
            let next = std::fs::read_to_string(tpl.join("README.md")).ok().and_then(|r| r.lines().find(|l| l.starts_with("Edit") || l.starts_with("Next")).map(|l| l.to_string()));
            println!("new: jobs/{job} (template {template}). {}", next.unwrap_or_else(|| "Edit its files".into()).replace("JOB", &job));
        }
        Cmd::Mix { job, from, to, out } => {
            let t0 = Instant::now();
            let job = Job::load(&job, layout)?;
            let a = from.unwrap_or(0.);
            let b = to.unwrap_or(job.dur).min(job.dur);
            let out = out.unwrap_or_else(|| review_new(&job, "mix.wav"));
            let extra = audio::comp_sfx(&job);
            let m = audio::mix(&job, a, b - a, &extra, &out)?;
            for n in &m.notes { println!("  note: {n}"); }
            println!("mix: {} ({:.1} LUFS, {:.1} dBTP, {:.2} s)", out.display(), m.lufs, m.tp, t0.elapsed().as_secs_f64());
        }
        Cmd::Perf { filter, n } => {
            let txt = std::fs::read_to_string(layout.dir(Key::Logs).join("perf.jsonl")).unwrap_or_default();
            let rows: Vec<serde_json::Value> = txt.lines().filter_map(|l| serde_json::from_str(l).ok())
                .filter(|v: &serde_json::Value| filter.iter().all(|f| v["cmd"].as_str() == Some(f.as_str()) || v["args"].as_str().map(|a| a.split_whitespace().any(|w| w == f)).unwrap_or(false)))
                .collect();
            let num = |v: &serde_json::Value, k: &str| v[k].as_f64().map(|x| format!("{x:.0}")).unwrap_or_else(|| "-".into());
            println!("{:<16} {:<8} {:<7} {:<22} {:>6} {:>6} {:>6} {:>7} {:>7} {:>7} {:>6} {:>5}", "date", "commit", "cmd", "args", "wall s", "frames", "fps", "vwait", "vread", "encode", "idle", "seeks");
            for v in rows.iter().skip(rows.len().saturating_sub(n)) {
                let fr = v["frames"].as_f64();
                let fps = match (fr, v["render_s"].as_f64()) { (Some(f), Some(r)) if r > 0. => format!("{:.0}", v["painted"].as_f64().unwrap_or(f) / r), _ => "-".into() };
                let mut a: String = v["args"].as_str().unwrap_or("").split_whitespace().filter(|w| !w.contains('/')).collect::<Vec<_>>().join(" ");
                if a.chars().count() > 22 { a = a.chars().take(21).collect::<String>() + "…"; }
                println!("{:<16} {:<8} {:<7} {:<22} {:>6.2} {:>6} {:>6} {:>7} {:>7} {:>7} {:>6} {:>5}",
                    v["date"].as_str().unwrap_or("").get(..16).unwrap_or(""), v["commit"].as_str().unwrap_or(""), v["cmd"].as_str().unwrap_or(""), a,
                    v["wall_s"].as_f64().unwrap_or(0.), fr.map(|f| format!("{f:.0}")).unwrap_or_else(|| "-".into()), fps,
                    num(v, "video_wait_ms"), num(v, "video_read_ms"), num(v, "consume_ms"), num(v, "dispatch_idle_ms"), num(v, "video_seeks"));
            }
            println!("(ms summed over threads; vwait = workers waiting for video, encode = handing frames to the encoder; {} runs logged)", rows.len());
        }
        Cmd::Sound { job, from, to, out } => {
            let t0 = Instant::now();
            let job = Job::load(&job, layout)?;
            let a = from.unwrap_or(0.);
            let b = to.unwrap_or(job.dur).min(job.dur);
            let extra = audio::comp_sfx(&job);
            let wav = cache::audio_path(&job, a, b - a, &extra);
            if !wav.exists() {
                let part = wav.with_extension(format!("{}.part.m4a", std::process::id()));   // per process: two mixes of one key at once
                let m = audio::mix(&job, a, b - a, &extra, &part)?;
                std::fs::rename(&part, &wav)?;
                for n in &m.notes { println!("  note: {n}"); }
            }
            let out = out.unwrap_or_else(|| review_new(&job, "sound.png"));
            sound::report(&job, &wav, a, b - a, &extra, &out)?;
            println!("({:.2} s)", t0.elapsed().as_secs_f64());
        }
        Cmd::Draft { job, from, to, out, look } => {
            let mut look = look;
            look.draft = look.width.is_none();
            let job = Job::load(&job, layout)?;
            let out = out.unwrap_or_else(|| if from.is_some() || to.is_some() {
                job::clip_path(&job, from.unwrap_or(0.), to.unwrap_or(job.dur))
            } else {
                review_new(&job, "draft.mp4")
            });
            with_sound(&job, &look, from, to, EncOpts::draft(), &out)?;
        }
        Cmd::Final { job, out, codec, look } => {
            let job = Job::load(&job, layout)?;
            let out = out.unwrap_or_else(|| job::final_path(&job, &chrono_date(), "mp4"));
            let mut enc = EncOpts::final_();
            enc.codec = codec;
            with_sound(&job, &look, None, None, enc, &out)?;
            check::report(&out, job.cfg["loudness"]["lufs"].as_f64().unwrap_or(-14.))?;
        }
        Cmd::Note { job, id, apply, undo, redo, full } => {
            let dir = job::resolve_job(&job, layout)?;
            let before = editfile::read(&dir).ok().and_then(|t| editfile::compile(&dir, &t, false, layout).ok()).and_then(|c| c.cfg["dur"].as_f64());
            let detail = if full { Detail::Full } else { Detail::Brief };
            let (id, what, r) = match (id, apply) {
                (None, _) => { note_list(&job, &dir, None, detail, layout)?; return Ok(()); }
                (Some(id), None) if !undo && !redo => { note_list(&job, &dir, Some(&id), detail, layout)?; return Ok(()); }
                (Some(id), _) if redo => { let r = editfile::redo_note(&dir, &id, layout)?; (id, "redone", r["lines"].clone()) }
                (Some(id), _) if undo => { let r = editfile::undo_note(&dir, &id, layout)?; (id, "undone", r["lines"].clone()) }
                (Some(id), Some(f)) => {
                    let a = if f == "-" { std::io::read_to_string(std::io::stdin())? } else { std::fs::read_to_string(&f).map_err(|e| anyhow!("{f}: {e}"))? };
                    let r = editfile::apply_answer(&dir, &id, &a, layout)?;
                    (id, match r["status"].as_str() { Some("ask") => "ask (options on the page)", Some("main") => "main (for the main chat)", _ => "done" }, r["changed"].clone())
                }
                (Some(_), None) => anyhow::bail!("give --apply FILE (the answer), --undo or --redo"),
            };
            let changed: Vec<String> = r.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
            note_report(&job, &id, what, &changed, before, layout)?;
        }
        Cmd::Seek { job, t, beat } => {
            let j = Job::load(&job, layout)?;
            serve::seek(&j, t, beat)?;
            println!("live page: {t:.2} s{}", beat.map(|b| format!(", beat {b}")).unwrap_or_default());
        }
        Cmd::Eval { job, expr, at } => {
            let job = Job::load(&job, layout)?;
            let assets = assets::Assets::new(job.dir.clone(), job.fps, true, true, 1.);
            let shaper = std::rc::Rc::new(std::cell::RefCell::new(text::Shaper::new()));
            let mut js = js::Js::new(&job.src, assets.clone(), shaper)?;
            if let Some(t) = at {
                let ops = js.record(t)?;
                eprintln!("draw({t}) recorded {} numbers", ops.len());
            }
            println!("{}", serde_json::to_string_pretty(&js.eval_json(&expr)?)?);
            assets.stop();
        }
        Cmd::Bench { job, from, to, encode, look } => {
            let job = Job::load(&job, layout)?;
            let (s, assets) = parts(&job, &look, false);
            let frames = frame_range(&job, from, to);
            let sink = if encode { Sink::Video { out: PathBuf::from("/tmp/aios-bench.mp4"), enc: EncOpts::draft() } } else { Sink::Null };
            let r = render_once(&job.src, assets, s.clone(), frames, sink, true)?;
            print_report(&r, &format!("bench {}x{}", s.out_w, s.out_h));
        }
    }
    Ok(())
}

mod sheet {
    use anyhow::{anyhow, Result};
    use skia_safe::{surfaces, AlphaType, Color, ColorType, Data, Font, FontMgr, FontStyle, ImageInfo, Paint, Rect};
    use std::path::Path;

    /// Stills side by side, numbered #1 #2 … with their seconds, at most 6 per row (360 px each; one still keeps up to 1080 px).
    pub fn write(out: &Path, imgs: &[(usize, Vec<u8>)], times: &[f64], w: u32, h: u32) -> Result<()> {
        let n = imgs.len();
        let cols = n.min(6);
        let rows = n.div_ceil(cols);
        let tw = (w as f32).min(if n == 1 { 1080. } else { 360. });
        let th = tw * h as f32 / w as f32;
        let lab = 34.;
        let mut surf = surfaces::raster_n32_premul(((tw * cols as f32) as i32, ((th + lab) * rows as f32) as i32)).ok_or_else(|| anyhow!("sheet surface"))?;
        let c = surf.canvas();
        c.clear(Color::from_rgb(20, 20, 20));
        let fm = FontMgr::new();
        let tf = fm.match_family_style("sans-serif", FontStyle::bold()).or_else(|| fm.legacy_make_typeface(None, FontStyle::bold()));
        let font = tf.map(|t| Font::from_typeface(t, 20.));
        let mut tp = Paint::default();
        tp.set_color(Color::from_rgb(240, 240, 240));
        tp.set_anti_alias(true);
        for (i, (_, px)) in imgs.iter().enumerate() {
            let (cx, cy) = ((i % cols) as f32 * tw, (i / cols) as f32 * (th + lab));
            let info = ImageInfo::new((w as i32, h as i32), ColorType::BGRA8888, AlphaType::Premul, None);
            if let Some(img) = skia_safe::images::raster_from_data(&info, Data::new_copy(px), (w * 4) as usize) {
                c.draw_image_rect_with_sampling_options(&img, None, Rect::from_xywh(cx, cy + lab, tw, th),
                    skia_safe::SamplingOptions::new(skia_safe::FilterMode::Linear, skia_safe::MipmapMode::Linear), &Paint::default());
            }
            if let Some(f) = &font {
                c.draw_str(format!("#{}  {:.2}s", i + 1, times.get(i).copied().unwrap_or(0.)), (cx + 10., cy + 25.), f, &tp);
            }
        }
        let img = surf.image_snapshot();
        let data = img.encode(None, skia_safe::EncodedImageFormat::PNG, 100).ok_or_else(|| anyhow!("png"))?;
        if let Some(p) = out.parent() { std::fs::create_dir_all(p)?; }
        std::fs::write(out, data.as_bytes())?;
        Ok(())
    }

    /// The frames blended into one (weights 1..n: the first faintest, the last strongest; as fframes' onion skin),
    /// with a label row: the range and the frame times.
    pub fn onion(out: &Path, imgs: &[(usize, Vec<u8>)], times: &[f64], w: u32, h: u32) -> Result<()> {
        let n = imgs.len();
        if n == 0 { return Err(anyhow!("no frames for the onion")); }
        let total = (n * (n + 1) / 2) as u32;
        let mut px = vec![0u8; imgs[0].1.len()];
        for (k, v) in px.iter_mut().enumerate() {
            let sum: u32 = imgs.iter().enumerate().map(|(i, (_, f))| f[k] as u32 * (i as u32 + 1)).sum();
            *v = ((sum + total / 2) / total) as u8;
        }
        let lab = 34.;
        let mut surf = surfaces::raster_n32_premul((w as i32, h as i32 + lab as i32)).ok_or_else(|| anyhow!("onion surface"))?;
        let c = surf.canvas();
        c.clear(Color::from_rgb(20, 20, 20));
        if let Some(img) = raster(&px, w, h) {
            c.draw_image(&img, (0., lab), None);
        }
        let fm = FontMgr::new();
        let tf = fm.match_family_style("sans-serif", FontStyle::bold()).or_else(|| fm.legacy_make_typeface(None, FontStyle::bold()));
        if let Some(f) = tf.map(|t| Font::from_typeface(t, 16.)) {
            let mut tp = Paint::default();
            tp.set_color(Color::from_rgb(240, 240, 240));
            tp.set_anti_alias(true);
            let ts: Vec<String> = times.iter().map(|t| format!("{t:.2}")).collect();
            c.draw_str(format!("{n} frames, faint = early: {} s", ts.join(" ")), (10., 23.), &f, &tp);
        }
        let img = surf.image_snapshot();
        let data = img.encode(None, skia_safe::EncodedImageFormat::PNG, 100).ok_or_else(|| anyhow!("png"))?;
        if let Some(p) = out.parent() { std::fs::create_dir_all(p)?; }
        std::fs::write(out, data.as_bytes())?;
        Ok(())
    }

    fn raster(px: &[u8], w: u32, h: u32) -> Option<skia_safe::Image> {
        let info = ImageInfo::new((w as i32, h as i32), ColorType::BGRA8888, AlphaType::Premul, None);
        skia_safe::images::raster_from_data(&info, Data::new_copy(px), (w * 4) as usize)
    }

    /// A note's sheet: the note's frame big (fits 1200 x 900) on a pixel grid in world px with the user's drawing
    /// and each element's box + id; below it the other frames (4 a row, 300 px), with thinner grid and boxes.
    /// `imgs[0]` is the note's frame, `labels` the raw clip time of each frame.
    #[allow(clippy::too_many_arguments)]
    pub fn note(out: &Path, imgs: &[Vec<u8>], w: u32, h: u32, world: (f64, f64), boxes: &[Vec<(String, [f64; 4])>], labels: &[String], drawing: Option<&Path>) -> Result<()> {
        let (ww, wh) = world;
        let sheet_w = 1200f32;
        let big = (sheet_w / ww as f32).min(900. / wh as f32);
        let (bw, bh) = (ww as f32 * big, wh as f32 * big);
        let lab = 30f32;
        let tw = sheet_w / 4.;
        let small = tw / ww as f32;
        let th = wh as f32 * small;
        let rest = imgs.len().saturating_sub(1);
        let rows = rest.div_ceil(4);
        let mut surf = surfaces::raster_n32_premul((sheet_w as i32, (lab + bh + (lab + th) * rows as f32) as i32)).ok_or_else(|| anyhow!("sheet surface"))?;
        let c = surf.canvas();
        c.clear(Color::from_rgb(20, 20, 20));
        let fm = FontMgr::new();
        let tf = fm.match_family_style("sans-serif", FontStyle::bold()).or_else(|| fm.legacy_make_typeface(None, FontStyle::bold()));
        let font = |sz: f32| tf.clone().map(|t| Font::from_typeface(t, sz));
        let paint = |r: u8, g: u8, b: u8, a: u8, sw: f32| {
            let mut p = Paint::default();
            p.set_color(Color::from_argb(a, r, g, b));
            p.set_anti_alias(true);
            if sw > 0. { p.set_style(skia_safe::paint::Style::Stroke); p.set_stroke_width(sw); }
            p
        };
        let samp = skia_safe::SamplingOptions::new(skia_safe::FilterMode::Linear, skia_safe::MipmapMode::Linear);
        let text = paint(240, 240, 240, 255, 0.);
        // one panel: frame, grid (step world px), boxes, label
        let panel = |c: &skia_safe::Canvas, i: usize, x: f32, y: f32, sc: f32, step: f64, big: bool| {
            let (pw, ph) = (ww as f32 * sc, wh as f32 * sc);
            if let Some(img) = imgs.get(i).and_then(|px| raster(px, w, h)) {
                c.draw_image_rect_with_sampling_options(&img, None, Rect::from_xywh(x, y + lab, pw, ph), samp, &Paint::default());
            }
            let mut g = 0.;
            while g <= ww.max(wh) {
                let major = (g / step).round() as i64 % 5 == 0;
                let p = if major { paint(255, 230, 0, 150, 1.) } else { paint(255, 230, 0, 70, 1.) };
                if g <= ww { c.draw_line((x + g as f32 * sc, y + lab), (x + g as f32 * sc, y + lab + ph), &p); }
                if g <= wh { c.draw_line((x, y + lab + g as f32 * sc), (x + pw, y + lab + g as f32 * sc), &p); }
                if big && g > 0. {
                    if let Some(f) = font(12.) {
                        let (lp, bg) = (paint(255, 230, 0, 255, 0.), paint(0, 0, 0, 170, 0.));
                        let tag = |px: f32, py: f32| {
                            let s = format!("{g:.0}");
                            c.draw_rect(Rect::from_xywh(px, py - 11., 7.5 * s.len() as f32 + 3., 14.), &bg);
                            c.draw_str(&s, (px + 1.5, py), &f, &lp);
                        };
                        if g < ww { tag(x + g as f32 * sc + 1., y + lab + 13.); }
                        if g < wh { tag(x + 1., y + lab + g as f32 * sc - 3.); }
                    }
                }
                g += step;
            }
            for (id, r) in boxes.get(i).map(|v| v.as_slice()).unwrap_or(&[]) {
                let rr = Rect::from_ltrb(x + r[0] as f32 * sc, y + lab + r[1] as f32 * sc, x + r[2] as f32 * sc, y + lab + r[3] as f32 * sc);
                c.draw_rect(rr, &paint(0, 220, 255, 230, if big { 2. } else { 1. }));
                if let Some(f) = font(if big { 16. } else { 11. }) {
                    c.draw_str(id, (rr.left + 2., rr.top - 3.), &f, &paint(0, 220, 255, 255, 0.));
                }
            }
            if let Some(f) = font(if big { 20. } else { 15. }) {
                let what = if big { format!("THE NOTE  {}", labels.get(i).map(String::as_str).unwrap_or("")) } else { format!("#{i}  {}", labels.get(i).map(String::as_str).unwrap_or("")) };
                c.draw_str(what, (x + 6., y + lab - 9.), &f, &text);
            }
        };
        panel(c, 0, (sheet_w - bw) / 2., 0., big, 100., true);
        if let Some(d) = drawing.and_then(|p| std::fs::read(p).ok()).and_then(|b| skia_safe::Image::from_encoded(Data::new_copy(&b))) {
            c.draw_image_rect_with_sampling_options(&d, None, Rect::from_xywh((sheet_w - bw) / 2., lab, bw, bh), samp, &Paint::default());
        }
        for i in 1..imgs.len() {
            let (col, row) = ((i - 1) % 4, (i - 1) / 4);
            panel(c, i, col as f32 * tw, lab + bh + row as f32 * (lab + th), small, 200., false);
        }
        let img = surf.image_snapshot();
        let data = img.encode(None, skia_safe::EncodedImageFormat::PNG, 100).ok_or_else(|| anyhow!("png"))?;
        if let Some(p) = out.parent() { std::fs::create_dir_all(p)?; }
        std::fs::write(out, data.as_bytes())?;
        Ok(())
    }
}

fn main() {
    trace::start();
    // big frame buffers (8 MB) must come from the heap and be reused: with glibc's default they are mmapped and
    // unmapped every frame, and the page faults serialise the workers (glibc only)
    #[cfg(target_os = "linux")]
    // SAFETY: called once at start, before any other thread exists; mallopt only changes allocator thresholds
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 512 << 20);
        libc::mallopt(libc::M_TRIM_THRESHOLD, 1 << 30);
        libc::mallopt(libc::M_ARENA_MAX, 32);
    }
    // the matches too, not only the parsed struct: they tell --root from AIOS_ROOT (aios config shows which)
    let m = Cli::command().get_matches();
    let cli = Cli::from_arg_matches(&m).unwrap_or_else(|e| e.exit());
    let given = cli.root.as_deref().map(|r| match m.value_source("root") {
        Some(ValueSource::EnvVariable) => (r, Found::Env),
        _ => (r, Found::Flag),
    });
    let layout = match Layout::load(given) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("aios: {e:#}");
            std::process::exit(layout::EX_CONFIG);
        }
    };
    let name = m.subcommand_name().unwrap_or_default().to_string();
    let r = run(cli.cmd, &layout);
    trace::finish(r.is_ok(), &name, &layout);
    if let Err(e) = r {
        eprintln!("aios: {e:#}");
        std::process::exit(1);
    }
}
