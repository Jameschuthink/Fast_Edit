# Fast_Edit: an AI video-editing agent with its own render engine

The user leads the creative direction and decides. The agent edits fast, finds the facts, and brings options with
its own pick. A video job is one text file, `jobs/JOB/edit.txt`; the Rust engine (`engine/`) renders it.
This copy runs on a Mac (Apple Silicon): CPU painting, libx264 encoding. Setup: `README.md`.

## Flow (one job)
1. `aios new YYMMDD_slug`: a job from `_templates/project/`; fill `jobs/JOB/BRIEF.md` with the user. A job copied
   in from another computer: its `song`/`sfx` lines need their files in `library/`, else turn those lines off (`# `).
2. Put the raw take in `jobs/JOB/media/`, then `aios take JOB jobs/JOB/media/FILE.mp4` (from the repo root): word times (hosted Parakeet), one
   `say` line per sentence, `skip` lines for pauses and fillers.
3. `aios serve JOB` in the background: the live page. The user scrubs it and writes notes on it.
4. `aios note JOB`: the new notes with their packets and the HOW TO APPLY rules; apply each with
   `aios note JOB ID --apply -`. Undo: `aios note JOB ID --undo`.
5. `aios still JOB --at 1,4,9` (a stills sheet) and `aios draft JOB [--from A --to B]` (540 px with sound) to show.
6. `aios final JOB`: full size + the Reels check, only when the user wants to publish.

## Where things are
| Need | File |
|---|---|
| Every edit.txt key (moment, say, skip, cap, big, fx, broll, song, sfx...) | `aios note JOB` prints KEYS; `engine/src/editfile.rs` |
| Each engine command, route, file | `engine/docs/ENGINE_MAP.md` (some Linux dev files it names are not in this copy) |
| Named effects and looks | `engine/js/fx/`, `engine/looks/` |
| Transcripts (asr) | `.claude/skills/nvidia/` (`nv.py asr`; the key `NVIDIA_API_KEY` in `.env`) |
| File steps outside the engine (exposure, colour) | `.claude/skills/ffmpeg-skill/scripts/` |
| Songs and sound effects | `library/songs/` (`songs.json`), `library/sfx/` (`index.json`): empty until the user adds some |
| Making the Mac faster (options to propose) | `engine/docs/MAC_SPEED.md` |

## Work rules
- Short, simple sentences. Options = a numbered list of 2-4 with your pick marked. Points to confirm get IDs (C1, C2).
- Show the cheapest thing that can be judged first: a still sheet, the live page, a 2 s draft; a full render only on yes.
- Taste is the user's call. Fix objective faults without asking; ask with options when a note is unclear.
- Test engine changes on a copy of a job (`cp -r jobs/JOB jobs/_test_X`), never on the user's job.
- Never commit `jobs/`, `.env` or media. A render is slow or a feature is missing: read `engine/docs/MAC_SPEED.md`
  and propose the matching option to the user; do not build it without a yes.
