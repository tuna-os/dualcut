/// Document schema types and validation.
///
/// This module contains the versioned, immutable document data model that
/// describes the structure of a video composition (tracks, clips, effects,
/// timeline). These types are independent of persistence format and UI state,
/// making them independently versionable and testable.
///
/// See dualcut#196 for the extraction roadmap.

use serde::{Deserialize, Serialize};

/// The root document structure defining a composition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSchema {
    /// Schema version for migration support.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    
    /// Metadata about the document.
    pub metadata: DocumentMetadata,
    
    /// Timeline structure.
    pub timeline: Timeline,
    
    /// Global effects and filters.
    #[serde(default)]
    pub global_effects: Vec<Effect>,
}

fn default_schema_version() -> u32 {
    1
}

/// Document metadata (title, description, aspect ratio, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Frame rate (fps).
    #[serde(default = "default_frame_rate")]
    pub frame_rate: f64,
    /// Width in pixels.
    #[serde(default = "default_width")]
    pub width: u32,
    /// Height in pixels.
    #[serde(default = "default_height")]
    pub height: u32,
}

fn default_frame_rate() -> f64 {
    24.0
}

fn default_width() -> u32 {
    1920
}

fn default_height() -> u32 {
    1080
}

/// Timeline: sequence of scenes (shots) with audio and overlay tracks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timeline {
    /// Ordered sequence of scenes.
    #[serde(default)]
    pub scenes: Vec<Scene>,
    
    /// Global audio tracks that overlay all scenes.
    #[serde(default)]
    pub audio_tracks: Vec<AudioTrack>,
    
    /// Global overlay (graphics, titles) tracks.
    #[serde(default)]
    pub overlay_tracks: Vec<OverlayTrack>,
}

/// A scene is one or more clips composited together (a "shot").
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

/// Transition between scenes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transition {
    #[serde(default)]
    pub kind: TransitionKind,
    /// Seconds of overlap with the previous scene.
    pub duration: f64,
}

/// Transition effect types.
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

/// Audio track in the timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioTrack {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Mute without removing clips.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub muted: bool,
    /// Audio clips on this track.
    #[serde(default)]
    pub clips: Vec<Clip>,
}

/// Overlay track (graphics, titles, effects).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayTrack {
    pub id: String,
    /// Mute the track's audio without touching its clips.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub muted: bool,
    /// Hide the track's video without touching its clips.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// Disable dragging clips on this track in the GUI.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub clips: Vec<Clip>,
}

/// A clip is a media element on a track (video, audio, or source).
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
    /// The media element (video, audio, text, etc.).
    #[serde(flatten)]
    pub element: Element,
    /// Transform (scale, rotation, position).
    #[serde(default)]
    pub transform: Transform,
    /// Keyframe animations.
    #[serde(default)]
    pub animations: Vec<Anim>,
    /// Video effects applied in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
}

/// Element types (video, audio, text, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Element {
    /// Video file or stream.
    Video {
        path: String,
    },
    /// Audio file or stream.
    Audio {
        path: String,
    },
    /// Text overlay.
    Text {
        content: String,
        font: String,
        size: f64,
    },
    /// Color fill.
    Color {
        color: String, // hex or rgb
    },
}

/// Transform properties (position, scale, rotation).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Transform {
    /// X position in normalized coordinates (0-1).
    #[serde(default)]
    pub x: f64,
    /// Y position in normalized coordinates (0-1).
    #[serde(default)]
    pub y: f64,
    /// Scale factor (1.0 = original size).
    #[serde(default = "default_scale")]
    pub scale: f64,
    /// Rotation in degrees.
    #[serde(default)]
    pub rotation: f64,
    /// Opacity (0-1).
    #[serde(default = "default_opacity")]
    pub opacity: f64,
}

fn default_scale() -> f64 {
    1.0
}

fn default_opacity() -> f64 {
    1.0
}

/// Video effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Effect {
    /// Color correction.
    ColorCorrection {
        brightness: f64,
        contrast: f64,
        saturation: f64,
    },
    /// Blur effect.
    Blur {
        radius: f64,
    },
    /// Other effects.
    #[serde(other)]
    Unknown,
}

/// Keyframe animation for properties.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Anim {
    /// Property being animated (e.g., "opacity", "scale").
    pub property: String,
    /// Keyframes at specific times.
    pub keyframes: Vec<Keyframe>,
}

/// Single keyframe at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Keyframe {
    /// Time in seconds.
    pub time: f64,
    /// Value at this keyframe.
    pub value: f64,
    /// Easing function.
    #[serde(default)]
    pub easing: EasingKind,
}

/// Easing function types.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EasingKind {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}

impl DocumentSchema {
    /// Create a new empty document schema with defaults.
    pub fn new(id: String, name: String) -> Self {
        Self {
            schema_version: 1,
            metadata: DocumentMetadata {
                id,
                name,
                description: String::new(),
                frame_rate: 24.0,
                width: 1920,
                height: 1080,
            },
            timeline: Timeline {
                scenes: Vec::new(),
                audio_tracks: Vec::new(),
                overlay_tracks: Vec::new(),
            },
            global_effects: Vec::new(),
        }
    }
    
    /// Get the total duration of the timeline.
    pub fn total_duration(&self) -> f64 {
        self.timeline.scenes.iter().map(|s| s.duration).sum()
    }
    
    /// Validate the schema for consistency.
    pub fn validate(&self) -> Result<(), String> {
        // Check that scene IDs are unique
        let mut scene_ids = std::collections::HashSet::new();
        for scene in &self.timeline.scenes {
            if !scene_ids.insert(&scene.id) {
                return Err(format!("Duplicate scene ID: {}", scene.id));
            }
        }
        
        // Check that metadata dimensions are reasonable
        if self.metadata.width == 0 || self.metadata.height == 0 {
            return Err("Document dimensions must be non-zero".to_string());
        }
        
        if self.metadata.frame_rate <= 0.0 {
            return Err("Frame rate must be positive".to_string());
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_schema_creation() {
        let schema = DocumentSchema::new("doc1".to_string(), "Test Doc".to_string());
        assert_eq!(schema.schema_version, 1);
        assert_eq!(schema.metadata.name, "Test Doc");
        assert_eq!(schema.metadata.width, 1920);
        assert_eq!(schema.metadata.height, 1080);
    }
    
    #[test]
    fn test_schema_validation() {
        let schema = DocumentSchema::new("doc1".to_string(), "Test".to_string());
        assert!(schema.validate().is_ok());
    }
    
    #[test]
    fn test_duration_calculation() {
        let mut schema = DocumentSchema::new("doc1".to_string(), "Test".to_string());
        schema.timeline.scenes.push(Scene {
            id: "s1".to_string(),
            name: "Scene 1".to_string(),
            duration: 5.0,
            transition: None,
            layers: Vec::new(),
        });
        schema.timeline.scenes.push(Scene {
            id: "s2".to_string(),
            name: "Scene 2".to_string(),
            duration: 3.0,
            transition: None,
            layers: Vec::new(),
        });
        
        assert_eq!(schema.total_duration(), 8.0);
    }
}
