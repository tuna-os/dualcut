use super::ShapeKind;
use serde::{Deserialize, Serialize};

/// A video effect on one clip.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Effect {
    /// Gaussian blur; `amount` is the blur sigma (0-50, ~3 is subtle).
    Blur { amount: f64 },
    /// Green-screen keying (#33): pixels near `color` turn transparent.
    ChromaKey {
        #[serde(default = "default_key_color")]
        color: String,
        /// Hue tolerance in degrees (1-90).
        #[serde(default = "default_key_angle")]
        angle: f64,
        /// Noise suppression level (0-64).
        #[serde(default)]
        noise: f64,
    },
    /// Crop pixels from the clip's edges (#33).
    Crop {
        #[serde(default)]
        left: i32,
        #[serde(default)]
        right: i32,
        #[serde(default)]
        top: i32,
        #[serde(default)]
        bottom: i32,
    },
    /// Three-band EQ in dB (-24..12) (#36).
    Eq {
        #[serde(default)]
        low: f64,
        #[serde(default)]
        mid: f64,
        #[serde(default)]
        high: f64,
    },
    /// Dynamic range compressor (#36).
    Compressor {
        /// Level above which compression starts (0-1).
        #[serde(default = "default_comp_threshold")]
        threshold: f64,
        /// Compression ratio (1-4 for audiodynamic).
        #[serde(default = "default_comp_ratio")]
        ratio: f64,
    },
    /// Noise suppression (WebRTC audio processing) (#36).
    Denoise {
        /// 0=low, 1=moderate, 2=high, 3=very-high.
        #[serde(default = "default_denoise_level")]
        level: u32,
    },
    /// Color balance. Neutral is brightness 0, contrast 1, saturation 1, hue 0.
    Color {
        #[serde(default)]
        brightness: f64,
        #[serde(default = "default_one")]
        contrast: f64,
        #[serde(default = "default_one")]
        saturation: f64,
        #[serde(default)]
        hue: f64,
    },
    /// Freeform shape mask: the clip is only visible inside `shape`
    /// (rendered full-frame, same shape catalog as `shape` clips). Needs
    /// the `vector` feature; skipped with a warning otherwise (#41).
    Mask {
        shape: ShapeKind,
        /// Gaussian blur sigma applied to the mask edge (0-50) for a
        /// soft/feathered cutoff instead of a hard edge.
        #[serde(default)]
        feather: f64,
        /// Show outside the shape instead of inside it.
        #[serde(default)]
        invert: bool,
    },
}

fn default_one() -> f64 {
    1.0
}
fn default_key_color() -> String {
    "#00ff00".into()
}
fn default_key_angle() -> f64 {
    20.0
}
fn default_comp_threshold() -> f64 {
    0.25
}
fn default_comp_ratio() -> f64 {
    2.0
}
fn default_denoise_level() -> u32 {
    1
}
