//! First-frame thumbnails for media clips (feature = "preview").
//! Cached as small PNGs next to the shape cache.

use anyhow::{Context, Result};
use gst::prelude::*;
use gstreamer_editing_services as ges;
use gstreamer as gst;
use gstreamer_app as gst_app;
use std::path::{Path, PathBuf};

const W: i32 = 128;
const H: i32 = 72;

/// Extract (or fetch from cache) a small first-frame thumbnail for a media
/// URI. Synchronous — call from a worker thread.
pub fn thumbnail_png(cache_dir: &Path, uri: &str) -> Result<PathBuf> {
    let key = format!("thumb-{:016x}.png", fxhash(uri));
    let file = cache_dir.join(key);
    if file.exists() {
        return Ok(file);
    }
    std::fs::create_dir_all(cache_dir)?;

    let pipeline = gst::parse::launch(&format!(
        "uridecodebin uri={uri} ! videoconvert ! videoscale ! \
         video/x-raw,format=RGB,width={W},height={H},pixel-aspect-ratio=1/1 ! \
         appsink name=sink sync=false"
    ))?
    .downcast::<gst::Pipeline>()
    .map_err(|_| anyhow::anyhow!("not a pipeline"))?;
    let sink = pipeline
        .by_name("sink")
        .context("appsink missing")?
        .downcast::<gst_app::AppSink>()
        .map_err(|_| anyhow::anyhow!("not an appsink"))?;

    pipeline.set_state(gst::State::Paused)?;
    let (res, _, _) = pipeline.state(gst::ClockTime::from_seconds(5));
    res.context("prerolling for thumbnail")?;
    let sample = sink.pull_preroll().context("pulling preroll sample")?;
    let buffer = sample.buffer().context("sample has no buffer")?;
    let map = buffer.map_readable()?;

    // RGB rows may be padded to 4-byte alignment.
    let stride = ((W * 3 + 3) & !3) as usize;
    let mut img = image::RgbImage::new(W as u32, H as u32);
    for y in 0..H as usize {
        let row = &map[y * stride..][..W as usize * 3];
        for x in 0..W as usize {
            let i = x * 3;
            img.put_pixel(x as u32, y as u32, image::Rgb([row[i], row[i + 1], row[i + 2]]));
        }
    }
    pipeline.set_state(gst::State::Null)?;
    img.save(&file)?;
    Ok(file)
}

/// Width of preview proxy media; height follows the source aspect.
const PROXY_W: i32 = 960;

/// Path a proxy for `uri` would live at (whether or not it exists yet).
pub fn proxy_path(cache_dir: &Path, uri: &str) -> PathBuf {
    cache_dir.join(format!("proxy-{:016x}.mkv", fxhash(uri)))
}

/// Transcode a media URI to a scrubbing-friendly preview proxy (960px-wide
/// MJPEG + Vorbis in Matroska — all-intra, so seeking is instant and it
/// decodes everywhere). Cached by URI hash; returns the existing file if
/// present. Synchronous and slow — call from a worker thread. Preview-only:
/// exports always read the original media.
pub fn proxy_mp4(cache_dir: &Path, uri: &str) -> Result<PathBuf> {
    use gstreamer_pbutils as gst_pbutils;
    use gst_pbutils::prelude::*;

    let file = proxy_path(cache_dir, uri);
    if file.exists() {
        return Ok(file);
    }
    std::fs::create_dir_all(cache_dir)?;

    // A matroskamux pad that never sees data stalls the mux forever, so
    // probe the source for audio up front instead of retry-on-hang.
    let discoverer = gst_pbutils::Discoverer::new(gst::ClockTime::from_seconds(15))?;
    let info = discoverer
        .discover_uri(uri)
        .with_context(|| format!("probing {uri} for proxy"))?;
    if info.video_streams().is_empty() {
        anyhow::bail!("no video stream in {uri}; not building a proxy");
    }
    let has_audio = !info.audio_streams().is_empty();

    // Write to a partial file and rename on success so a killed transcode
    // never leaves a half-written proxy that later hits the cache check.
    let part = cache_dir.join(format!("proxy-{:016x}.mkv.part", fxhash(uri)));
    let _ = std::fs::remove_file(&part);
    let video = format!(
        "uridecodebin uri={uri} name=d \
         d. ! queue ! videoconvert ! videoscale ! \
         video/x-raw,width={PROXY_W},pixel-aspect-ratio=1/1 ! \
         jpegenc quality=70 ! queue ! matroskamux name=m ! \
         filesink location=\"{}\"",
        part.display()
    );
    let audio = " d. ! queue ! audioconvert ! audioresample ! vorbisenc ! queue ! m.";
    let desc = if has_audio { format!("{video}{audio}") } else { video };

    let pipeline = gst::parse::launch(&desc)?
        .downcast::<gst::Pipeline>()
        .map_err(|_| anyhow::anyhow!("not a pipeline"))?;
    pipeline.set_state(gst::State::Playing)?;
    let bus = pipeline.bus().context("pipeline has no bus")?;
    let mut result = Ok(());
    for msg in bus.iter_timed(gst::ClockTime::NONE) {
        use gst::MessageView;
        match msg.view() {
            MessageView::Eos(_) => break,
            MessageView::Error(e) => {
                result = Err(anyhow::anyhow!("proxy transcode failed: {}", e.error()));
                break;
            }
            _ => {}
        }
    }
    let _ = pipeline.set_state(gst::State::Null);
    if let Err(e) = result {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, &file)?;
    Ok(file)
}

fn fxhash(s: &str) -> u64 {
    // Tiny stable hash; only used for cache filenames.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

const WF_W: u32 = 240;
const WF_H: u32 = 28;

/// Render (or fetch cached) a peak waveform strip for a media URI's audio.
/// Synchronous — call from a worker thread.
pub fn waveform_png(cache_dir: &Path, uri: &str) -> Result<PathBuf> {
    let file = cache_dir.join(format!("wave-{:016x}.png", fxhash(uri)));
    if file.exists() {
        return Ok(file);
    }
    std::fs::create_dir_all(cache_dir)?;

    let pipeline = gst::parse::launch(&format!(
        "uridecodebin uri={uri} ! audioconvert ! audioresample ! \
         audio/x-raw,format=F32LE,channels=1,rate=8000 ! \
         appsink name=sink sync=false"
    ))?
    .downcast::<gst::Pipeline>()
    .map_err(|_| anyhow::anyhow!("not a pipeline"))?;
    let sink = pipeline
        .by_name("sink")
        .context("appsink missing")?
        .downcast::<gst_app::AppSink>()
        .map_err(|_| anyhow::anyhow!("not an appsink"))?;

    pipeline.set_state(gst::State::Playing)?;
    let mut samples: Vec<f32> = Vec::new();
    // Pull until EOS or error ends the stream.
    while let Ok(sample) = sink.pull_sample() {
        if let Some(buffer) = sample.buffer() {
            let map = buffer.map_readable()?;
            samples.extend(
                map.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            );
        }
    }
    pipeline.set_state(gst::State::Null)?;
    if samples.is_empty() {
        anyhow::bail!("no audio samples decoded");
    }

    // Peak per bucket.
    let bucket = (samples.len() / WF_W as usize).max(1);
    let mut img = image::RgbaImage::new(WF_W, WF_H);
    for x in 0..WF_W {
        let range = &samples[(x as usize * bucket).min(samples.len() - 1)
            ..((x as usize + 1) * bucket).min(samples.len())];
        let peak = range.iter().fold(0.0f32, |m, s| m.max(s.abs())).min(1.0);
        let half = ((peak * (WF_H as f32 / 2.0 - 1.0)) as u32).max(1);
        let mid = WF_H / 2;
        for y in (mid - half)..(mid + half) {
            img.put_pixel(x, y, image::Rgba([0x5d, 0xd3, 0x9e, 0xff]));
        }
    }
    img.save(&file)?;
    Ok(file)
}

/// Render a one-frame preview of a def instance into the cache; returns
/// the PNG path. Cached by def content hash. Runs its own short-lived GES
/// pipeline, so call it off the UI thread.
pub fn template_png(
    cache_dir: &Path,
    project: &crate::document::Project,
    name: &str,
    base_dir: &Path,
) -> Result<PathBuf> {
    use crate::document::{Clip, Element, Meta, Project, Scene};
    use ges::prelude::*;

    let def = project
        .defs
        .get(name)
        .with_context(|| format!("unknown def {name:?}"))?;
    let def_json = serde_json::to_string(def)?;
    let out = cache_dir.join(format!("tpl-{:016x}.png", fxhash(&def_json)));
    if out.exists() {
        return Ok(out);
    }
    std::fs::create_dir_all(cache_dir)?;

    // One-second scene holding the instance; params sample as their names.
    let args = def.params.iter().map(|p| (p.clone(), p.to_uppercase())).collect();
    let mini = Project {
        meta: Meta {
            title: name.into(),
            width: project.meta.width,
            height: project.meta.height,
            fps: project.meta.fps,
        },
        library: Vec::new(),
        defs: project.defs.clone(),
        scenes: vec![Scene {
            id: "tpl".into(),
            name: String::new(),
            duration: 1.0,
            transition: None,
            layers: vec![Clip {
                id: "inst".into(),
                start: 0.0,
                duration: 0.0,
                element: Element::CompRef { r#ref: name.into(), args },
                transform: Default::default(),
                animations: vec![],
                effects: vec![],
            }],
        }],
        overlays: vec![],
    };
    let compiled = crate::mapping::compile(&mini, base_dir)?;

    let pipeline = ges::Pipeline::new();
    pipeline.set_timeline(&compiled.timeline).map_err(|e| anyhow::anyhow!("{e}"))?;
    let sink = gst_app::AppSink::builder()
        .caps(
            &gst::Caps::builder("video/x-raw")
                .field("format", "RGBA")
                .field("width", 320i32)
                .field("height", 180i32)
                .build(),
        )
        .build();
    let bin = gst::Bin::new();
    let convert = gst::ElementFactory::make("videoconvert").build()?;
    let scale = gst::ElementFactory::make("videoscale").build()?;
    bin.add_many([&convert, &scale, sink.upcast_ref()])?;
    gst::Element::link_many([&convert, &scale, sink.upcast_ref()])?;
    let pad = convert.static_pad("sink").context("convert sink pad")?;
    bin.add_pad(&gst::GhostPad::with_target(&pad)?)?;
    pipeline.set_video_sink(Some(&bin));

    pipeline.set_state(gst::State::Paused)?;
    let (res, _, _) = pipeline.state(gst::ClockTime::from_seconds(5));
    res.map_err(|e| anyhow::anyhow!("preroll failed: {e}"))?;
    // Discard the t=0 preroll, then seek near the end so intro
    // animations have landed; the flush produces a fresh preroll.
    let _ = sink.try_pull_preroll(gst::ClockTime::from_seconds(5));
    pipeline.seek_simple(
        gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
        gst::ClockTime::from_mseconds(900),
    )?;
    let (res, _, _) = pipeline.state(gst::ClockTime::from_seconds(5));
    res.map_err(|e| anyhow::anyhow!("post-seek preroll failed: {e}"))?;
    let sample = sink
        .try_pull_preroll(gst::ClockTime::from_seconds(5))
        .context("no preroll sample")?;
    let buffer = sample.buffer().context("no buffer")?;
    let map = buffer.map_readable()?;
    let img: image::RgbaImage =
        image::ImageBuffer::from_raw(320, 180, map.as_slice().to_vec())
            .context("bad frame stride")?;
    let _ = pipeline.set_state(gst::State::Null);
    img.save(&out)?;
    Ok(out)
}
