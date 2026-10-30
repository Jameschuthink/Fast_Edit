//! Video out: frames converted to YUV 4:2:0 (BT.709, limited range) inside the render workers, then piped in order
//! to one ffmpeg that encodes on the GPU (NVENC) — or with x264 when asked or when NVENC is missing.

use anyhow::{anyhow, Result};
use std::io::Write;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};

#[derive(Clone, Debug)]
pub struct EncOpts {
    pub codec: String, // h264_nvenc | hevc_nvenc | libx264 | ...
    pub quality: u32,  // cq / crf (lower = better)
    pub preset: String,
    pub audio: Option<String>,
}

impl EncOpts {
    /// Drafts: x264 (starts in 0.15 s where NVENC needs 0.6 s, and a 540 px frame is cheap on the CPU).
    pub fn draft() -> Self {
        EncOpts { codec: "libx264".into(), quality: 23, preset: "veryfast".into(), audio: None }
    }
    pub fn final_() -> Self {
        EncOpts { codec: "h264_nvenc".into(), quality: 18, preset: "p4".into(), audio: None }
    }
}

pub struct Encoder {
    child: Child,
    stdin: Option<ChildStdin>,
    pub frames: usize,
}

pub fn nvenc_ok() -> bool {
    static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OK.get_or_init(|| {
        Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i", "color=c=black:s=256x256:r=30", "-frames:v", "2", "-c:v", "h264_nvenc", "-f", "null", "-"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

impl Encoder {
    pub fn new(out: &Path, w: u32, h: u32, fps: f64, o: &EncOpts) -> Result<Encoder> {
        Self::spawn(out, w, h, fps, o, None)
    }

    /// One encoder for a run of cache chunks: an IDR frame and a new file at every split (seconds from the run's
    /// start); files are `pattern` (e.g. dir/%05d.mp4), numbered from 0.
    pub fn segments(pattern: &Path, w: u32, h: u32, fps: f64, o: &EncOpts, splits: &[f64]) -> Result<Encoder> {
        Self::spawn(pattern, w, h, fps, o, Some(splits))
    }

    fn spawn(out: &Path, w: u32, h: u32, fps: f64, o: &EncOpts, splits: Option<&[f64]>) -> Result<Encoder> {
        let mut codec = o.codec.clone();
        if codec.ends_with("_nvenc") && !nvenc_ok() {
            eprintln!("NVENC not available: encoding with libx264");
            codec = "libx264".into();
        }
        let mut c = Command::new("ffmpeg");
        c.args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "yuv420p", "-s", &format!("{w}x{h}"), "-r", &format_fps(fps)]);
        // the frames are BT.709 limited (bgra_to_yuv420, yuv.rs): say so on the input too, or ffmpeg takes them as
        // unknown (= BT.601) and inserts a scaler that converts every frame to the BT.709 asked for below (a CPU pass
        // and a colour shift: luma -2.3 on average)
        c.args(["-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709", "-color_range", "tv", "-i", "pipe:0"]);
        if let Some(a) = &o.audio {
            c.args(["-i", a]);
        }
        c.args(["-map", "0:v:0"]);
        if o.audio.is_some() {
            c.args(["-map", "1:a:0", "-c:a", "aac", "-b:a", "192k", "-shortest"]);
        }
        c.args(["-c:v", &codec]);
        if codec.ends_with("_nvenc") {
            c.args(["-preset", &o.preset, "-tune", "hq", "-rc", "vbr", "-cq", &o.quality.to_string(), "-b:v", "0"]);
            if o.quality <= 20 {
                // finals: B-frames and adaptive quantisation (slower, better)
                c.args(["-bf", "2", "-spatial-aq", "1"]);
            }
            if codec == "h264_nvenc" {
                c.args(["-profile:v", "high"]);
            }
        } else if codec == "libx264" {
            let preset = if o.preset.starts_with('p') { "veryfast".to_string() } else { o.preset.clone() };
            c.args(["-preset", &preset, "-crf", &o.quality.to_string(), "-profile:v", "high", "-threads", "4"]);
        }
        c.args(["-pix_fmt", "yuv420p", "-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709", "-color_range", "tv"]);
        if let Some(sp) = splits {
            if codec.ends_with("_nvenc") {
                c.args(["-forced-idr", "1"]);
            }
            let times: Vec<String> = sp.iter().map(|t| format!("{t:.6}")).collect();
            if !times.is_empty() {
                c.args(["-force_key_frames", &times.join(",")]);
            }
            c.args(["-f", "segment", "-segment_format", "mp4", "-reset_timestamps", "1"]);
            if !times.is_empty() {
                c.args(["-segment_times", &times.join(",")]);
            }
            c.arg(out);
        } else {
            let ext = out.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "mp4" || ext == "mov" {
                c.args(["-movflags", "+faststart"]);
            }
            c.arg(out);
        }
        c.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
        let mut child = c.spawn().map_err(|e| anyhow!("ffmpeg: {e}"))?;
        let stdin = child.stdin.take();
        #[cfg(target_os = "linux")]
        if let Some(s) = &stdin {
            // a 1 MB pipe instead of 64 KB: a 1080x1920 frame is 3 MB, so far fewer wake-ups (Linux only)
            use std::os::fd::AsRawFd;
            unsafe { libc::fcntl(s.as_raw_fd(), libc::F_SETPIPE_SZ, 1 << 20) };
        }
        Ok(Encoder { child, stdin, frames: 0 })
    }

    pub fn write(&mut self, yuv: &[u8]) -> Result<()> {
        self.stdin.as_mut().ok_or_else(|| anyhow!("encoder closed"))?.write_all(yuv).map_err(|e| anyhow!("encoder pipe: {e}"))?;
        self.frames += 1;
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        drop(self.stdin.take());
        let out = self.child.wait_with_output()?;
        if !out.status.success() {
            return Err(anyhow!("ffmpeg encode failed: {}", String::from_utf8_lossy(&out.stderr)));
        }
        Ok(())
    }
}

pub fn format_fps(f: f64) -> String {
    if (f - f.round()).abs() < 1e-9 { format!("{}", f.round() as i64) } else { format!("{f:.6}") }
}

/// BGRA (premultiplied, opaque or not: alpha is dropped over black) → planar YUV 4:2:0, BT.709 limited range.
/// Two rows per pass: Y for both, U/V from the 2x2 average.
pub fn bgra_to_yuv420(src: &[u8], w: usize, h: usize, out: &mut Vec<u8>) {
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    out.resize(w * h + 2 * cw * ch, 0);
    let (yp, uv) = out.split_at_mut(w * h);
    let (up, vp) = uv.split_at_mut(cw * ch);
    #[inline(always)]
    fn luma(b: i32, g: i32, r: i32) -> u8 {
        ((11966 * r + 40254 * g + 4064 * b + 32768 + (16 << 16)) >> 16) as u8
    }
    for cy in 0..ch {
        let y0 = cy * 2;
        let y1 = (y0 + 1).min(h - 1);
        let r0 = &src[y0 * w * 4..y0 * w * 4 + w * 4];
        let r1 = &src[y1 * w * 4..y1 * w * 4 + w * 4];
        let (ya, yb) = yp.split_at_mut(y1.max(y0 + 1).min(h) * w);
        let yr0 = &mut ya[y0 * w..y0 * w + w];
        let ur = &mut up[cy * cw..cy * cw + cw];
        let vr = &mut vp[cy * cw..cy * cw + cw];
        let pairs = w / 2;
        for cx in 0..pairs {
            let i0 = cx * 8;
            let (b00, g00, r00) = (r0[i0] as i32, r0[i0 + 1] as i32, r0[i0 + 2] as i32);
            let (b01, g01, r01) = (r0[i0 + 4] as i32, r0[i0 + 5] as i32, r0[i0 + 6] as i32);
            let (b10, g10, r10) = (r1[i0] as i32, r1[i0 + 1] as i32, r1[i0 + 2] as i32);
            let (b11, g11, r11) = (r1[i0 + 4] as i32, r1[i0 + 5] as i32, r1[i0 + 6] as i32);
            yr0[cx * 2] = luma(b00, g00, r00);
            yr0[cx * 2 + 1] = luma(b01, g01, r01);
            if y1 != y0 {
                let yr1 = &mut yb[cx * 2..cx * 2 + 2];
                yr1[0] = luma(b10, g10, r10);
                yr1[1] = luma(b11, g11, r11);
            }
            let (b, g, r) = (b00 + b01 + b10 + b11, g00 + g01 + g10 + g11, r00 + r01 + r10 + r11);
            ur[cx] = ((-6596 * r - 22189 * g + 28785 * b + 131072 + (128 << 18)) >> 18).clamp(0, 255) as u8;
            vr[cx] = ((28785 * r - 26145 * g - 2640 * b + 131072 + (128 << 18)) >> 18).clamp(0, 255) as u8;
        }
        if w % 2 == 1 {
            let x = w - 1;
            let (b, g, r) = (r0[x * 4] as i32, r0[x * 4 + 1] as i32, r0[x * 4 + 2] as i32);
            yr0[x] = luma(b, g, r);
            if y1 != y0 { yb[x] = luma(r1[x * 4] as i32, r1[x * 4 + 1] as i32, r1[x * 4 + 2] as i32); }
            ur[cw - 1] = ((-6596 * r - 22189 * g + 28785 * b + 32768 + (128 << 16)) >> 16).clamp(0, 255) as u8;
            vr[cw - 1] = ((28785 * r - 26145 * g - 2640 * b + 32768 + (128 << 16)) >> 16).clamp(0, 255) as u8;
        }
    }
}
