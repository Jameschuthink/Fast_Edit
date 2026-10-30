//! `aios sound JOB`: the mix read without listening. One row per beat (from one graphic's entrance to the next):
//! RMS and peak level and the silent time in it, the whole mix's loudness and true peak (as the Reels check
//! measures them), and a waveform PNG with a line at each beat and a tick at each placed sound effect.

use crate::job::Job;
use anyhow::{anyhow, Result};
use serde_json::Value;
use skia_safe::{surfaces, Color, Font, FontMgr, FontStyle, Paint, Rect};
use std::path::Path;
use std::process::Command;

const RATE: usize = 8000; // decoded rate: enough for levels and a waveform, 6x less than 48 kHz
const SILENT_DB: f64 = -45.; // a 50 ms block below this is silent
const BLOCK: usize = RATE / 20;

/// A beat: where a graphic (text or picture) enters, in output seconds.
pub struct Beat {
    pub t: f64,
    pub label: String,
}

/// The job's beats: edit.txt text and pictures by their start; none (an older job) = one row per second.
pub fn beats(job: &Job) -> Vec<Beat> {
    let mut v: Vec<Beat> = Vec::new();
    for k in ["items", "pics"] {
        for i in job.cfg["edit"][k].as_array().into_iter().flatten() {
            let Some(t) = i["t0"].as_f64() else { continue };
            let id = i["id"].as_str().unwrap_or("");
            let label = match i["text"].as_str() {
                Some(x) => format!("{id} \"{x}\""),
                None => id.to_string(),
            };
            v.push(Beat { t, label });
        }
    }
    v.sort_by(|a, b| a.t.total_cmp(&b.t));
    // graphics that enter together are one beat
    let mut out: Vec<Beat> = Vec::new();
    for b in v {
        match out.last_mut() {
            Some(l) if b.t - l.t < 0.05 => { l.label.push_str(", "); l.label.push_str(&b.label); }
            _ => out.push(b),
        }
    }
    out
}

/// Placed effects in output seconds (the time their sync point lands) and a short name.
pub fn sfx_ticks(job: &Job, extra: &[Value]) -> Vec<(f64, String)> {
    let mut v: Vec<(f64, String)> = Vec::new();
    for e in job.cfg["sfx"].as_array().into_iter().flatten().chain(extra) {
        let Some(t) = e["t"].as_f64() else { continue };
        let f = e["file"].as_str().unwrap_or("");
        let name = Path::new(f).file_stem().and_then(|s| s.to_str()).unwrap_or(f);
        v.push((t, name.chars().take(14).collect()));
    }
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v
}

/// The mix as mono samples (the louder channel per sample, so a one-sided peak is kept) at RATE.
fn decode(p: &Path) -> Result<Vec<f32>> {
    let o = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(p)
        .args(["-ac", "2", "-ar", &RATE.to_string(), "-f", "f32le", "-"])
        .output()?;
    if !o.status.success() {
        return Err(anyhow!("decode mix: {}", String::from_utf8_lossy(&o.stderr)));
    }
    Ok(o.stdout
        .chunks_exact(8)
        .map(|c| {
            let l = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            let r = f32::from_le_bytes([c[4], c[5], c[6], c[7]]);
            if l.abs() >= r.abs() { l } else { r }
        })
        .collect())
}

fn db(x: f64) -> f64 {
    if x <= 1e-9 { -99. } else { 20. * x.log10() }
}

/// Level of samples [a, b): (RMS dBFS, peak dBFS, silent seconds).
fn level(s: &[f32], a: usize, b: usize) -> (f64, f64, f64) {
    let w = &s[a.min(s.len())..b.min(s.len())];
    if w.is_empty() { return (-99., -99., 0.); }
    let ms = w.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / w.len() as f64;
    let pk = w.iter().fold(0f32, |m, x| m.max(x.abs())) as f64;
    let silent = w.chunks(BLOCK).filter(|c| {
        let m = c.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / c.len() as f64;
        db(m.sqrt()) < SILENT_DB
    }).map(|c| c.len()).sum::<usize>();
    (db(ms.sqrt()), db(pk), silent as f64 / RATE as f64)
}

/// Prints the report and writes the waveform. `t0` = the mix's first output second.
pub fn report(job: &Job, mix: &Path, t0: f64, dur: f64, extra: &[Value], png: &Path) -> Result<()> {
    let s = decode(mix)?;
    let (lufs, tp) = crate::audio::measure(mix);
    let t1 = t0 + dur;
    let mut bs: Vec<Beat> = beats(job).into_iter().filter(|b| b.t >= t0 && b.t < t1).collect();
    let per_second = bs.is_empty();
    if per_second {
        bs = (0..dur.ceil() as usize).map(|i| Beat { t: t0 + i as f64, label: String::new() }).collect();
    }
    if bs.first().map(|b| b.t > t0 + 0.05).unwrap_or(true) {
        bs.insert(0, Beat { t: t0, label: "(start)".into() });
    }
    println!("mix {lufs:.1} LUFS, {tp:.1} dBTP ({:.1}-{:.1} s){}", t0, t1, if per_second { ", no graphics: one row a second" } else { "" });
    let idx = |t: f64| (((t - t0).max(0.)) * RATE as f64) as usize;
    for (k, b) in bs.iter().enumerate() {
        let e = bs.get(k + 1).map(|n| n.t).unwrap_or(t1);
        let (rms, pk, sil) = level(&s, idx(b.t), idx(e));
        let mut flags = String::new();
        if pk > -1. { flags.push_str("  PEAK"); }
        if sil > 0.25 * (e - b.t).max(0.05) { flags.push_str(&format!("  silent {sil:.2} s")); }
        println!("#{} {:.2}-{:.2} s  rms {rms:6.1}  peak {pk:5.1} dBFS{flags}  {}", k + 1, b.t, e, b.label);
    }
    let ticks: Vec<(f64, String)> = sfx_ticks(job, extra).into_iter().filter(|x| x.0 >= t0 && x.0 < t1).collect();
    if !ticks.is_empty() {
        println!("sfx: {}", ticks.iter().map(|(t, n)| format!("{t:.2} {n}")).collect::<Vec<_>>().join(", "));
    }
    // long silences anywhere (0.3 s or more)
    let mut gaps: Vec<(f64, f64)> = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in s.chunks(BLOCK).enumerate() {
        let m = c.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / c.len() as f64;
        let quiet = db(m.sqrt()) < SILENT_DB;
        match (quiet, start) {
            (true, None) => start = Some(i),
            (false, Some(a)) => { gaps.push((a as f64, i as f64)); start = None; }
            _ => {}
        }
    }
    if let Some(a) = start { gaps.push((a as f64, s.len().div_ceil(BLOCK) as f64)); }
    let bt = BLOCK as f64 / RATE as f64;
    let gaps: Vec<String> = gaps.iter().filter(|(a, b)| (b - a) * bt >= 0.3).map(|(a, b)| format!("{:.2}-{:.2}", t0 + a * bt, t0 + b * bt)).collect();
    if !gaps.is_empty() { println!("silent (< {SILENT_DB} dBFS, 0.3 s+): {} s", gaps.join(", ")); }
    waveform(&s, t0, dur, &bs, &ticks, png)?;
    println!("waveform: {}", png.display());
    Ok(())
}

/// Min/max envelope per column, beat lines (blue, numbered), sfx ticks (orange, named), a mark each second.
fn waveform(s: &[f32], t0: f64, dur: f64, bs: &[Beat], ticks: &[(f64, String)], out: &Path) -> Result<()> {
    let (w, h) = (1600f32, 320f32);
    let (top, wave_h) = (24f32, 240f32);
    let mid = top + wave_h / 2.;
    let mut surf = surfaces::raster_n32_premul((w as i32, h as i32)).ok_or_else(|| anyhow!("waveform surface"))?;
    let c = surf.canvas();
    c.clear(Color::from_rgb(20, 20, 20));
    let paint = |r: u8, g: u8, b: u8, a: u8| {
        let mut p = Paint::default();
        p.set_color(Color::from_argb(a, r, g, b));
        p.set_anti_alias(true);
        p
    };
    let x_of = |t: f64| ((t - t0) / dur.max(1e-6)) as f32 * w;
    let fm = FontMgr::new();
    let font = fm.match_family_style("sans-serif", FontStyle::bold()).or_else(|| fm.legacy_make_typeface(None, FontStyle::bold())).map(|t| Font::from_typeface(t, 12.));
    // -1 dBFS guides and seconds
    let guide = paint(120, 40, 40, 255);
    let g = 10f32.powf(-1. / 20.) * wave_h / 2.;
    c.draw_rect(Rect::from_xywh(0., mid - g, w, 1.), &guide);
    c.draw_rect(Rect::from_xywh(0., mid + g, w, 1.), &guide);
    let sec = paint(70, 70, 70, 255);
    let txt = paint(200, 200, 200, 255);
    for i in 0..=dur.floor() as usize {
        let x = x_of(t0 + i as f64);
        c.draw_rect(Rect::from_xywh(x, top + wave_h, 1., 8.), &sec);
        if let (Some(f), true) = (&font, i % 5 == 0) { c.draw_str(format!("{:.0}", t0 + i as f64), (x + 2., top + wave_h + 20.), f, &txt); }
    }
    // envelope
    let wave = paint(170, 210, 170, 255);
    let per = (s.len() as f32 / w).max(1.);
    for x in 0..w as usize {
        let a = (x as f32 * per) as usize;
        let b = (((x + 1) as f32 * per) as usize).min(s.len());
        if a >= b { continue; }
        let (lo, hi) = s[a..b].iter().fold((0f32, 0f32), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
        c.draw_rect(Rect::from_xywh(x as f32, mid - hi * wave_h / 2., 1., ((hi - lo) * wave_h / 2.).max(1.)), &wave);
    }
    // beats
    let blue = paint(90, 160, 255, 220);
    for (k, b) in bs.iter().enumerate() {
        let x = x_of(b.t);
        c.draw_rect(Rect::from_xywh(x, top, 1.5, wave_h), &blue);
        if let Some(f) = &font { c.draw_str(format!("#{}", k + 1), (x + 3., 16.), f, &blue); }
    }
    // sfx
    let orange = paint(255, 160, 40, 255);
    for (i, (t, n)) in ticks.iter().enumerate() {
        let x = x_of(*t);
        c.draw_rect(Rect::from_xywh(x - 1., top + wave_h - 14., 3., 14.), &orange);
        if let Some(f) = &font { c.draw_str(n, (x + 3., h - 22. + 12. * (i % 2) as f32), f, &orange); }
    }
    let img = surf.image_snapshot();
    let data = img.encode(None, skia_safe::EncodedImageFormat::PNG, 100).ok_or_else(|| anyhow!("png"))?;
    if let Some(p) = out.parent() { std::fs::create_dir_all(p)?; }
    std::fs::write(out, data.as_bytes())?;
    Ok(())
}
