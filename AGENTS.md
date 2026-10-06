# Editing dualcut projects (for agents)

The entire video is one declarative JSON document (`*.json` project
file). Everything below describes that document and the surfaces for
editing it. Domain glossary: CONTEXT.md. Types: engine/schema/.

The Rust engine in `engine/` uses document model v2 — **scenes** (sequential
narrative spine) + **overlays** (tracks crossing scene cuts) + **defs**
(reusable parameterised compositions). Full types: `engine/src/document.rs`.

## HTTP API Reference

All endpoints run on `127.0.0.1:7357` by default (set `DUALCUT_API_PORT` to override, or `0` to disable). Port binding occurs when the app or `cargo run --bin serve` launches with a project.

### GET /project

Retrieve the current project document.

**Response (200 OK):**
```json
{
  "meta": { "title": "...", "width": 1920, "height": 1080, "fps": 30 },
  "defs": { /* ... */ },
  "scenes": [ /* ... */ ],
  "overlays": [ /* ... */ ]
}
```

**Errors:**
- `500 Internal Server Error` — project file unreadable or invalid JSON

### POST /project

Replace the entire project document. The new document is validated against the schema before being written.

**Request body:** Complete project document (JSON)

**Response (200 OK):**
```json
{ "ok": true }
```

**Errors:**
- `400 Bad Request` — invalid JSON or schema validation failed; error describes the issue
- `500 Internal Server Error` — file write failed

### POST /op

Execute a structural editing operation with nontrivial math, so you don't reimplement offset/animation splitting. All operations return the updated project, validated and saved to disk.

#### split

Split a clip at an absolute time, advancing media offsets and dividing animations.

**Request:**
```json
{ "op": "split", "id": "clip-id", "at": 5.0 }
```

- `id` (string) — clip ID to split
- `at` (number) — absolute time in seconds

**Response (200 OK):**
```json
{ "ok": true, "new_id": "clip-id-1" }
```

The second (new) clip receives the generated ID.

**Errors:**
- `400 Bad Request` — clip not found, invalid `at` value, or clip cannot be split

#### ripple_delete

Delete a clip and close the gap (shift later clips backward).

**Request:**
```json
{ "op": "ripple_delete", "id": "clip-id" }
```

**Response (200 OK):**
```json
{ "ok": true }
```

**Errors:**
- `400 Bad Request` — clip not found or operation failed

#### detach_audio

For a video clip, set its `volume: 0` and create a matching audio clip elsewhere. The audio clip receives a generated ID.

**Request:**
```json
{ "op": "detach_audio", "id": "video-clip-id" }
```

**Response (200 OK):**
```json
{ "ok": true, "new_id": "audio-clip-id" }
```

**Errors:**
- `400 Bad Request` — clip not found or is not a video clip

#### move_to_lane

Move a clip to a different overlay track at a new time.

**Request:**
```json
{ "op": "move_to_lane", "id": "clip-id", "lane": 1, "at": 3.5 }
```

- `id` (string) — clip ID
- `lane` (integer) — overlay track index (0-based); creates the track if needed
- `at` (number) — absolute time in seconds

**Response (200 OK):**
```json
{ "ok": true }
```

**Errors:**
- `400 Bad Request` — clip not found or invalid parameters

#### remove_silence

**Requires:** `preview` feature (e.g., `cargo build --features preview`). Disabled when serving headless on a build without this feature; requests will return 400 with error message.

Detect and remove silent stretches from a video or audio clip's media.

**Request:**
```json
{
  "op": "remove_silence",
  "id": "clip-id",
  "threshold_db": -40.0,
  "min_duration": 0.5
}
```

- `id` (string) — clip ID (must have `src` for video/audio)
- `threshold_db` (number, optional) — silence threshold in dBFS; default `-40.0`
- `min_duration` (number, optional) — minimum silent duration to remove in seconds; default `0.5`

**Response (200 OK):**
```json
{ "ok": true, "removed": 3.5 }
```

The `removed` value is the total duration (seconds) of silence that was excised.

**Errors:**
- `400 Bad Request` — clip not found, has no media, or feature not compiled in
- `500 Internal Server Error` — media file unreadable or GStreamer pipeline failed

### POST /script

**Requires:** `scripting` feature. Run a TypeScript transformation function against the project.

**Request body:** TypeScript source code

```typescript
export function edit(project: Project): Project {
  // Modify project in-place or return a new one
  project.scenes[0].duration = 5.0;
  return project;
}
```

**Response (200 OK):**
```json
{ "ok": true }
```

The returned project is validated and saved to disk.

**Errors:**
- `400 Bad Request` — syntax error, type error, or runtime error in the script; error message describes the issue
- `500 Internal Server Error` — file write failed

### POST /render

**Requires:** `preview` feature. Render the project to a video file.

**Request:**
```json
{
  "out": "/path/to/output.mp4",
  "profile": "mp4"
}
```

- `out` (string) — output file path; `.mp4` or `.webm` extension determines codec if `profile` not given
- `profile` (string, optional) — output profile: `mp4` (H.264/AAC) or `webm` (VP8/Vorbis); auto-detected from extension if omitted

**Response (200 OK):**
```json
{ "ok": true, "duration": 120.5, "warnings": [ "Clip 'c3' has no source" ] }
```

**Errors:**
- `400 Bad Request` — invalid output path or unsupported profile
- `500 Internal Server Error` — encoder pipeline failed or write permission denied

### GET /status

Health check and operational info.

**Response (200 OK):**
```json
{
  "engine": "dualcut",
  "version": "0.20.0",
  "project": "My Project",
  "duration": 42.5,
  "scenes": 5
}
```

**Errors:**
- `500 Internal Server Error` — project file unreadable

### HTTP Status Codes

- **200 OK** — operation succeeded
- **400 Bad Request** — invalid request format, missing required parameters, validation failed, or feature not compiled in
- **404 Not Found** — endpoint does not exist
- **500 Internal Server Error** — server error (file I/O, encoder failure, unhandled exception)

### Error Response Format

All error responses are JSON:

```json
{ "error": "descriptive message" }
```

For validation errors on POST /project or POST /script, the message describes which field or rule was violated.

### Common Patterns

**Read-modify-write cycle:** Always GET first before POSTing an update:

```sh
curl localhost:7357/project > project.json
# Edit project.json locally
curl -X POST --data-binary @project.json localhost:7357/project
```

**Scripted bulk edits:** Use POST /script for operations that read and write the document in one shot (rename scenes, retiming clips, generating layers from data).

**Headless rendering:** Use POST /render or the CLI `cargo run --bin render` to export video from a script or CI pipeline without opening the UI.

## HTTP ops detail

Ops with nontrivial math, so you don't reimplement them:
`{"op": "split", "id": "clip", "at": 5.0}` → splits at absolute time
(media offsets advance, animations divide) and returns `new_id`;
`{"op": "ripple_delete", "id": "clip"}` closes the gap;
`{"op": "detach_audio", "id": "clip"}`;
`{"op": "move_to_lane", "id": "clip", "lane": 2, "at": 3.5}`.
Recipes: docs/recipes/ (auto-captions).

## Editing surfaces

1. **File**: edit the project JSON (e.g. `engine/examples/demo-project.json`).
2. **HTTP** (while `cargo run --bin serve -- <project.json> [port]` runs,
   default port 7357; see **HTTP API Reference** below for full endpoint docs):
   - `GET  /project` — current document
   - `POST /project` — replace document (validated, saved to disk)
   - `POST /op` — structural edits: split, ripple_delete, detach_audio, move_to_lane, remove_silence
   - `POST /script` — run TypeScript transformation
   - `POST /render` — `{"out": "path.mp4"}` renders and reports warnings
   - `GET  /status` — engine info
3. **CLI render**: `cargo run --bin render -- <project.json> <out.mp4>`

## Schema summary

```jsonc
{
  "meta": { "title": "…", "width": 1280, "height": 720, "fps": 30 },
  "defs": {
    "lower-third": {
      "params": ["name", "role"],
      "layers": [ /* clips; "{name}" etc. substituted at instantiation */ ]
    }
  },
  "scenes": [               // sequential, no gaps; order = time
    { "id": "s1", "duration": 3, "layers": [ /* clips, start relative to scene */ ] }
  ],
  "overlays": [             // absolute timing, cross scene cuts freely
    { "id": "o1", "clips": [ /* subtitles, music, watermarks */ ] }
  ]
}
```

Clip: `{ id, start, duration, type, …element fields, transform?, animations?, effects? }`
- text extras: `align` (left|center|right — overrides x positioning),
  `outline` (color), `shadow` (bool)
- overlay tracks: `muted` / `hidden` booleans (non-destructive)
- `library`: media paths the user imported (relative to the project)
- effects: `{type: "blur", amount}` (sigma 0-50); `{type: "color",
  brightness?, contrast?, saturation?, hue?}`; `{type: "chromakey",
  color?, angle?, noise?}` (green screen); `{type: "crop", left?,
  right?, top?, bottom?}`; `{type: "mask", shape, feather?, invert?}`
  (freeform shape mask, `video`/`test` clips only — see below); audio:
  `{type: "eq", low?, mid?, high?}` (dB),
  `{type: "compressor", threshold?, ratio?}`, and
  `{type: "denoise", level?}` (0-3)
- scene transition kinds: crossfade | wipe-lr | wipe-tb | box-wipe | iris | clock
- defs may nest (compref inside a def); cycles are rejected at validation
- `type`: `text` (text/font/color) · `video`/`audio` (src/offset/volume) ·
  `image` (src) · `shape` (M3, skipped with warning for now) ·
  `compref` (ref + args) · `test`
- `transform`: `{ x, y, width, height, opacity }` pixels; 0 = natural
- animations, two forms: tween `{ property, from, to, start, end, easing }`
  or keyframes `{ property, keyframes: [{t, value, easing}, …] }` (>= 2,
  strictly increasing t; property is x|y|width|height|opacity|volume|rate;
  volume 1.0 = unity, rate 1.0 = normal speed — animate volume for
  audio fades/ducking, animate rate for speed ramps),
  times relative to the clip; easing: linear|easeIn|easeOut|easeInOut
- `duration: 0` on a scene layer = fill the rest of the scene

Rules: unique ids everywhere; scenes need `duration > 0`; `compref` targets
must exist in `defs`; nested defs must not form a reference cycle. The
*Save as template* action cannot create a def from a selection that already
contains a `compref`, even though hand-authored nested defs are valid.
Audio policy: a video clip's own audio is scene-local; music/VO belongs on
overlays.
"Detach audio" = set the video clip's `volume: 0` and add an `audio` clip
with the same `src`/`offset` wherever you want it.

## Shapes (M3)

`shape` clips render on the GPU (Vello) when the engine is built with the
`vector` feature: rect, circle, ellipse, star, polygon, line, arrow, with
`fill` (#rrggbb/#aarrggbb) and size from `transform.width/height`.
Rasters cache under `<project dir>/.dualcut-cache/`.

## Freeform shape masks (#41)

`{type: "mask", shape, feather?, invert?}` on a `video`/`test` clip
compile-time bakes a real alpha-channel copy of that clip (shape
rasterized via Vello, combined with the source through `alphacombine`,
encoded FFV1/A420 in Matroska, cached under `.dualcut-cache/`) and swaps
it in for the original source. GES's own layer compositing then reveals
whatever's on a lower layer through the transparent region -- true
track-matte, not a solid-color cutout. GES itself can't build the
alphacombine bin as a single-clip effect (multi-source bins are
rejected), so this bakes ahead of time instead, the same "slow but
cached" tradeoff already used for preview proxies. Needs the `vector`
feature; other clip types warn and skip.

## Karaoke-style captions (#53)

`dualcut_engine::karaoke::karaoke_captions_to_clips` turns flat word-level
`(start, end, word)` segments (whisper.cpp `--max-len 1`) into caption
clips where the *whole line* stays visible and the currently-spoken word
is highlighted, instead of one word popping on at a time. GES's
`TitleClip` can't do this directly: `GstBaseTextOverlay`'s `auto-resize`
isn't exposed as a settable child property (an isolated word clip and the
full-sentence clip rescale inconsistently), and `TitleClip`'s `text`
property is always markup-escaped (Pango markup only works through a raw
`textoverlay`'s `pango-markup` sink caps, which GES doesn't expose
either). Instead, each word-highlight state is rasterized once with
Pango/Cairo (full line, one color-attribute span on the active word,
same font/layout box every state so nothing rescales) and landed as an
ordinary `Image` clip -- no GES text primitives involved. Needs the
`karaoke` feature (pulled in automatically by `preview`).

## Scripting (M4)

`POST /script` with a TypeScript body: `export function edit(p: Project): Project`.
Runs in-process, result validated and saved. Types: `engine/schema/dualcut.d.ts`;
JSON Schema: `engine/schema/dualcut.schema.json`.

## Live vector sources (vello://)

With the `vector` feature, the engine registers a `vellosrc` GStreamer
element with a `vello://` URI handler. Any `video` clip can use one as its
`src` for live per-frame GPU vector rendering:

```jsonc
{ "id": "spinner", "type": "video", "start": 0, "duration": 3,
  "src": "vello://star?fill=%23ff5470&w=200&h=200&spin=1" }
```

Shapes: rect|circle|ellipse|star|polygon|line|arrow. Query params: `fill`
(url-encoded hex), `w`/`h` (px), `spin=1` (rotation demo of per-frame
rendering). Static `shape` clips keep using cached PNG rasters (zero
per-frame cost); use vello:// when you need live animation.
