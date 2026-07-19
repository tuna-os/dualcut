# Editing videos in this project (for agents)

This is a dual-mode video editor. The entire video is one declarative JSON
document, and you can edit it three ways — all stay in sync live:

1. **Edit `composition.json`** at the project root. While `npm run dev` is
   running, a file watcher pushes your change into the open editor instantly.
2. **HTTP endpoint** (while the dev server runs, default `http://localhost:5173`):
   - `GET /__composition` — read the current document
   - `POST /__composition` — replace the document (JSON body). The running
     editor updates immediately.
3. The human uses the timeline/inspector UI or the in-app Code panel; their
   edits are saved back to `composition.json` (debounced ~400 ms), so always
   re-read the file before editing if the app may be open.

## Document schema

Full TypeScript types: `src/document/types.ts`. Summary:

```jsonc
{
  "meta": {
    "title": "My video",
    "width": 1280, "height": 720,     // composition pixels
    "fps": 30,
    "duration": 10,                    // seconds
    "background": "#0b0d12"
  },
  "tracks": [                          // tracks[0] renders on top
    {
      "id": "track-1", "name": "Text",
      "muted": false, "hidden": false, // optional
      "clips": [
        {
          "id": "clip-1",              // must be unique across the document
          "name": "Title",
          "start": 0.5,                // seconds on the timeline
          "duration": 4,
          "x": 140, "y": 260,          // top-left corner, composition px
          "width": 1000, "height": 120,
          "rotate": 0, "scale": 1, "opacity": 1,   // optional, defaults shown
          "element": { ... },          // see element types below
          "animations": [ ... ]        // see animations below
        }
      ]
    }
  ]
}
```

### Element types

```jsonc
{ "type": "text",  "text": "Hello", "fontSize": 52, "color": "#fff",
  "fontFamily": "Inter", "fontWeight": 700, "align": "center" }

{ "type": "shape", "shape": "rect" | "ellipse", "fill": "#5468ff", "radius": 12 }

{ "type": "image", "src": "https://…", "fit": "cover" | "contain" }

{ "type": "video", "src": "https://…", "fit": "cover", "volume": 1,
  "offset": 0 }   // offset = seek into the source file, seconds

{ "type": "audio", "src": "https://…", "volume": 1, "offset": 0 }
```

### Animations

Each animation tweens one property over a time window **relative to the
clip's own start** (Remotion-style hardcoded from/to ranges — no expressions):

```jsonc
{ "property": "x" | "y" | "scale" | "rotate" | "opacity",
  "from": 0, "to": 1,
  "start": 0, "end": 0.5,             // seconds, relative to clip start
  "easing": "linear" | "easeIn" | "easeOut" | "easeInOut" | "spring" }
```

Before its window an animation holds `from`; after it holds `to`. Later
animations on the same property take over once their window starts (so a
fade-in at 0–0.5 and a fade-out at 3.5–4 compose naturally).

## Rules

- Keep every clip `id` unique; never reuse ids when duplicating clips.
- `animations` must always be an array (use `[]`).
- Times are seconds (floats fine); coordinates are composition pixels.
- Preserve parts of the document you are not intentionally changing —
  read, modify, write back.
- Invalid documents are rejected: the endpoint 400s on bad JSON, and the
  app ignores structurally invalid files (see `validateComposition` in
  `src/document/types.ts` for the exact rules).

## Common recipes

Fade a clip in: add
`{ "property": "opacity", "from": 0, "to": 1, "start": 0, "end": 0.5, "easing": "easeOut" }`.

Slide in from the left: animate `x` from `finalX - 300` to `finalX` with
easing `"spring"`.

Title card sequence: one track, consecutive text clips (`start` of each =
previous `start + duration`), each with a fade-in and fade-out.

---

# Native engine (M1+): project documents

The Rust engine in `engine/` uses document model v2 — **scenes** (sequential
narrative spine) + **overlays** (tracks crossing scene cuts) + **defs**
(reusable parameterised compositions). Full types: `engine/src/document.rs`.

## Editing surfaces

1. **File**: edit the project JSON (e.g. `engine/examples/demo-project.json`).
2. **HTTP** (while `cargo run --bin serve -- <project.json> [port]` runs,
   default port 7357):
   - `GET  /project` — current document
   - `POST /project` — replace document (validated, saved to disk)
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
- effects: `[{type: "blur", amount}]` (sigma 0-50) and
  `[{type: "color", brightness?, contrast?, saturation?, hue?}]`
- scene transition kinds: crossfade | wipe-lr | wipe-tb | box-wipe | iris | clock
- defs may nest (compref inside a def); cycles are rejected at validation
- `type`: `text` (text/font/color) · `video`/`audio` (src/offset/volume) ·
  `image` (src) · `shape` (M3, skipped with warning for now) ·
  `compref` (ref + args) · `test`
- `transform`: `{ x, y, width, height, opacity }` pixels; 0 = natural
- animations, two forms: tween `{ property, from, to, start, end, easing }`
  or keyframes `{ property, keyframes: [{t, value, easing}, …] }` (>= 2,
  strictly increasing t; property is x|y|width|height|opacity|volume;
  volume 1.0 = unity — animate it for audio fades/ducking),
  times relative to the clip; easing: linear|easeIn|easeOut|easeInOut
- `duration: 0` on a scene layer = fill the rest of the scene

Rules: unique ids everywhere; scenes need `duration > 0`; `compref` targets
must exist in `defs`; defs cannot (yet) reference other defs. Audio policy:
a video clip's own audio is scene-local; music/VO belongs on overlays.
"Detach audio" = set the video clip's `volume: 0` and add an `audio` clip
with the same `src`/`offset` wherever you want it.

## Shapes (M3)

`shape` clips render on the GPU (Vello) when the engine is built with the
`vector` feature: rect, circle, ellipse, star, polygon, line, arrow, with
`fill` (#rrggbb/#aarrggbb) and size from `transform.width/height`.
Rasters cache under `<project dir>/.dualcut-cache/`.

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
