# AIOS3 engine

Compositions are written like a web canvas (JavaScript, Canvas2D, a pure function of time) and rendered like a native
engine: Skia on every core, video in through NVDEC, video out through NVENC, an op-level cache, no browser.
At the root, not in `_tools/` (broken on purpose, `dev/archive/icm/TARGET.md` B3): `engine_dir()` (`src/job.rs`) finds
`js/`, `looks/`, `fonts/` from the folder compiled into the binary (or `AIOS_ENGINE`); a move = a rebuild.

```
comp.js (draw(g, t))  ──QuickJS──▶  op record (numbers + strings)  ──Rust──▶  Skia paint  ──▶  YUV 4:2:0  ──▶  NVENC / x264
        │                                   │                                    ▲
   JOB (job.json, words,                analysis: one (identity, hash,       decoded frames
   beats, captions)                     device box) per draw op              (ffmpeg NVDEC, shared)
                                            │
                     frame hash ─▶ chunk cache (1 s chunks, reused as encoded) · motion blur sizing · lint
```

Build: `cargo build --release` (Skia comes prebuilt from `vendor/`, so a full build takes ~1 min and an incremental one
~10 s; JS changes need no build at all).

| file | what |
|---|---|
| `src/main.rs` | the CLI (`aios render/still/frames/draft/final/mix/lint/serve/new/eval/bench`), stills sheet, Reels check |
| `src/js.rs` | one QuickJS runtime per worker: prelude, libraries, JOB, the comp; `record(t)` returns the op record |
| `js/prelude.js` | the Canvas2D API as a recorder (`g`), Path2D, gradients, patterns, `img/video/seq/font`, browser shims for AIOS2 comps |
| `src/canvas.rs` | the interpreter: Canvas2D state machine → Skia (Draw) or device boxes + hashes (Analyze) |
| `src/text.rs` | CSS font strings, font registry (variable fonts pinned per weight), SkParagraph shaping → TextBlobs, cached |
| `src/render.rs` | workers (JS + surface), adaptive linear-light motion blur on dirty regions, the persistent worker pool |
| `src/cache.rs` | frame hashes → 1-second chunk files (segment encoder, IDR at each chunk) → concat; the sound cache |
| `src/video.rs` | ffmpeg decoders: sequential (window shared by workers, YUV→RGB in the workers) and random (seek proxy, frame windows) |
| `src/assets.rs` | stills (png/jpg/webp/svg, HTML via headless Chromium), videos, image sequences; content hashes |
| `src/encode.rs` | the encoder process (NVENC/x264, 1 MB pipe), BGRA → YUV 4:2:0 BT.709 |
| `src/audio.rs` | voice + ducked song + effects → two-pass loudnorm → AAC; mux |
| `src/serve.rs` | the live page (warm pool, file watcher, timeline) |
| `js/lib/motion.js` | onetake's moves (`OM`): springs, eases, entrances, carries, camera, sims |
| `js/lib/look.js` | the six onetake looks (`LOOK`): colours + fonts from `fonts/looks` |
| `js/lib/edit.js` | draws a job made from one `edit.txt` (`src/editfile.rs` compiles it): each kept part from its clip's prep, then each text line with its style |
| `js/lib/captions.js` | approved v3 captions (`CAPTIONS`) |
| `js/lib/layout.js`, `poster.js`, `kit/` | the layout grid, posters, kit comps (`docs/COMP.md`) |
| `fonts/` | `captions/`, `looks/` |
| `tools/` | Python next to the engine: `mograph.py` (motion parts), `motion_refs/` (motion sources, reference only), small checks (`caption_eval.py`, `trace_sum.py` …) |
| `docs/` | `ENGINE_MAP.md` (where each part lives, for an engine change), `FACTS.md` (speeds and limits measured here), `COMP.md` (JS comps, the layout grid, posters) |

Licences: Skia (BSD), QuickJS (MIT), the onetake move library and looks data are PolyForm Noncommercial (the user's
choice, as in AIOS2); fonts in `fonts/looks` are SIL OFL; caption fonts are the user's.
