/**
 * dualcut project document — TypeScript declarations.
 * Mirrors engine/src/document.rs (the source of truth).
 *
 * Scripts POSTed to the engine's /script endpoint must export:
 *   export function edit(project: Project): Project
 */

export interface Project {
  meta: Meta;
  /** Reusable parameterised compositions, instantiated via CompRefClip. */
  defs?: Record<string, CompDef>;
  /** Sequential narrative spine; order = time, no gaps. */
  scenes: Scene[];
  /** Absolutely-timed tracks that cross scene cuts (subtitles, music…). */
  overlays?: OverlayTrack[];
}

export interface Meta {
  title: string;
  width: number;
  height: number;
  fps: number;
}

export interface CompDef {
  /** Occurrences of `{param}` in string fields substitute at instantiation. */
  params?: string[];
  layers: Clip[];
}

export interface Scene {
  id: string;
  name?: string;
  /** Seconds. */
  duration: number;
  /** Transition from the previous scene into this one; omit = hard cut. */
  transition?: Transition;
  /** Composited top-first (index 0 renders on top). */
  layers?: Clip[];
}

export interface Transition {
  kind?: "crossfade";
  /** Seconds of overlap with the previous scene; must be < scene duration. */
  duration: number;
}

export interface OverlayTrack {
  id: string;
  name?: string;
  clips?: Clip[];
}

export type Clip = ClipBase &
  (
    | TextClip
    | VideoClip
    | AudioClip
    | ImageClip
    | ShapeClip
    | CompRefClip
    | TestClip
  );

export interface ClipBase {
  /** Unique across the whole document. */
  id: string;
  /** Seconds. Scene layers: relative to the scene. Overlays: absolute. */
  start?: number;
  /** Seconds. 0 on a scene layer = fill the rest of the scene. */
  duration?: number;
  transform?: Transform;
  animations?: Anim[];
}

export interface TextClip {
  type: "text";
  text: string;
  /** Pango font description, e.g. "Sans Bold 32". */
  font?: string;
  /** "#rrggbb" or "#aarrggbb". */
  color?: string;
}

export interface VideoClip {
  type: "video";
  src: string;
  /** Seek into the source file, seconds. */
  offset?: number;
  /** 0..1; 0 mutes (see detach-audio pattern in AGENTS.md). */
  volume?: number;
}

export interface AudioClip {
  type: "audio";
  src: string;
  offset?: number;
  volume?: number;
}

export interface ImageClip {
  type: "image";
  src: string;
}

export interface ShapeClip {
  type: "shape";
  shape: "rect" | "circle" | "ellipse" | "star" | "polygon" | "line" | "arrow";
  fill?: string;
}

export interface CompRefClip {
  type: "compref";
  /** Key into Project.defs. */
  ref: string;
  args?: Record<string, string>;
}

export interface TestClip {
  type: "test";
}

/** Position/size in composition pixels; 0 width/height = natural size. */
export interface Transform {
  x?: number;
  y?: number;
  width?: number;
  height?: number;
  /** 0..1 */
  opacity?: number;
}

export interface Anim {
  property: "x" | "y" | "width" | "height" | "opacity";
  /** Tween window form (ignored when keyframes is set). */
  from?: number;
  to?: number;
  /** Seconds relative to the clip's own start. */
  start?: number;
  end?: number;
  easing?: "linear" | "easeIn" | "easeOut" | "easeInOut";
  /** Explicit keyframes (>= 2, strictly increasing t). Wins over the tween. */
  keyframes?: Keyframe[];
}

export interface Keyframe {
  /** Seconds relative to the clip. */
  t: number;
  value: number;
  /** Easing from the previous keyframe into this one. */
  easing?: "linear" | "easeIn" | "easeOut" | "easeInOut";
}
