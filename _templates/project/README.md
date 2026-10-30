A job (reel by default; `aios new JOB --template poster` for a poster). `aios new YYMMDD_slug` copies this folder to jobs/YYMMDD_slug:
- CONTEXT.md: status (open|done), stage, next step, open choices. Every chat on this job reads it first.
- BRIEF.md: line 1 `type:`, line 2 `brand:`, then the intake answers and the refs.
- script.md: the script, if there is one. script_work/: notes, angles, hooks.
- media/: takes, screenshots, downloads, b-roll, photos; media/refs/ = style references (one line each in BRIEF.md).
- review/: what the user reviews; each new file takes the next run number (001_stills.png, 002_draft.mp4 …).
- _ai/: the agent's files (notes.txt, words/, fx/, cache/, job code, HANDOFF.md); the user does not need to open it.
Next: fill jobs/JOB/BRIEF.md, then the flow in AGENTS.md (take -> serve -> notes -> draft -> final).
