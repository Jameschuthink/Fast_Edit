status: open
stage: take
next: fill BRIEF.md
open choices: none
# Contract (the next chat reads this file first)
reads: BRIEF.md, AGENTS.md (the flow)
writes: script.md, script_work/, media/, edit.txt, review/
check: the user, at each stop the stage contract names
handoff: _ai/HANDOFF.md
# Status rules
- Only the user says "archive JOB" ("done" never moves a job). Then: status: done, the archive audit, move the job to jobs/archive/.
- stage = where the job is in the AGENTS.md flow: take, edit, final.
