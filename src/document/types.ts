// The composition document is the single source of truth for the editor.
// It is edited by three surfaces: the timeline UI, the in-app code panel,
// and external agents (via composition.json / the /__composition endpoint).
// All times are in seconds. All coordinates are in composition pixels.

export interface Composition {
  meta: CompositionMeta;
  tracks: Track[];
}

export interface CompositionMeta {
  title: string;
  width: number;
  height: number;
  fps: number;
  duration: number;
  background: string;
}

export interface Track {
  id: string;
  name: string;
  muted?: boolean;
  hidden?: boolean;
  clips: Clip[];
}

export interface Clip {
  id: string;
  name: string;
  /** When the clip appears on the composition timeline, in seconds. */
  start: number;
  /** How long the clip is visible, in seconds. */
  duration: number;
  x: number;
  y: number;
  width: number;
  height: number;
  rotate?: number;
  scale?: number;
  opacity?: number;
  element: Element;
  animations: Anim[];
}

export type Element =
  | TextElement
  | ShapeElement
  | ImageElement
  | VideoElement
  | AudioElement;

export interface TextElement {
  type: 'text';
  text: string;
  fontSize: number;
  color: string;
  fontFamily?: string;
  fontWeight?: number;
  align?: 'left' | 'center' | 'right';
}

export interface ShapeElement {
  type: 'shape';
  shape: 'rect' | 'ellipse';
  fill: string;
  radius?: number;
}

export interface ImageElement {
  type: 'image';
  src: string;
  fit?: 'cover' | 'contain';
}

export interface VideoElement {
  type: 'video';
  src: string;
  fit?: 'cover' | 'contain';
  volume?: number;
  /** Seek offset into the source file, in seconds. */
  offset?: number;
}

export interface AudioElement {
  type: 'audio';
  src: string;
  volume?: number;
  offset?: number;
}

export type AnimProperty = 'x' | 'y' | 'scale' | 'rotate' | 'opacity';
export type Easing = 'linear' | 'easeIn' | 'easeOut' | 'easeInOut' | 'spring';

export interface Anim {
  property: AnimProperty;
  from: number;
  to: number;
  /** Start of the animation relative to the clip's own start, in seconds. */
  start: number;
  end: number;
  easing: Easing;
}

export function findClip(
  comp: Composition,
  clipId: string,
): { track: Track; trackIndex: number; clip: Clip; clipIndex: number } | null {
  for (let ti = 0; ti < comp.tracks.length; ti++) {
    const track = comp.tracks[ti];
    for (let ci = 0; ci < track.clips.length; ci++) {
      if (track.clips[ci].id === clipId) {
        return { track, trackIndex: ti, clip: track.clips[ci], clipIndex: ci };
      }
    }
  }
  return null;
}

let idCounter = 0;
export function newId(prefix: string): string {
  idCounter += 1;
  return `${prefix}-${Date.now().toString(36)}-${idCounter.toString(36)}`;
}

/** Structural validation for documents arriving from the code panel or disk. */
export function validateComposition(value: unknown): string | null {
  if (typeof value !== 'object' || value === null) return 'document must be an object';
  const doc = value as Partial<Composition>;
  const meta = doc.meta;
  if (!meta) return 'missing "meta"';
  for (const key of ['width', 'height', 'fps', 'duration'] as const) {
    if (typeof meta[key] !== 'number' || !(meta[key]! > 0)) {
      return `meta.${key} must be a positive number`;
    }
  }
  if (!Array.isArray(doc.tracks)) return '"tracks" must be an array';
  const seen = new Set<string>();
  for (const track of doc.tracks) {
    if (typeof track.id !== 'string' || !track.id) return 'every track needs a string id';
    if (!Array.isArray(track.clips)) return `track ${track.id}: "clips" must be an array`;
    for (const clip of track.clips) {
      if (typeof clip.id !== 'string' || !clip.id) return 'every clip needs a string id';
      if (seen.has(clip.id)) return `duplicate clip id "${clip.id}"`;
      seen.add(clip.id);
      if (typeof clip.start !== 'number' || clip.start < 0) {
        return `clip ${clip.id}: "start" must be a number >= 0`;
      }
      if (typeof clip.duration !== 'number' || !(clip.duration > 0)) {
        return `clip ${clip.id}: "duration" must be a positive number`;
      }
      if (!clip.element || typeof (clip.element as Element).type !== 'string') {
        return `clip ${clip.id}: missing element.type`;
      }
      if (!Array.isArray(clip.animations)) {
        return `clip ${clip.id}: "animations" must be an array (use [])`;
      }
    }
  }
  return null;
}
