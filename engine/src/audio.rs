//! Sound: voice + song (ducked under the voice) + sound effects, mastered to the Reels level, built as one ffmpeg
//! graph while the picture renders. Same recipe as AIOS2's reel.py mix (ffmpeg-skill audio.py + loudness.py):
//! stereo voice base, song from `from` with gain / fade-out / optional jump, sidechain ducking (threshold 0.05,
//! ratio duck/3, attack 20 ms, release 400 ms), effects never ducked, two-pass loudnorm to -14 LUFS / -1.5 dBTP.
//!
//! job.json:
//!   "voice": "voice.wav"                     (default: the clip's own audio)
//!   "song": {"file", "from", "gain_db", "fade_out", "duck_db", "jump": {"at", "to", "xfade"}, "holes": [[a, b], …]}
//!                                            holes: the song is silent from video second a to b (30 ms ramps), so a
//!                                            riser end or an impact lands in a gap (sfx skill)
//!   "sfx": [{"t": 3.2, "file": "SFX/whoosh.mp3", "gain_db": -6, "sync": 0.4, "from", "to"}, …]   placed sounds
//!                                            (library: the aios.toml `sfx` folder, INDEX.md); sync = the second of the file that lands on t
//!                                            (a riser's end, a whoosh's peak; default: from, or 0 = the file start);
//!                                            "fade_in" / "fade_out" (s, fade_out needs "to"): default 5 ms on a cut edge
//!   "sfx_bed": "sfx.wav"                     a pre-mixed effects track (AIOS2 jobs have one)
//!   "loudness": {"lufs": -14, "tp": -1.5}
//!   "mute": ["song", "voice", "sfx"]         leave these out of the mix but keep their settings (a quick listen);
//!                                            "song": {…, "mute": true} works too. A muted voice still ducks the song

use crate::job::Job;
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn has_audio(p: &Path) -> bool {
    Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "a:0", "-show_entries", "stream=index", "-of", "csv=p=0"])
        .arg(p)
        .output()
        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false)
}

/// A sound file named in job.json: relative to the job, to AIOS3, or (ported jobs) to AIOS2.
pub fn find(job: &Job, f: &str) -> Option<PathBuf> {
    let p = PathBuf::from(f);
    if p.is_absolute() {
        return p.exists().then_some(p);
    }
    let mut cands = vec![job.dir.join(f), job.layout.root().join(f)];
    if let Some(a2) = job.cfg["aios2"].as_str() {
        let a2 = PathBuf::from(a2);
        cands.push(a2.join(f));
        if let Some(root) = a2.parent().and_then(|p| p.parent()) {
            cands.push(root.join(f));
        }
    }
    cands.into_iter().find(|c| c.exists())
}

pub struct Mix {
    pub wav: PathBuf,
    pub lufs: f64,
    pub tp: f64,
    pub notes: Vec<String>,
}

/// Sounds the comp places itself (a global SFX list), read once from a JS runtime.
pub fn comp_sfx(job: &Job) -> Vec<Value> {
    let assets = crate::assets::Assets::new(job.dir.clone(), job.fps, false, true, 1.);
    let shaper = std::rc::Rc::new(std::cell::RefCell::new(crate::text::Shaper::new()));
    let r = crate::js::Js::new(&job.src, assets.clone(), shaper)
        .and_then(|js| js.eval_json("typeof SFX !== 'undefined' ? SFX : []"));
    assets.stop();
    match r {
        Ok(Value::Array(a)) => a,
        _ => Vec::new(),
    }
}

/// Mixes the job's sound for [t0, t0+dur) into `out` (48 kHz stereo, AAC or wav by its name: `codec_args`), mastered.
/// `extra`: placed sounds from the comp ({t, file, gain_db}), added to job.json "sfx".
pub fn mix(job: &Job, t0: f64, dur: f64, extra: &[Value], out: &Path) -> Result<Mix> {
    let c = &job.cfg;
    let mut notes = Vec::new();
    let mut args: Vec<String> = vec!["-v".into(), "error".into(), "-y".into(), "-nostdin".into()];
    let mut graph: Vec<String> = Vec::new();
    let mut idx = 0usize;
    let mut mix_in: Vec<String> = Vec::new();
    let fmt = "aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo";
    let muted = |k: &str| {
        c["mute"].as_array().is_some_and(|a| a.iter().any(|v| v.as_str() == Some(k))) || c[k]["mute"].as_bool() == Some(true)
    };
    for k in ["voice", "song", "sfx"] {
        if muted(k) { notes.push(format!("muted: {k}")); }
    }
    // voice
    let voice = c["voice"].as_str().and_then(|v| find(job, v)).or_else(|| {
        c["clip"].as_str().and_then(|v| find(job, v)).filter(|p| has_audio(p))
    });
    let mut voice_label = None;
    if let Some(v) = &voice {
        args.extend(["-ss".into(), format!("{t0:.6}"), "-i".into(), v.to_string_lossy().to_string()]);
        graph.push(format!("[{idx}:a]{fmt},apad,atrim=0:{dur:.6}[voice0]"));
        voice_label = Some("voice0".to_string());
        idx += 1;
    } else {
        notes.push("no voice: the clip has no audio and job.json has no \"voice\"".into());
    }
    // song
    if let Some(s) = c["song"].as_object().filter(|_| !muted("song")) {
        let file = s.get("file").and_then(|v| v.as_str()).unwrap_or("");
        match find(job, file) {
            None => notes.push(format!("song not found: {file}")),
            Some(sp) => {
                let from = s.get("from").and_then(|v| v.as_f64()).unwrap_or(0.) + t0;
                let gain = s.get("gain_db").and_then(|v| v.as_f64()).unwrap_or(-12.);
                let fo = s.get("fade_out").and_then(|v| v.as_f64()).unwrap_or(1.5);
                let duck = s.get("duck_db").and_then(|v| v.as_f64()).unwrap_or(12.);
                let total = job.dur;
                let fade_st = (total - fo - t0).max(0.);
                // holes: silent from video second a to b; gain = product of per-hole ramps (30 ms), evaluated every 256 samples
                let holes: Vec<(f64, f64)> = s.get("holes").and_then(|v| v.as_array()).map(|a| {
                    a.iter().filter_map(|h| Some((h.get(0)?.as_f64()?, h.get(1)?.as_f64()?))).filter(|(a, b)| b > a).collect()
                }).unwrap_or_default();
                let holes = if holes.is_empty() {
                    String::new()
                } else {
                    let tt = format!("(t+{t0:.6})");
                    let g: Vec<String> = holes.iter()
                        .map(|(a, b)| format!("max(clip(({a:.4}-{tt})/0.03,0,1),clip(({tt}-{b:.4})/0.03,0,1))"))
                        .collect();
                    format!(",asetnsamples=n=256:p=0,volume='{}':eval=frame", g.join("*"))
                };
                if let Some(j) = s.get("jump").and_then(|v| v.as_object()) {
                    // play from `from`; at video second `at` cross (equal power) to song second `to`
                    let at = j.get("at").and_then(|v| v.as_f64()).unwrap_or(0.);
                    let to = j.get("to").and_then(|v| v.as_f64()).unwrap_or(0.);
                    let h = j.get("xfade").and_then(|v| v.as_f64()).unwrap_or(0.3) / 2.;
                    let a_len = (at + h - t0).max(0.01);
                    let b_from = to - h + (t0 - at).max(0.);
                    args.extend(["-ss".into(), format!("{from:.6}"), "-t".into(), format!("{a_len:.6}"), "-i".into(), sp.to_string_lossy().to_string()]);
                    args.extend(["-ss".into(), format!("{b_from:.6}"), "-i".into(), sp.to_string_lossy().to_string()]);
                    graph.push(format!("[{}:a][{}:a]acrossfade=d={:.6}:c1=qsin:c2=qsin,{fmt},volume={gain}dB,apad,atrim=0:{dur:.6},afade=t=out:st={fade_st:.6}:d={fo}{holes}[song]", idx, idx + 1, 2. * h));
                    idx += 2;
                } else {
                    args.extend(["-ss".into(), format!("{from:.6}"), "-i".into(), sp.to_string_lossy().to_string()]);
                    graph.push(format!("[{idx}:a]{fmt},volume={gain}dB,apad,atrim=0:{dur:.6},afade=t=out:st={fade_st:.6}:d={fo}{holes}[song]"));
                    idx += 1;
                }
                if let Some(v) = &voice_label {
                    graph.push(format!("[{v}]asplit=2[voice][sc]"));
                    graph.push(format!(
                        "[song][sc]sidechaincompress=threshold=0.05:ratio={:.1}:attack=20:release=400:makeup=1[ducked]",
                        (duck / 3.).max(2.)
                    ));
                    voice_label = Some("voice".into());
                    mix_in.push("ducked".into());
                } else {
                    mix_in.push("song".into());
                }
            }
        }
    }
    if let Some(v) = &voice_label {
        if muted("voice") {
            graph.push(format!("[{v}]anullsink"));
            voice_label = None;
        } else {
            mix_in.insert(0, v.clone());
        }
    }
    // effects bed
    if let Some(b) = c["sfx_bed"].as_str().filter(|_| !muted("sfx")) {
        match find(job, b) {
            Some(bp) => {
                args.extend(["-ss".into(), format!("{t0:.6}"), "-i".into(), bp.to_string_lossy().to_string()]);
                let g = c["sfx_bed_gain_db"].as_f64().unwrap_or(0.);
                graph.push(format!("[{idx}:a]{fmt},volume={g}dB,apad,atrim=0:{dur:.6}[bed]"));
                mix_in.push("bed".into());
                idx += 1;
            }
            None => notes.push(format!("sfx_bed not found: {b}")),
        }
    }
    // placed effects
    let mut placed: Vec<Value> = c["sfx"].as_array().cloned().unwrap_or_default();
    placed.extend(extra.iter().cloned());
    if muted("sfx") { placed.clear(); }
    {
        let list = &placed;
        for (k, e) in list.iter().enumerate() {
            // `sync`: the second of the file that lands on `t` (default `from`, or 0): the file's `from` point plays at t - (sync - from)
            let from0 = e["from"].as_f64().unwrap_or(0.);
            let t = e["t"].as_f64().unwrap_or(0.) - (e["sync"].as_f64().unwrap_or(from0) - from0) - t0;
            let file = e["file"].as_str().unwrap_or("");
            if t < -60. || t > dur {
                continue;
            }
            let Some(fp) = find(job, file) else {
                notes.push(format!("sfx not found: {file}"));
                continue;
            };
            let g = e["gain_db"].as_f64().unwrap_or(0.);
            let (ss, delay) = if t < 0. { (-t, 0.) } else { (0., t) };
            let mut len = None;
            if let Some(from) = e["from"].as_f64() {
                args.extend(["-ss".into(), format!("{:.6}", from + ss)]);
                if let Some(to) = e["to"].as_f64() {
                    let l = (to - from - ss).max(0.01);
                    args.extend(["-t".into(), format!("{l:.6}")]);
                    len = Some(l);
                }
            } else if ss > 0. {
                args.extend(["-ss".into(), format!("{ss:.6}")]);
            }
            args.extend(["-i".into(), fp.to_string_lossy().to_string()]);
            // fades (s): `fade_in`, `fade_out` (with `to`); default 5 ms where the file is cut, so a trimmed sound has no click
            let fi = e["fade_in"].as_f64().unwrap_or(if from0 + ss > 0. { 0.005 } else { 0. });
            let fo = e["fade_out"].as_f64().unwrap_or(if len.is_some() { 0.005 } else { 0. });
            let mut fades = String::new();
            if fi > 0. {
                fades += &format!(",afade=t=in:d={fi:.4}");
            }
            if let (Some(l), true) = (len, fo > 0.) {
                fades += &format!(",afade=t=out:st={:.4}:d={:.4}", (l - fo).max(0.), fo.min(l));
            }
            graph.push(format!("[{idx}:a]{fmt},volume={g}dB{fades},adelay={}:all=1[fx{k}]", (delay * 1000.).round() as i64));
            mix_in.push(format!("fx{k}"));
            idx += 1;
        }
    }
    let raw = out.with_extension("raw.wav");
    if voice_label.is_none() {
        // no voice: a silent base first, so the mix (duration=first) runs the whole length, not to the end of one effect;
        // with nothing else in the mix the base is the track (silence, so the video still has one)
        args.extend(["-f".into(), "lavfi".into(), "-t".into(), format!("{dur:.6}"), "-i".into(), "anullsrc=r=48000:cl=stereo".into()]);
        graph.push(format!("[{idx}:a]{fmt}[base]"));
        mix_in.insert(0, "base".into());
    }
    let inputs: String = mix_in.iter().map(|l| format!("[{l}]")).collect();
    graph.push(format!("{inputs}amix=inputs={}:duration=first:dropout_transition=0:normalize=0,atrim=0:{dur:.6}[mix]", mix_in.len()));
    args.extend(["-filter_complex".into(), graph.join(";"), "-map".into(), "[mix]".into(), "-c:a".into(), "pcm_f32le".into()]);
    args.push(raw.to_string_lossy().to_string());
    run(&args)?;
    // master: two-pass loudnorm
    let (lufs, tp) = master(&raw, out, c)?;
    let _ = std::fs::remove_file(&raw);
    Ok(Mix { wav: out.to_path_buf(), lufs, tp, notes })
}

fn run(args: &[String]) -> Result<()> {
    let o = Command::new("ffmpeg").args(args).output()?;
    if !o.status.success() {
        return Err(anyhow!("ffmpeg audio: {}", String::from_utf8_lossy(&o.stderr).lines().take(6).collect::<Vec<_>>().join(" | ")));
    }
    Ok(())
}

/// The codec of a mastered sound file, by its name: `.m4a` (the mix cache, stream-copied into the video) = AAC 256k,
/// anything else = 16-bit wav. Every write of `out` asks here: PCM in an `.m4a` fails (ffmpeg's ipod muxer holds no
/// PCM) or, copied in as bytes, reaches the video as a PCM track.
fn codec_args(out: &Path) -> &'static [&'static str] {
    if out.extension().is_some_and(|e| e == "m4a") { &["-c:a", "aac", "-b:a", "256k"] } else { &["-c:a", "pcm_s16le"] }
}

fn master(raw: &Path, out: &Path, c: &Value) -> Result<(f64, f64)> {
    let lufs = c["loudness"]["lufs"].as_f64().unwrap_or(-14.);
    let tp = c["loudness"]["tp"].as_f64().unwrap_or(-1.5);
    let lra = 11.;
    let o = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostdin", "-i"])
        .arg(raw)
        .args(["-af", &format!("loudnorm=I={lufs}:TP={tp}:LRA={lra}:print_format=json"), "-f", "null", "-"])
        .output()?;
    let err = String::from_utf8_lossy(&o.stderr);
    let js = err.rfind('{').and_then(|a| err[a..].find('}').map(|b| &err[a..a + b + 1]));
    let m: Value = js.and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default();
    let gi = |k: &str| m[k].as_str().unwrap_or("0").to_string();
    if gi("input_i") == "-inf" {
        // silence: nothing to normalize, only the codec `out` asks for
        let o = Command::new("ffmpeg").args(["-v", "error", "-y", "-nostdin", "-i"]).arg(raw).args(codec_args(out)).arg(out).output()?;
        if !o.status.success() {
            return Err(anyhow!("silence: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("")));
        }
        return Ok((f64::NEG_INFINITY, f64::NEG_INFINITY));
    }
    // the encoder can push peaks over the ceiling (AAC overshoot): measure the encoded file and lower the ceiling
    // by the overshoot, up to three times (AIOS2's loudness.py did the same)
    let mut ceiling = tp;
    let mut last = (f64::NAN, f64::NAN);
    for _ in 0..3 {
        let af = format!(
            "loudnorm=I={lufs}:TP={ceiling:.2}:LRA={lra}:measured_I={}:measured_TP={}:measured_LRA={}:measured_thresh={}:offset={}:linear=true:print_format=json",
            gi("input_i"), gi("input_tp"), gi("input_lra"), gi("input_thresh"), gi("target_offset")
        );
        let o2 = Command::new("ffmpeg")
            .args(["-hide_banner", "-nostdin", "-y", "-i"])
            .arg(raw)
            .args(["-af", &af, "-ar", "48000"])
            .args(codec_args(out))
            .arg(out)
            .output()?;
        if !o2.status.success() {
            return Err(anyhow!("loudnorm: {}", String::from_utf8_lossy(&o2.stderr).lines().last().unwrap_or("")));
        }
        let (i_out, tp_out) = measure(out);
        last = (i_out, tp_out);
        if !(tp_out > tp) {
            break;
        }
        ceiling -= tp_out - tp + 0.2;
    }
    Ok(last)
}

/// Integrated loudness and true peak of a file, as the Reels check measures them (ebur128).
pub fn measure(p: &Path) -> (f64, f64) {
    let o = Command::new("ffmpeg").args(["-hide_banner", "-nostdin", "-i"]).arg(p).args(["-af", "ebur128=peak=true", "-f", "null", "-"]).output();
    let Ok(o) = o else { return (f64::NAN, f64::NAN) };
    let err = String::from_utf8_lossy(&o.stderr);
    let grab = |key: &str| -> f64 {
        err.rfind(key).and_then(|i| err[i + key.len()..].split_whitespace().next().and_then(|v| v.parse().ok())).unwrap_or(f64::NAN)
    };
    (grab("I:"), grab("Peak:"))
}

/// Picture + sound → one file (picture stream copied, sound AAC 256k).
pub fn mux(video: &Path, wav: &Path, out: &Path) -> Result<()> {
    let o = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-nostdin", "-i"])
        .arg(video)
        .arg("-i")
        .arg(wav)
        .args(["-map", "0:v:0", "-map", "1:a:0", "-c:v", "copy"])
        .args(if wav.extension().map(|e| e == "m4a").unwrap_or(false) { vec!["-c:a", "copy"] } else { vec!["-c:a", "aac", "-b:a", "256k"] })
        .args(["-shortest", "-movflags", "+faststart"])
        .arg(out)
        .output()?;
    if !o.status.success() {
        return Err(anyhow!("mux: {}", String::from_utf8_lossy(&o.stderr)));
    }
    Ok(())
}
