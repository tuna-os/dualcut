use super::super::schema::*;
use anyhow::{bail, Result};

/// A timeline lane: scene layer slots first (0..max layers), then overlay
/// tracks. Mirrors the editor strip's row order.
pub fn lane_count(project: &Project) -> usize {
    let max_layers = project.scenes.iter().map(|s| s.layers.len()).max().unwrap_or(0);
    max_layers + project.overlays.len()
}

/// Move a clip to another lane, keeping its on-screen absolute time.
/// Scene-layer targets re-anchor scene-relative (the scene containing
/// `abs_start`); overlay targets store absolute time.
pub fn move_clip_to_lane(
    project: &mut Project,
    id: &str,
    lane: usize,
    abs_start: f64,
) -> Result<()> {
    let max_layers = project.scenes.iter().map(|s| s.layers.len()).max().unwrap_or(0);
    if lane >= lane_count(project) {
        bail!("lane {lane} out of range");
    }
    // Detach the clip from wherever it lives.
    let mut found = None;
    for scene in &mut project.scenes {
        if let Some(pos) = scene.layers.iter().position(|c| c.id == id) {
            found = Some(scene.layers.remove(pos));
            break;
        }
    }
    if found.is_none() {
        for track in &mut project.overlays {
            if let Some(pos) = track.clips.iter().position(|c| c.id == id) {
                found = Some(track.clips.remove(pos));
                break;
            }
        }
    }
    let Some(mut clip) = found else { bail!("clip {id:?} not found") };

    if lane < max_layers {
        // Scene layer slot: find the scene containing abs_start.
        let index = (0..project.scenes.len())
            .rev()
            .find(|&i| abs_start >= project.scene_offset(i))
            .unwrap_or(0);
        let offset = project.scene_offset(index);
        let scene = &mut project.scenes[index];
        clip.start = (abs_start - offset).clamp(0.0, (scene.duration - 0.1).max(0.0));
        while scene.layers.len() < lane {
            // Pad missing slots so the clip lands on the requested lane.
            scene.layers.push(Clip {
                id: format!("{id}-slotpad-{}", scene.layers.len()),
                start: 0.0,
                duration: 0.1,
                element: Element::Test {},
                transform: Transform { opacity: 0.0, ..Default::default() },
                animations: Vec::new(),
                effects: Vec::new(),
            });
        }
        scene.layers.insert(lane.min(scene.layers.len()), clip);
    } else {
        let track = &mut project.overlays[lane - max_layers];
        clip.start = abs_start.max(0.0);
        track.clips.push(clip);
    }
    project.validate()
}
