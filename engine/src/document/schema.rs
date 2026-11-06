//! Serde data model and wire defaults. Domain behavior lives in sibling modules.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod animation;
mod effects;
pub use animation::*;
pub use effects::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub meta: Meta,
    /// Media files the user imported into the project's library. Paths
    /// are relative to the project file (or absolute).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub library: Vec<String>,
    /// Reusable compositions, instantiated by `Layer::CompRef`.
    #[serde(default)]
    pub defs: BTreeMap<String, CompDef>,
    /// Sequential scenes; order defines time. No gaps.
    pub scenes: Vec<Scene>,
    /// Composition-spanning tracks (subtitles, music, watermarks) with
    /// absolute timing; they freely cross scene cuts.
    #[serde(default)]
    pub overlays: Vec<OverlayTrack>,
    /// Per-layer-index lock/hide/mute for scene layers (#21). Scene
    /// layers are positional (a vec index, not an identity), so this is
    /// indexed the same way: `scene_lanes[i]` applies to layer `i` of
    /// every scene. Missing/short entries default to unlocked/visible.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scene_lanes: Vec<LaneMeta>,
}

/// Lock/hide/mute state for one timeline lane (#21). Hide/mute are
/// applied non-destructively at compile time (opacity/volume to 0);
/// lock only affects the GUI (drag is disabled).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LaneMeta {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub muted: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
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
    #[serde(rename = "wipe-lr")]
    WipeLr,
    #[serde(rename = "wipe-tb")]
    WipeTb,
    #[serde(rename = "box-wipe")]
    BoxWipe,
    Iris,
    Clock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayTrack {
    pub id: String,
    /// Mute the track's audio without touching its clips (#31).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub muted: bool,
    /// Hide the track's video without touching its clips (#31).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// Disable dragging clips on this track in the GUI (#21).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
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
    /// Video effects applied in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
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
        /// Horizontal alignment; overrides transform.x positioning.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        align: Option<TextAlign>,
        /// Outline color (#rrggbb / #aarrggbb).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outline: Option<String>,
        /// Draw a drop shadow behind the text.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        shadow: bool,
    },
    Video {
        src: String,
        /// Seek into the source, seconds.
        #[serde(default)]
        offset: f64,
        #[serde(default = "default_volume")]
        volume: f64,
        /// Playback speed multiplier (0.1–10; 1.0 = normal). Duration
        /// stays in timeline seconds; media consumed = duration x rate.
        #[serde(default = "default_rate", skip_serializing_if = "is_default_rate")]
        rate: f64,
    },
    Audio {
        src: String,
        #[serde(default)]
        offset: f64,
        #[serde(default = "default_volume")]
        volume: f64,
        /// Playback speed multiplier (0.1–10; 1.0 = normal).
        #[serde(default = "default_rate", skip_serializing_if = "is_default_rate")]
        rate: f64,
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

fn default_font() -> String {
    "Sans Bold 32".into()
}
fn default_color() -> String {
    "#ffffff".into()
}
fn default_rate() -> f64 {
    1.0
}
fn is_default_rate(r: &f64) -> bool {
    (*r - 1.0).abs() < f64::EPSILON
}
fn default_volume() -> f64 {
    1.0
}
fn default_opacity() -> f64 {
    1.0
}
