//! Headless GES render.
//!
//! Usage:
//!   render <project.json> <output.mp4>     render a document (M1 path)
//!   render <output.mp4> [media-uri]        render the built-in M0 demo

use anyhow::{Context, Result};
use dualcut_engine::{build_demo_timeline, encoding_profile, init, run_to_eos};
use dualcut_engine::{document::Project, mapping};
use ges::prelude::*;
use gstreamer as gst;
use gstreamer_editing_services as ges;

fn main() -> Result<()> {
    init()?;

    let mut args = std::env::args().skip(1);
    let first = args.next().unwrap_or_else(|| "out.mp4".into());

    // `render new <project.json> [title]` scaffolds a starter project.
    if first == "new" {
        let path = args.next().unwrap_or_else(|| "project.json".into());
        let title = args.next().unwrap_or_else(|| "Untitled".into());
        let project = dualcut_engine::templates::new_project(&title);
        std::fs::write(&path, project.to_json()).with_context(|| format!("writing {path}"))?;
        println!("scaffolded {path} ({title:?}, {} starter templates)", project.defs.len());
        return Ok(());
    }

    let (timeline, out) = if first.ends_with(".json") {
        let out = args.next().unwrap_or_else(|| "out.mp4".into());
        let json = std::fs::read_to_string(&first).with_context(|| format!("reading {first}"))?;
        let project = Project::from_json(&json)?;
        let base_dir = std::path::Path::new(&first)
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf();
        let compiled = mapping::compile(&project, &base_dir)?;
        for warning in &compiled.warnings {
            eprintln!("warning: {warning}");
        }
        println!(
            "project {:?}: {} scene(s), {} overlay track(s), {:.1}s",
            project.meta.title,
            project.scenes.len(),
            project.overlays.len(),
            project.duration()
        );
        (compiled.timeline, out)
    } else {
        let media_uri = args.next();
        (build_demo_timeline(media_uri.as_deref())?, first)
    };

    let pipeline = ges::Pipeline::new();
    pipeline.set_timeline(&timeline).context("attaching timeline")?;

    let out_abs = std::path::absolute(&out)?;
    let uri = format!("file://{}", out_abs.display());
    pipeline
        .set_render_settings(&uri, &encoding_profile(&out)?)
        .context("setting render settings")?;
    pipeline
        .set_mode(ges::PipelineFlags::RENDER)
        .context("setting render mode")?;

    println!("rendering -> {}", out_abs.display());
    let start = std::time::Instant::now();
    run_to_eos(&pipeline)?;
    println!("done in {:.1?}", start.elapsed());
    Ok(())
}
