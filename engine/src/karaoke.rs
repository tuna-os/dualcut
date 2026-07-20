//! Karaoke-style captions (#53): the whole line stays on screen while the
//! actively-spoken word is highlighted, instead of one word popping on at a
//! time. GES's `TitleClip` can't do this: `GstBaseTextOverlay`'s
//! `auto-resize` (rescales font per-clip to fit that clip's own text) isn't
//! exposed as a settable child property, so an isolated word clip and the
//! full-sentence clip render at inconsistent sizes; and TitleClip's `text`
//! property is always markup-escaped (only a raw `textoverlay`'s
//! `pango-markup`-typed sink pad honors it), so per-word color spans can't
//! reach GES that way either.
//!
//! The fix used by mainstream caption tools (CapCut, Premiere, YouTube):
//! lay out the *entire* line once in a real text engine, color only the
//! active word's run, and rasterize each highlight state as its own image.
//! Same layout box and font every time, so nothing rescales between
//! states. Each state becomes an ordinary `Image` clip -- no GES text
//! primitives involved at all.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Group flat word-level `(start, end, word)` segments (e.g. whisper.cpp
/// `--max-len 1` output) back into caption lines. A line breaks when
/// adding the next word would exceed `max_chars`, when the gap since the
/// previous word exceeds `max_gap` seconds (a pause is a natural phrase
/// boundary), or after a word ending a sentence (`.`/`?`/`!`). Pure and
/// unit-testable without any rendering.
pub fn group_words_into_lines(
    words: &[(f64, f64, String)],
    max_chars: usize,
    max_gap: f64,
) -> Vec<Vec<(f64, f64, String)>> {
    let mut lines: Vec<Vec<(f64, f64, String)>> = Vec::new();
    let mut current: Vec<(f64, f64, String)> = Vec::new();
    let mut current_len = 0usize;
    for (start, end, word) in words {
        let word = word.trim();
        if word.is_empty() {
            continue;
        }
        let gap_break =
            current.last().is_some_and(|(_, prev_end, _): &(f64, f64, String)| start - prev_end > max_gap);
        let len_break = !current.is_empty() && current_len + word.len() + 1 > max_chars;
        if gap_break || len_break {
            lines.push(std::mem::take(&mut current));
            current_len = 0;
        }
        current_len += word.len() + 1;
        let sentence_end = word.ends_with(['.', '?', '!']);
        current.push((*start, *end, word.to_string()));
        if sentence_end {
            lines.push(std::mem::take(&mut current));
            current_len = 0;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Rasterize one karaoke highlight state: the full `line_words` joined with
/// spaces, laid out once, with `active_idx`'s word colored gold and every
/// other word white, black-outlined for legibility over any footage.
/// Cropped to the ink bounding box (plus a small margin) so the returned
/// PNG is only as large as the rendered text, not a full-canvas image.
/// Cached under `cache_dir` by content.
#[cfg(feature = "karaoke")]
pub fn render_karaoke_frame(
    cache_dir: &Path,
    line_words: &[String],
    active_idx: usize,
    max_width: u32,
) -> Result<(PathBuf, u32, u32)> {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (line_words, active_idx, max_width).hash(&mut hasher);
    let key = hasher.finish();
    std::fs::create_dir_all(cache_dir)?;
    let file = cache_dir.join(format!("karaoke-{key:016x}.png"));
    if file.exists() {
        let (w, h) = image::image_dimensions(&file).context("reading cached karaoke frame size")?;
        return Ok((file, w, h));
    }

    let text = line_words.join(" ");
    let mut offset = 0usize;
    let mut active_range = (0u32, 0u32);
    for (i, w) in line_words.iter().enumerate() {
        if i == active_idx {
            active_range = (offset as u32, (offset + w.len()) as u32);
        }
        offset += w.len() + 1;
    }

    const MARGIN: i32 = 20;
    const FONT: &str = "Sans Bold 40";

    let build_layout = |cr: &cairo::Context, wrap_width: i32| -> pango::Layout {
        let layout = pangocairo::functions::create_layout(cr);
        layout.set_font_description(Some(&pango::FontDescription::from_string(FONT)));
        layout.set_width(wrap_width * pango::SCALE);
        layout.set_wrap(pango::WrapMode::Word);
        layout.set_alignment(pango::Alignment::Center);
        layout.set_text(&text);
        layout
    };

    // Measure ink extents at the target wrap width first, so the final
    // surface is only as big as the rendered text actually needs.
    let probe = cairo::ImageSurface::create(cairo::Format::ARgb32, 4, 4)
        .context("creating probe surface")?;
    let cr = cairo::Context::new(&probe).context("cairo context")?;
    let wrap_width = max_width as i32 - 2 * MARGIN;
    let layout = build_layout(&cr, wrap_width);
    let (ink, _logical) = layout.pixel_extents();
    drop(cr);

    let w = (ink.width() + 2 * MARGIN).max(10);
    let h = (ink.height() + 2 * MARGIN).max(10);
    let surface =
        cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).context("creating render surface")?;
    let cr = cairo::Context::new(&surface).context("cairo context")?;
    let layout = build_layout(&cr, wrap_width);
    cr.translate((MARGIN - ink.x()) as f64, (MARGIN - ink.y()) as f64);

    // Outline pass: stroke the glyph outlines in black.
    cr.set_source_rgba(0.0, 0.0, 0.0, 1.0);
    pangocairo::functions::layout_path(&cr, &layout);
    cr.set_line_width(4.0);
    cr.stroke().context("stroking text outline")?;

    // Fill pass: white base, gold for the active word.
    let attrs = pango::AttrList::new();
    let u8_to_16 = |v: u16| v * 257;
    let mut gold = pango::AttrColor::new_foreground(u8_to_16(255), u8_to_16(200), u8_to_16(0));
    gold.set_start_index(active_range.0);
    gold.set_end_index(active_range.1);
    attrs.insert(gold);
    layout.set_attributes(Some(&attrs));
    cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
    pangocairo::functions::show_layout(&cr, &layout);
    drop(cr);

    let mut png = Vec::new();
    surface
        .write_to_png(&mut std::io::Cursor::new(&mut png))
        .context("encoding karaoke frame png")?;
    std::fs::write(&file, &png)?;
    Ok((file, w as u32, h as u32))
}

#[cfg(not(feature = "karaoke"))]
pub fn render_karaoke_frame(
    _cache_dir: &Path,
    _line_words: &[String],
    _active_idx: usize,
    _max_width: u32,
) -> Result<(PathBuf, u32, u32)> {
    anyhow::bail!("karaoke captions need the \"karaoke\" feature")
}

/// Build the karaoke caption clips: one `Image` clip per word-highlight
/// state, centered near the bottom of the frame, timed to that word's own
/// span. `canvas_w`/`canvas_h` are the project's frame size.
pub fn karaoke_captions_to_clips(
    words: &[(f64, f64, String)],
    cache_dir: &Path,
    canvas_w: u32,
    canvas_h: u32,
) -> Result<Vec<crate::document::Clip>> {
    let lines = group_words_into_lines(words, 42, 0.6);
    let mut clips = Vec::new();
    let max_width = (canvas_w as f64 * 0.9) as u32;
    for (li, line) in lines.iter().enumerate() {
        let line_words: Vec<String> = line.iter().map(|(_, _, w)| w.clone()).collect();
        for (wi, (start, end, _)) in line.iter().enumerate() {
            let (png, png_w, _png_h) = render_karaoke_frame(cache_dir, &line_words, wi, max_width)
                .with_context(|| format!("rendering karaoke line {li} word {wi}"))?;
            let png = png.canonicalize().context("resolving karaoke frame path")?;
            clips.push(crate::document::Clip {
                id: format!("sub-{li}-{wi}"),
                start: (*start * 100.0).round() / 100.0,
                duration: (((*end - *start) * 100.0).round() / 100.0).max(0.01),
                element: crate::document::Element::Image { src: png.display().to_string() },
                transform: crate::document::Transform {
                    x: (canvas_w as f64 - png_w as f64) / 2.0,
                    y: canvas_h as f64 * 0.78,
                    width: 0.0,
                    height: 0.0,
                    opacity: 1.0,
                },
                animations: Vec::new(),
                effects: Vec::new(),
            });
        }
    }
    Ok(clips)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(start: f64, end: f64, text: &str) -> (f64, f64, String) {
        (start, end, text.to_string())
    }

    #[test]
    fn groups_short_run_into_one_line() {
        let words = vec![w(0.0, 0.2, "hello"), w(0.2, 0.4, "world")];
        let lines = group_words_into_lines(&words, 42, 0.6);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 2);
    }

    #[test]
    fn breaks_line_on_a_long_pause() {
        let words = vec![w(0.0, 0.2, "hello"), w(3.0, 3.2, "world")];
        let lines = group_words_into_lines(&words, 42, 0.6);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn breaks_line_after_sentence_end_punctuation() {
        let words = vec![w(0.0, 0.2, "Hi."), w(0.3, 0.5, "Bye.")];
        let lines = group_words_into_lines(&words, 42, 0.6);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn breaks_line_when_it_would_exceed_max_chars() {
        let words: Vec<_> =
            (0..20).map(|i| w(i as f64, i as f64 + 0.5, "word")).collect();
        let lines = group_words_into_lines(&words, 20, 10.0);
        assert!(lines.len() > 1);
        for line in &lines {
            let len: usize = line.iter().map(|(_, _, w)| w.len() + 1).sum();
            assert!(len <= 24, "line exceeded max_chars by more than one word: {len}");
        }
    }

    #[test]
    fn empty_words_produce_no_lines() {
        assert!(group_words_into_lines(&[], 42, 0.6).is_empty());
    }

    #[test]
    fn blank_words_are_skipped() {
        let words = vec![w(0.0, 0.1, "  "), w(0.1, 0.2, "hi")];
        let lines = group_words_into_lines(&words, 42, 0.6);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 1);
    }

    #[cfg(feature = "karaoke")]
    #[test]
    fn render_karaoke_frame_produces_a_readable_png() {
        let dir = std::env::temp_dir().join("dualcut-karaoke-test");
        let words = ["this".to_string(), "is".to_string(), "karaoke".to_string()];
        let (png, w, h) = render_karaoke_frame(&dir, &words, 2, 1080).expect("renders");
        let img = image::open(&png).expect("valid png").to_rgba8();
        assert_eq!((img.width(), img.height()), (w, h));
        assert!(img.width() > 10 && img.height() > 10);
        // At least one pixel should carry the gold highlight color (allow
        // for anti-aliasing blending it toward white/black at edges).
        let has_gold = img.pixels().any(|p| p[0] > 200 && p[1] > 140 && p[1] < 220 && p[2] < 60);
        assert!(has_gold, "no gold-ish pixel found in the rendered frame");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// End-to-end (#53): real frames, a real GES timeline, a real render --
    /// not just unit-level assertions on the document model. Confirms the
    /// clips actually composite correctly over footage, which is exactly
    /// where the original TitleClip-based approach silently broke (it
    /// "worked" at the document level; only the rendered pixels showed the
    /// font-size mismatch).
    #[cfg(feature = "karaoke")]
    #[test]
    fn karaoke_captions_render_and_show_the_highlighted_word() {
        use gstreamer as gst;
        use gstreamer::prelude::*;

        crate::init().expect("gst/ges init");
        let dir = std::env::temp_dir().join("dualcut-karaoke-e2e-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cache = dir.join(".dualcut-cache");

        let words = vec![
            (0.0, 0.3, "hello".to_string()),
            (0.3, 0.6, "karaoke".to_string()),
            (0.6, 0.9, "world".to_string()),
        ];
        let clips = karaoke_captions_to_clips(&words, &cache, 640, 360).expect("builds clips");
        assert_eq!(clips.len(), 3);

        let project = crate::document::Project {
            meta: crate::document::Meta { title: "karaoke-e2e".into(), width: 640, height: 360, fps: 25 },
            library: Vec::new(),
            defs: Default::default(),
            scenes: vec![crate::document::Scene {
                id: "s1".into(),
                name: String::new(),
                duration: 0.9,
                transition: None,
                layers: vec![crate::document::Clip {
                    id: "bg".into(),
                    start: 0.0,
                    duration: 0.9,
                    element: crate::document::Element::Test {},
                    transform: Default::default(),
                    animations: Vec::new(),
                    effects: Vec::new(),
                }],
            }],
            overlays: vec![crate::document::OverlayTrack {
                id: "subtitles".into(),
                muted: false,
                hidden: false,
                locked: false,
                name: "Subtitles".into(),
                clips,
            }],
            scene_lanes: Vec::new(),
        };
        project.validate().expect("valid project");

        let out = dir.join("out.mkv");
        let json = project.to_json();
        let warnings = crate::render_project(&json, &dir, out.to_str().unwrap(), "ffv1")
            .expect("renders without error");
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

        // Extract the frame at the middle word's active window (t=0.45s,
        // "karaoke" highlighted) and confirm a gold-ish pixel is present
        // somewhere in the caption band near the bottom of the frame.
        let desc = format!(
            "filesrc location=\"{}\" ! decodebin ! videoconvert ! video/x-raw,format=RGB ! \
             appsink name=sink sync=false",
            out.display()
        );
        let pipeline = gst::parse::launch(&desc)
            .unwrap()
            .downcast::<gst::Pipeline>()
            .map_err(|_| ())
            .unwrap();
        let sink = pipeline
            .by_name("sink")
            .unwrap()
            .downcast::<gstreamer_app::AppSink>()
            .map_err(|_| ())
            .unwrap();
        pipeline.set_state(gst::State::Paused).unwrap();
        pipeline.state(gst::ClockTime::from_seconds(10)).0.unwrap();
        pipeline
            .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE, gst::ClockTime::from_mseconds(450))
            .unwrap();
        pipeline.set_state(gst::State::Playing).unwrap();
        let sample = sink.pull_sample().expect("pulling sample");
        let buffer = sample.buffer().unwrap();
        let caps = sample.caps().unwrap();
        let s = caps.structure(0).unwrap();
        let width: i32 = s.get("width").unwrap();
        let height: i32 = s.get("height").unwrap();
        let map = buffer.map_readable().unwrap();
        let stride = ((width * 3 + 3) / 4) * 4;
        let has_gold = (0..height).any(|y| {
            (0..width).any(|x| {
                let i = (y * stride + x * 3) as usize;
                let (r, g, b) = (map[i], map[i + 1], map[i + 2]);
                r > 200 && (140..220).contains(&g) && b < 60
            })
        });
        pipeline.set_state(gst::State::Null).unwrap();
        assert!(has_gold, "no gold-ish pixel in the rendered frame at t=0.45s");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
