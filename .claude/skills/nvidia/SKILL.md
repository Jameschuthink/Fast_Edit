---
name: nvidia
description: Clean the voice (NVIDIA Studio Voice), relight the person warmly (Maxine Relighting), fix eye contact (Maxine Eye Contact) and get word times (local CrisperWhisper with fillers and pauses, faster-whisper, or NVIDIA Parakeet) for a talking-head clip. Use when a new talking-head take arrives, when the user asks for a cleaner voice, warmer light, eyes on the lens, or a transcript with word times.
---

# nvidia

Execution skill. It runs a service and checks the output; it decides nothing.
Run: `python3 .claude/skills/nvidia/scripts/nv.py <cmd> IN -o OUT [--dry-run] [--json] [--overwrite]`

| User says | Do | Time (a test job) |
|---|---|---|
| "clean the voice", any new take | `nv.py voice take.mp4 -o voice.wav` (or `-o take_voice.mp4` to mux) | 7 s / 54 s clip |
| "relight", "warmer light" | `nv.py relight take.mp4 -o take_relight.mp4 [--hdr map.hdr] [--keep-texture]` | 10 s / 5 s clip |
| "look at the lens", "eye contact" | `nv.py eyes take.mp4 -o take_eyes.mp4 [--strong]` | 14 s / 5 s clip |
| words with times (cuts, captions, graphics) | `nv.py asr voice.wav -o take.words.json [--script FILE]` | ~10 s / min + 7 s load |
| words, script spelling as hotwords (old engine) | `nv.py asr voice.wav -o take.words.json --whisper [--script FILE]` | 9 s / 54 s |
| fast rough words only | `nv.py asr voice.wav -o take.words.json --hosted` | 2 s / 54 s |
| person matte (words or graphics behind the person) | `nv.py matte clip.mp4 -o jobs/JOB/matte --from A --to B [--fps 30]` -> `00001.webp ..` (RGB = the clip, A = the person) + `matte.json` | ~30 s / 5 s of 1080p (service 19-60 s, key 7 s) |
| does it work here? | `nv.py doctor` | — |

Rules:
- Studio Voice returns the voice about 143 ms late (measured 2026-10-06). `voice` measures the lag against the raw audio and removes it, then fails if more than 10 ms is left. The old AIOS a test job finals have this 143 ms lag.
- Run `voice` first. Run `asr` on the clean voice WAV, not on the raw clip.
- `words.json` is `{"words":[{"word","start","end"}]}`. The engine reads it (`aios take`).
- `asr` defaults to local CrisperWhisper 2.0 large (`models/crisperwhisper/CONTEXT.md`: setup, license, limits): verbatim
  words with [UM] / [UH] / [laughter] tags and exact word times, so pauses and breaths sit between words (user,
  2026-10-29). It has no hotwords: names and rare words can be misspelled; `script vs heard` lists them.
  `--whisper` = the old faster-whisper large-v3-turbo (drops fillers, stretches words over pauses). NVIDIA Parakeet (`--hosted`) is 4x faster, but it dropped a fast list of 12 words on a test job ("execution, cuts, captions, sound"). Use it only for a rough pass on speech. On a SONG it is the better one (a test job 2026-10-11: 13/14 sung words, Whisper 1/14; starts in 80 ms steps, ends often = start): use `songs` `lyrics.py`, which runs it.
- Relight and eyes re-encode the clip upright first (the services ignore rotation metadata). The frame count must stay equal, or the run fails.
- The default light map is `assets/studio.hdr` (user pick "A" 2026-10-06: key light at 45 deg from above, top light, rim from behind, dark warm room), with background gain 0.25, specular 0.6, blur 0. It was built from the reference `library/refs/relight-hHtQf4zpp9E/NOTES.md` with `scripts/studio_hdr.py`. The older warm room map is `assets/warm.hdr` (2026-10-05). `scripts/make_hdr.py` makes a room-photo map.
- `matte` holds all frames in RAM: 6,564 frames at 1080p > 14 GB (it stops). Make it in 30 s pieces (`--from/--to`, 85-123 s
  each) and join them; a 901-frame piece repeats the boundary frame: drop it (script: a small shell loop).
- `matte` = relight on a flat green background (`--background-source 1 --background-color 0x00FF00 --lossless`), keyed: alpha = 1 - clip((G - max(R,B)) x 1.25). Only the alpha is used, the picture stays the clip. Tested 2026-10-11 on a test job (150 frames): IoU vs RVM 0.978 mean, 0.991 median; the 9 frames below 0.95 are fast hands and a swinging arm that NVIDIA keeps and RVM partly drops. The user prefers this route (2026-10-11); offline fallback: `models/people/matte.py` (RVM).
- The face smoothing that relight adds is wanted. The background must stay sharp, with no softness. Measured: door edges 3.32 -> 3.20 (unchanged); the flat wall loses only noise. `--keep-texture` restores skin detail if a reel needs it.
- The clips go to NVIDIA's cloud (free trial API). Function IDs are in `nv.py` (`FUNC`). If a call fails with "function not found", get the current ID from the model's page on build.nvidia.com.

Verification: each run verifies its own output (duration or frame count) and exits 1 on failure. Then use `ffprobe`.
