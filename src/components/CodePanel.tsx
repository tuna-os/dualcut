import { useEffect, useMemo, useRef, useState } from 'react';
import CodeMirror from '@uiw/react-codemirror';
import { json } from '@codemirror/lang-json';
import { useEditor } from '../store';
import { validateComposition, type Composition } from '../document/types';

const APPLY_DEBOUNCE_MS = 600;

/**
 * The in-app programmatic surface: the composition document as editable
 * JSON. Edits are parsed, validated and applied to the store (debounced);
 * changes from any other surface refresh the editor text unless the user
 * is mid-edit with a parse error.
 */
export function CodePanel() {
  const comp = useEditor((s) => s.comp);
  const origin = useEditor((s) => s.origin);
  const rev = useEditor((s) => s.rev);
  const { setComposition } = useEditor.getState();

  const [text, setText] = useState(() => JSON.stringify(comp, null, 2));
  const [error, setError] = useState<string | null>(null);
  const applyTimer = useRef<number | undefined>(undefined);
  const lastAppliedRev = useRef(rev);

  // Refresh editor text when the document changes elsewhere (UI, external).
  useEffect(() => {
    if (origin === 'code' && rev === lastAppliedRev.current) return;
    setText(JSON.stringify(comp, null, 2));
    setError(null);
  }, [rev, origin, comp]);

  const onChange = (value: string) => {
    setText(value);
    window.clearTimeout(applyTimer.current);
    applyTimer.current = window.setTimeout(() => {
      let parsed: unknown;
      try {
        parsed = JSON.parse(value);
      } catch (err) {
        setError(`JSON: ${(err as Error).message}`);
        return;
      }
      const problem = validateComposition(parsed);
      if (problem) {
        setError(problem);
        return;
      }
      setError(null);
      setComposition(parsed as Composition, 'code');
      lastAppliedRev.current = useEditor.getState().rev;
    }, APPLY_DEBOUNCE_MS);
  };

  const extensions = useMemo(() => [json()], []);

  return (
    <div className="code-panel">
      <div className={`code-status ${error ? 'error' : 'ok'}`}>
        {error ?? 'Live — valid JSON is applied automatically'}
      </div>
      <CodeMirror
        value={text}
        onChange={onChange}
        extensions={extensions}
        theme="dark"
        height="100%"
        style={{ flex: 1, overflow: 'auto', fontSize: 12 }}
      />
    </div>
  );
}
