//! In-process TypeScript scripting (feature = "scripting").
//!
//! Scripts receive the current document and return an edited one:
//!   export function edit(project: Project): Project
//! Types for authors: schema/dualcut.d.ts.

use crate::document::Project;
use anyhow::{Context, Result};
use rustyscript::{json_args, Module, Runtime, RuntimeOptions};

pub fn run_script(source: &str, project: &Project) -> Result<Project> {
    let mut runtime = Runtime::new(RuntimeOptions::default())?;
    let module = Module::new("agent-script.ts", source);
    let handle = runtime.load_module(&module)?;
    let value: serde_json::Value = runtime.call_function(
        Some(&handle),
        "edit",
        json_args!(serde_json::to_value(project)?),
    )?;
    let edited: Project =
        serde_json::from_value(value).context("script returned invalid document")?;
    edited.validate()?;
    Ok(edited)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Clip, Element, Meta, Scene};

    fn small_project() -> Project {
        Project {
            meta: Meta { title: "before".into(), width: 320, height: 180, fps: 25 },
            library: Vec::new(),
            defs: Default::default(),
            scenes: vec![Scene {
                id: "s1".into(),
                name: String::new(),
                duration: 1.0,
                transition: None,
                layers: vec![Clip {
                    id: "c1".into(),
                    start: 0.0,
                    duration: 1.0,
                    element: Element::Test {},
                    transform: Default::default(),
                    animations: Vec::new(),
                    effects: Vec::new(),
                }],
            }],
            overlays: Vec::new(),
            scene_lanes: Vec::new(),
        }
    }

    #[test]
    fn script_can_edit_project_metadata() {
        let project = small_project();
        let script = "export function edit(p) { p.meta.title = 'after'; return p; }";
        let edited = run_script(script, &project).expect("script should run");
        assert_eq!(edited.meta.title, "after");
        // Untouched fields should round-trip through the JS boundary intact.
        assert_eq!(edited.scenes.len(), project.scenes.len());
    }

    #[test]
    fn script_can_add_a_clip() {
        let project = small_project();
        let script = "export function edit(p) { \
            p.scenes[0].layers.push({id: 'c2', start: 0, duration: 1, type: 'test'}); \
            return p; \
        }";
        let edited = run_script(script, &project).expect("script should run");
        assert_eq!(edited.scenes[0].layers.len(), 2);
    }

    #[test]
    fn script_returning_an_invalid_document_is_rejected() {
        let project = small_project();
        // Duplicate clip ids: valid JS, but Project::validate() should
        // reject the *document*, not just fail to parse.
        let script = "export function edit(p) { \
            p.scenes[0].layers.push({id: 'c1', start: 0, duration: 1, type: 'test'}); \
            return p; \
        }";
        let result = run_script(script, &project);
        assert!(result.is_err(), "duplicate clip ids should fail validation");
    }

    #[test]
    fn script_with_a_syntax_error_fails_cleanly() {
        let project = small_project();
        let result = run_script("this is not valid typescript {{{", &project);
        assert!(result.is_err());
    }

    #[test]
    fn script_missing_the_edit_export_fails_cleanly() {
        let project = small_project();
        let result = run_script("export function notEdit(p) { return p; }", &project);
        assert!(result.is_err());
    }

    #[test]
    fn script_returning_wrong_shape_fails_cleanly() {
        let project = small_project();
        let result = run_script("export function edit(p) { return { nope: true }; }", &project);
        assert!(result.is_err());
    }
}
