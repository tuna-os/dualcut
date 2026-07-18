//! Document model v2 (see ROADMAP.md): scenes are the sequential narrative
//! spine, overlays span scene cuts, defs are reusable parameterised
//! compositions. This document — not GES — is what the UI, in-app scripts,
//! and external agents edit; `crate::mapping` compiles it into a GES
//! timeline.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub meta: Meta,
    /// Reusable compositions, instantiated by `Layer::CompRef`.
    #[serde(default)]
    pub defs: BTreeMap<String, CompDef>,
    /// Sequential scenes; order defines time. No gaps.
    pub scenes: Vec<Scene>,
    /// Composition-spanning tracks (subtitles, music, watermarks) with
    /// absolute timing; they freely cross scene cuts.
    #[serde(default)]
    pub overlays: Vec<OverlayTrack>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub title: String,
    pub width: i32,
    pub height: i32,
    pub fps: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompDef {
    /// Parameter names; occurrences of `{name}` in the def's string fields
    /// are substituted at instantiation.
    #[serde(default)]
    pub params: Vec<String>,
    pub layers: Vec<Clip>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Seconds.
    pub duration: f64,
    /// Transition from the previous scene into this one (ignored on the
    /// first scene). None = hard cut.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<Transition>,
    /// Layers composited top-first (index 0 renders on top).
    #[serde(default)]
    pub layers: Vec<Clip>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transition {
    #[serde(default)]
    pub kind: TransitionKind,
    /// Seconds of overlap with the previous scene.
    pub duration: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransitionKind {
    #[default]
    Crossfade,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayTrack {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub clips: Vec<Clip>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    pub id: String,
    /// Seconds. Relative to the scene for scene layers; absolute for
    /// overlay clips and def layers (relative to instantiation start).
    #[serde(default)]
    pub start: f64,
    /// Seconds. Defaults to the remainder of the scene/def when 0.
    #[serde(default)]
    pub duration: f64,
    #[serde(flatten)]
    pub element: Element,
    #[serde(default)]
    pub transform: Transform,
    #[serde(default)]
    pub animations: Vec<Anim>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Element {
    Text {
        text: String,
        #[serde(default = "default_font")]
        font: String,
        #[serde(default = "default_color")]
        color: String,
    },
    Video {
        src: String,
        /// Seek into the source, seconds.
        #[serde(default)]
        offset: f64,
        #[serde(default = "default_volume")]
        volume: f64,
    },
    Audio {
        src: String,
        #[serde(default)]
        offset: f64,
        #[serde(default = "default_volume")]
        volume: f64,
    },
    Image {
        src: String,
    },
    /// GPU vector shapes; rendered by the Vello compositor from M3.
    /// The M1 GES mapping skips them with a warning.
    Shape {
        shape: ShapeKind,
        #[serde(default = "default_color")]
        fill: String,
    },
    /// Instantiate a reusable composition from `defs`.
    CompRef {
        r#ref: String,
        #[serde(default)]
        args: BTreeMap<String, String>,
    },
    /// Built-in test pattern (useful for scripts/tests).
    Test {},
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShapeKind {
    Rect,
    Circle,
    Ellipse,
    Star,
    Polygon,
    Line,
    Arrow,
}

/// Position/size in composition pixels; opacity 0..1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transform {
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    /// 0 = natural / full size.
    #[serde(default)]
    pub width: f64,
    #[serde(default)]
    pub height: f64,
    #[serde(default = "default_opacity")]
    pub opacity: f64,
}

impl Default for Transform {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0, width: 0.0, height: 0.0, opacity: 1.0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Anim {
    pub property: AnimProperty,
    pub from: f64,
    pub to: f64,
    /// Seconds relative to the clip's own start.
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub easing: Easing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnimProperty {
    X,
    Y,
    Opacity,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Easing {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}

impl Project {
    pub fn from_json(json: &str) -> Result<Self> {
        let project: Project = serde_json::from_str(json)?;
        project.validate()?;
        Ok(project)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("project serializes")
    }

    /// Total duration in seconds. Scenes are sequential; a transition
    /// overlaps a scene with its predecessor, shortening the total.
    pub fn duration(&self) -> f64 {
        match self.scenes.len() {
            0 => 0.0,
            n => self.scene_offset(n - 1) + self.scenes[n - 1].duration,
        }
    }

    /// Absolute start time of a scene by index (transition overlaps pull
    /// scenes earlier).
    pub fn scene_offset(&self, index: usize) -> f64 {
        let mut offset = 0.0;
        for i in 0..index {
            offset += self.scenes[i].duration;
            offset -= self.scenes[i + 1].transition.as_ref().map_or(0.0, |t| t.duration);
        }
        offset.max(0.0)
    }

    pub fn validate(&self) -> Result<()> {
        if self.meta.width <= 0 || self.meta.height <= 0 || self.meta.fps <= 0 {
            bail!("meta.width/height/fps must be positive");
        }
        let mut ids = std::collections::HashSet::new();
        let mut check = |id: &str, what: &str| -> Result<()> {
            if id.is_empty() {
                bail!("every {what} needs a non-empty id");
            }
            if !ids.insert(id.to_string()) {
                bail!("duplicate id {id:?}");
            }
            Ok(())
        };
        for scene in &self.scenes {
            check(&scene.id, "scene")?;
            if scene.duration <= 0.0 {
                bail!("scene {:?}: duration must be > 0", scene.id);
            }
            if let Some(t) = &scene.transition {
                if t.duration <= 0.0 || t.duration >= scene.duration {
                    bail!("scene {:?}: transition duration must be > 0 and < scene duration", scene.id);
                }
            }
            for clip in &scene.layers {
                check(&clip.id, "clip")?;
                clip.validate(&self.defs)?;
            }
        }
        for track in &self.overlays {
            check(&track.id, "overlay track")?;
            for clip in &track.clips {
                check(&clip.id, "clip")?;
                clip.validate(&self.defs)?;
            }
        }
        for (name, def) in &self.defs {
            for clip in &def.layers {
                if let Element::CompRef { r#ref, .. } = &clip.element {
                    bail!("def {name:?} references def {ref:?}: defs cannot nest (yet)");
                }
                clip.validate(&self.defs)?;
            }
        }
        Ok(())
    }
}

impl Clip {
    fn validate(&self, defs: &BTreeMap<String, CompDef>) -> Result<()> {
        if self.start < 0.0 || self.duration < 0.0 {
            bail!("clip {:?}: start/duration must be >= 0", self.id);
        }
        for anim in &self.animations {
            if anim.end <= anim.start {
                bail!("clip {:?}: animation end must be > start", self.id);
            }
        }
        if let Element::CompRef { r#ref, .. } = &self.element {
            if !defs.contains_key(r#ref) {
                bail!("clip {:?} references unknown def {ref:?}", self.id);
            }
        }
        Ok(())
    }
}

fn default_font() -> String {
    "Sans Bold 32".into()
}
fn default_color() -> String {
    "#ffffff".into()
}
fn default_volume() -> f64 {
    1.0
}
fn default_opacity() -> f64 {
    1.0
}

/// Parse "#rrggbb" / "#aarrggbb" into ARGB u32 (as GES title colors expect).
pub fn parse_color(color: &str) -> u32 {
    let hex = color.trim_start_matches('#');
    let value = u32::from_str_radix(hex, 16).unwrap_or(0x00ff_ffff);
    if hex.len() <= 6 {
        0xff00_0000 | value
    } else {
        value
    }
}

/// Find a clip anywhere in the project by id.
pub fn find_clip<'a>(project: &'a Project, id: &str) -> Option<&'a Clip> {
    project
        .scenes
        .iter()
        .flat_map(|s| s.layers.iter())
        .chain(project.overlays.iter().flat_map(|t| t.clips.iter()))
        .find(|c| c.id == id)
}

pub fn find_clip_mut<'a>(project: &'a mut Project, id: &str) -> Option<&'a mut Clip> {
    let in_scene = project
        .scenes
        .iter_mut()
        .flat_map(|s| s.layers.iter_mut())
        .find(|c| c.id == id);
    if in_scene.is_some() {
        return in_scene;
    }
    project
        .overlays
        .iter_mut()
        .flat_map(|t| t.clips.iter_mut())
        .find(|c| c.id == id)
}

pub fn remove_clip(project: &mut Project, id: &str) {
    for scene in &mut project.scenes {
        scene.layers.retain(|c| c.id != id);
    }
    for track in &mut project.overlays {
        track.clips.retain(|c| c.id != id);
    }
}

/// The classic editor op: silence a video clip's embedded audio and give it
/// an independent audio clip on the "detached-audio" overlay track.
/// Returns the new clip's id.
pub fn detach_audio(project: &mut Project, id: &str) -> Option<String> {
    let (src, offset, volume, start, duration) = {
        let clip = find_clip(project, id)?;
        match &clip.element {
            Element::Video { src, offset, volume } => {
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
    if let Some(clip) = find_clip_mut(project, id) {
        if let Element::Video { volume, .. } = &mut clip.element {
            *volume = 0.0;
        }
    }
    let new_id = format!("{id}-audio");
    let audio = Clip {
        id: new_id.clone(),
        start: abs_start,
        duration,
        element: Element::Audio { src, offset, volume },
        transform: Default::default(),
        animations: Vec::new(),
    };
    if let Some(track) = project.overlays.iter_mut().find(|t| t.id == "detached-audio") {
        track.clips.push(audio);
    } else {
        project.overlays.push(OverlayTrack {
            id: "detached-audio".into(),
            name: "Detached audio".into(),
            clips: vec![audio],
        });
    }
    Some(new_id)
}

/// Save the given clips as a reusable def named `name` (clips stay in
/// place; the def gets copies with starts normalized so the earliest is 0).
pub fn save_as_def(project: &mut Project, ids: &[String], name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("template name must not be empty");
    }
    if project.defs.contains_key(name) {
        bail!("a def named {name:?} already exists");
    }
    let mut clips: Vec<Clip> = ids
        .iter()
        .filter_map(|id| find_clip(project, id).cloned())
        .collect();
    if clips.is_empty() {
        bail!("no clips selected");
    }
    if clips.iter().any(|c| matches!(c.element, Element::CompRef { .. })) {
        bail!("templates cannot contain template instances (defs cannot nest)");
    }
    let min_start = clips.iter().map(|c| c.start).fold(f64::INFINITY, f64::min);
    for clip in &mut clips {
        clip.start -= min_start;
    }
    project.defs.insert(name.to_string(), CompDef { params: Vec::new(), layers: clips });
    project.validate()
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> Project {
        Project::from_json(include_str!("../examples/demo-project.json")).unwrap()
    }

    #[test]
    fn parses_and_validates_demo() {
        let p = demo();
        assert_eq!(p.scenes.len(), 2);
        assert_eq!(p.duration(), 7.0);
        assert_eq!(p.scene_offset(1), 3.0);
    }

    #[test]
    fn roundtrips_json() {
        let p = demo();
        let p2 = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(p.to_json(), p2.to_json());
    }

    #[test]
    fn rejects_duplicate_ids() {
        let mut p = demo();
        let clip = p.scenes[0].layers[0].clone();
        p.scenes[0].layers.push(clip);
        assert!(p.validate().is_err());
    }

    #[test]
    fn rejects_unknown_def_ref() {
        let mut p = demo();
        if let Some(c) = find_clip_mut(&mut p, "media-lower-third") {
            c.element = Element::CompRef { r#ref: "nope".into(), args: Default::default() };
        }
        assert!(p.validate().is_err());
    }

    #[test]
    fn detach_audio_silences_video_and_adds_overlay_clip() {
        let mut p = demo();
        let new_id = detach_audio(&mut p, "media-ball").expect("detaches");
        // Video muted.
        match &find_clip(&p, "media-ball").unwrap().element {
            Element::Video { volume, .. } => assert_eq!(*volume, 0.0),
            _ => panic!("still a video clip"),
        }
        // Audio clip exists on the detached-audio overlay with absolute time
        // (scene-media starts at 3.0, clip start 0 -> abs 3.0).
        let audio = find_clip(&p, &new_id).unwrap();
        assert_eq!(audio.start, 3.0);
        assert!(matches!(audio.element, Element::Audio { .. }));
        assert!(p.validate().is_ok());
    }

    #[test]
    fn remove_clip_removes_everywhere() {
        let mut p = demo();
        remove_clip(&mut p, "wm-text");
        assert!(find_clip(&p, "wm-text").is_none());
        assert!(p.validate().is_ok());
    }

    #[test]
    fn transitions_overlap_scene_offsets() {
        let mut p = demo();
        p.scenes[1].transition = Some(Transition { kind: TransitionKind::Crossfade, duration: 1.0 });
        assert_eq!(p.scene_offset(1), 2.0); // pulled 1s into scene 1
        assert_eq!(p.duration(), 6.0); // 3 + 4 - 1
        assert!(p.validate().is_ok());
        p.scenes[1].transition = Some(Transition { kind: TransitionKind::Crossfade, duration: 5.0 });
        assert!(p.validate().is_err()); // longer than the scene itself
    }

    #[test]
    fn save_as_def_normalizes_and_validates() {
        let mut p = demo();
        save_as_def(
            &mut p,
            &["intro-title".into(), "wm-text".into()],
            "my-template",
        )
        .unwrap();
        let def = &p.defs["my-template"];
        assert_eq!(def.layers.len(), 2);
        // earliest start normalized to 0 (intro-title 0.4, wm-text 0.5)
        assert_eq!(def.layers.iter().map(|c| c.start).fold(f64::INFINITY, f64::min), 0.0);
        assert!(p.validate().is_ok());
        // duplicate name refused
        assert!(save_as_def(&mut p, &["intro-bg".into()], "my-template").is_err());
        // compref content refused
        assert!(save_as_def(&mut p, &["media-lower-third".into()], "t2").is_err());
    }

    #[test]
    fn move_clip_between_lanes() {
        let mut p = demo();
        // wm-text lives on the overlay (lane 2: 2 scene slots + overlay 0).
        // Move it into scene layer slot 0 at abs 4.0 (scene-media, rel 1.0).
        move_clip_to_lane(&mut p, "wm-text", 0, 4.0).unwrap();
        let scene = p.scenes.iter().find(|s| s.id == "scene-media").unwrap();
        let clip = scene.layers.iter().find(|c| c.id == "wm-text").unwrap();
        assert!((clip.start - 1.0).abs() < 1e-9);
        assert!(p.overlays.iter().all(|t| t.clips.iter().all(|c| c.id != "wm-text")));

        // And back out to the overlay track, absolute time preserved.
        let max_layers = p.scenes.iter().map(|s| s.layers.len()).max().unwrap();
        move_clip_to_lane(&mut p, "wm-text", max_layers, 2.5).unwrap();
        let clip = p.overlays[0].clips.iter().find(|c| c.id == "wm-text").unwrap();
        assert_eq!(clip.start, 2.5);
        assert!(p.validate().is_ok());
    }

    #[test]
    fn parse_color_handles_rgb_and_argb() {
        assert_eq!(parse_color("#ffffff"), 0xffffffff);
        assert_eq!(parse_color("#80ffffff"), 0x80ffffff);
    }
}
