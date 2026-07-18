import type { Anim, AnimProperty, Clip, Easing } from './types';

const easings: Record<Easing, (t: number) => number> = {
  linear: (t) => t,
  easeIn: (t) => t * t * t,
  easeOut: (t) => 1 - Math.pow(1 - t, 3),
  easeInOut: (t) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2),
  spring: (t) => {
    const c1 = 1.70158;
    const c3 = c1 + 1;
    return 1 + c3 * Math.pow(t - 1, 3) + c1 * Math.pow(t - 1, 2);
  },
};

export interface ResolvedProps {
  x: number;
  y: number;
  scale: number;
  rotate: number;
  opacity: number;
}

function evalAnim(anim: Anim, localTime: number): number {
  const span = Math.max(anim.end - anim.start, 1e-6);
  const t = Math.min(Math.max((localTime - anim.start) / span, 0), 1);
  return anim.from + (anim.to - anim.from) * easings[anim.easing](t);
}

/**
 * Resolve a clip's animatable properties at a time local to the clip
 * (0 = clip start). Animations are applied in document order; when several
 * target the same property, one that has started overrides earlier ones.
 */
export function resolveClipProps(clip: Clip, localTime: number): ResolvedProps {
  const props: ResolvedProps = {
    x: clip.x,
    y: clip.y,
    scale: clip.scale ?? 1,
    rotate: clip.rotate ?? 0,
    opacity: clip.opacity ?? 1,
  };
  const started: Partial<Record<AnimProperty, boolean>> = {};
  for (const anim of clip.animations) {
    if (localTime >= anim.start || !started[anim.property]) {
      props[anim.property] = evalAnim(anim, localTime);
      started[anim.property] = true;
    }
  }
  return props;
}
