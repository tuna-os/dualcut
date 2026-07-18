import { useEffect, useState } from 'react';
import { useEditor } from './store';
import { usePersistence } from './persistence';
import { Preview } from './components/Preview';
import { Timeline } from './components/Timeline';
import { Inspector } from './components/Inspector';
import { CodePanel } from './components/CodePanel';
import { findClip } from './document/types';
import './App.css';

function fmt(t: number): string {
  const m = Math.floor(t / 60);
  const s = t - m * 60;
  return `${m}:${s.toFixed(2).padStart(5, '0')}`;
}

function usePlaybackLoop() {
  const playing = useEditor((s) => s.playing);
  useEffect(() => {
    if (!playing) return;
    let raf = 0;
    let last = performance.now();
    const tick = (now: number) => {
      const dt = (now - last) / 1000;
      last = now;
      const { currentTime, comp, seek, setPlaying } = useEditor.getState();
      const next = currentTime + dt;
      if (next >= comp.meta.duration) {
        seek(comp.meta.duration);
        setPlaying(false);
        return;
      }
      seek(next);
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [playing]);
}

function useKeyboardShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      if (target.closest('input, textarea, select, .cm-editor')) return;
      const { setPlaying, playing, seek, currentTime, undo, redo, selectedClipId, update, select, comp } =
        useEditor.getState();
      if (e.code === 'Space') {
        e.preventDefault();
        if (!playing && currentTime >= comp.meta.duration) seek(0);
        setPlaying(!playing);
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'z') {
        e.preventDefault();
        if (e.shiftKey) redo();
        else undo();
      } else if (e.key === 'Delete' || e.key === 'Backspace') {
        if (!selectedClipId) return;
        e.preventDefault();
        update((draft) => {
          const f = findClip(draft, selectedClipId);
          if (f) f.track.clips.splice(f.clipIndex, 1);
        });
        select(null);
      } else if (e.key === 'ArrowLeft') {
        seek(currentTime - (e.shiftKey ? 1 : 1 / comp.meta.fps));
      } else if (e.key === 'ArrowRight') {
        seek(currentTime + (e.shiftKey ? 1 : 1 / comp.meta.fps));
      } else if (e.key === 'Home') {
        seek(0);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
}

function TopBar() {
  const comp = useEditor((s) => s.comp);
  const playing = useEditor((s) => s.playing);
  const currentTime = useEditor((s) => s.currentTime);
  const syncStatus = useEditor((s) => s.syncStatus);
  const canUndo = useEditor((s) => s.undoStack.length > 0);
  const canRedo = useEditor((s) => s.redoStack.length > 0);
  const { setPlaying, seek, undo, redo } = useEditor.getState();

  return (
    <div className="topbar">
      <span className="logo">▞ dualcut</span>
      <span className="title">{comp.meta.title}</span>
      <div className="spacer" />
      <button title="Undo (Ctrl+Z)" disabled={!canUndo} onClick={undo}>↩</button>
      <button title="Redo (Ctrl+Shift+Z)" disabled={!canRedo} onClick={redo}>↪</button>
      <div className="transport">
        <button title="Go to start (Home)" onClick={() => seek(0)}>⏮</button>
        <button
          className="play"
          title="Play/Pause (Space)"
          onClick={() => {
            if (!playing && currentTime >= comp.meta.duration) seek(0);
            setPlaying(!playing);
          }}
        >
          {playing ? '⏸' : '▶'}
        </button>
        <span className="timecode">
          {fmt(currentTime)} / {fmt(comp.meta.duration)}
        </span>
      </div>
      <div className="spacer" />
      <span className={`sync sync-${syncStatus}`} title="composition.json sync status">
        ● {syncStatus}
      </span>
    </div>
  );
}

export default function App() {
  usePersistence();
  usePlaybackLoop();
  useKeyboardShortcuts();
  const [tab, setTab] = useState<'inspect' | 'code'>('inspect');

  return (
    <div className="app">
      <TopBar />
      <div className="main">
        <div className="center">
          <Preview />
          <Timeline />
        </div>
        <div className="sidebar">
          <div className="tabs">
            <button className={tab === 'inspect' ? 'active' : ''} onClick={() => setTab('inspect')}>
              Inspect
            </button>
            <button className={tab === 'code' ? 'active' : ''} onClick={() => setTab('code')}>
              Code
            </button>
          </div>
          {tab === 'inspect' ? <Inspector /> : <CodePanel />}
        </div>
      </div>
    </div>
  );
}
