//! Built-in template library: starter defs merged into new projects.

use crate::document::{CompDef, Meta, Project, Scene};
use std::collections::BTreeMap;

const STARTER: &str = include_str!("../templates/starter.json");

pub fn starter_defs() -> BTreeMap<String, CompDef> {
    serde_json::from_str(STARTER).expect("starter templates parse")
}

/// Scaffold a new project: 1080p30, one title-card scene, starter defs.
pub fn new_project(title: &str) -> Project {
    let mut args = BTreeMap::new();
    args.insert("title".to_string(), title.to_string());
    args.insert("subtitle".to_string(), "Made with dualcut".to_string());
    Project {
        meta: Meta { title: title.into(), width: 1920, height: 1080, fps: 30 },
        defs: starter_defs(),
        scenes: vec![Scene {
            id: "scene-1".into(),
            name: "Title".into(),
            duration: 4.0,
            transition: None,
            layers: vec![crate::document::Clip {
                id: "opening-card".into(),
                start: 0.0,
                duration: 0.0,
                element: crate::document::Element::CompRef { r#ref: "title-card".into(), args },
                transform: Default::default(),
                animations: Vec::new(),
                effects: Vec::new(),
            }],
        }],
        overlays: Vec::new(),
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
        assert_eq!(p.duration(), 4.0);
    }
}
