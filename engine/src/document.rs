//! Document facade: persisted model, persistence, and stateless editing.
//!
//! UI selection, viewport, and history remain in the preview editor; none are
//! required to parse, validate, persist, or edit a project. Existing callers can
//! continue importing the model and operations through `document::*`.

pub mod schema;
pub mod persistence;
mod operations;
mod traversal;

pub use schema::*;
pub use persistence::DocumentRepository;
pub use operations::*;
pub use traversal::{effective_duration, find_clip, find_clip_mut, remove_clip};
