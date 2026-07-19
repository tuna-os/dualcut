# Feature parity

Where Dualcut stands against mainstream editors (#28). Columns split
the **GUI** (what a human can do in the app) from the **Backend** (what
the document/engine/agent surface supports) because they intentionally
lead-lag each other.

Legend: ✅ solid · 🟡 partial/basic · ❌ absent · — not applicable

## Media & library

| Feature | CapCut Desktop | iMovie | Kdenlive | Dualcut GUI | Dualcut Backend |
|---|---|---|---|---|---|
| Import media library | ✅ | ✅ | ✅ | ✅ | ✅ (`library`) |
| Drag-and-drop import | ✅ | ✅ | ✅ | ✅ | — |
| Thumbnails / waveforms | ✅ | ✅ | ✅ | ✅ | ✅ (cached) |
| Proxy media for smooth preview | ✅ | ✅ | ✅ | ✅ | ✅ (cached, preview-only) |
| Stock/cloud asset store | ✅ | 🟡 | ❌ | ❌ | ❌ |

## Timeline

| Feature | CapCut Desktop | iMovie | Kdenlive | Dualcut GUI | Dualcut Backend |
|---|---|---|---|---|---|
| Multi-track layers | ✅ | 🟡 (main + overlays) | ✅ | ✅ (scene lanes + overlays) | ✅ |
| Scene/section grouping | ❌ | ❌ | ❌ | ✅ (ruler) | ✅ |
| Drag to move / retime | ✅ | ✅ | ✅ | ✅ | ✅ |
| Edge trim | ✅ | ✅ | ✅ | ✅ (right edge) | ✅ |
| Split at playhead | ✅ | ✅ | ✅ | ❌ | 🟡 (script can) |
| Snapping | ✅ | ✅ | ✅ | ✅ | ✅ |
| Zoom | ✅ | ✅ | ✅ | ✅ | — |
| Playhead indicator | ✅ | ✅ | ✅ | ✅ | — |
| Ripple/roll/slip edits | ✅ | 🟡 | ✅ | ❌ | ❌ |
| Track mute/hide toggles | ✅ | ✅ | ✅ | ❌ | 🟡 (volume/opacity) |

## Editing & compositing

| Feature | CapCut Desktop | iMovie | Kdenlive | Dualcut GUI | Dualcut Backend |
|---|---|---|---|---|---|
| Transform (position/scale) | ✅ | 🟡 | ✅ | ✅ (preview handles) | ✅ |
| Opacity | ✅ | ✅ | ✅ | ✅ | ✅ |
| Keyframe animation | ✅ | ❌ | ✅ | ✅ | ✅ (tween + keyframes) |
| Animation presets (in/out) | ✅ | ✅ | 🟡 | ✅ | ✅ |
| Scene transitions (wipes etc.) | ✅ | ✅ | ✅ | ✅ (6 kinds) | ✅ |
| Effects (blur, color) | ✅ | 🟡 | ✅ | ✅ | ✅ |
| Full color grading | ✅ | 🟡 | ✅ | ❌ | 🟡 (videobalance) |
| Masks / chroma key | ✅ | ✅ | ✅ | ❌ | ❌ |
| Speed ramping | ✅ | ✅ | ✅ | ❌ | ❌ |
| Vector shapes | 🟡 (stickers) | ❌ | 🟡 | ✅ (7 shapes, live GPU) | ✅ (`vello://`) |

## Text & templates

| Feature | CapCut Desktop | iMovie | Kdenlive | Dualcut GUI | Dualcut Backend |
|---|---|---|---|---|---|
| Text clips | ✅ | ✅ | ✅ | ✅ | ✅ |
| Rich text styling | ✅ | 🟡 | ✅ | 🟡 (font/color) | 🟡 |
| Title templates | ✅ | ✅ | ✅ | ✅ (defs + thumbnails) | ✅ |
| Parameterised/nested templates | 🟡 | ❌ | ❌ | ✅ | ✅ (defs nest) |
| Save selection as template | 🟡 | ❌ | 🟡 | ✅ | ✅ |
| Auto-captions (STT) | ✅ | ❌ | 🟡 | ❌ | ❌ |

## Audio

| Feature | CapCut Desktop | iMovie | Kdenlive | Dualcut GUI | Dualcut Backend |
|---|---|---|---|---|---|
| Audio tracks | ✅ | ✅ | ✅ | ✅ (overlays) | ✅ |
| Waveform display | ✅ | ✅ | ✅ | ✅ | — |
| Volume keyframes/fades | ✅ | ✅ | ✅ | ✅ (presets + keyframes) | ✅ |
| Detach audio from video | ✅ | ✅ | ✅ | ✅ | ✅ |
| Auto-crossfade at cuts | ✅ | ✅ | 🟡 | ✅ | ✅ |
| Audio effects (EQ, denoise) | ✅ | 🟡 | ✅ | ❌ | ❌ |

## Export

| Feature | CapCut Desktop | iMovie | Kdenlive | Dualcut GUI | Dualcut Backend |
|---|---|---|---|---|---|
| H.264 / H.265 | ✅ | ✅ | ✅ | ✅ | ✅ |
| VP8/VP9/AV1 | 🟡 | ❌ | ✅ | ✅ | ✅ |
| ProRes / lossless (FFV1) | ❌ | ✅ (ProRes) | ✅ | ✅ | ✅ |
| Audio-only export | ✅ | ✅ | ✅ | ✅ (5 formats) | ✅ |
| Overwrite guard, dir picker | ✅ | ✅ | ✅ | ✅ | — |
| Render progress | ✅ | ✅ | ✅ | 🟡 (busy state) | — |
| Background render queue | ✅ | ❌ | ✅ | ❌ | 🟡 (HTTP `/render`) |

## Automation (Dualcut's home turf)

| Feature | CapCut Desktop | iMovie | Kdenlive | Dualcut GUI | Dualcut Backend |
|---|---|---|---|---|---|
| Human-readable project format | ❌ | ❌ | 🟡 (XML) | ✅ (Code tab) | ✅ (JSON + schema) |
| Hot-reload on external edit | ❌ | ❌ | ❌ | ✅ | ✅ |
| Scripting | ❌ | ❌ | 🟡 (Python, limited) | ✅ (TypeScript) | ✅ |
| HTTP API | ❌ | ❌ | ❌ | — | ✅ (port 7357) |
| Agent skill / docs for AI edits | ❌ | ❌ | ❌ | ✅ (installer) | ✅ |
| Headless render CLI | ❌ | ❌ | ✅ (melt) | — | ✅ |

## Biggest gaps to close (GUI-first)

1. **Split at playhead** — the most-used edit primitive we lack (#21).
2. **Track mute/solo/hide** toggles on lanes.
3. **Masks / chroma key** — needs engine work (frei0r or custom).
4. **Speed ramping** — GES supports rate; unexposed in the document.
5. **Auto-captions** — pairs naturally with the agent surface (STT →
   subtitle overlay track).
