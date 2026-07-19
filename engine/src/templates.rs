//! Built-in template library: starter defs merged into new projects.

use crate::document::{CompDef, Meta, Project, Scene};
use std::collections::BTreeMap;

const STARTER: &str = include_str!("../templates/starter.json");

pub fn starter_defs() -> BTreeMap<String, CompDef> {
    serde_json::from_str(STARTER).expect("starter templates parse")
}

/// Scaffold a new project: 1080p30, one title-card scene, starter defs.
/// A genuinely blank project: resolution + fps and nothing else (#21 QA
/// feedback — starter defs used to be baked into every new project's
/// `defs`, so they lingered in the Code tab even after the clip that
/// used them was deleted). Starter templates are still one click away
/// in the Templates tab (`starter_defs()`); a def only enters the
/// document when the user actually inserts it.
pub fn new_project(title: &str) -> Project {
    Project {
        meta: Meta { title: title.into(), width: 1920, height: 1080, fps: 30 },
        library: Vec::new(),
        defs: BTreeMap::new(),
        scenes: vec![Scene {
            id: "scene-1".into(),
            name: String::new(),
            duration: 5.0,
            transition: None,
            layers: Vec::new(),
        }],
        overlays: Vec::new(),
        scene_lanes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_defs_parse_and_new_project_validates() {
        let defs = starter_defs();
        assert!(defs.contains_key("lower-third"));
        assert!(defs.contains_key("title-card"));
        assert!(defs.contains_key("caption"));
        let p = new_project("Test");
        assert!(p.validate().is_ok());
        assert_eq!(p.duration(), 5.0);
        // Blank on purpose: no defs, no clips (#21).
        assert!(p.defs.is_empty());
        assert!(p.scenes[0].layers.is_empty());
    }
}
