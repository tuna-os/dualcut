//! Edit history types and snapshot/diff utilities.
//!
//! Extracted from `preview.rs` (dualcut#78): `EditSource` tracks whether a
//! change originated from the GUI, an agent HTTP request, or external file edit;
//! `HistoryEntry` captures prior document snapshots; `diff_summary` provides
//! human-readable summaries; and `take_agent_marker` detects agent edits.

use super::*;

/// Who made an edit-history entry (surfaced in the History panel so it's
/// clear which changes came from the GUI vs. an agent driving the HTTP API
/// vs. someone editing the project file directly).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum EditSource {
    Gui,
    Agent,
    ExternalFile,
}

#[derive(Clone)]
pub(crate) struct HistoryEntry {
    /// Document JSON *before* this edit -- what undo, or jumping to this
    /// entry in the History panel, restores.
    pub(crate) snapshot: String,
    pub(crate) source: EditSource,
    pub(crate) summary: String,
    pub(crate) at: SystemTime,
}

/// Coarse, cheap description of what changed between two document states,
/// for the Edit History panel. Not a real diff (no per-field tracking of
/// which clip/effect changed) -- counts clips/scenes/tracks/defs and falls
/// back to a generic label, which is honest about its own resolution
/// without threading a label through every one of commit_document's many
/// call sites.
pub(crate) fn diff_summary(prev: &Project, new: &Project) -> String {
    let clip_count = |p: &Project| -> usize {
        p.scenes.iter().map(|s| s.layers.len()).sum::<usize>()
            + p.overlays.iter().map(|t| t.clips.len()).sum::<usize>()
    };
    let (pc, nc) = (clip_count(prev), clip_count(new));
    if nc > pc {
        return format!("Added {} clip{}", nc - pc, if nc - pc == 1 { "" } else { "s" });
    }
    if nc < pc {
        return format!("Removed {} clip{}", pc - nc, if pc - nc == 1 { "" } else { "s" });
    }
    if new.scenes.len() != prev.scenes.len() {
        return if new.scenes.len() > prev.scenes.len() { "Added scene" } else { "Removed scene" }
            .into();
    }
    if new.overlays.len() != prev.overlays.len() {
        return if new.overlays.len() > prev.overlays.len() {
            "Added overlay track"
        } else {
            "Removed overlay track"
        }
        .into();
    }
    if new.defs.len() != prev.defs.len() {
        return "Edited templates".into();
    }
    if (prev.duration() - new.duration()).abs() > 0.001 {
        return "Changed timing".into();
    }
    "Edited project".into()
}

/// Consume the agent-edit marker (written by `api::serve_file_api` just
/// before the file write that's about to trigger this reload) if it's
/// fresh, tagging the resulting history entry as agent-sourced with the
/// request's own summary. Stale/missing marker => a human edited the file
/// directly, not through the HTTP API.
pub(crate) fn take_agent_marker(project_path: &std::path::Path) -> Option<(EditSource, String)> {
    let cache = project_path.parent()?.join(".dualcut-cache");
    let marker = cache.join("agent-edit.json");
    let raw = std::fs::read_to_string(&marker).ok()?;
    let _ = std::fs::remove_file(&marker);
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let at_ms = v["at_unix_ms"].as_u64()?;
    let now_ms =
        SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64;
    if now_ms.saturating_sub(at_ms) > 5000 {
        return None;
    }
    Some((EditSource::Agent, v["summary"].as_str().unwrap_or("Agent edit").to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_summary_detects_added_and_removed_clips() {
        let prev = dualcut_engine::templates::new_project("t");
        let mut added = prev.clone();
        added.scenes[0].layers.push(document::Clip {
            id: "c1".into(),
            start: 0.0,
            duration: 1.0,
            element: document::Element::Test {},
            transform: Default::default(),
            animations: Vec::new(),
            effects: Vec::new(),
        });
        assert_eq!(diff_summary(&prev, &added), "Added 1 clip");
        assert_eq!(diff_summary(&added, &prev), "Removed 1 clip");
    }

    #[test]
    fn diff_summary_detects_scene_changes() {
        let prev = dualcut_engine::templates::new_project("t");
        let mut next = prev.clone();
        next.scenes.push(document::Scene {
            id: "s2".into(),
            name: "Scene 2".into(),
            duration: 5.0,
            transition: None,
            layers: Vec::new(),
        });
        assert_eq!(diff_summary(&prev, &next), "Added scene");
        assert_eq!(diff_summary(&next, &prev), "Removed scene");
    }

    #[test]
    fn diff_summary_detects_timing_changes() {
        let prev = dualcut_engine::templates::new_project("t");
        let mut next = prev.clone();
        next.scenes[0].duration = 10.0;
        assert_eq!(diff_summary(&prev, &next), "Changed timing");
    }
}
