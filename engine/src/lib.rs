//! dualcut engine — M0 pipeline spike.
//!
//! Proves the core of the native stack from ROADMAP.md: a GES timeline
//! built programmatically (the document→GES mapping arrives in M1),
//! rendered to a file headless (`render` bin) or previewed in a GTK4
//! window via gtk4paintablesink (`preview` bin, feature = "preview").

use anyhow::{Context, Result};
use ges::prelude::*;
use gstreamer as gst;
use gst::prelude::*;
use gstreamer_pbutils as gst_pbutils;
use gstreamer_editing_services as ges;

pub mod document;
pub mod api;
#[cfg(feature = "preview")]
pub mod thumbs;
pub mod mapping;
pub mod templates;
#[cfg(feature = "scripting")]
pub mod scripting;
#[cfg(feature = "vector")]
pub mod vector;

pub fn init() -> Result<()> {
    gst::init().context("initializing GStreamer")?;
    ges::init().context("initializing GES")?;
    Ok(())
}

/// Build the M0 demo timeline:
/// layer 0 (top): title text
/// layer 1: an optional media clip (any file/URL GStreamer can decode)
/// layer 2: animated test pattern as background
pub fn build_demo_timeline(media_uri: Option<&str>) -> Result<ges::Timeline> {
    let timeline = ges::Timeline::new_audio_video();

    let title_layer = timeline.append_layer();
    let media_layer = timeline.append_layer();
    let bg_layer = timeline.append_layer();

    // Background: GES's built-in test source (SMPTE pattern + silence).
    let bg = ges::TestClip::new().context("creating test clip")?;
    bg.set_start(gst::ClockTime::ZERO);
    bg.set_duration(gst::ClockTime::from_seconds(8));
    bg.set_vpattern(ges::VideoTestPattern::Smpte);
    bg_layer.add_clip(&bg).context("adding background clip")?;

    if let Some(uri) = media_uri {
        let clip = ges::UriClip::new(uri).context("creating uri clip")?;
        clip.set_start(gst::ClockTime::from_seconds(2));
        clip.set_inpoint(gst::ClockTime::ZERO);
        clip.set_duration(gst::ClockTime::from_seconds(4));
        media_layer.add_clip(&clip).context("adding media clip")?;
    }

    // Title on top, seconds 1–6.
    let title = ges::TitleClip::new().context("creating title clip")?;
    title.set_start(gst::ClockTime::from_seconds(1));
    title.set_duration(gst::ClockTime::from_seconds(5));
    title_layer.add_clip(&title).context("adding title clip")?;
    title.set_child_property("text", &"dualcut M0: GES timeline works".to_value())?;
    title.set_child_property("font-desc", &"Sans Bold 28".to_value())?;
    // Default title background is opaque; make it transparent (ARGB).
    title.set_child_property("background", &0x00000000u32.to_value())?;
    title.set_child_property("color", &0xffffffffu32.to_value())?;

    timeline.commit_sync();
    Ok(timeline)
}

/// Pick an encoding profile by name or output-file extension.
/// Supported: "mp4" (H.264+AAC), "webm" (VP8+Vorbis).
pub fn encoding_profile(name: &str) -> anyhow::Result<gst_pbutils::EncodingContainerProfile> {
    match name.rsplit('.').next().unwrap_or(name) {
        "mp4" | "h264" => Ok(mp4_profile()),
        "webm" | "vp8" => Ok(webm_profile()),
        other => anyhow::bail!("unknown encoding profile {other:?} (use mp4 or webm)"),
    }
}

/// WebM (VP8 + Vorbis) profile.
pub fn webm_profile() -> gst_pbutils::EncodingContainerProfile {
    let video = gst_pbutils::EncodingVideoProfile::builder(
        &gst::Caps::builder("video/x-vp8").build(),
    )
    .build();
    let audio = gst_pbutils::EncodingAudioProfile::builder(
        &gst::Caps::builder("audio/x-vorbis").build(),
    )
    .build();
    gst_pbutils::EncodingContainerProfile::builder(&gst::Caps::builder("video/webm").build())
        .name("dualcut-webm")
        .add_profile(video)
        .add_profile(audio)
        .build()
}

/// MP4 (H.264 + AAC) encoding profile for `ges::Pipeline::set_render_settings`.
pub fn mp4_profile() -> gst_pbutils::EncodingContainerProfile {
    let video = gst_pbutils::EncodingVideoProfile::builder(
        &gst::Caps::builder("video/x-h264").field("profile", "high").build(),
    )
    .build();
    let audio = gst_pbutils::EncodingAudioProfile::builder(
        &gst::Caps::builder("audio/mpeg")
            .field("mpegversion", 4i32)
            .field("base-profile", "lc")
            .build(),
    )
    .build();
    gst_pbutils::EncodingContainerProfile::builder(
        &gst::Caps::builder("video/quicktime").field("variant", "iso").build(),
    )
    .name("dualcut-mp4")
    .add_profile(video)
    .add_profile(audio)
    .build()
}

/// Render a project document to a file. Self-contained (parses the JSON
/// itself) so callers can run it on a worker thread — GES objects are not
/// Send, so everything GStreamer stays inside this call.
pub fn render_project(
    project_json: &str,
    base_dir: &std::path::Path,
    out: &str,
    profile: &str,
) -> Result<Vec<String>> {
    let project = document::Project::from_json(project_json)?;
    let compiled = mapping::compile(&project, base_dir)?;
    let pipeline = ges::Pipeline::new();
    pipeline.set_timeline(&compiled.timeline).context("attaching timeline")?;
    if let Some(parent) = std::path::Path::new(out).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let out_abs = std::path::absolute(out)?;
    pipeline.set_render_settings(&format!("file://{}", out_abs.display()), &encoding_profile(profile)?)?;
    pipeline.set_mode(ges::PipelineFlags::RENDER)?;
    run_to_eos(&pipeline)?;
    Ok(compiled.warnings)
}

/// Run a pipeline until EOS or error, printing progress.
pub fn run_to_eos(pipeline: &ges::Pipeline) -> Result<()> {
    let bus = pipeline.bus().context("pipeline has no bus")?;
    pipeline
        .set_state(gst::State::Playing)
        .context("setting pipeline to Playing")?;

    for msg in bus.iter_timed(gst::ClockTime::NONE) {
        use gst::MessageView;
        match msg.view() {
            MessageView::Eos(..) => break,
            MessageView::Error(err) => {
                pipeline.set_state(gst::State::Null)?;
                anyhow::bail!(
                    "pipeline error from {:?}: {} ({:?})",
                    err.src().map(|s| s.path_string()),
                    err.error(),
                    err.debug()
                );
            }
            _ => {}
        }
    }
    pipeline.set_state(gst::State::Null)?;
    Ok(())
}
