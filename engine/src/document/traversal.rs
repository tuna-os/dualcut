//! Generic traversal over clips stored in scenes and overlay tracks.

use super::{Clip, Project};

/// Find a clip anywhere in the project by id.
pub fn find_clip<'a>(project: &'a Project, id: &str) -> Option<&'a Clip> {
    project
        .scenes
        .iter()
        .flat_map(|scene| scene.layers.iter())
        .chain(
            project
                .overlays
                .iter()
                .flat_map(|track| track.clips.iter()),
        )
        .find(|clip| clip.id == id)
}

/// Find a mutable clip anywhere in the project by id.
pub fn find_clip_mut<'a>(project: &'a mut Project, id: &str) -> Option<&'a mut Clip> {
    if let Some(clip) = project
        .scenes
        .iter_mut()
        .flat_map(|scene| scene.layers.iter_mut())
        .find(|clip| clip.id == id)
    {
        return Some(clip);
    }

    project
        .overlays
        .iter_mut()
        .flat_map(|track| track.clips.iter_mut())
        .find(|clip| clip.id == id)
}

/// Remove every clip with the given id from scenes and overlay tracks.
pub fn remove_clip(project: &mut Project, id: &str) {
    for scene in &mut project.scenes {
        scene.layers.retain(|clip| clip.id != id);
    }
    for track in &mut project.overlays {
        track.clips.retain(|clip| clip.id != id);
    }
}
