use super::super::schema::*;
use super::super::{ease, find_clip, find_clip_mut};
use anyhow::{bail, Result};

/// Split a clip at an absolute timeline time (#29). The original keeps
/// the left side; a new clip (returned id) takes the right, with media
/// offsets advanced and animations divided between the halves.
pub fn split_clip(project: &mut Project, id: &str, abs_time: f64) -> Result<String> {
    // Locate the clip and the timeline offset of its container.
    let mut container_offset = 0.0;
    let mut container_span = f64::MAX;
    let mut found = false;
    for (i, scene) in project.scenes.iter().enumerate() {
        if scene.layers.iter().any(|c| c.id == id) {
            container_offset = project.scene_offset(i);
            container_span = scene.duration;
            found = true;
            break;
        }
    }
    if !found && !project.overlays.iter().any(|t| t.clips.iter().any(|c| c.id == id)) {
        bail!("no clip {id:?}");
    }

    let new_id = {
        let mut n = 2;
        let mut candidate = format!("{id}-{n}");
        while find_clip(project, &candidate).is_some() {
            n += 1;
            candidate = format!("{id}-{n}");
        }
        candidate
    };

    let clip = find_clip_mut(project, id).expect("checked above");
    let local = abs_time - container_offset;
    let effective = if clip.duration > 0.0 {
        clip.duration
    } else {
        (container_span - clip.start).max(0.0)
    };
    let split_rel = local - clip.start;
    if split_rel <= 0.05 || split_rel >= effective - 0.05 {
        bail!("split point must fall inside the clip");
    }

    let mut right = clip.clone();
    right.id = new_id.clone();
    right.start = clip.start + split_rel;
    right.duration = effective - split_rel;
    clip.duration = split_rel;

    // Media clips: the right half starts later in the source file.
    match &mut right.element {
        Element::Video { offset, .. } | Element::Audio { offset, .. } => *offset += split_rel,
        _ => {}
    }

    // Animations: keep each half's share, clipped/shifted; a straddling
    // tween is split at the boundary with the interpolated value.
    let value_at = |a: &Anim, t: f64| -> f64 {
        if a.end <= a.start {
            return a.to;
        }
        let p = ((t - a.start) / (a.end - a.start)).clamp(0.0, 1.0);
        a.from + (a.to - a.from) * ease(a.easing, p)
    };
    let left_anims: Vec<Anim> = clip
        .animations
        .iter()
        .filter_map(|a| {
            if !a.keyframes.is_empty() {
                let kfs: Vec<Keyframe> =
                    a.keyframes.iter().filter(|k| k.t <= split_rel).cloned().collect();
                if kfs.len() < 2 {
                    return None;
                }
                let mut a = a.clone();
                a.keyframes = kfs;
                return Some(a);
            }
            if a.start >= split_rel {
                return None;
            }
            let mut a = a.clone();
            if a.end > split_rel {
                a.to = value_at(&a, split_rel);
                a.end = split_rel;
            }
            Some(a)
        })
        .collect();
    let right_anims: Vec<Anim> = right
        .animations
        .iter()
        .filter_map(|a| {
            if !a.keyframes.is_empty() {
                let kfs: Vec<Keyframe> = a
                    .keyframes
                    .iter()
                    .filter(|k| k.t >= split_rel)
                    .map(|k| Keyframe { t: k.t - split_rel, ..k.clone() })
                    .collect();
                if kfs.len() < 2 {
                    return None;
                }
                let mut a = a.clone();
                a.keyframes = kfs;
                return Some(a);
            }
            if a.end <= split_rel {
                return None;
            }
            let mut a = a.clone();
            if a.start < split_rel {
                a.from = value_at(&a, split_rel);
                a.start = 0.0;
            } else {
                a.start -= split_rel;
            }
            a.end -= split_rel;
            Some(a)
        })
        .collect();
    clip.animations = left_anims;
    right.animations = right_anims;

    // Insert the right half next to the original.
    for scene in &mut project.scenes {
        if let Some(pos) = scene.layers.iter().position(|c| c.id == id) {
            scene.layers.insert(pos + 1, right);
            return Ok(new_id);
        }
    }
    for track in &mut project.overlays {
        if let Some(pos) = track.clips.iter().position(|c| c.id == id) {
            track.clips.insert(pos + 1, right);
            return Ok(new_id);
        }
    }
    unreachable!("clip location verified above");
}
