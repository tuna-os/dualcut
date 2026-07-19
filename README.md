# dualcut

A GNOME video editor with **dual usage**: edit manually (scene timeline +
inspector) or programmatically (live JSON document, TypeScript scripts,
HTTP API for agents) — every surface stays in sync while the app runs.

> Concept and spec by **[KiKaraage](https://github.com/KiKaraage)** (main author).
> Implementation built with Claude Code.

![dualcut editor](docs/screenshot.png)

**[📘 User Guide](docs/USER_GUIDE.md)** — screenshots regenerated automatically on every release.

**Install / update (Flatpak):**

```sh
curl -LO https://github.com/hanthor/dualcut/releases/latest/download/dualcut.flatpak && flatpak install --user --reinstall -y dualcut.flatpak
flatpak run io.github.hanthor.Dualcut ~/Videos/myproject.json
```

## How it works

One JSON **project document** is the single source of truth —
**scenes** (sequential narrative spine) + **overlays** (tracks that cross
scene cuts) + **defs** (reusable parameterised templates). Rendering is
GStreamer Editing Services; shapes draw on the GPU via Vello; the UI is
GTK4/libadwaita.

| Surface | What | Sync |
|---|---|---|
| App | scene strip w/ thumbnails, inspector, script tab, preview | writes the document, hot-reloads external edits |
| File | edit the project JSON in any editor/agent | app reloads live (mtime watch) |
| HTTP | `GET/POST /project`, `POST /script` (TypeScript), `/status` on `127.0.0.1:7357` | file-backed, works against the running app |

Agents: point your tool at **[skills/dualcut/SKILL.md](skills/dualcut/SKILL.md)**
(ships in-repo, versioned with every commit). Deep reference:
[AGENTS.md](AGENTS.md) · types [engine/schema/dualcut.d.ts](engine/schema/dualcut.d.ts)
· [JSON Schema](engine/schema/dualcut.schema.json).

## From source

```sh
cd engine
cargo run --features preview,vector,scripting --bin preview -- examples/demo-project.json
cargo run --bin render -- new myproject.json "My video"   # scaffold
cargo run --bin render -- myproject.json out.mp4          # or out.webm
cargo run --bin serve  -- myproject.json                  # headless agent API
```

Needs GStreamer (+GES), GTK4, libadwaita dev packages; see
[engine/build-aux/io.github.hanthor.Dualcut.json](engine/build-aux/io.github.hanthor.Dualcut.json)
for the canonical dependency list.

## Features

- Scene-based editing with crossfade transitions and overlay tracks
- Text, video, audio, image clips + GPU vector shapes (rect, circle,
  ellipse, star, polygon, line, arrow)
- Tweened animations (x/y/opacity, easing) compiled to GStreamer control sources
- Reusable parameterised templates (lower third, title card, caption built in)
- Detach audio, undo/redo, multi-select, first-frame thumbnails
- TypeScript scripting in-app and over HTTP (`export function edit(p) {…}`)
- MP4 (H.264/AAC) and WebM (VP8/Vorbis) export
- Releases: every `v*` tag auto-builds the Flatpak (`scripts/release.sh`)

Roadmap and status: [ROADMAP.md](ROADMAP.md) ·
open work: [issues](https://github.com/hanthor/dualcut/issues)

## v0 web prototype (reference)

The original browser prototype (Vite + React, `src/`) established the
dual-editing model and stays as a reference: `npm install && npm run dev`,
then edit `composition.json` or `GET/POST /__composition`. Superseded by
the native app; kept until the native timeline reaches full parity (#2).
