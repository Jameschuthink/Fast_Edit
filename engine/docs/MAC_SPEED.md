# MAC_SPEED: options to make this engine faster or fuller on a Mac (none built yet)

The engine was built on Linux with an NVIDIA card. On a Mac it falls back: painting on the CPU (`gpu_none.rs`),
encoding with libx264, decoding on the CPU. It works; it is slower. When a render feels slow or a feature is
missing, propose the matching option below to the user (numbered, your pick marked, the cost). Build only on a yes.

## Order
1. **Measure first (no code).** `aios draft JOB` on a real job: note the seconds; `aios bench JOB` prints where the
   time goes (painting, decoding, encoding); `aios perf` shows the speed log. Pick the next step from the biggest part.
   On an M1 laptop: 8 cores (4 fast + 4 efficiency); a MacBook Air has no fan and slows on long renders; with 8 GB of
   memory try fewer workers (`--workers N`, see `aios draft --help`).

## Options
| # | Option | Helps | Where | Size | Trade-off |
|---|---|---|---|---|---|
| S1 | VideoToolbox encode: `h264_videotoolbox -q:v 65` (Apple Silicon has no `-crf` there) as the Mac default codec | encoding moves off the CPU, so all cores paint | `engine/src/encode.rs` (`EncOpts`, the codec args), the `--codec` defaults in `main.rs` | small | lower quality per file size than x264; check the Reels check still passes |
| S2 | VideoToolbox decode: `-hwaccel videotoolbox` for source clips | decoding moves off the CPU | `engine/src/video.rs` (`spawn`, the seek proxy), `parts` in `main.rs` (hwaccel is off outside Linux now) | small-medium | the CUDA scale filters there do not apply; frames must match the CPU decode |
| S3 | Metal painting: a Metal `Gpu` (Skia `metal` feature is already on in `Cargo.toml`) | painting, the biggest part on most jobs | `engine/src/gpu_none.rs` -> a Metal context; `render.rs` uses `crate::gpu::Gpu` | big | new GPU code; the GPU YUV pass (`yuv_none.rs`) stays on the CPU unless ported too |
| S4 | Fewer workers / smaller drafts by default on a Mac | memory and heat on a laptop | `settings` in `main.rs` | small | slower on a big Mac |

## Not on this Mac (Linux only today)
| Missing | Why | Option |
|---|---|---|
| `voice` take key (clean voice) | the NVIDIA Studio Voice clients are not in this copy | add the NVIDIA client scripts under `.claude/skills/nvidia/vendor/`, then let `aios take` write `voice` again (`main.rs` Take) |
| `frame=` take key (reframe a wide take) | a CUDA model | a CPU crop that follows the face, or keep takes vertical |
| CrisperWhisper (verbatim words with um/uh) | a ~4 GB local model on CUDA | the hosted Parakeet is used (it may drop fillers); a Mac port: the section below |
| HTML images in a job | needs headless Chromium at a Linux path | set `AIOS_CHROME` to a headless Chromium on the Mac |
| Songs and sound effects | the library is empty in this copy | the user adds files + `library/songs/songs.json` / `library/sfx/index.json` |

## CrisperWhisper on a Mac (if the user wants it; checked 2026-10-30 on crisperwhisper 2.0.3)
- The hook: `nv.py asr` runs `models/crisperwhisper/.venv/bin/python models/crisperwhisper/asr.py IN.wav` when that
  venv exists. asr.py prints JSON `[{"word","start","end"}]` (seconds) on stdout; logs go to stderr. Neither
  `asr.py` nor `setup.sh` is in this copy: write them here (own venv, never shared).
- Blocker 1: the `ct2` extra needs `ctranslate2-crisperwhisper`, which ships only Linux x86_64 wheels. On a Mac use
  `crisperwhisper[transformers]==2.0.3` with `backend="transformers"`.
- Blocker 2: `transcribe_dual()` raises on any backend but ct2. So: two `transcribe()` calls (mode "verbatim" with
  `word_timestamps=True`, then mode "intended"); the verbatim words carry the times, the intended pass only gives digits
  and sentence ends.
- Device: `device="auto"` picks CUDA or CPU, never `mps`; pass `device="mps"` yourself (untested), with CPU as the
  fallback. Measure seconds per minute of audio before you propose it as the default.
- Weights: CrisperWhisper 2.0 large, ~1.5 GB, non-commercial license (internal use only). Hotwords do nothing there.
