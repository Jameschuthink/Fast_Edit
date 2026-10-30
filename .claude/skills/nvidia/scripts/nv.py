#!/usr/bin/env python3
"""nv.py: NVIDIA hosted APIs for talking-head reels (execution skill).

  nv.py voice  IN -o OUT.wav|OUT.mp4   Studio Voice 48k-hq, then loudness -16 LUFS / -1.5 dBTP.
                                        A video OUT gets the clean voice muxed in (picture stream-copied).
  nv.py relight IN.mp4 -o OUT.mp4 [--hdr FILE] [--keep-texture]   Maxine Relighting, studio map (assets/studio.hdr: key 45 deg,
                                                 top, rim; user pick "A" 2026-10-06), background blur 0 (the user wants NO
                                                 softness on the background). The service smooths the face: the user likes
                                                 that. --keep-texture puts the original skin detail back (6.56 -> 4.41 -> 6.23).
  nv.py eyes   IN.mp4 -o OUT.mp4 [--strong]      Maxine Eye Contact, glance-away off.
  nv.py asr    IN -o OUT.words.json             CrisperWhisper 2.0 large on the local GPU (default; models/crisperwhisper/):
                                                 verbatim words (fillers [um], repeats, cut-offs, sound events) with word
                                                 times from its trained cross-attention (~30 ms), so a breath or a pause
                                                 sits between two words (faster-whisper dropped the fillers and stretched
                                                 a word over them: user, 2026-10-29). Digits come from the intended pass of
                                                 the same decode; each sounding gap is decoded again (asr.py doc). No
                                                 hotwords (Pro models only).
               [--script FILE]                   the script (its last ``` block, else all of it): a word diff script vs
                                                 heard for the agent to check ([um] and other tags left out of the diff).
               [--whisper]                       the old engine: faster-whisper large-v3-turbo (about 9 s per minute).
                                                 With --script: the script as hotwords (spelling bias on every 30 s window;
                                                 initial_prompt only reaches the first one). Hotwords also invent words
                                                 (a test job: "effortless" -> "better"), so a plain pass runs too (+7 s) and a
                                                 part where the two differ takes the hot words only when they, with a word
                                                 each side, are in the script.
               [--hosted]                        NVIDIA Parakeet (2 s, but it dropped a fast list of 12 words on a test job).
  nv.py matte  IN.mp4 -o DIR [--from A --to B --fps 30]   Person matte: relight on flat green, keyed; RGB = the clip.
                                                 DIR/00001.webp .. for the engine's seq(). Offline fallback: models/people/matte.py.
  nv.py doctor                                   key present, clients present, ffmpeg present.

Shared flags: --dry-run (print the plan, run nothing, write nothing), --json (one JSON document on stdout),
--overwrite (allow an existing OUT). The input is never changed. Each run verifies its output
(frame count equal for video, duration within 0.05 s for audio, words non-empty) and exits 1 on failure.
"""
import argparse, json, os, re, shutil, subprocess, sys, tempfile, time
from pathlib import Path

SKILL = Path(__file__).resolve().parents[1]
ROOT = SKILL.parents[2]                      # ~/AIOS3
PY = SKILL / ".venv/bin/python"
CW = ROOT / "models/crisperwhisper"        # its own venv: its CTranslate2 fork must not share one with faster-whisper
NIM = SKILL / "vendor/nim-clients"
HOST = "grpc.nvcf.nvidia.com:443"
FUNC = {  # build.nvidia.com function IDs (checked 2026-10-06)
    "voice": "3f0aeba3-6d91-4465-b8cc-cc2aef355186",
    "relight": "253bc77f-2d6c-4b9d-86c6-c6fb0f9e239c",
    "eyes": "480bffcb-6fea-4571-9741-eb50212c9c0f",
    "asr": "d3fe9151-442b-4204-a70d-5fcc597fd610",
}
STRONG = [a for k in ("gaze-pitch", "gaze-yaw", "head-pitch", "head-yaw")
          for a in (f"--{k}-threshold-low", "34", f"--{k}-threshold-high", "35")] + ["--eye-size-sensitivity", "5"]


def key():
    for line in (ROOT / ".env").read_text().splitlines():
        if line.startswith("NVIDIA_API_KEY="):
            return line.split("=", 1)[1].strip()
    die("NVIDIA_API_KEY missing in ~/AIOS3/.env", "missing_key")


def die(msg, kind="error", code=1):
    if ARGS and ARGS.json:
        print(json.dumps({"status": "error", "kind": kind, "message": msg}))
    else:
        print(f"nv.py: {msg}", file=sys.stderr)
    sys.exit(code)


def run(cmd, secret=None, **kw):
    p = subprocess.run([str(c) for c in cmd], capture_output=True, text=True, **kw)
    log = (p.stdout + p.stderr).replace("\r", "\n")
    if secret:
        log = log.replace(secret, "***")
    return p.returncode, log


def probe(path):
    out = subprocess.run(["ffprobe", "-v", "error", "-count_packets", "-show_entries",
                          "stream=codec_type,nb_read_packets,width,height,r_frame_rate:format=duration",
                          "-of", "json", str(path)], capture_output=True, text=True).stdout
    d = json.loads(out or "{}")
    v = next((s for s in d.get("streams", []) if s.get("codec_type") == "video"), {})
    a = next((s for s in d.get("streams", []) if s.get("codec_type") == "audio"), None)
    return {"duration": float(d.get("format", {}).get("duration", 0) or 0), "frames": int(v.get("nb_read_packets", 0) or 0),
            "width": v.get("width"), "height": v.get("height"), "fps": v.get("r_frame_rate"), "audio": a is not None}


def ff(*args):
    rc, log = run(["ffmpeg", "-v", "error", "-y", *args])
    if rc:
        die("ffmpeg failed: " + log[-400:])


def loudnorm(inp, out, I=-16, tp=-1.5, lra=11):
    """Two-pass loudnorm (linear) to a 16-bit wav at the input's sample rate; lower the peak ceiling while the
    written file's true peak is still over tp (same steps as ffmpeg-skill loudness.py, which it replaces)."""
    def measure(p):
        rc, log = run(["ffmpeg", "-hide_banner", "-nostdin", "-i", str(p), "-vn", "-af",
                       f"loudnorm=I={I}:TP={tp}:LRA={lra}:print_format=json", "-f", "null", "-"])
        m = re.search(r"\{[^{}]*\"input_i\"[^{}]*\}", log, re.S)
        if rc or not m:
            die("loudness measurement failed: " + log[-400:])
        return json.loads(m.group(0))
    st = measure(inp)
    if st["input_i"] == "-inf":
        die("the voice is silent: nothing to level")
    sr = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "a:0", "-show_entries", "stream=sample_rate",
                         "-of", "csv=p=0", str(inp)], capture_output=True, text=True).stdout.strip() or "48000"
    ceiling = tp
    for _ in range(6):
        ff("-i", inp, "-af", f"loudnorm=I={I}:TP={ceiling}:LRA={lra}:measured_I={st['input_i']}:measured_TP={st['input_tp']}"
           f":measured_LRA={st['input_lra']}:measured_thresh={st['input_thresh']}:offset={st['target_offset']}"
           f":linear=true:print_format=summary", "-ar", sr, "-vn", "-c:a", "pcm_s16le", out)
        peak = float(measure(out)["input_tp"])
        if peak <= tp + 0.1:
            return
        ceiling -= peak - tp + 0.2
    die(f"true peak still {peak:.2f} dBTP over {tp} after 6 passes")


def guard(inp, out):
    if not Path(inp).is_file():
        die(f"input not found: {inp}", "input")
    if Path(out).resolve() == Path(inp).resolve():
        die("output must not be the input (inputs are never changed)", "input")
    if Path(out).exists() and not ARGS.overwrite:
        die(f"output exists: {out} (pass --overwrite to replace it)", "input")
    Path(out).parent.mkdir(parents=True, exist_ok=True)


def report(doc, human):
    doc.setdefault("status", "ok")
    print(json.dumps(doc, indent=1) if ARGS.json else human)


def upright(src, tmp):
    """The services ignore rotation metadata: re-encode upright first (also gives a clean CFR stream)."""
    up = tmp / "upright.mp4"
    ff("-i", src, "-an", "-c:v", "libx264", "-preset", "fast", "-crf", "14", "-pix_fmt", "yuv420p", up)
    return up


def cmd_voice(a):
    guard(a.input, a.output)
    out = Path(a.output)
    plan = [f"extract 48 kHz mono WAV from {a.input}", f"Studio Voice 48k-hq ({HOST})",
            "loudness -16 LUFS, -1.5 dBTP (two-pass loudnorm)"] + (["mux into video, picture stream-copied"] if out.suffix != ".wav" else [])
    if a.dry_run:
        return report({"dry_run": True, "plan": plan, "output": str(out)}, "\n".join(plan))
    t0, k = time.time(), key()
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        raw, clean, level = td / "raw.wav", td / "clean.wav", td / "level.wav"
        ff("-i", a.input, "-vn", "-ac", "1", "-ar", "48000", "-c:a", "pcm_s16le", raw)
        rc, log = run([PY, "studio_voice.py", "--preview-mode", "--target", HOST, "--function-id", FUNC["voice"],
                       "--api-key", k, "--input", raw, "--output", clean, "--model-type", "48k-hq"],
                      secret=k, cwd=NIM / "studio-voice/scripts")
        if rc or not clean.exists():
            die("Studio Voice failed: " + log[-500:], "service")
        # Studio Voice returns the voice late (measured 143 ms on a test job, 2026-10-06): measure the lag against the raw
        # audio by cross-correlation and cut it from the head (pad the tail), so the lips stay in sync.
        lag = measure_lag(raw, clean)
        if abs(lag) > 0.005:
            fixed = td / "aligned.wav"
            af = f"atrim=start={lag:.4f},asetpts=PTS-STARTPTS,apad" if lag > 0 else f"adelay={-lag * 1000:.1f}:all=1"
            ff("-i", clean, "-af", af, "-t", f"{probe(raw)['duration']:.4f}", fixed)
            clean = fixed
            rest = measure_lag(raw, clean)
            if abs(rest) > 0.01:
                die(f"voice still {rest * 1000:.0f} ms off after the fix", "verify")
        loudnorm(clean, level)
        if out.suffix == ".wav":
            shutil.copy(level, out)
        else:
            ff("-i", a.input, "-i", level, "-map", "0:v", "-map", "1:a", "-c:v", "copy", "-c:a", "aac", "-b:a", "192k", "-shortest", out)
    pi, po = probe(a.input), probe(out)
    ok = abs(pi["duration"] - po["duration"]) <= 0.05 + (1 / 30)
    if not ok:
        die(f"duration changed {pi['duration']:.3f} -> {po['duration']:.3f}", "verify")
    report({"output": str(out), "seconds": round(time.time() - t0, 1), "duration": po["duration"], "lag_fixed_ms": round(lag * 1000, 1),
            "verified": True}, f"voice: {out} ({po['duration']:.2f} s, lag {lag * 1000:.0f} ms removed, {time.time() - t0:.0f} s)")


LAG = r'''
import sys, numpy as np, soundfile as sf
from scipy.signal import fftconvolve
a, ra = sf.read(sys.argv[1]); b, rb = sf.read(sys.argv[2])
a = a if a.ndim == 1 else a.mean(1); b = b if b.ndim == 1 else b.mean(1)
n = min(len(a), len(b), ra * 40); a = a[:n] - a[:n].mean(); b = b[:n] - b[:n].mean()
c = fftconvolve(b, a[::-1], mode="full"); print((np.argmax(c) - (n - 1)) / ra)
'''


def measure_lag(ref, test):
    """Seconds `test` lags `ref` (positive = late), cross-correlation over the first 40 s."""
    p = subprocess.run([str(PY), "-c", LAG, str(ref), str(test)], capture_output=True, text=True)
    if p.returncode:
        die("lag measure failed: " + p.stderr[-300:])
    return float(p.stdout.strip())


def service(name, up, res, extra, k):
    """One hosted video call: upright MP4 in, MP4 out."""
    script, flag = {"relight": ("relighting/scripts/relighting.py", "--video-input"),
                    "eyes": ("eye-contact/scripts/eye-contact.py", "--input")}[name]
    rc, log = run([PY, Path(script).name, "--preview-mode", "--target", HOST, "--function-id", FUNC[name], "--api-key", k,
                   flag, up, "--output", res, "--bitrate", "12000000", *extra],
                  secret=k, cwd=NIM / Path(script).parent)
    if rc or not res.exists() or "error occurred" in log.lower():
        die(f"{name} failed: " + log[-500:], "service")


def video_service(a, name, extra):
    guard(a.input, a.output)
    plan = [f"upright re-encode of {a.input}", f"{name} ({HOST})", "mux the original audio back", "verify frame count"]
    if a.dry_run:
        return report({"dry_run": True, "plan": plan, "output": a.output}, "\n".join(plan))
    t0, k = time.time(), key()
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        up, res = upright(a.input, td), td / "service.mp4"
        service(name, up, res, extra, k)
        if name == "relight" and getattr(a, "keep_texture", False):
            tex = td / "texture.mp4"
            p = subprocess.run([str(PY), "-c", TEXTURE, str(up), str(res), str(tex)], capture_output=True, text=True)
            if p.returncode or not tex.exists():
                die("texture restore failed: " + p.stderr[-300:])
            res = tex
        if probe(a.input)["audio"]:
            ff("-i", res, "-i", a.input, "-map", "0:v", "-map", "1:a", "-c:v", "copy", "-c:a", "copy", a.output)
        else:
            shutil.copy(res, a.output)
        fi, fo = probe(up)["frames"], probe(a.output)["frames"]
    if fi != fo:
        die(f"frame count changed {fi} -> {fo}", "verify")
    report({"output": a.output, "frames": fo, "seconds": round(time.time() - t0, 1), "verified": True},
           f"{name}: {a.output} ({fo} frames, {time.time() - t0:.0f} s)")


TEXTURE = r'''
import subprocess, sys, cv2, numpy as np
src, rel, out, s, k = sys.argv[1], sys.argv[2], sys.argv[3], 2.0, 1.0
a, b = cv2.VideoCapture(src), cv2.VideoCapture(rel)
W, H, fps = int(b.get(3)), int(b.get(4)), b.get(5)
enc = subprocess.Popen(["ffmpeg", "-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "bgr24", "-s", f"{W}x{H}", "-r", str(fps), "-i", "-",
                        "-c:v", "libx264", "-crf", "14", "-preset", "fast", "-pix_fmt", "yuv420p", out], stdin=subprocess.PIPE)
n = 0
while True:
    ok1, O = a.read(); ok2, R = b.read()
    if not (ok1 and ok2): break
    yo = cv2.cvtColor(O, cv2.COLOR_BGR2YCrCb).astype(np.float32)[..., 0]
    yr = cv2.cvtColor(R, cv2.COLOR_BGR2YCrCb).astype(np.float32)
    hp = lambda y: y - cv2.GaussianBlur(y, (0, 0), s)
    yr[..., 0] = np.clip(yr[..., 0] + k * (hp(yo) - hp(yr[..., 0])), 0, 255)   # put back the detail the relight removed
    enc.stdin.write(cv2.cvtColor(yr.astype(np.uint8), cv2.COLOR_YCrCb2BGR).tobytes()); n += 1
enc.stdin.close(); enc.wait(); print(n)
'''


def cmd_relight(a):
    hdr = Path(a.hdr or SKILL / "assets/studio.hdr")
    video_service(a, "relight", ["--hdr", hdr, "--background-source", "0", "--background-gain", "0.25", "--specular", "0.6", "--blur", "0"])


KEY = r'''
import sys, cv2, numpy as np
from concurrent.futures import ThreadPoolExecutor
src, green, out, gain = sys.argv[1], sys.argv[2], sys.argv[3], float(sys.argv[4])
a, b = cv2.VideoCapture(src), cv2.VideoCapture(green)
def write(n, O, G):                                       # WebP encode is the slow part: 12 threads (cv2 frees the GIL)
    g = G.astype(np.float32) / 255                       # BGR
    al = 1 - np.clip((g[..., 1] - np.maximum(g[..., 0], g[..., 2])) * gain, 0, 1)
    cv2.imwrite(f"{out}/{n:05d}.webp", np.dstack([O, (al * 255 + 0.5).astype(np.uint8)]), [cv2.IMWRITE_WEBP_QUALITY, 90])
n, jobs = 0, []
with ThreadPoolExecutor(12) as pool:
    while True:
        ok1, O = a.read(); ok2, G = b.read()
        if not (ok1 and ok2): break
        n += 1; jobs.append(pool.submit(write, n, O, G))
    for j in jobs: j.result()
print(n)
'''


def cmd_matte(a):
    """Person matte from Maxine Relighting on a flat green background: alpha = the green key, RGB = the clip itself
    (the relit picture is not used). Same output as models/people/matte.py (RVM, the offline fallback)."""
    if not Path(a.input).is_file():
        die(f"input not found: {a.input}", "input")
    out = Path(a.output)
    if any(out.glob("*.webp")) and not a.overwrite:
        die(f"output has frames: {out} (pass --overwrite to replace them)", "input")
    t1 = a.to if a.to is not None else probe(a.input)["duration"]
    plan = [f"{a.input} {a.t0:g}-{t1:g} s at {a.fps:g} fps", f"relight on green 0x00FF00, lossless ({HOST})",
            f"key (gain {a.gain:g}) -> {out}/00001.webp .. (RGB = the clip)", "verify frame count"]
    if a.dry_run:
        return report({"dry_run": True, "plan": plan, "output": str(out)}, "\n".join(plan))
    t0, k = time.time(), key()
    out.mkdir(parents=True, exist_ok=True)
    for f in out.glob("*.webp"): f.unlink()
    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        seg, res = td / "seg.mp4", td / "green.mp4"
        ff("-ss", f"{a.t0}", "-to", f"{t1}", "-i", a.input, "-vf", f"fps={a.fps}", "-an",
           "-c:v", "libx264", "-preset", "fast", "-crf", "12", "-pix_fmt", "yuv420p", seg)
        service("relight", seg, res, ["--hdr", SKILL / "assets/studio.hdr", "--background-source", "1",
                                      "--background-color", "0x00FF00", "--lossless"], k)
        p = subprocess.run([str(PY), "-c", KEY, str(seg), str(res), str(out), str(a.gain)], capture_output=True, text=True)
        if p.returncode:
            die("key failed: " + p.stderr[-300:])
        n, fi = int(p.stdout.strip()), probe(seg)["frames"]
    if n != fi:
        die(f"frame count changed {fi} -> {n}", "verify")
    meta = {"clip": a.input, "from": a.t0, "to": t1, "fps": a.fps, "n": n, "model": "nvidia-relight-green-key",
            "gain": a.gain}
    (out / "matte.json").write_text(json.dumps(meta, indent=1))
    report({"output": str(out), "frames": n, "seconds": round(time.time() - t0, 1), "verified": True},
           f"matte: {out}/00001.webp .. {n:05d}.webp ({n} frames, {time.time() - t0:.0f} s)\n"
           f"  comp: seq('{out.name}/%05d.webp', {n}, {a.fps:g}).at(t - {a.t0:g})")


def cmd_eyes(a):
    video_service(a, "eyes", ["--enable-lookaway", "0"] + (STRONG if a.strong else []))


def cmd_asr(a):
    guard(a.input, a.output)
    # no CrisperWhisper on this computer (the Mac: no heavy local model, user 2026-10-30) = the hosted Parakeet
    if not (a.hosted or a.whisper) and not (CW / ".venv/bin/python").exists():
        if not (ROOT / ".env").exists() or "NVIDIA_API_KEY=" not in (ROOT / ".env").read_text():
            die(f"no CrisperWhisper venv and no NVIDIA_API_KEY: run `bash {CW}/setup.sh` once (~4 GB), or add the key "
                "to .env for the hosted Parakeet", "setup")
        print("asr: no CrisperWhisper here, using the hosted Parakeet (it may drop um/uh)", file=sys.stderr)
        a.hosted = True
    engine = (f"parakeet-tdt-0.6b-v2 ({HOST})" if a.hosted else "faster-whisper large-v3-turbo (local)" if a.whisper
              else "crisperwhisper 2.0 large verbatim (local)")
    if a.dry_run:
        return report({"dry_run": True, "plan": [f"16 kHz mono WAV from {a.input}", engine, "write words.json"]}, engine)
    t0 = time.time()
    with tempfile.TemporaryDirectory() as td:
        wav = Path(td) / "a.wav"
        ff("-i", a.input, "-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le", wav)
        env = dict(os.environ, NVK=key() if a.hosted else "")
        hot = script_text(a.script) if a.script and a.whisper else ""
        if a.hosted or a.whisper:
            if a.whisper:  # CTranslate2 needs cuBLAS/cuDNN from the pip wheels
                libs = sorted(str(d) for d in (SKILL / ".venv/lib").glob("python*/site-packages/nvidia/*/lib"))
                env["LD_LIBRARY_PATH"] = ":".join(libs + [env.get("LD_LIBRARY_PATH", "")])
            cmd = [str(PY), "-c", HOSTED if a.hosted else LOCAL, str(wav), hot]
        else:
            cmd = [str(CW / ".venv/bin/python"), str(CW / "asr.py"), str(wav)]
        p = subprocess.run(cmd, capture_output=True, text=True, env=env)
        if p.returncode:
            die("asr failed: " + (p.stderr[-500:].replace(env["NVK"], "***") if env["NVK"] else p.stderr[-500:]), "service")
        words = json.loads(p.stdout)
        if hot:
            words, n = vote(words["plain"], words["hot"], hot)
            print(f"asr --script: {n} part(s) taken from the hotwords pass")
    if not words:
        die("no words recognised", "verify")
    doc = {"source": engine, "words": words, "text": " ".join(w["word"] for w in words)}
    Path(a.output).write_text(json.dumps(doc, indent=1))
    if a.script:
        print(script_diff(script_text(a.script), words))
    report({"output": a.output, "words": len(words), "seconds": round(time.time() - t0, 1), "engine": engine, "verified": True},
           f"asr: {a.output} ({len(words)} words, {engine}, {time.time() - t0:.0f} s)")


def script_text(path):
    """The spoken words of a script file: its last ``` block (the teleprompter or as-filmed block), else all of it."""
    t = Path(path).read_text()
    blocks = re.findall(r"```[^\n]*\n(.*?)```", t, re.S)
    return " ".join((blocks[-1] if blocks else t).split())


def _norm(w):
    return re.sub(r"[^a-z0-9']", "", w.lower())


def vote(plain, hot, script):
    """Plain words, except a part where the hot pass differs and its words (with one word each side) are in the script.
    Where both agree, the hot words: their punctuation follows the script (a test job: 9 sentences, plain 4)."""
    import difflib
    sc = " " + " ".join(_norm(w) for w in script.split()) + " "
    a, b = [_norm(w["word"]) for w in plain], [_norm(w["word"]) for w in hot]
    out, n = [], 0
    for op, i1, i2, j1, j2 in difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes():
        ctx = " " + " ".join(a[max(i1 - 1, 0):i1] + b[j1:j2] + a[i2:i2 + 1]) + " "
        if op == "equal":
            out += hot[j1:j2]
        elif j2 > j1 and ctx in sc:
            out += hot[j1:j2]; n += 1
        else:
            out += plain[i1:i2]
    return out, n


def script_diff(script, words):
    """Where the take differs from the script, by word: '#N ~T s: script "a b" / heard "c d"'.
    A tag ([um], [laughter]) is no difference: a script has none. Repeats and cut-offs stay (retakes, step 5 Clean)."""
    import difflib
    words = [w for w in words if not w["word"].startswith("[")] or words
    sw = script.split()
    a, b = [_norm(w) for w in sw], [_norm(w["word"]) for w in words]
    out = []
    for op, i1, i2, j1, j2 in difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes():
        if op == "equal":
            continue
        t = words[min(j1, len(words) - 1)]["start"]
        out.append(f'#{len(out) + 1} ~{t:.1f} s: script "{" ".join(sw[i1:i2])}" / heard "{" ".join(w["word"] for w in words[j1:j2])}"')
    return "script vs heard: " + (f"{len(out)} differences (check the spelling ones; the heard words stay)\n" + "\n".join(out) if out else "same words")


HOSTED = r'''
import json, os, sys, wave, riva.client
auth = riva.client.Auth(uri="grpc.nvcf.nvidia.com:443", use_ssl=True,
    metadata_args=[["function-id", "d3fe9151-442b-4204-a70d-5fcc597fd610"], ["authorization", "Bearer " + os.environ["NVK"]]])
asr = riva.client.ASRService(auth)
with wave.open(sys.argv[1], "rb") as w:
    sr, data = w.getframerate(), w.readframes(w.getnframes())
cfg = riva.client.RecognitionConfig(encoding=riva.client.AudioEncoding.LINEAR_PCM, sample_rate_hertz=sr, language_code="en-US",
    max_alternatives=1, enable_automatic_punctuation=True, enable_word_time_offsets=True)
resp = asr.offline_recognize(data, cfg)
out = []
for r in resp.results:
    for w in r.alternatives[0].words:
        out.append({"word": w.word, "start": round(w.start_time / 1000, 3), "end": round(w.end_time / 1000, 3)})
print(json.dumps(out))
'''

LOCAL = r'''
import json, sys
from faster_whisper import WhisperModel
m = WhisperModel("large-v3-turbo", device="cuda", compute_type="int8_float16")
def words(hot=None):
    segs, _ = m.transcribe(sys.argv[1], word_timestamps=True, language="en", vad_filter=True, beam_size=5,
                           condition_on_previous_text=False, hotwords=hot)
    return [{"word": w.word.strip(), "start": round(w.start, 3), "end": round(w.end, 3)} for s in segs for w in s.words]
print(json.dumps({"plain": words(), "hot": words(sys.argv[2])} if sys.argv[2] else words()))
'''


def cmd_doctor(a):
    checks = {
        "key": any(l.startswith("NVIDIA_API_KEY=") and len(l) > 20 for l in (ROOT / ".env").read_text().splitlines()),
        "venv": PY.exists(),
        "crisperwhisper": (CW / ".venv/bin/python").exists() and any((CW / "weights").glob("*/model.bin")),
        "clients": all((NIM / p).exists() for p in ("studio-voice/scripts/studio_voice.py", "relighting/scripts/relighting.py",
                                                     "eye-contact/scripts/eye-contact.py")),
        "ffmpeg": shutil.which("ffmpeg") is not None,
    }
    ok = all(checks.values())
    report({"status": "ok" if ok else "error", "checks": checks}, "\n".join(f"{'OK  ' if v else 'FAIL'} {k}" for k, v in checks.items()))
    sys.exit(0 if ok else 1)


ARGS = None


def main():
    global ARGS
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("voice", "relight", "eyes", "asr"):
        s = sub.add_parser(name)
        s.add_argument("input")
        s.add_argument("-o", "--output", required=True)
        s.add_argument("--dry-run", action="store_true")
        s.add_argument("--json", action="store_true")
        s.add_argument("--overwrite", action="store_true")
        if name == "relight":
            s.add_argument("--hdr")
            s.add_argument("--keep-texture", action="store_true")
        if name == "eyes":
            s.add_argument("--strong", action="store_true")
        if name == "asr":
            s.add_argument("--hosted", action="store_true", help="NVIDIA Parakeet (fast, rough)")
            s.add_argument("--whisper", action="store_true", help="the old engine: faster-whisper large-v3-turbo")
            s.add_argument("--script", help="script file: a word diff script vs heard (with --whisper: hotwords too)")
    m = sub.add_parser("matte")
    m.add_argument("input")
    m.add_argument("-o", "--output", required=True, help="frame folder, e.g. jobs/JOB/matte")
    m.add_argument("--from", dest="t0", type=float, default=0.0)
    m.add_argument("--to", type=float)
    m.add_argument("--fps", type=float, default=30)
    m.add_argument("--gain", type=float, default=1.25, help="key strength (AIOS test 2026-10-05: 1.25)")
    for f in ("--dry-run", "--json", "--overwrite"):
        m.add_argument(f, action="store_true")
    d = sub.add_parser("doctor")
    d.add_argument("--json", action="store_true")
    d.add_argument("--dry-run", action="store_true")
    ARGS = ap.parse_args()
    {"voice": cmd_voice, "relight": cmd_relight, "eyes": cmd_eyes, "asr": cmd_asr, "matte": cmd_matte, "doctor": cmd_doctor}[ARGS.cmd](ARGS)


if __name__ == "__main__":
    main()
