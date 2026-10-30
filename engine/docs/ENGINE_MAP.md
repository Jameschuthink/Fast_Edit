# Engine map

Where each part of the engine lives, for the rare engine change made in the main chat (a note's `engine:` spec, a new
key, a new route). Read this first, then only the file and the lines named here. Line numbers are from 2026-10-12;
grep the function name if they moved. How to USE the engine: the `reel` skill (JS comps, posters: `engine/docs/COMP.md`). Folders: `engine/README.md`.

## The path of one frame
`aios CMD JOB` (main.rs) -> `Job::load` (job.rs: job.json, or edit.txt compiled by editfile.rs into the same cfg)
-> per worker one QuickJS runtime (js.rs: prelude.js + the libs in cfg `lib` + `JOB` + the comp files)
-> `draw(g, t)` records Canvas2D calls as ops (js/prelude.js) -> canvas.rs replays them on Skia (Draw) or only hashes
them (Analyze) -> render.rs (workers, adaptive motion blur, ordered writer) -> encode.rs (NVENC / x264) or PNG/JPEG.
cache.rs reuses 1-second chunks whose frame hashes did not change; audio.rs builds the mix in parallel.

## Rust (engine/src)
| File | What | Main entry points |
|---|---|---|
| main.rs | the CLI: `enum Cmd` (l.71: Render, Take, Still, Onion, Frames, Perf, Sound, Mix, Draft, Final, Export, Lint, Serve, New, Note, Seek, Eval, Bench, Config; global `--root`), `run` (l.647) | `note_list` l.375 (the note packet: KEYS, FX, LOOKS, per note boxes + sheet; a note with "each/every" + "change/photo/cut..." gets a PICTURE CHANGES line from `picture_changes`: the element's box diffed frame by frame, jumps in raw time; AIOS_DEBUG_CHANGES=1 dumps the diffs), `note_report` l.458 (the check printed after a note) |
| editfile.rs | edit.txt + notes.txt (`read` joins them, `tidy` splits them): parse, compile, scene/big lines (`scenes`), notes, page edits | `Line` l.55, `parse_line` l.147, `fmt_line` l.220, `compile` l.580 (size, fonts, clips, segs, looks, styles, text, pic, fx, caps, mattes -> cfg), `caps` l.920, fx l.1023-1119, looks l.1043-1083, `toggle` l.1142, `set_line` l.1167 (POST /lineset), `undo_line` l.1248, `add_note` l.1287, `undo_note`/`redo_note` l.1306/1357, `apply_answer` l.1582 (an agent's answer -> lines, `#nN<` undo marks, fx check), `take` l.1634 (`aios take`), `KEYS` l.1733 (the syntax the agent sees), `waiting` l.1737 |
| serve.rs | the live page server; routes listed in its header (l.1-33); /lift paints a frame drag's 3 pictures once (plate / element / top: canvas.rs `Lift`, render.rs `Worker::lift`); a reload keeps the open video decoders whose files did not change (assets.rs `Assets::renew`) | `serve` l.182 (route match from ~l.290), `watched` l.78 (reload on file change), `run_draft` l.69 |
| job.rs | job folder -> `Job` (cfg, world size, fps, sources, the `Layout`) | `engine_dir` (the engine's own files), `resolve_job`, `Job::load(arg, &layout)` |
| layout.rs | the workspace folders from `aios.toml` (dev/engine/CLI_LAYOUT.md): `--root` > `AIOS_ROOT` > the first aios.toml from the current folder up; every key required, a typo = exit 78 | `Layout::load`, `Layout::dir(Key::Jobs …)`, `Layout::root`, `aios config` (`Layout::report`) |
| js.rs | QuickJS per worker | `Js::new`, `record(t)` (ops without paint), natives l.127 |
| canvas.rs | the Canvas2D interpreter on Skia | `Exec` l.243; filters l.1597; marks (`g.mark`) give the boxes for /boxes and lint |
| render.rs | workers, motion blur, `Pool` (serve's warm pool) | `Worker` l.100, `diff` l.397, `Pool` l.456, `render_once` l.662 |
| text.rs | fonts and shaping (HarfBuzz) | `register_font` l.142, `Shaper` l.257 |
| video.rs | NVDEC decode, constant-rate frames, seek proxy; RAM: `ahead_for` (read-ahead by frame size, 4K 15), `IDLE` (a source not asked for 2 s frees its frames + ffmpeg) | `probe` l.34, `VideoSource` l.133 |
| assets.rs | images, videos, image sequences, SVG, HTML snapshots | `Assets` l.26 |
| audio.rs | voice + song (ducked) + sfx, mastered to -14 LUFS | `mix` l.74, `master` l.258 |
| cache.rs | 1-second chunk cache | `video` l.56, `prune` l.182 |
| lint.rs | `aios lint` layout checks | `report` l.241 |
| encode.rs, gpu.rs, color.rs | encoder, Vulkan/Skia GPU, CSS colours | |
| yuv.rs | BGRA -> yuv420p on the GPU for GPU workers (render.rs `Worker::gpu_yuv`; `AIOS_GPU_YUV=0` off, `=check` compares with the CPU) | `GpuYuv::convert` |

## JavaScript (engine/js)
| File | What |
|---|---|
| prelude.js | the Canvas2D API in JS: every call becomes an op; `img`, `video`, `seq`, `font`, `g.mark` |
| lib/edit.js | draws an edit.txt job: `drawText` l.38 (style + text keys: size, fit, from/dist, pop/in/grow, fade, behind, bg), `fxOn`/`fxRun` (the fx hooks video / under / over), `draw` l.85 (order: video, pics, fx under, behind words, matte, words, captions, title card, fx over). `FX.def` API at the top |
| lib/captions.js | captions v3 (`cap` lines and captions.json): timing, place, face check |
| lib/motion.js, look.js, layout.js, poster.js | `OM` moves, `LOOK` colours/fonts, `LAYOUT` grid, `POSTER` |
| live.html | the page (read from disk on each load: edit, reload; no rebuild). Sections (`// ───`): load l.145, viewer l.174, timeline l.330, edits l.559, notes l.647, edit-file job l.690, frame move/resize l.803 (the drag moves the /lift pictures: no render until the release), the song-loudness slider (`renderSong`: drag -> POST /lineset gain=) l.920, controls l.940, polled state l.973 |
| fx/NAME.js | the effect library; a job's own in jobs/JOB/fx/ |

## Python tools (engine/tools)
| File | What |
|---|---|
| promote.py | run by each `aios note JOB`: a kept note's job fx -> engine/js/fx/, its style -> engine/looks/ (a failure prints `promote.py failed`). Rules: the chat writes `_shared/TASTE.md` after a yes (patterns.py, examples.py and the page's notes agent deleted 2026-10-24) |
| caption_eval.py, context_cost.py | test sets, cost (the AIOS2 porters: `_archive/engine/tools/`) |

Libraries of saved work: `engine/looks/NAME.txt` (styles), `engine/js/fx/NAME.js` (effects).

## Recipes
- **New text or style key** (edit.txt): read it in `drawText` (lib/edit.js; text key over style key: `it.K ?? s.K`),
  add it to `KEYS` (editfile.rs). Keys pass through compile as they are (numbers parsed, `a,b` -> pair). A bare flag:
  add it to the flag list in `fmt_line`. Rebuild (KEYS is in Rust).
- **New line kind**: `KINDS` (editfile.rs l.49), a block in `compile` that adds it to cfg `edit`, the drawing in
  lib/edit.js `draw`, `KEYS`.
- **New page route**: the route match in `serve` (serve.rs) + a line in the header list; page code in live.html.
- **New CLI command**: a variant in `enum Cmd` + its arm in `run` (main.rs).
- **New Canvas2D feature**: the op in prelude.js, its replay in canvas.rs `Exec` (and its hash, so the cache sees it).
- **Notes packet**: the text in `note_list` (main.rs) and `packet` + `words_at` (editfile.rs). Brief by default: the
  FX and look names, the full KEYS/FX/LOOKS in `_ai/notes/reference.txt` (`--full` prints them). The apply report
  (`note_report`, main.rs) prints raw = page seconds per changed line. After BOXES: COMING IN (ids that enter
  within `AHEAD_S`), WENT OUT (ids gone in the last `BEHIND_S`: a late pause) and UNDER THE POINT (the note's `pt=x,y` world px, `ptby=dot|rest`). The point: both note pages
  (`editor.html` press, `live.html` `#pd` dot over the paused clip; else the mouse rest) -> `/enote` `pt` (serve.rs).
  Check: `dev/tools/pwcheck/note_point_check.py`.

## Build, test, restart
- Build: `cd engine && cargo build --release` (~6 s after a small change; `aios` on PATH runs the new binary at once; setup: `README.md`).
- Folders: `aios.toml` at the workspace root names jobs, templates, logs, songs, sfx, refs, skills; the engine
  holds no folder names. A job's own folders (`review/`, `final/`, `_ai/`) are inside the job (`job.rs`). `aios config` prints each key, its folder, its line, and "missing". A folder moves = one line
  in aios.toml, no rebuild. Outside the workspace: `--root DIR` or `AIOS_ROOT=DIR` (else exit 78).
- Test on a copy only: `cp -r jobs/JOB jobs/_test_NAME`, its own server `aios serve _test_NAME --port 8767`, then
  delete the copy (its review/ goes with it).
- A running `aios serve` keeps the old binary: after a Rust change, kill only that process (match `serve JOB` in
  /proc/PID/cmdline; a broad pkill once killed the shell) and start it again. live.html needs only a page reload.
- Speed log: `trace.rs` (counters `trace::add`, spans `trace::span`, fields `trace::note`; `finish` at exit writes `logs/perf.jsonl`, and with AIOS_TRACE=1 a Chrome trace). Read with `aios perf`.
- Check: `aios still JOB --at T` (one frame), `aios onion JOB --from A --to B` (one move), `aios sound JOB` (the mix per beat, `sound.rs`), `aios lint JOB`, `aios eval JOB "expr" --at T`.

## Live page: notes and drag (moved from AGENTS.md, 2026-10-15)
- Notes are applied only by a chat: `check notes JOB [ID]` (the page's notes agent and its "chat" button were deleted
  2026-10-24). Older jobs can still hold status=main notes; `aios note` lists them with the new ones.
- Drag on the frame (Ctrl+Z): it moves/resizes a `text` line and the placed fx (card, noplay,
  music_note, button, chips, chipstack) with one /lineset write of x=/y= (+w / size / r on a corner). Bands, full-frame
  fx (filmstrip, blackout, bottomfade, shade, scan) and captions stay out.
  An fx with parts (`f.part(g, 'title', () => ...)` in its hook, lib/edit.js) drags per part: `nudge.PART=dx,dy,k` on
  its line (editfile.rs nudge map, element id `ID.PART`); the whole layer of such an fx, or a full-frame fx, is blocked.
- ✎ draw: a drag = a rectangle; drag inside a drawn box = move, a corner = resize (editor.html `inkAt`).
- `aios seek JOB T` shows the user a moment on the page. Sound-only changes re-mix in ~3 s.
- edit.txt lines: `song m1 "library/songs/FILE.mp3" from=SEC gain=-14 [duck= fade= in=]`; `sfx w1 A 13.39 "whoosh-impact"`
  (an `sfx` skill recipe at a raw second; `sfx.py resolve` picks the file, hit point and level).
