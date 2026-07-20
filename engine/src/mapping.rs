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
    compile_scaled(project, base_dir, 1.0)
}

/// Compile with the video track restricted to `scale` × the project
/// resolution — previews render faster at half/quarter size while
/// exports keep using [`compile`] at full quality.
pub fn compile_scaled(
    project: &Project,
    base_dir: &std::path::Path,
    scale: f64,
) -> Result<Compiled> {
    project.validate()?;
    // Track/lane mute/hide (#31, #21): applied non-destructively at
    // compile time by zeroing volume/opacity on the affected clips.
    // scene_lanes[i] applies to layer index i of every scene (scene
    // layers are positional, not identities, so there's no per-scene
    // lane state to track separately).
    let needs_adjust = project.overlays.iter().any(|t| t.muted || t.hidden)
        || project.scene_lanes.iter().any(|l| l.muted || l.hidden);
    let adjusted;
    let project = if needs_adjust {
        let mut p = project.clone();
        for track in &mut p.overlays {
            for clip in &mut track.clips {
                if track.hidden {
                    clip.transform.opacity = 0.0;
                }
                if track.muted
                    && let Element::Video { volume, .. } | Element::Audio { volume, .. } =
                        &mut clip.element
                {
                    *volume = 0.0;
                }
            }
        }
        for scene in &mut p.scenes {
            for (i, clip) in scene.layers.iter_mut().enumerate() {
                let Some(lane) = p.scene_lanes.get(i) else { continue };
                if lane.hidden {
                    clip.transform.opacity = 0.0;
                }
                if lane.muted
                    && let Element::Video { volume, .. } | Element::Audio { volume, .. } =
                        &mut clip.element
                {
                    *volume = 0.0;
                }
            }
        }
        adjusted = p;
        &adjusted
    } else {
        project
    };
    let timeline = ges::Timeline::new_audio_video();
    // Crossfades: scenes with a transition overlap their predecessor on the
    // same GES layers; auto-transition renders the blend.
    timeline.set_auto_transition(true);
    let mut warnings = Vec::new();

    // Restrict the video track to the project's frame size and rate so
    // render output matches meta instead of GES defaults.
    for track in timeline.tracks() {
        if let Ok(video_track) = track.clone().downcast::<ges::VideoTrack>() {
            let w = ((project.meta.width as f64 * scale) as i32).max(2) & !1;
            let h = ((project.meta.height as f64 * scale) as i32).max(2) & !1;
            let caps = gst::Caps::builder("video/x-raw")
                .field("width", w)
                .field("height", h)
                .field("framerate", gst::Fraction::new(project.meta.fps, 1))
                .build();
            video_track.set_restriction_caps(&caps);
        }
    }

    // Each "slot" (an overlay track, or a scene-layer index) gets as many
    // GES layers as its deepest def expansion needs: multi-layer defs put
    // each def layer on its own GES layer (same-layer full overlaps are
    // invalid in GES).
    let slot_depth =
        |clips: &[Clip]| -> usize { clips.iter().map(|c| clip_slot_depth(project, c)).max().unwrap_or(1) };

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
    retype_transitions(project, &timeline);
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
    // Keyframed rate ramp (#40): expand into static-rate sub-clips before
    // GES ever sees a rate animation -- see expand_rate_ramp's doc comment
    // for why a live rate binding isn't safe.
    if let Some(segments) = crate::document::expand_rate_ramp(clip) {
        for seg in &segments {
            add_clip(project, slot, seg, offset, base_dir, warnings)
                .with_context(|| format!("rate ramp segment {:?}", seg.id))?;
        }
        return Ok(());
    }
    let layer = &slot[0];
    let start = secs(offset + clip.start);
    let duration = secs(if clip.duration > 0.0 { clip.duration } else { 1.0 });

    let ges_clip: Option<ges::Clip> = match &clip.element {
        Element::Text { text, font, color, align, outline, shadow } => {
            let title = ges::TitleClip::new().context("title clip")?;
            title.set_start(start);
            title.set_duration(duration);
            layer.add_clip(&title)?;
            title.set_child_property("text", text.to_value())?;
            title.set_child_property("font-desc", font.to_value())?;
            title.set_child_property("color", parse_color(color).to_value())?;
            title.set_child_property("background", 0u32.to_value())?;
            // Rich text styling (#38); warn rather than fail if this GES
            // build lacks a property.
            if let Some(outline) = outline
                && let Err(e) =
                    title.set_child_property("outline-color", parse_color(outline).to_value())
            {
                warnings.push(format!("clip {:?}: outline unsupported: {e}", clip.id));
            }
            if *shadow
                && let Err(e) = title.set_child_property("shadow", true.to_value())
            {
                warnings.push(format!("clip {:?}: shadow unsupported: {e}", clip.id));
            }
            if let Some(align) = align {
                let name = match align {
                    crate::document::TextAlign::Left => "left",
                    crate::document::TextAlign::Center => "center",
                    crate::document::TextAlign::Right => "right",
                };
                title.set_child_property("halignment", name.to_value())?;
            }
            if clip.transform.x != 0.0 || clip.transform.y != 0.0 {
                if align.is_none() {
                    title.set_child_property("halignment", "absolute".to_value())?;
                }
                title.set_child_property("valignment", "absolute".to_value())?;
                title.set_child_property(
                    "xpos",
                    (clip.transform.x / project.meta.width as f64).to_value(),
                )?;
                title.set_child_property(
                    "ypos",
                    (clip.transform.y / project.meta.height as f64).to_value(),
                )?;
            }
            Some(title.upcast())
        }
        Element::Video { src, offset: inpoint, volume, rate }
        | Element::Audio { src, offset: inpoint, volume, rate } => {
            let uri = to_uri(src, base_dir)?;
            let media = ges::UriClip::new(&uri).with_context(|| format!("opening {src}"))?;
            media.set_start(start);
            media.set_inpoint(secs(*inpoint));
            media.set_duration(duration);
            if matches!(clip.element, Element::Audio { .. }) {
                media.set_supported_formats(ges::TrackType::AUDIO);
            }
            layer.add_clip(&media)?;
            // Speed (#32): GES auto-registers pitch/videorate rate
            // properties and drives nle's media consumption itself
            // (verified by reproducer; see issue).
            let animates_rate =
                clip.animations.iter().any(|a| a.property == AnimProperty::Rate);
            if (*rate - 1.0).abs() > f64::EPSILON || animates_rate {
                let pitch = ges::Effect::new(&format!("pitch name=dcrate tempo={rate}"))
                    .context("pitch effect (rate)")?;
                media.add(&pitch).context("adding pitch rate effect")?;
                if matches!(clip.element, Element::Video { .. }) {
                    let vrate = ges::Effect::new(&format!("videorate name=dcvrate rate={rate}"))
                        .context("videorate effect")?;
                    media.add(&vrate).context("adding videorate effect")?;
                }
            }
            // Volume rides an explicit effect; GES's own child-property
            // route scales values wrongly on this GStreamer (see ADR).
            let animates_volume =
                clip.animations.iter().any(|a| a.property == AnimProperty::Volume);
            if (*volume - 1.0).abs() > f64::EPSILON || animates_volume {
                let fx = ges::Effect::new(&format!("volume name=dcvol volume={volume}"))
                    .context("volume effect")?;
                media.add(&fx).context("adding volume effect")?;
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
            // Video only: the default audiotestsrc sine would drown real
            // audio in the mix.
            test.set_supported_formats(ges::TrackType::VIDEO);
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
            // Each def layer gets its own GES layer span within the slot;
            // nested comprefs consume a span as wide as their own depth.
            let mut di = 0usize;
            for sub in &def.layers {
                let mut sub = substitute(sub, args);
                sub.id = format!("{}/{}", clip.id, sub.id);
                sub.start += clip.start;
                if sub.duration <= 0.0 {
                    sub.duration = clip.duration - (sub.start - clip.start);
                }
                let sub_slot = &slot[di.min(slot.len() - 1)..];
                add_clip(project, sub_slot, &sub, offset, base_dir, warnings)
                    .with_context(|| format!("def {:?} layer {:?}", r#ref, sub.id))?;
                di += clip_slot_depth(project, &sub);
            }
            None
        }
    };

    if let Some(ges_clip) = ges_clip {
        apply_transform_and_animations(project, &ges_clip, clip, warnings)?;
        apply_effects(&ges_clip, clip, warnings);
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
            ges_clip.set_child_property("posx", (t.x as i32).to_value())?;
            ges_clip.set_child_property("posy", (t.y as i32).to_value())?;
        }
        if t.width > 0.0 {
            ges_clip.set_child_property("width", (t.width as i32).to_value())?;
        }
        if t.height > 0.0 {
            ges_clip.set_child_property("height", (t.height as i32).to_value())?;
        }
    }
    if (t.opacity - 1.0).abs() > f64::EPSILON {
        ges_clip.set_child_property("alpha", t.opacity.to_value())?;
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
            AnimProperty::Volume => "volume",
            AnimProperty::Rate => "tempo",
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
    if prop == "volume" {
        // Bind straight to the gst volume element inside the clip's volume
        // effect: raw values, no GES child-property scaling.
        let fx = ges_clip
            .children(false)
            .into_iter()
            .filter_map(|c| c.downcast::<ges::Effect>().ok())
            .find(|e| ges::prelude::TimelineElementExt::lookup_child(e, "volume").is_some())
            .context("no volume effect on clip")?;
        let bin = fx
            .nleobject()
            .downcast::<gst::Bin>()
            .map_err(|_| anyhow::anyhow!("effect nleobject is not a bin"))?;
        let vol = find_by_factory(&bin, "volume").context("no volume element in effect")?;
        let binding = gst_controller::DirectControlBinding::new_absolute(&vol, "volume", &source);
        vol.add_control_binding(&binding)?;
    } else if prop == "tempo" {
        // Keyframed speed ramps are not implemented: GES class-level
        // auto-registers pitch/videorate as *rate properties*, which
        // recomputes timeline timing synchronously whenever they change
        // (ges-timeline.c's edit-API thread-ownership assertion fires
        // when that happens from the streaming thread a GstController
        // callback runs on -- confirmed by direct reproduction, not
        // speculation). A safe ramp needs segmentation: split the clip
        // into several sub-clips, each holding a static tempo sampled
        // from the curve, rather than live-animating the property. See
        // the rate-ramp tracking issue for the design.
        anyhow::bail!(
            "keyframed rate/tempo animation is not yet supported (GES rate-property              live-binding is unsafe); use a constant clip.rate, or split the clip into              segments with different rates"
        );
    } else {
        element
            .set_control_source(&source, prop, "direct-absolute")
            .then_some(())
            .context("set_control_source failed")?;
    }
    Ok(())
}

/// Control sources take normalized doubles for some properties; GES
/// "direct-absolute" mode takes raw values, so only alpha needs clamping.
/// GES layers a clip needs: 1, or for comprefs the sum of its def's
/// layers' needs (validation rejects cycles).
fn clip_slot_depth(project: &Project, clip: &Clip) -> usize {
    match &clip.element {
        Element::CompRef { r#ref, .. } => project
            .defs
            .get(r#ref)
            .map_or(1, |d| d.layers.iter().map(|l| clip_slot_depth(project, l)).sum::<usize>().max(1)),
        _ => 1,
    }
}

/// Depth-first search for an element by factory name inside a bin.
fn find_by_factory(bin: &gst::Bin, factory: &str) -> Option<gst::Element> {
    bin.iterate_recurse()
        .into_iter()
        .flatten()
        .find(|child| child.factory().is_some_and(|f| f.name() == factory))
}

/// Attach the clip's effects chain. Runs after the clip joins a layer so
/// GES can create the effect track elements.
fn apply_effects(ges_clip: &ges::Clip, clip: &Clip, warnings: &mut Vec<String>) {
    for effect in &clip.effects {
        let desc = match effect {
            crate::document::Effect::Blur { amount } => format!("gaussianblur sigma={amount}"),
            crate::document::Effect::ChromaKey { color, angle, noise } => {
                let argb = crate::document::parse_color(color);
                let (r, g, b) = ((argb >> 16) & 0xff, (argb >> 8) & 0xff, argb & 0xff);
                format!(
                    "alpha method=custom target-r={r} target-g={g} target-b={b} angle={angle} noise-level={noise}"
                )
            }
            crate::document::Effect::Crop { left, right, top, bottom } => {
                format!("videocrop left={left} right={right} top={top} bottom={bottom}")
            }
            crate::document::Effect::Eq { low, mid, high } => {
                format!("equalizer-3bands band0={low} band1={mid} band2={high}")
            }
            crate::document::Effect::Compressor { threshold, ratio } => {
                format!("audiodynamic mode=compressor threshold={threshold} ratio={ratio}")
            }
            crate::document::Effect::Denoise { level } => {
                format!("webrtcdsp echo-cancel=false noise-suppression=true noise-suppression-level={level}")
            }
            crate::document::Effect::Color { brightness, contrast, saturation, hue } => format!(
                "videobalance brightness={brightness} contrast={contrast} saturation={saturation} hue={hue}"
            ),
        };
        let fx = ges::Effect::new(&desc);
        match fx {
            Ok(fx) => {
                if let Err(e) = ges_clip.add(&fx) {
                    warnings.push(format!("clip {:?}: effect {desc:?} not added: {e}", clip.id));
                }
            }
            Err(e) => warnings.push(format!("clip {:?}: effect {desc:?} unavailable: {e}", clip.id)),
        }
    }
}

/// GES auto-transitions are crossfades; retype the video transitions that
/// fall inside a scene boundary whose document transition asks for a wipe.
fn retype_transitions(project: &Project, timeline: &ges::Timeline) {
    use ges::VideoStandardTransitionType as V;
    let mut wanted: Vec<(f64, f64, V)> = Vec::new();
    for (i, scene) in project.scenes.iter().enumerate().skip(1) {
        if let Some(tr) = &scene.transition {
            let vtype = match tr.kind {
                crate::document::TransitionKind::Crossfade => continue,
                crate::document::TransitionKind::WipeLr => V::BarWipeLr,
                crate::document::TransitionKind::WipeTb => V::BarWipeTb,
                crate::document::TransitionKind::BoxWipe => V::BoxWipeTl,
                crate::document::TransitionKind::Iris => V::IrisRect,
                crate::document::TransitionKind::Clock => V::ClockCw12,
            };
            let start = project.scene_offset(i);
            wanted.push((start, start + tr.duration, vtype));
        }
    }
    if wanted.is_empty() {
        return;
    }
    for layer in timeline.layers() {
        for clip in layer.clips() {
            if let Ok(tclip) = clip.downcast::<ges::TransitionClip>() {
                let start = tclip.start().nseconds() as f64 / 1e9;
                let end = start + tclip.duration().nseconds() as f64 / 1e9;
                for (ws, we, vtype) in &wanted {
                    // Auto transitions sit exactly in the overlap window.
                    if start >= ws - 0.01 && end <= we + 0.01 {
                        tclip.set_property("vtype", vtype);
                    }
                }
            }
        }
    }
}

fn normalize(prop: &str, value: f64) -> f64 {
    match prop {
        "alpha" => value.clamp(0.0, 1.0),
        "volume" => value.max(0.0),
        _ => value,
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
        Element::Text { text, font, color, outline, .. } => {
            apply(text);
            apply(font);
            apply(color);
            if let Some(o) = outline {
                apply(o);
            }
        }
        Element::Video { src, .. } | Element::Audio { src, .. } | Element::Image { src } => {
            apply(src)
        }
        Element::Shape { fill, .. } => apply(fill),
        // Nested defs: pass substitutions through the inner instantiation's args.
        Element::CompRef { args: inner, .. } => {
            for v in inner.values_mut() {
                apply(v);
            }
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
