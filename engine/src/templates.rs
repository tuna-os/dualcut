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
    new_project_sized(title, 1920, 1080)
}

/// Same as [`new_project`] but with a caller-chosen canvas size -- e.g.
/// 1080x1920 for vertical/portrait export (#48), which pairs with the
/// `vertical-center-crop` / `vertical-top-bottom-split` starter defs.
pub fn new_project_sized(title: &str, width: i32, height: i32) -> Project {
    Project {
        meta: Meta { title: title.into(), width, height, fps: 30 },
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
        assert!(defs.contains_key("vertical-center-crop"));
        assert!(defs.contains_key("vertical-top-bottom-split"));
        let p = new_project("Test");
        assert!(p.validate().is_ok());
        assert_eq!(p.duration(), 5.0);
        // Blank on purpose: no defs, no clips (#21).
        assert!(p.defs.is_empty());
        assert!(p.scenes[0].layers.is_empty());
    }

    #[test]
    fn vertical_project_validates_with_split_template_instantiated() {
        use crate::document::{Clip, Element};
        let mut p = new_project_sized("Vertical Test", 1080, 1920);
        assert_eq!(p.meta.width, 1080);
        assert_eq!(p.meta.height, 1920);
        p.defs.insert("vertical-top-bottom-split".into(), starter_defs()["vertical-top-bottom-split"].clone());
        p.scenes[0].layers.push(Clip {
            id: "split".into(),
            start: 0.0,
            duration: 0.0,
            element: Element::CompRef {
                r#ref: "vertical-top-bottom-split".into(),
                args: [
                    ("top_clip".into(), "assets/a.mp4".into()),
                    ("bottom_clip".into(), "assets/b.mp4".into()),
                ]
                .into_iter()
                .collect(),
            },
            transform: Default::default(),
            animations: vec![],
            effects: vec![],
        });
        assert!(p.validate().is_ok());
    }
}
