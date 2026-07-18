//! First-frame thumbnails for media clips (feature = "preview").
//! Cached as small PNGs next to the shape cache.

use anyhow::{Context, Result};
use gst::prelude::*;
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
    loop {
        match sink.pull_sample() {
            Ok(sample) => {
                if let Some(buffer) = sample.buffer() {
                    let map = buffer.map_readable()?;
                    samples.extend(
                        map.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                    );
                }
            }
            Err(_) => break, // EOS or error ends the stream
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
