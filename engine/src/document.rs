//! Document model v2 (see ROADMAP.md): scenes are the sequential narrative
//! spine, overlays span scene cuts, defs are reusable parameterised
//! compositions. This document — not GES — is what the UI, in-app scripts,
//! and external agents edit; `crate::mapping` compiles it into a GES
//! timeline.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub meta: Meta,
    /// Reusable compositions, instantiated by `Layer::CompRef`.
    #[serde(default)]
    pub defs: HashMap<String, CompDef>,
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
    /// Layers composited top-first (index 0 renders on top).
    #[serde(default)]
    pub layers: Vec<Clip>,
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
        args: HashMap<String, String>,
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

    /// Total duration in seconds (scenes are sequential).
    pub fn duration(&self) -> f64 {
        self.scenes.iter().map(|s| s.duration).sum()
    }

    /// Absolute start time of a scene by index.
    pub fn scene_offset(&self, index: usize) -> f64 {
        self.scenes[..index].iter().map(|s| s.duration).sum()
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
    fn validate(&self, defs: &HashMap<String, CompDef>) -> Result<()> {
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
