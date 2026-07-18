import { useEditor } from '../store';
import { findClip, newId, type Anim, type AnimProperty, type Clip, type Easing, type Element } from '../document/types';

const ANIM_PROPS: AnimProperty[] = ['x', 'y', 'scale', 'rotate', 'opacity'];
const EASINGS: Easing[] = ['linear', 'easeIn', 'easeOut', 'easeInOut', 'spring'];

function Num({ label, value, step = 1, onChange }: { label: string; value: number; step?: number; onChange: (v: number) => void }) {
  return (
    <label className="field">
      <span>{label}</span>
      <input
        type="number"
        value={Number.isFinite(value) ? Math.round(value * 1000) / 1000 : 0}
        step={step}
        onChange={(e) => {
          const v = parseFloat(e.target.value);
          if (Number.isFinite(v)) onChange(v);
        }}
      />
    </label>
  );
}

function Text({ label, value, onChange }: { label: string; value: string; onChange: (v: string) => void }) {
  return (
    <label className="field">
      <span>{label}</span>
      <input type="text" value={value} onChange={(e) => onChange(e.target.value)} />
    </label>
  );
}

function Color({ label, value, onChange }: { label: string; value: string; onChange: (v: string) => void }) {
  return (
    <label className="field">
      <span>{label}</span>
      <input type="color" value={value} onChange={(e) => onChange(e.target.value)} />
    </label>
  );
}

function Select<T extends string>({ label, value, options, onChange }: { label: string; value: T; options: readonly T[]; onChange: (v: T) => void }) {
  return (
    <label className="field">
      <span>{label}</span>
      <select value={value} onChange={(e) => onChange(e.target.value as T)}>
        {options.map((o) => (
          <option key={o} value={o}>{o}</option>
        ))}
      </select>
    </label>
  );
}

function ElementFields({ clip, edit }: { clip: Clip; edit: (fn: (c: Clip) => void) => void }) {
  const el = clip.element;
  const set = <E extends Element>(fn: (e: E) => void) =>
    edit((c) => fn(c.element as E));

  switch (el.type) {
    case 'text':
      return (
        <>
          <label className="field field-wide">
            <span>Text</span>
            <textarea rows={3} value={el.text} onChange={(e) => set<typeof el>((x) => (x.text = e.target.value))} />
          </label>
          <Num label="Font size" value={el.fontSize} onChange={(v) => set<typeof el>((x) => (x.fontSize = v))} />
          <Num label="Weight" value={el.fontWeight ?? 400} step={100} onChange={(v) => set<typeof el>((x) => (x.fontWeight = v))} />
          <Color label="Color" value={el.color} onChange={(v) => set<typeof el>((x) => (x.color = v))} />
          <Select label="Align" value={el.align ?? 'center'} options={['left', 'center', 'right'] as const} onChange={(v) => set<typeof el>((x) => (x.align = v))} />
        </>
      );
    case 'shape':
      return (
        <>
          <Select label="Shape" value={el.shape} options={['rect', 'ellipse'] as const} onChange={(v) => set<typeof el>((x) => (x.shape = v))} />
          <Color label="Fill" value={el.fill} onChange={(v) => set<typeof el>((x) => (x.fill = v))} />
          {el.shape === 'rect' && (
            <Num label="Radius" value={el.radius ?? 0} onChange={(v) => set<typeof el>((x) => (x.radius = v))} />
          )}
        </>
      );
    case 'image':
      return (
        <>
          <Text label="Src" value={el.src} onChange={(v) => set<typeof el>((x) => (x.src = v))} />
          <Select label="Fit" value={el.fit ?? 'cover'} options={['cover', 'contain'] as const} onChange={(v) => set<typeof el>((x) => (x.fit = v))} />
        </>
      );
    case 'video':
      return (
        <>
          <Text label="Src" value={el.src} onChange={(v) => set<typeof el>((x) => (x.src = v))} />
          <Select label="Fit" value={el.fit ?? 'cover'} options={['cover', 'contain'] as const} onChange={(v) => set<typeof el>((x) => (x.fit = v))} />
          <Num label="Offset (s)" value={el.offset ?? 0} step={0.1} onChange={(v) => set<typeof el>((x) => (x.offset = v))} />
          <Num label="Volume" value={el.volume ?? 1} step={0.1} onChange={(v) => set<typeof el>((x) => (x.volume = v))} />
        </>
      );
    case 'audio':
      return (
        <>
          <Text label="Src" value={el.src} onChange={(v) => set<typeof el>((x) => (x.src = v))} />
          <Num label="Offset (s)" value={el.offset ?? 0} step={0.1} onChange={(v) => set<typeof el>((x) => (x.offset = v))} />
          <Num label="Volume" value={el.volume ?? 1} step={0.1} onChange={(v) => set<typeof el>((x) => (x.volume = v))} />
        </>
      );
  }
}

function AnimRow({ anim, index, edit }: { anim: Anim; index: number; edit: (fn: (c: Clip) => void) => void }) {
  const setA = (fn: (a: Anim) => void) => edit((c) => fn(c.animations[index]));
  return (
    <div className="anim-row">
      <Select label="Prop" value={anim.property} options={ANIM_PROPS} onChange={(v) => setA((a) => (a.property = v))} />
      <Num label="From" value={anim.from} step={0.1} onChange={(v) => setA((a) => (a.from = v))} />
      <Num label="To" value={anim.to} step={0.1} onChange={(v) => setA((a) => (a.to = v))} />
      <Num label="Start" value={anim.start} step={0.1} onChange={(v) => setA((a) => (a.start = v))} />
      <Num label="End" value={anim.end} step={0.1} onChange={(v) => setA((a) => (a.end = v))} />
      <Select label="Ease" value={anim.easing} options={EASINGS} onChange={(v) => setA((a) => (a.easing = v))} />
      <button className="mini danger" onClick={() => edit((c) => c.animations.splice(index, 1))}>✕</button>
    </div>
  );
}

const NEW_ELEMENTS: Record<string, Element> = {
  text: { type: 'text', text: 'New text', fontSize: 48, color: '#ffffff', fontWeight: 600, align: 'center' },
  shape: { type: 'shape', shape: 'rect', fill: '#5468ff' },
  image: { type: 'image', src: 'https://picsum.photos/640/360', fit: 'cover' },
  video: { type: 'video', src: '', fit: 'cover' },
  audio: { type: 'audio', src: '' },
};

export function Inspector() {
  const comp = useEditor((s) => s.comp);
  const selectedClipId = useEditor((s) => s.selectedClipId);
  const currentTime = useEditor((s) => s.currentTime);
  const { update, select } = useEditor.getState();

  const found = selectedClipId ? findClip(comp, selectedClipId) : null;

  const addClip = (kind: keyof typeof NEW_ELEMENTS) => {
    const id = newId('clip');
    update((draft) => {
      const track = draft.tracks[0] ?? { id: newId('track'), name: 'Track 1', clips: [] };
      if (!draft.tracks.length) draft.tracks.push(track);
      draft.tracks[0].clips.push({
        id,
        name: `New ${kind}`,
        start: Math.round(currentTime * 10) / 10,
        duration: 3,
        x: draft.meta.width / 2 - 200,
        y: draft.meta.height / 2 - 100,
        width: 400,
        height: 200,
        element: structuredClone(NEW_ELEMENTS[kind]),
        animations: [],
      });
    });
    select(id);
  };

  if (!found) {
    return (
      <div className="inspector">
        <h3>Composition</h3>
        <div className="fields">
          <Text label="Title" value={comp.meta.title} onChange={(v) => update((d) => (d.meta.title = v))} />
          <Num label="Width" value={comp.meta.width} onChange={(v) => update((d) => (d.meta.width = v))} />
          <Num label="Height" value={comp.meta.height} onChange={(v) => update((d) => (d.meta.height = v))} />
          <Num label="Duration (s)" value={comp.meta.duration} step={0.5} onChange={(v) => update((d) => (d.meta.duration = v))} />
          <Num label="FPS" value={comp.meta.fps} onChange={(v) => update((d) => (d.meta.fps = v))} />
          <Color label="Background" value={comp.meta.background} onChange={(v) => update((d) => (d.meta.background = v))} />
        </div>
        <h3>Add clip</h3>
        <div className="add-buttons">
          {Object.keys(NEW_ELEMENTS).map((kind) => (
            <button key={kind} onClick={() => addClip(kind as keyof typeof NEW_ELEMENTS)}>
              + {kind}
            </button>
          ))}
        </div>
        <p className="hint">Select a clip in the timeline or preview to edit its parameters.</p>
      </div>
    );
  }

  const { clip } = found;
  const edit = (fn: (c: Clip) => void) =>
    update((draft) => {
      const f = findClip(draft, clip.id);
      if (f) fn(f.clip);
    });

  const addAnim = (preset?: 'fade-in' | 'fade-out' | 'slide-in') => {
    edit((c) => {
      if (preset === 'fade-in') {
        c.animations.push({ property: 'opacity', from: 0, to: 1, start: 0, end: 0.5, easing: 'easeOut' });
      } else if (preset === 'fade-out') {
        c.animations.push({ property: 'opacity', from: 1, to: 0, start: Math.max(c.duration - 0.5, 0), end: c.duration, easing: 'easeIn' });
      } else if (preset === 'slide-in') {
        c.animations.push({ property: 'x', from: c.x - 300, to: c.x, start: 0, end: 0.6, easing: 'spring' });
      } else {
        c.animations.push({ property: 'opacity', from: 0, to: 1, start: 0, end: 1, easing: 'linear' });
      }
    });
  };

  return (
    <div className="inspector">
      <div className="inspector-head">
        <h3>{clip.name || clip.element.type}</h3>
        <button
          className="mini"
          title="Duplicate clip"
          onClick={() => {
            const copy = structuredClone(clip);
            copy.id = newId('clip');
            copy.name = `${clip.name} copy`;
            copy.start = clip.start + clip.duration;
            update((draft) => {
              const f = findClip(draft, clip.id);
              if (f) f.track.clips.push(copy);
            });
            select(copy.id);
          }}
        >
          ⧉
        </button>
        <button
          className="mini danger"
          title="Delete clip"
          onClick={() => {
            update((draft) => {
              const f = findClip(draft, clip.id);
              if (f) f.track.clips.splice(f.clipIndex, 1);
            });
            select(null);
          }}
        >
          ✕
        </button>
      </div>
      <div className="fields">
        <Text label="Name" value={clip.name} onChange={(v) => edit((c) => (c.name = v))} />
        <Num label="Start (s)" value={clip.start} step={0.1} onChange={(v) => edit((c) => (c.start = Math.max(0, v)))} />
        <Num label="Duration (s)" value={clip.duration} step={0.1} onChange={(v) => edit((c) => (c.duration = Math.max(0.1, v)))} />
      </div>
      <h4>Transform</h4>
      <div className="fields">
        <Num label="X" value={clip.x} onChange={(v) => edit((c) => (c.x = v))} />
        <Num label="Y" value={clip.y} onChange={(v) => edit((c) => (c.y = v))} />
        <Num label="Width" value={clip.width} onChange={(v) => edit((c) => (c.width = v))} />
        <Num label="Height" value={clip.height} onChange={(v) => edit((c) => (c.height = v))} />
        <Num label="Rotate" value={clip.rotate ?? 0} onChange={(v) => edit((c) => (c.rotate = v))} />
        <Num label="Scale" value={clip.scale ?? 1} step={0.05} onChange={(v) => edit((c) => (c.scale = v))} />
        <Num label="Opacity" value={clip.opacity ?? 1} step={0.05} onChange={(v) => edit((c) => (c.opacity = v))} />
      </div>
      <h4>{clip.element.type}</h4>
      <div className="fields">
        <ElementFields clip={clip} edit={edit} />
      </div>
      <h4>Animations</h4>
      <div className="anim-list">
        {clip.animations.map((anim, i) => (
          <AnimRow key={i} anim={anim} index={i} edit={edit} />
        ))}
      </div>
      <div className="add-buttons">
        <button onClick={() => addAnim('fade-in')}>+ Fade in</button>
        <button onClick={() => addAnim('fade-out')}>+ Fade out</button>
        <button onClick={() => addAnim('slide-in')}>+ Slide in</button>
        <button onClick={() => addAnim()}>+ Custom</button>
      </div>
    </div>
  );
}
