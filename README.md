# Fast_Edit

An AI video-editing agent's workspace with its own render engine: a reel is one text file (`jobs/JOB/edit.txt`),
the Rust engine renders it. The agent's guide: [AGENTS.md](AGENTS.md); the engine: [engine/README.md](engine/README.md).

```
aios new JOB                                           # a job from _templates/project
aios take JOB jobs/JOB/media/take.mp4                  # word times -> edit.txt
aios serve JOB                                         # live page, hot reload, notes
aios draft JOB                                         # 540 px draft with sound
aios final JOB                                         # full size + Reels check
```

## Setup on a Mac (Apple Silicon), once
```
brew install git ffmpeg rust python
git clone git@github.com:Jameschuthink/Fast_Edit.git && cd Fast_Edit   # the Mac's SSH key must be on GitHub
(cd engine && cargo build --release)                   # downloads a prebuilt Skia; no Skia compile
mkdir -p ~/.local/bin && ln -sf "$PWD/engine/target/release/aios" ~/.local/bin/aios
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc && source ~/.zshrc
python3 -m venv .claude/skills/nvidia/.venv && .claude/skills/nvidia/.venv/bin/pip install nvidia-riva-client
echo 'NVIDIA_API_KEY=nvapi-...' > .env                 # transcripts: the hosted Parakeet (a build.nvidia.com key)
aios config                                            # each folder from aios.toml
```
`aios` finds the workspace from `aios.toml` (the current folder or one above it); elsewhere use `--root DIR`.
The Mac paints on the CPU and encodes with libx264. What is slower or missing, and the options to fix it:
`engine/docs/MAC_SPEED.md`. Songs and sound effects: add your own to `library/songs/` and `library/sfx/`.
