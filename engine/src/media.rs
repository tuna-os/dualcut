//! Media URI resolution.
//!
//! Every place that hands a path to GStreamer needs the same `file://`
//! URI, and every place that reads a project's `src` field needs the
//! same relative-to-the-project-directory resolution. Both used to be
//! open-coded at each call site (`mapping.rs`, `api.rs`, `render.rs`,
//! `lib.rs`, the preview binary), which let the rules drift apart: one
//! copy checked that the file exists, one canonicalized, one did
//! neither. This module is the single definition of both operations.

use anyhow::{bail, Context, Result};
use gst::prelude::*;
use gstreamer as gst;
use std::path::Path;

/// Set a string property on the element called `element` in a pipeline
/// built with `gst::parse::launch`.
///
/// A media URI or a file path must reach its element this way, never by
/// being formatted into the pipeline text. In that text a `"` ends the
/// value and the rest is read as more pipeline, so a project `src` (or a
/// file or directory name) carrying one would add elements of its own:
/// ones that read, write or send files. A property value is not parsed.
pub fn set_str_property(
    pipeline: &gst::Pipeline,
    element: &str,
    property: &str,
    value: &str,
) -> Result<()> {
    pipeline
        .by_name(element)
        .with_context(|| format!("pipeline has no element named {element:?}"))?
        .set_property(property, value.to_value());
    Ok(())
}

/// A local path as the string an element's `location` property takes.
pub fn path_str(path: &Path) -> Result<&str> {
    path.to_str().with_context(|| format!("path is not valid UTF-8: {}", path.display()))
}

/// `file://` URI for an absolute local path.
///
/// Used for render outputs and cache artifacts, which do not exist yet
/// (or are already canonical), so the path is formatted as given.
pub fn uri_from_path(path: &Path) -> String {
    format!("file://{}", path.display())
}

/// Resolve a project `src` field to a URI GStreamer can open.
///
/// A `src` that already carries a scheme (`file://`, `vello://`, …) is
/// returned unchanged. Anything else is a path relative to the project
/// directory: it must exist, and is canonicalized before it becomes a
/// URI.
pub fn resolve(src: &str, base_dir: &Path) -> Result<String> {
    if src.contains("://") {
        return Ok(src.to_string());
    }
    let path = base_dir.join(src);
    if !path.exists() {
        bail!("media file not found: {}", path.display());
    }
    Ok(uri_from_path(&path.canonicalize()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_bearing_src_passes_through() {
        let base = Path::new("/nonexistent");
        assert_eq!(resolve("file:///a/b.mp4", base).unwrap(), "file:///a/b.mp4");
        assert_eq!(resolve("vello://star", base).unwrap(), "vello://star");
        assert_eq!(resolve("https://example.com/a.mp4", base).unwrap(), "https://example.com/a.mp4");
    }

    #[test]
    fn missing_relative_src_is_an_error() {
        let err = resolve("no-such-file.mp4", Path::new(".")).unwrap_err();
        assert!(format!("{err:#}").contains("media file not found"));
    }

    #[test]
    fn existing_relative_src_becomes_a_canonical_file_uri() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("clip.mp4"), b"").unwrap();
        let uri = resolve("clip.mp4", dir.path()).unwrap();
        assert!(uri.starts_with("file://"));
        assert!(uri.ends_with("clip.mp4"));
    }

    #[test]
    fn uri_from_path_does_not_require_the_file_to_exist() {
        assert_eq!(uri_from_path(Path::new("/tmp/out.part")), "file:///tmp/out.part");
    }
}
