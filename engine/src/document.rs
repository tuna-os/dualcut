//! Document model v2 (see ROADMAP.md): scenes are the sequential narrative
//! spine, overlays span scene cuts, defs are reusable parameterised
//! compositions. This document — not GES — is what the UI, in-app scripts,
//! and external agents edit; `crate::mapping` compiles it into a GES
//! timeline.

pub mod schema;
pub mod commands;
pub mod persistence;
pub mod validate;
mod color;
mod timing;
mod traversal;

pub use schema::*;
pub use commands::*;
pub use color::parse_color;
pub use timing::{ease, expand_rate_ramp};
pub use traversal::{effective_duration, find_clip, find_clip_mut, remove_clip};

#[cfg(test)]
mod tests;
