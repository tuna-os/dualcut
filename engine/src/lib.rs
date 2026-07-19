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
#[cfg(feature = "vector")]
pub mod vellosrc;

pub fn init() -> Result<()> {
    gst::init().context("initializing GStreamer")?;
    ges::init().context("initializing GES")?;
    #[cfg(feature = "vector")]
    vellosrc::register().context("registering vellosrc")?;
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
    title.set_child_property("text", "dualcut M0: GES timeline works".to_value())?;
    title.set_child_property("font-desc", "Sans Bold 28".to_value())?;
    // Default title background is opaque; make it transparent (ARGB).
    title.set_child_property("background", 0x00000000u32.to_value())?;
    title.set_child_property("color", 0xffffffffu32.to_value())?;

    timeline.commit_sync();
    Ok(timeline)
}

/// Pick an encoding profile by name or output-file extension.
/// Supported: "mp4" (H.264+AAC), "webm" (VP8+Vorbis).
pub fn encoding_profile(name: &str) -> anyhow::Result<gst_pbutils::EncodingContainerProfile> {
    match name.rsplit('.').next().unwrap_or(name) {
        "mp4" | "h264" => Ok(mp4_profile()),
        "webm" | "vp8" => Ok(webm_profile()),
        "h265" | "hevc" => Ok(mp4_video_profile("video/x-h265")),
        "vp9" => Ok(vp9_profile()),
        "av1" => Ok(mp4_video_profile("video/x-av1")),
        "prores" | "mov" => Ok(prores_profile()),
        "ffv1" | "mkv" => Ok(ffv1_profile()),
        "m4a" => Ok(audio_profile(
            "video/quicktime",
            "audio/mpeg",
            Some(("mpegversion", 4)),
        )),
        "ogg" | "opus" => Ok(audio_profile("application/ogg", "audio/x-opus", None)),
        "flac" => Ok(audio_profile("application/ogg", "audio/x-flac", None)),
        "mp3" => Ok(audio_profile("application/x-id3", "audio/mpeg", Some(("mpegversion", 1)))),
        "wav" => Ok(audio_profile("audio/x-wav", "audio/x-raw", None)),
        other => anyhow::bail!(
            "unknown encoding profile {other:?} (video: mp4, webm, h265, vp9, av1, prores, ffv1; audio: m4a, ogg, flac, mp3, wav)"
        ),
    }
}

/// Lossless archival: FFV1 + FLAC in Matroska.
pub fn ffv1_profile() -> gst_pbutils::EncodingContainerProfile {
    let video = gst_pbutils::EncodingVideoProfile::builder(
        &gst::Caps::builder("video/x-ffv").build(),
    )
    .build();
    let audio = gst_pbutils::EncodingAudioProfile::builder(
        &gst::Caps::builder("audio/x-flac").build(),
    )
    .build();
    gst_pbutils::EncodingContainerProfile::builder(
        &gst::Caps::builder("video/x-matroska").build(),
    )
    .name("dualcut-ffv1")
    .add_profile(video)
    .add_profile(audio)
    .build()
}

/// Audio-only export: container caps + one audio stream.
pub fn audio_profile(
    container: &str,
    audio_caps: &str,
    extra: Option<(&str, i32)>,
) -> gst_pbutils::EncodingContainerProfile {
    let mut caps = gst::Caps::builder(audio_caps);
    if let Some((k, v)) = extra {
        caps = caps.field(k, v);
    }
    let audio = gst_pbutils::EncodingAudioProfile::builder(&caps.build()).build();
    gst_pbutils::EncodingContainerProfile::builder(&gst::Caps::builder(container).build())
        .name("dualcut-audio")
        .add_profile(audio)
        .build()
}

/// MP4 container with the given video codec caps + AAC audio.
pub fn mp4_video_profile(video_caps: &str) -> gst_pbutils::EncodingContainerProfile {
    let video = gst_pbutils::EncodingVideoProfile::builder(
        &gst::Caps::builder(video_caps).build(),
    )
    .build();
    let audio = gst_pbutils::EncodingAudioProfile::builder(
        &gst::Caps::builder("audio/mpeg").field("mpegversion", 4i32).build(),
    )
    .build();
    gst_pbutils::EncodingContainerProfile::builder(
        &gst::Caps::builder("video/quicktime").field("variant", "iso").build(),
    )
    .name("dualcut-mp4v")
    .add_profile(video)
    .add_profile(audio)
    .build()
}

/// WebM with VP9 + Opus.
pub fn vp9_profile() -> gst_pbutils::EncodingContainerProfile {
    let video = gst_pbutils::EncodingVideoProfile::builder(
        &gst::Caps::builder("video/x-vp9").build(),
    )
    .build();
    let audio = gst_pbutils::EncodingAudioProfile::builder(
        &gst::Caps::builder("audio/x-opus").build(),
    )
    .build();
    gst_pbutils::EncodingContainerProfile::builder(&gst::Caps::builder("video/webm").build())
        .name("dualcut-vp9")
        .add_profile(video)
        .add_profile(audio)
        .build()
}

/// QuickTime with ProRes + PCM — an editing/interchange master.
pub fn prores_profile() -> gst_pbutils::EncodingContainerProfile {
    let video = gst_pbutils::EncodingVideoProfile::builder(
        &gst::Caps::builder("video/x-prores").build(),
    )
    .build();
    let audio = gst_pbutils::EncodingAudioProfile::builder(
        &gst::Caps::builder("audio/x-raw").field("format", "S24LE").build(),
    )
    .build();
    gst_pbutils::EncodingContainerProfile::builder(
        &gst::Caps::builder("video/quicktime").build(),
    )
    .name("dualcut-prores")
    .add_profile(video)
    .add_profile(audio)
    .build()
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
    // No base-profile restriction: avenc_aac (the encoder present in the
    // flatpak) does not advertise it, and encodebin refuses caps its
    // encoder cannot intersect (#26).
    let audio = gst_pbutils::EncodingAudioProfile::builder(
        &gst::Caps::builder("audio/mpeg").field("mpegversion", 4i32).build(),
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
