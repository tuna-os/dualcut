import { create } from 'zustand';
import type { Composition } from './document/types';
import { demoComposition } from './document/demo';

/**
 * Where a document change came from. The persistence layer and the code
 * panel use this to avoid feedback loops (e.g. an external file edit must
 * not be POSTed straight back to disk).
 */
export type Origin = 'init' | 'ui' | 'code' | 'external';

export type SyncStatus = 'saved' | 'saving' | 'offline';

interface EditorState {
  comp: Composition;
  /** Monotonic revision counter; bumps on every document change. */
  rev: number;
  origin: Origin;

  selectedClipId: string | null;
  currentTime: number;
  playing: boolean;
  pxPerSec: number;
  syncStatus: SyncStatus;

  undoStack: Composition[];
  redoStack: Composition[];
  transientBase: Composition | null;

  setComposition: (comp: Composition, origin: Origin) => void;
  /** Clone-and-mutate update. UI edits go through here. */
  update: (fn: (draft: Composition) => void) => void;
  /** Start a continuous gesture (drag/trim); one undo entry per gesture. */
  beginGesture: () => void;
  endGesture: () => void;
  undo: () => void;
  redo: () => void;

  select: (clipId: string | null) => void;
  seek: (time: number) => void;
  setPlaying: (playing: boolean) => void;
  setPxPerSec: (v: number) => void;
  setSyncStatus: (s: SyncStatus) => void;
}

const MAX_UNDO = 100;

export const useEditor = create<EditorState>((set, get) => ({
  comp: demoComposition,
  rev: 0,
  origin: 'init',

  selectedClipId: null,
  currentTime: 0,
  playing: false,
  pxPerSec: 80,
  syncStatus: 'saving',

  undoStack: [],
  redoStack: [],
  transientBase: null,

  setComposition: (comp, origin) =>
    set((s) => ({
      comp,
      origin,
      rev: s.rev + 1,
      undoStack: [...s.undoStack.slice(-MAX_UNDO), s.comp],
      redoStack: [],
      currentTime: Math.min(s.currentTime, comp.meta.duration),
    })),

  update: (fn) => {
    const s = get();
    const draft = structuredClone(s.comp);
    fn(draft);
    set({
      comp: draft,
      origin: 'ui',
      rev: s.rev + 1,
      // During a gesture the undo entry was already captured by beginGesture.
      undoStack: s.transientBase ? s.undoStack : [...s.undoStack.slice(-MAX_UNDO), s.comp],
      redoStack: s.transientBase ? s.redoStack : [],
    });
  },

  beginGesture: () => {
    const s = get();
    if (s.transientBase) return;
    set({
      transientBase: s.comp,
      undoStack: [...s.undoStack.slice(-MAX_UNDO), s.comp],
      redoStack: [],
    });
  },

  endGesture: () => set({ transientBase: null }),

  undo: () => {
    const s = get();
    const prev = s.undoStack[s.undoStack.length - 1];
    if (!prev) return;
    set({
      comp: prev,
      origin: 'ui',
      rev: s.rev + 1,
      undoStack: s.undoStack.slice(0, -1),
      redoStack: [...s.redoStack, s.comp],
      transientBase: null,
    });
  },

  redo: () => {
    const s = get();
    const next = s.redoStack[s.redoStack.length - 1];
    if (!next) return;
    set({
      comp: next,
      origin: 'ui',
      rev: s.rev + 1,
      redoStack: s.redoStack.slice(0, -1),
      undoStack: [...s.undoStack, s.comp],
      transientBase: null,
    });
  },

  select: (clipId) => set({ selectedClipId: clipId }),
  seek: (time) =>
    set((s) => ({
      currentTime: Math.min(Math.max(time, 0), s.comp.meta.duration),
    })),
  setPlaying: (playing) => set({ playing }),
  setPxPerSec: (v) => set({ pxPerSec: Math.min(Math.max(v, 10), 400) }),
  setSyncStatus: (syncStatus) => set({ syncStatus }),
}));
