import { useRef } from 'react';
import type React from 'react';
import { useEditor } from '../store';
import { findClip, newId, type Clip, type Track } from '../document/types';

const TRACK_HEIGHT = 44;
const RULER_HEIGHT = 24;

type DragMode = 'move' | 'trim-left' | 'trim-right';

interface DragState {
  mode: DragMode;
  clipId: string;
  startClientX: number;
  startClientY: number;
  origStart: number;
  origDuration: number;
  origTrackIndex: number;
}

function fmtTime(t: number): string {
  const m = Math.floor(t / 60);
  const s = t - m * 60;
  return `${m}:${s.toFixed(1).padStart(4, '0')}`;
}

function ClipBlock({ clip, onDragStart }: { clip: Clip; onDragStart: (e: React.PointerEvent, mode: DragMode, clip: Clip) => void }) {
  const pxPerSec = useEditor((s) => s.pxPerSec);
  const selected = useEditor((s) => s.selectedClipId === clip.id);
  const kindClass = `clip-block clip-${clip.element.type}${selected ? ' selected' : ''}`;

  return (
    <div
      className={kindClass}
      style={{ left: clip.start * pxPerSec, width: Math.max(clip.duration * pxPerSec, 8) }}
      onPointerDown={(e) => onDragStart(e, 'move', clip)}
    >
      <div className="clip-handle left" onPointerDown={(e) => onDragStart(e, 'trim-left', clip)} />
      <span className="clip-label">{clip.name || clip.element.type}</span>
      <div className="clip-handle right" onPointerDown={(e) => onDragStart(e, 'trim-right', clip)} />
    </div>
  );
}

export function Timeline() {
  const comp = useEditor((s) => s.comp);
  const pxPerSec = useEditor((s) => s.pxPerSec);
  const currentTime = useEditor((s) => s.currentTime);
  const { update, beginGesture, endGesture, select, seek, setPxPerSec } = useEditor.getState();

  const lanesRef = useRef<HTMLDivElement>(null);
  const drag = useRef<DragState | null>(null);
  const scrubbing = useRef(false);

  const width = Math.max(comp.meta.duration * pxPerSec + 200, 600);

  const onDragStart = (e: React.PointerEvent, mode: DragMode, clip: Clip) => {
    e.stopPropagation();
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    const found = findClip(comp, clip.id);
    if (!found) return;
    select(clip.id);
    beginGesture();
    drag.current = {
      mode,
      clipId: clip.id,
      startClientX: e.clientX,
      startClientY: e.clientY,
      origStart: clip.start,
      origDuration: clip.duration,
      origTrackIndex: found.trackIndex,
    };
  };

  const onPointerMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const dt = (e.clientX - d.startClientX) / pxPerSec;
    update((draft) => {
      const found = findClip(draft, d.clipId);
      if (!found) return;
      const { clip } = found;
      if (d.mode === 'move') {
        clip.start = Math.max(0, d.origStart + dt);
        const rowDelta = Math.round((e.clientY - d.startClientY) / TRACK_HEIGHT);
        const targetIndex = Math.min(
          Math.max(d.origTrackIndex + rowDelta, 0),
          draft.tracks.length - 1,
        );
        if (targetIndex !== found.trackIndex) {
          found.track.clips.splice(found.clipIndex, 1);
          draft.tracks[targetIndex].clips.push(clip);
        }
      } else if (d.mode === 'trim-left') {
        const maxDelta = d.origDuration - 0.1;
        const delta = Math.min(Math.max(dt, -d.origStart), maxDelta);
        clip.start = d.origStart + delta;
        clip.duration = d.origDuration - delta;
      } else {
        clip.duration = Math.max(0.1, d.origDuration + dt);
      }
    });
  };

  const onPointerUp = () => {
    if (drag.current) {
      drag.current = null;
      endGesture();
    }
  };

  const timeFromEvent = (e: React.PointerEvent) => {
    const rect = lanesRef.current!.getBoundingClientRect();
    return (e.clientX - rect.left) / pxPerSec;
  };

  const onRulerDown = (e: React.PointerEvent) => {
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    scrubbing.current = true;
    seek(timeFromEvent(e));
  };
  const onRulerMove = (e: React.PointerEvent) => {
    if (scrubbing.current) seek(timeFromEvent(e));
  };
  const onRulerUp = () => (scrubbing.current = false);

  const addTrack = () => {
    update((draft) => {
      draft.tracks.push({ id: newId('track'), name: `Track ${draft.tracks.length + 1}`, clips: [] });
    });
  };

  const removeTrack = (trackId: string) => {
    update((draft) => {
      const i = draft.tracks.findIndex((t) => t.id === trackId);
      if (i >= 0) draft.tracks.splice(i, 1);
    });
  };

  const toggle = (trackId: string, key: 'muted' | 'hidden') => {
    update((draft) => {
      const t = draft.tracks.find((t) => t.id === trackId);
      if (t) t[key] = !t[key];
    });
  };

  const seconds = Math.ceil(comp.meta.duration) + 2;

  return (
    <div className="timeline">
      <div className="timeline-toolbar">
        <button onClick={addTrack}>+ Track</button>
        <div className="spacer" />
        <button onClick={() => setPxPerSec(pxPerSec / 1.4)}>−</button>
        <span className="zoom-label">{Math.round(pxPerSec)} px/s</span>
        <button onClick={() => setPxPerSec(pxPerSec * 1.4)}>+</button>
      </div>
      <div className="timeline-scroll">
        <div className="timeline-inner" style={{ width: width + 160 }}>
          <div className="track-headers" style={{ paddingTop: RULER_HEIGHT }}>
            {comp.tracks.map((track: Track) => (
              <div className="track-header" key={track.id} style={{ height: TRACK_HEIGHT }}>
                <span className="track-name" title={track.name}>{track.name}</span>
                <button
                  className={`mini ${track.hidden ? 'off' : ''}`}
                  title="Toggle visibility"
                  onClick={() => toggle(track.id, 'hidden')}
                >
                  {track.hidden ? '🙈' : '👁'}
                </button>
                <button
                  className={`mini ${track.muted ? 'off' : ''}`}
                  title="Toggle mute"
                  onClick={() => toggle(track.id, 'muted')}
                >
                  {track.muted ? '🔇' : '🔊'}
                </button>
                <button className="mini danger" title="Delete track" onClick={() => removeTrack(track.id)}>
                  ✕
                </button>
              </div>
            ))}
          </div>
          <div className="lanes-column">
            <div
              className="ruler"
              style={{ height: RULER_HEIGHT, width }}
              onPointerDown={onRulerDown}
              onPointerMove={onRulerMove}
              onPointerUp={onRulerUp}
            >
              {Array.from({ length: seconds }, (_, i) => (
                <span key={i} className="tick" style={{ left: i * pxPerSec }}>
                  {fmtTime(i)}
                </span>
              ))}
            </div>
            <div
              className="lanes"
              ref={lanesRef}
              style={{ width }}
              onPointerMove={onPointerMove}
              onPointerUp={onPointerUp}
              onPointerDown={() => select(null)}
            >
              {comp.tracks.map((track) => (
                <div className="lane" key={track.id} style={{ height: TRACK_HEIGHT }}>
                  {track.clips.map((clip) => (
                    <ClipBlock key={clip.id} clip={clip} onDragStart={onDragStart} />
                  ))}
                </div>
              ))}
              <div
                className="playhead"
                style={{ left: currentTime * pxPerSec, top: -RULER_HEIGHT }}
              />
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
