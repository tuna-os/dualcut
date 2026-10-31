use super::super::schema::*;
use super::super::find_clip;
use super::split::split_clip;
use anyhow::{bail, Result};

/// Grow a scene's duration so a scene-layer clip (by id) fits, if it
/// doesn't already (#21: "make scene duration flexible" — dragging or
/// trimming a clip past the current scene end expands the scene rather
/// than clamping the clip). No-op for overlay clips (already
/// unbounded) or if the clip already fits.
pub fn grow_scene_for_clip(project: &mut Project, id: &str) {
    for scene in &mut project.scenes {
        let Some(clip) = scene.layers.iter().find(|c| c.id == id) else { continue };
        let needed = clip.start + if clip.duration > 0.0 { clip.duration } else { 0.1 };
        if needed > scene.duration {
            scene.duration = needed;
        }
        return;
    }
}

/// Ripple delete (#34): remove the clip and close the gap it leaves.
/// Overlay clips shift everything after them in the track left; scene
/// clips shrink the scene toward the remaining content.
pub fn ripple_delete(project: &mut Project, id: &str) -> Result<()> {
    for track in &mut project.overlays {
        if let Some(pos) = track.clips.iter().position(|c| c.id == id) {
            let removed = track.clips.remove(pos);
            let span = removed.duration.max(0.0);
            for clip in &mut track.clips {
                if clip.start >= removed.start {
                    clip.start = (clip.start - span).max(0.0);
                }
            }
            return Ok(());
        }
    }
    for si in 0..project.scenes.len() {
        let scene = &mut project.scenes[si];
        if let Some(pos) = scene.layers.iter().position(|c| c.id == id) {
            let removed = scene.layers.remove(pos);
            let span = if removed.duration > 0.0 {
                removed.duration
            } else {
                (scene.duration - removed.start).max(0.0)
            };
            // Shrink the scene, but never below the remaining content.
            let content_end = scene
                .layers
                .iter()
                .map(|c| c.start + if c.duration > 0.0 { c.duration } else { 0.1 })
                .fold(0.1f64, f64::max);
            scene.duration = (scene.duration - span).max(content_end);
            return Ok(());
        }
    }
    bail!("no clip {id:?}")
}

/// Splice out the stretches of `id`'s own media that fall within
/// `silent_ranges` (media-relative seconds, e.g. from
/// `crate::silence::detect_silence`), closing the gap left behind (#46).
/// Only clips at normal playback rate (1.0) are supported, matching
/// `split_clip`'s timeline-second contract. Returns how many ranges were
/// actually removed (a range entirely outside the clip's used media
/// window is silently skipped).
///
/// Overlay clips ripple the whole track (via `ripple_delete`, already
/// correct there); scene layers are independent lanes stacked in time
/// (`ripple_delete` deliberately leaves siblings alone for those), so this
/// shifts only the edited clip's own later pieces and shrinks the scene to
/// fit -- other layers in the same scene keep their timing.
pub fn remove_silence(project: &mut Project, id: &str, silent_ranges: &[(f64, f64)]) -> Result<usize> {
    let mut container_offset = 0.0;
    let mut found = false;
    for (i, scene) in project.scenes.iter().enumerate() {
        if scene.layers.iter().any(|c| c.id == id) {
            container_offset = project.scene_offset(i);
            found = true;
            break;
        }
    }
    if !found && !project.overlays.iter().any(|t| t.clips.iter().any(|c| c.id == id)) {
        bail!("no clip {id:?}");
    }

    let clip = find_clip(project, id).expect("checked above");
    let (offset, rate) = match &clip.element {
        Element::Video { offset, rate, .. } | Element::Audio { offset, rate, .. } => (*offset, *rate),
        _ => bail!("clip {id:?} has no media offset (not video/audio)"),
    };
    if (rate - 1.0).abs() > 1e-6 {
        bail!("remove_silence only supports rate 1.0 clips (clip {id:?} has rate {rate})");
    }
    let media_span = if clip.duration > 0.0 { clip.duration } else { f64::MAX };
    let clip_start_abs = container_offset + clip.start;

    // Media-relative ranges -> timeline-absolute, clipped to the clip's
    // actual used media window, latest-first so earlier split points stay
    // valid as later cuts ripple everything after them backward.
    let mut abs_ranges: Vec<(f64, f64)> = silent_ranges
        .iter()
        .filter_map(|&(ms, me)| {
            let s = ms.max(offset);
            let e = me.min(offset + media_span);
            (e > s).then_some((clip_start_abs + (s - offset), clip_start_abs + (e - offset)))
        })
        .collect();
    abs_ranges.sort_by(|a, b| a.0.total_cmp(&b.0));

    // Individual ranges can legitimately fail to split (e.g. silence
    // sitting exactly at the clip's edge, which split_clip rejects as "not
    // inside the clip") -- skip those rather than aborting a batch that's
    // otherwise fine.
    let mut removed = 0;
    for (start, end) in abs_ranges.into_iter().rev() {
        let Ok(tail_id) = split_clip(project, id, start) else { continue };
        let Ok(after_id) = split_clip(project, &tail_id, end) else { continue };
        let span = end - start;
        let is_overlay = project.overlays.iter().any(|t| t.clips.iter().any(|c| c.id == tail_id));
        if is_overlay {
            // Overlay tracks are one shared sequential timeline, so
            // ripple_delete already shifts every later clip on the track
            // correctly.
            if ripple_delete(project, &tail_id).is_ok() {
                removed += 1;
            }
            continue;
        }
        // Scene layers are independent lanes stacked in time (captions
        // shouldn't jump just because the video layer lost a silent
        // stretch) -- ripple_delete deliberately leaves siblings alone
        // there, so close this clip's own gap by hand: shift only its own
        // continuation (after_id) back, then shrink the scene to fit.
        for scene in &mut project.scenes {
            let Some(pos) = scene.layers.iter().position(|c| c.id == tail_id) else { continue };
            scene.layers.remove(pos);
            if let Some(after) = scene.layers.iter_mut().find(|c| c.id == after_id) {
                after.start = (after.start - span).max(0.0);
            }
            scene.duration = scene
                .layers
                .iter()
                .map(|c| c.start + if c.duration > 0.0 { c.duration } else { 0.1 })
                .fold(0.1f64, f64::max);
            break;
        }
        removed += 1;
    }
    Ok(removed)
}
