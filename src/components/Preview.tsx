import { useLayoutEffect, useRef, useState } from 'react';
import { useEditor } from '../store';
import { resolveClipProps } from '../document/interpolate';
import type { Clip, Element, Track } from '../document/types';

function ElementView({ element }: { element: Element }) {
  switch (element.type) {
    case 'text':
      return (
        <div
          style={{
            width: '100%',
            height: '100%',
            display: 'flex',
            alignItems: 'center',
            justifyContent:
              element.align === 'left' ? 'flex-start' : element.align === 'right' ? 'flex-end' : 'center',
            textAlign: element.align ?? 'center',
            fontSize: element.fontSize,
            color: element.color,
            fontFamily: element.fontFamily ?? 'Inter, system-ui, sans-serif',
            fontWeight: element.fontWeight ?? 400,
            lineHeight: 1.15,
            whiteSpace: 'pre-wrap',
          }}
        >
          {element.text}
        </div>
      );
    case 'shape':
      return (
        <div
          style={{
            width: '100%',
            height: '100%',
            background: element.fill,
            borderRadius: element.shape === 'ellipse' ? '50%' : (element.radius ?? 0),
          }}
        />
      );
    case 'image':
      return (
        <img
          src={element.src}
          draggable={false}
          style={{ width: '100%', height: '100%', objectFit: element.fit ?? 'cover' }}
        />
      );
    case 'video':
      return null; // rendered by VideoClipView so it can sync playback
    case 'audio':
      return null;
  }
}

function VideoClipView({ clip, localTime }: { clip: Clip; localTime: number }) {
  const playing = useEditor((s) => s.playing);
  const ref = useRef<HTMLVideoElement>(null);
  const el = clip.element.type === 'video' ? clip.element : null;

  useLayoutEffect(() => {
    const video = ref.current;
    if (!video || !el) return;
    const target = (el.offset ?? 0) + localTime;
    if (Math.abs(video.currentTime - target) > 0.25) video.currentTime = target;
    if (playing && video.paused) video.play().catch(() => {});
    if (!playing && !video.paused) video.pause();
  });

  if (!el) return null;
  return (
    <video
      ref={ref}
      src={el.src}
      muted={(el.volume ?? 1) === 0}
      style={{ width: '100%', height: '100%', objectFit: el.fit ?? 'cover' }}
    />
  );
}

function AudioClipView({ clip, localTime }: { clip: Clip; localTime: number }) {
  const playing = useEditor((s) => s.playing);
  const ref = useRef<HTMLAudioElement>(null);
  const el = clip.element.type === 'audio' ? clip.element : null;

  useLayoutEffect(() => {
    const audio = ref.current;
    if (!audio || !el) return;
    audio.volume = el.volume ?? 1;
    const target = (el.offset ?? 0) + localTime;
    if (Math.abs(audio.currentTime - target) > 0.25) audio.currentTime = target;
    if (playing && audio.paused) audio.play().catch(() => {});
    if (!playing && !audio.paused) audio.pause();
  });

  if (!el) return null;
  return <audio ref={ref} src={el.src} />;
}

function ClipView({ clip, zIndex, muted }: { clip: Clip; zIndex: number; muted?: boolean }) {
  const currentTime = useEditor((s) => s.currentTime);
  const selected = useEditor((s) => s.selectedClipId === clip.id);
  const select = useEditor((s) => s.select);
  const localTime = currentTime - clip.start;
  const props = resolveClipProps(clip, localTime);

  if (clip.element.type === 'audio') {
    return muted ? null : <AudioClipView clip={clip} localTime={localTime} />;
  }

  return (
    <div
      onPointerDown={(e) => {
        e.stopPropagation();
        select(clip.id);
      }}
      style={{
        position: 'absolute',
        left: props.x,
        top: props.y,
        width: clip.width,
        height: clip.height,
        transform: `rotate(${props.rotate}deg) scale(${props.scale})`,
        opacity: props.opacity,
        zIndex,
        outline: selected ? '2px solid #5468ff' : 'none',
        outlineOffset: 2,
        cursor: 'pointer',
      }}
    >
      {clip.element.type === 'video' ? (
        <VideoClipView clip={clip} localTime={localTime} />
      ) : (
        <ElementView element={clip.element} />
      )}
    </div>
  );
}

function activeClips(track: Track, time: number): Clip[] {
  return track.clips.filter((c) => time >= c.start && time < c.start + c.duration);
}

export function Preview() {
  const comp = useEditor((s) => s.comp);
  const currentTime = useEditor((s) => s.currentTime);
  const select = useEditor((s) => s.select);
  const containerRef = useRef<HTMLDivElement>(null);
  const [fitScale, setFitScale] = useState(0.5);

  useLayoutEffect(() => {
    const node = containerRef.current;
    if (!node) return;
    const observer = new ResizeObserver(() => {
      const pad = 32;
      const scaleX = (node.clientWidth - pad) / comp.meta.width;
      const scaleY = (node.clientHeight - pad) / comp.meta.height;
      setFitScale(Math.max(Math.min(scaleX, scaleY), 0.05));
    });
    observer.observe(node);
    return () => observer.disconnect();
  }, [comp.meta.width, comp.meta.height]);

  return (
    <div className="preview-outer" ref={containerRef} onPointerDown={() => select(null)}>
      <div
        className="preview-stage"
        style={{
          width: comp.meta.width,
          height: comp.meta.height,
          background: comp.meta.background,
          transform: `scale(${fitScale})`,
        }}
      >
        {comp.tracks.map((track, ti) =>
          track.hidden
            ? null
            : activeClips(track, currentTime).map((clip) => (
                <ClipView
                  key={clip.id}
                  clip={clip}
                  zIndex={comp.tracks.length - ti}
                  muted={track.muted}
                />
              )),
        )}
      </div>
    </div>
  );
}
