# dualcut

A video editor with **dual usage**: edit manually (multi-track timeline +
parameter sidebar) or programmatically (live JSON document, editable in-app
or by external agents) — both at the same time, always in sync.

> Concept and spec by **[KiKaraage](https://github.com/KiKaraage)** (main author).
> Implementation scaffolded with Claude Code.

Inspired by CapCut/iMovie/Clipchamp (editing UX), Remotion & Hyperframes
(declarative programmatic video), and tldraw offline / MagicPath (dual
manual + programmatic editing with an agent-facing surface).

## How it works

One declarative document — `composition.json` — is the single source of
truth. Three surfaces edit it:

| Surface | Where | Sync |
|---|---|---|
| Manual UI | multi-track timeline, drag/trim clips, inspector sidebar | writes to the store, autosaved to disk |
| In-app code | **Code** tab (CodeMirror JSON editor) | valid JSON applies live, debounced |
| External agents | edit `composition.json`, or `GET`/`POST /__composition` | pushed into the running app over the Vite websocket |

See [AGENTS.md](AGENTS.md) for the document schema and agent workflow.

## Run it

```sh
npm install
npm run dev
```

Open http://localhost:5173. Try:

- Drag clips around the timeline, trim their edges, drag between tracks.
- Select a clip → edit its parameters and animations in the sidebar.
- Switch to the **Code** tab and edit the JSON — the preview updates live.
- From another terminal:
  `curl -s localhost:5173/__composition | jq '.meta.title = "Hi"' | curl -sX POST -d @- localhost:5173/__composition`
  — the open editor updates instantly.
- Or just edit `composition.json` in your $EDITOR.

### Shortcuts

Space play/pause · ←/→ step frame (Shift = 1 s) · Home go to start ·
Ctrl+Z / Ctrl+Shift+Z undo/redo · Delete remove selected clip

## Features

- Multi-track compositing: text, shapes, images, video, audio
- Per-clip animations (`from`/`to` tweens with easing incl. spring)
- Scrubbable ruler, zoomable timeline, track mute/hide
- Undo/redo across all three edit surfaces
- Live bidirectional sync between UI ⇄ code ⇄ disk

## Not yet

- Rendered export (WebCodecs/ffmpeg) — preview is DOM-based for now
- Waveforms/thumbnails on clips, snapping, multi-select
