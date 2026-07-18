import { useEffect, useRef } from 'react';
import { useEditor } from './store';
import { validateComposition, type Composition } from './document/types';

const SAVE_DEBOUNCE_MS = 400;

/**
 * Keeps the store and composition.json in sync:
 * - initial load from GET /__composition (falls back to the in-memory demo)
 * - debounced POST of ui/code edits
 * - live application of external edits pushed over the Vite websocket
 */
export function usePersistence() {
  const saveTimer = useRef<number | undefined>(undefined);

  useEffect(() => {
    let cancelled = false;

    fetch('/__composition')
      .then(async (res) => {
        if (!res.ok) throw new Error(String(res.status));
        const doc = (await res.json()) as Composition;
        if (cancelled) return;
        const problem = validateComposition(doc);
        if (problem) {
          console.warn(`composition.json invalid (${problem}); keeping demo document`);
          useEditor.getState().setSyncStatus('saved');
          return;
        }
        useEditor.getState().setComposition(doc, 'external');
        useEditor.getState().setSyncStatus('saved');
      })
      .catch(() => {
        // No file yet (fresh project) — save the demo document so agents
        // have something to edit.
        if (!cancelled) save(useEditor.getState().comp);
      });

    if (import.meta.hot) {
      import.meta.hot.on('dualcut:external-composition', (doc: unknown) => {
        const problem = validateComposition(doc);
        if (problem) {
          console.warn(`ignoring external composition update: ${problem}`);
          return;
        }
        useEditor.getState().setComposition(doc as Composition, 'external');
        useEditor.getState().setSyncStatus('saved');
      });
    }

    const save = (comp: Composition) => {
      useEditor.getState().setSyncStatus('saving');
      fetch('/__composition', {
        method: 'POST',
        headers: {
          'content-type': 'application/json',
          'x-dualcut-client': 'app',
        },
        body: JSON.stringify(comp),
      })
        .then((res) => {
          useEditor.getState().setSyncStatus(res.ok ? 'saved' : 'offline');
        })
        .catch(() => useEditor.getState().setSyncStatus('offline'));
    };

    const unsubscribe = useEditor.subscribe((state, prev) => {
      if (state.rev === prev.rev) return;
      if (state.origin === 'external' || state.origin === 'init') return;
      window.clearTimeout(saveTimer.current);
      saveTimer.current = window.setTimeout(() => save(state.comp), SAVE_DEBOUNCE_MS);
    });

    return () => {
      cancelled = true;
      unsubscribe();
      window.clearTimeout(saveTimer.current);
    };
  }, []);
}
