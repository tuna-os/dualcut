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
