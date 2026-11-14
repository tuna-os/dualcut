use super::super::schema::*;
use super::super::{find_clip, find_clip_mut};

/// The classic editor op: silence a video clip's embedded audio and give it
/// an independent audio clip on the "detached-audio" overlay track.
/// Returns the new clip's id.
pub fn detach_audio(project: &mut Project, id: &str) -> Option<String> {
    let (src, offset, volume, start, duration) = {
        let clip = find_clip(project, id)?;
        match &clip.element {
            Element::Video { src, offset, volume, .. } => {
                (src.clone(), *offset, *volume, clip.start, clip.duration)
            }
            _ => return None,
        }
    };
    // Scene clips are scene-relative; the detached audio lives on an
    // overlay track, which is absolutely timed.
    let abs_start = project
        .scenes
        .iter()
        .enumerate()
        .find(|(_, s)| s.layers.iter().any(|c| c.id == id))
        .map(|(i, _)| project.scene_offset(i) + start)
        .unwrap_or(start);
    if let Some(clip) = find_clip_mut(project, id)
        && let Element::Video { volume, .. } = &mut clip.element {
            *volume = 0.0;
        }
    let new_id = format!("{id}-audio");
    let audio = Clip {
        id: new_id.clone(),
        start: abs_start,
        duration,
        element: Element::Audio { src, offset, volume, rate: 1.0 },
        transform: Default::default(),
        animations: Vec::new(),
        effects: Vec::new(),
    };
    if let Some(track) = project.overlays.iter_mut().find(|t| t.id == "detached-audio") {
        track.clips.push(audio);
    } else {
        project.overlays.push(OverlayTrack {
            muted: false,
            hidden: false,
            locked: false,
            id: "detached-audio".into(),
            name: "Detached audio".into(),
            clips: vec![audio],
        });
    }
    Some(new_id)
}
