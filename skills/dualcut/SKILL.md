---
name: dualcut
description: Create and edit videos with dualcut — a dual-mode (GUI + programmatic) video editor. Use when asked to make a video, edit a dualcut project, drive the dualcut app, or automate video composition. Covers project scaffolding, the document schema, the live HTTP agent API, TypeScript scripting, and rendering to MP4/WebM.
---

# dualcut: video editing for agents

dualcut edits video through one JSON **project document**. Three surfaces
stay in sync: the GNOME app (timeline + inspector), the project file on
disk, and a local HTTP API. You can use any of them; the app hot-reloads
external edits live.

This skill lives in the dualcut repo, so it always matches the checked-out
version. Deep reference: [AGENTS.md](../../AGENTS.md) ·
types: [engine/schema/dualcut.d.ts](../../engine/schema/dualcut.d.ts) ·
JSON Schema: [engine/schema/dualcut.schema.json](../../engine/schema/dualcut.schema.json).

## Setup

App (Flatpak, released builds):
```sh
curl -LO https://github.com/hanthor/dualcut/releases/latest/download/dualcut.flatpak
flatpak install --user --reinstall -y dualcut.flatpak
flatpak run io.github.hanthor.Dualcut ~/Videos/myproject.json   # opens + serves API
```

From source (`engine/` dir): `cargo build --features preview,vector,scripting`.
Binaries: `preview` (app), `render` (CLI), `serve` (headless API).

## Start a project

```sh
cargo run --bin render -- new myproject.json "My video"   # scaffold w/ templates
```
Or write the JSON directly (validate against the schema). Minimal shape:

```jsonc
{
  "meta": { "title": "T", "width": 1920, "height": 1080, "fps": 30 },
  "defs": { },            // reusable parameterised comps ({param} substitution)
  "scenes": [             // sequential; scene layers start relative to scene
    { "id": "s1", "duration": 3, "transition": {"kind":"crossfade","duration":0.5},
      "layers": [ { "id": "c1", "type": "text", "text": "Hi", "start": 0, "duration": 0 } ] }
  ],
  "overlays": [ ]         // absolute timing, crosses scene cuts (subtitles, music)
}
```

Clip types: `text` · `video`/`audio` (src, offset, volume) · `image` ·
`shape` (rect|circle|ellipse|star|polygon|line|arrow, GPU-rendered) ·
`compref` (instantiate a def with args) · `test`. A `video` clip's src may
be a live vector source: `vello://star?fill=%23ff5470&w=200&h=200&spin=1`.
Animations: tween `{property, from, to, start, end, easing}` or keyframes
`{property, keyframes: [{t, value, easing}, ...]}` (>=2, increasing t);
property: x|y|width|height|opacity|volume (volume for audio fades).
Effects: `effects: [{type: blur|color|chromakey|crop|eq|compressor, ...}]`
(chromakey: color/angle/noise green-screen; crop: left/right/top/bottom px;
eq: low/mid/high dB; compressor: threshold/ratio).
Text extras: align (left|center|right), outline (color), shadow (bool).
Overlay tracks: muted/hidden booleans.
HTTP POST /op: split (id, at) | ripple_delete (id) | detach_audio (id) |
move_to_lane (id, lane, at) — returns new ids where relevant.
Transitions: crossfade | wipe-lr | wipe-tb | box-wipe | iris | clock. Defs may nest (no cycles).
Detach audio = set video `volume: 0` + add an `audio` clip with same src.

## Drive a running app (HTTP, port 7357)

While the app (or `serve`) has a project open:

```sh
curl localhost:7357/project                      # read document
curl -X POST --data-binary @doc.json localhost:7357/project   # replace (validated)
curl -X POST --data-binary @edit.ts localhost:7357/script     # run TypeScript
curl localhost:7357/status
```

Script contract (`edit.ts`):
```ts
export function edit(project: Project): Project {
  project.scenes.push({ id: "outro", duration: 2, layers: [/* … */] });
  return project;
}
```

Editing the project file directly also works — the app watches mtime and
hot-reloads, preserving playback position. Port: `DUALCUT_API_PORT` env
(0 disables).

## Render

```sh
cargo run --bin render -- myproject.json out.mp4    # or out.webm
curl -X POST -d '{"out":"out.webm","profile":"webm"}' localhost:7357/render  # serve bin only
```

## Rules

- Unique `id` everywhere; `animations: []` when none; times in seconds.
- Read-modify-write: never blind-write over a document the app may have
  changed — GET (or re-read the file) first.
- `duration: 0` on a scene layer = fill the rest of the scene.
- Media paths resolve relative to the project file's directory.
