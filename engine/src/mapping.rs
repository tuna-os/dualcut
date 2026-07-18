//! Compile a `document::Project` into a GES timeline.
//!
//! GES layer stack (priority 0 = topmost):
//!   [overlay tracks..] then [scene layer slots..]
//! Scenes are laid out sequentially; scene-layer clips get their scene's
//! offset added. Overlay clips keep absolute times and freely cross cuts.
//!
//! Shapes are skipped with a warning until the Vello compositor lands (M3).

use crate::document::{parse_color, Anim, AnimProperty, Clip, Easing, Element, Project};
use anyhow::{bail, Context, Result};
use ges::prelude::*;
use gstreamer as gst;
use gstreamer_controller as gst_controller;
use gstreamer_editing_services as ges;
use std::collections::BTreeMap;

fn secs(t: f64) -> gst::ClockTime {
    gst::ClockTime::from_useconds((t.max(0.0) * 1_000_000.0) as u64)
}

pub struct Compiled {
    pub timeline: ges::Timeline,
    pub warnings: Vec<String>,
}

pub fn compile(project: &Project, base_dir: &std::path::Path) -> Result<Compiled> {
    project.validate()?;
    let timeline = ges::Timeline::new_audio_video();
    // Crossfades: scenes with a transition overlap their predecessor on the
    // same GES layers; auto-transition renders the blend.
    timeline.set_auto_transition(true);
    let mut warnings = Vec::new();

    // Restrict the video track to the project's frame size and rate so
    // render output matches meta instead of GES defaults.
    for track in timeline.tracks() {
        if let Ok(video_track) = track.clone().downcast::<ges::VideoTrack>() {
            let caps = gst::Caps::builder("video/x-raw")
                .field("width", project.meta.width)
                .field("height", project.meta.height)
                .field("framerate", gst::Fraction::new(project.meta.fps, 1))
                .build();
            video_track.set_restriction_caps(&caps);
        }
    }

    // Each "slot" (an overlay track, or a scene-layer index) gets as many
    // GES layers as its deepest def expansion needs: multi-layer defs put
    // each def layer on its own GES layer (same-layer full overlaps are
    // invalid in GES).
    let slot_depth = |clips: &[Clip]| -> usize {
        clips
            .iter()
            .map(|c| match &c.element {
                Element::CompRef { r#ref, .. } => {
                    project.defs.get(r#ref).map_or(1, |d| d.layers.len().max(1))
                }
                _ => 1,
            })
            .max()
            .unwrap_or(1)
    };

    let mut slots: Vec<Vec<ges::Layer>> = Vec::new();
    let overlay_count = project.overlays.len();
    for track in &project.overlays {
        let depth = slot_depth(&track.clips);
        slots.push((0..depth).map(|_| timeline.append_layer()).collect());
    }
    let max_scene_layers = project.scenes.iter().map(|s| s.layers.len()).max().unwrap_or(0);
    for li in 0..max_scene_layers.max(1) {
        let depth = project
            .scenes
            .iter()
            .filter_map(|s| s.layers.get(li).map(|c| slot_depth(std::slice::from_ref(c))))
            .max()
            .unwrap_or(1);
        slots.push((0..depth).map(|_| timeline.append_layer()).collect());
    }

    // Overlay tracks first (top of the stack), absolute timing.
    for (i, track) in project.overlays.iter().enumerate() {
        for clip in &track.clips {
            add_clip(project, &slots[i], clip, 0.0, base_dir, &mut warnings)
                .with_context(|| format!("overlay clip {:?}", clip.id))?;
        }
    }

    // Scenes: sequential offsets, layers below the overlays.
    for (index, scene) in project.scenes.iter().enumerate() {
        let offset = project.scene_offset(index);
        for (li, clip) in scene.layers.iter().enumerate() {
            let mut clip = clip.clone();
            if clip.duration <= 0.0 {
                clip.duration = (scene.duration - clip.start).max(0.1);
            }
            add_clip(project, &slots[overlay_count + li], &clip, offset, base_dir, &mut warnings)
                .with_context(|| format!("scene {:?} clip {:?}", scene.id, clip.id))?;
        }
    }

    timeline.commit_sync();
    Ok(Compiled { timeline, warnings })
}

fn add_clip(
    project: &Project,
    slot: &[ges::Layer],
    clip: &Clip,
    offset: f64,
    base_dir: &std::path::Path,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let layer = &slot[0];
    let start = secs(offset + clip.start);
    let duration = secs(if clip.duration > 0.0 { clip.duration } else { 1.0 });

    let ges_clip: Option<ges::Clip> = match &clip.element {
        Element::Text { text, font, color } => {
            let title = ges::TitleClip::new().context("title clip")?;
            title.set_start(start);
            title.set_duration(duration);
            layer.add_clip(&title)?;
            title.set_child_property("text", &text.to_value())?;
            title.set_child_property("font-desc", &font.to_value())?;
            title.set_child_property("color", &parse_color(color).to_value())?;
            title.set_child_property("background", &0u32.to_value())?;
            if clip.transform.x != 0.0 || clip.transform.y != 0.0 {
                title.set_child_property("halignment", &"absolute".to_value())?;
                title.set_child_property("valignment", &"absolute".to_value())?;
                title.set_child_property(
                    "xpos",
                    &(clip.transform.x / project.meta.width as f64).to_value(),
                )?;
                title.set_child_property(
                    "ypos",
                    &(clip.transform.y / project.meta.height as f64).to_value(),
                )?;
            }
            Some(title.upcast())
        }
        Element::Video { src, offset: inpoint, volume }
        | Element::Audio { src, offset: inpoint, volume } => {
            let uri = to_uri(src, base_dir)?;
            let media = ges::UriClip::new(&uri).with_context(|| format!("opening {src}"))?;
            media.set_start(start);
            media.set_inpoint(secs(*inpoint));
            media.set_duration(duration);
            if matches!(clip.element, Element::Audio { .. }) {
                media.set_supported_formats(ges::TrackType::AUDIO);
            }
            layer.add_clip(&media)?;
            if (*volume - 1.0).abs() > f64::EPSILON {
                let _ = media.set_child_property("volume", &volume.to_value());
            }
            Some(media.upcast())
        }
        Element::Image { src } => {
            let uri = to_uri(src, base_dir)?;
            let media = ges::UriClip::new(&uri).with_context(|| format!("opening {src}"))?;
            media.set_start(start);
            media.set_duration(duration);
            layer.add_clip(&media)?;
            Some(media.upcast())
        }
        Element::Test {} => {
            let test = ges::TestClip::new().context("test clip")?;
            test.set_start(start);
            test.set_duration(duration);
            test.set_vpattern(ges::VideoTestPattern::Smpte);
            layer.add_clip(&test)?;
            Some(test.upcast())
        }
        Element::Shape { shape, fill } => {
            #[cfg(feature = "vector")]
            {
                let w = if clip.transform.width > 0.0 { clip.transform.width } else { 200.0 } as u32;
                let h = if clip.transform.height > 0.0 { clip.transform.height } else { 200.0 } as u32;
                let cache = base_dir.join(".dualcut-cache");
                match crate::vector::shape_png(&cache, *shape, fill, w, h) {
                    Ok(png) => {
                        let uri = format!("file://{}", png.canonicalize()?.display());
                        let media = ges::UriClip::new(&uri).context("shape image clip")?;
                        media.set_start(start);
                        media.set_duration(duration);
                        layer.add_clip(&media)?;
                        Some(media.upcast())
                    }
                    Err(e) => {
                        warnings.push(format!("clip {:?}: shape render failed: {e}", clip.id));
                        None
                    }
                }
            }
            #[cfg(not(feature = "vector"))]
            {
                warnings.push(format!(
                    "clip {:?}: shape {:?} needs the \"vector\" feature",
                    clip.id, shape
                ));
                None
            }
        }
        Element::CompRef { r#ref, args } => {
            let def = project.defs.get(r#ref).expect("validated");
            for (di, sub) in def.layers.iter().enumerate() {
                let mut sub = substitute(sub, args);
                sub.id = format!("{}/{}", clip.id, sub.id);
                sub.start += clip.start;
                if sub.duration <= 0.0 {
                    sub.duration = clip.duration - (sub.start - clip.start);
                }
                // Each def layer gets its own GES layer within the slot.
                let sub_slot = &slot[di.min(slot.len() - 1)..];
                add_clip(project, sub_slot, &sub, offset, base_dir, warnings)
                    .with_context(|| format!("def {:?} layer {:?}", r#ref, sub.id))?;
            }
            None
        }
    };

    if let Some(ges_clip) = ges_clip {
        apply_transform_and_animations(project, &ges_clip, clip, warnings)?;
    }
    Ok(())
}

fn apply_transform_and_animations(
    _project: &Project,
    ges_clip: &ges::Clip,
    clip: &Clip,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let t = &clip.transform;
    let is_title = ges_clip.clone().downcast::<ges::TitleClip>().is_ok();
    // Titles position via xpos/ypos (handled at creation); other video
    // clips use the frame positioner's pixel-space properties.
    if !is_title {
        if t.x != 0.0 || t.y != 0.0 {
            ges_clip.set_child_property("posx", &(t.x as i32).to_value())?;
            ges_clip.set_child_property("posy", &(t.y as i32).to_value())?;
        }
        if t.width > 0.0 {
            ges_clip.set_child_property("width", &(t.width as i32).to_value())?;
        }
        if t.height > 0.0 {
            ges_clip.set_child_property("height", &(t.height as i32).to_value())?;
        }
    }
    if (t.opacity - 1.0).abs() > f64::EPSILON {
        ges_clip.set_child_property("alpha", &t.opacity.to_value())?;
    }

    // One control source per property: GES allows a single binding per
    // child property, so all of a property's animation windows merge into
    // one sampled track with hold points between and after windows.
    let mut by_prop: std::collections::BTreeMap<&str, Vec<&Anim>> = Default::default();
    for anim in &clip.animations {
        let prop = match anim.property {
            AnimProperty::Opacity => "alpha",
            AnimProperty::X => "posx",
            AnimProperty::Y => "posy",
            AnimProperty::Width => "width",
            AnimProperty::Height => "height",
        };
        by_prop.entry(prop).or_default().push(anim);
    }
    for (prop, mut anims) in by_prop {
        let sort_key = |a: &Anim| a.keyframes.first().map_or(a.start, |k| k.t);
        anims.sort_by(|a, b| sort_key(a).total_cmp(&sort_key(b)));
        if let Err(e) = apply_property_animations(ges_clip, clip, prop, &anims) {
            warnings.push(format!("clip {:?}: animations on {prop} skipped: {e}", clip.id));
        }
    }
    Ok(())
}

/// Bind all of one property's animation windows to a single interpolation
/// control source on the clip's matching track element. Hold points before,
/// between, and after windows prevent linear extrapolation drift.
fn apply_property_animations(
    ges_clip: &ges::Clip,
    clip: &Clip,
    prop: &str,
    anims: &[&Anim],
) -> Result<()> {
    let element = ges_clip
        .children(false)
        .into_iter()
        .filter_map(|c| c.downcast::<ges::TrackElement>().ok())
        .find(|te| ges::prelude::TimelineElementExt::lookup_child(te, prop).is_some())
        .with_context(|| format!("no track element exposes {prop:?}"))?;

    let cs = gst_controller::InterpolationControlSource::new();
    cs.set_property("mode", gst_controller::InterpolationMode::Linear);
    let set = |time: f64, value: f64| {
        <gst_controller::InterpolationControlSource as gst_controller::prelude::TimedValueControlSourceExt>::set(
            &cs,
            secs(time),
            normalize(prop, value),
        );
    };

    let inpoint = element.inpoint().nseconds() as f64 / 1e9;
    let first_value = |a: &Anim| a.keyframes.first().map_or(a.from, |k| k.value);
    let last_of = |a: &Anim| a.keyframes.last().map_or((a.end, a.to), |k| (k.t, k.value));
    // Hold the first animation's initial value from the clip's beginning.
    set(inpoint, first_value(anims[0]));
    for anim in anims {
        if anim.keyframes.is_empty() {
            // Tween window: sample eased values densely; the control source
            // interpolates linearly between samples, so curves survive.
            let steps = 24.max(((anim.end - anim.start) * 30.0) as usize);
            for i in 0..=steps {
                let p = i as f64 / steps as f64;
                let value = anim.from + (anim.to - anim.from) * ease(anim.easing, p);
                set(inpoint + anim.start + (anim.end - anim.start) * p, value);
            }
        } else {
            // Keyframes: sample each eased segment between neighbours.
            set(inpoint + anim.keyframes[0].t, anim.keyframes[0].value);
            for pair in anim.keyframes.windows(2) {
                let (a, b) = (&pair[0], &pair[1]);
                let steps = 12.max(((b.t - a.t) * 30.0) as usize);
                for i in 1..=steps {
                    let p = i as f64 / steps as f64;
                    let value = a.value + (b.value - a.value) * ease(b.easing, p);
                    set(inpoint + a.t + (b.t - a.t) * p, value);
                }
            }
        }
    }
    // Hold the last value to the end of the clip so linear mode never
    // extrapolates past the final segment.
    let (last_t, last_v) = last_of(anims.last().unwrap());
    let clip_end = inpoint + clip.duration.max(last_t) + 1.0;
    set(clip_end, last_v);

    let source: gst::ControlSource = cs.upcast();
    element
        .set_control_source(&source, prop, "direct-absolute")
        .then_some(())
        .context("set_control_source failed")?;
    Ok(())
}

/// Control sources take normalized doubles for some properties; GES
/// "direct-absolute" mode takes raw values, so only alpha needs clamping.
fn normalize(prop: &str, value: f64) -> f64 {
    if prop == "alpha" {
        value.clamp(0.0, 1.0)
    } else {
        value
    }
}

fn ease(easing: Easing, t: f64) -> f64 {
    match easing {
        Easing::Linear => t,
        Easing::EaseIn => t * t * t,
        Easing::EaseOut => 1.0 - (1.0 - t).powi(3),
        Easing::EaseInOut => {
            if t < 0.5 {
                4.0 * t * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
            }
        }
    }
}

fn substitute(clip: &Clip, args: &BTreeMap<String, String>) -> Clip {
    let mut clip = clip.clone();
    let apply = |s: &mut String| {
        for (k, v) in args {
            *s = s.replace(&format!("{{{k}}}"), v);
        }
    };
    match &mut clip.element {
        Element::Text { text, font, color } => {
            apply(text);
            apply(font);
            apply(color);
        }
        Element::Video { src, .. } | Element::Audio { src, .. } | Element::Image { src } => {
            apply(src)
        }
        _ => {}
    }
    clip
}

fn to_uri(src: &str, base_dir: &std::path::Path) -> Result<String> {
    if src.contains("://") {
        return Ok(src.to_string());
    }
    let path = base_dir.join(src);
    if !path.exists() {
        bail!("media file not found: {}", path.display());
    }
    Ok(format!("file://{}", path.canonicalize()?.display()))
}
