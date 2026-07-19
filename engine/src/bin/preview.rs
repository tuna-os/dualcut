//! dualcut editor (M2): GNOME/libadwaita app around a project document.
//!
//! - Preview through GES + gtk4paintablesink, transport + seek.
//! - Scene strip (widths ∝ duration, click to jump) + overlay track rows.
//! - Inspector sidebar: select any clip, edit its parameters; every edit
//!   writes the project JSON to disk — the document stays the single
//!   source of truth, so agents and $EDITOR see the same file.
//! - Live reload on external edits (mtime poll), undo/redo (Ctrl+Z/+Shift)
//!   as document snapshots, detach-audio op on video clips.
//!
//! Usage: preview [project.json | media-uri]

use anyhow::{Context, Result};
use dualcut_engine::{build_demo_timeline, document, document::Project, init, mapping};
use document::{detach_audio, find_clip, find_clip_mut, move_clip_to_lane, remove_clip, save_as_def};
use ges::prelude::*;
use gstreamer as gst;
use gstreamer_editing_services as ges;
use gtk::glib;
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::SystemTime;

const SCENE_PX_PER_SEC: f64 = 42.0;

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id("io.github.hanthor.Dualcut")
        // Skip D-Bus name acquisition: dev/CI environments may have a
        // broken or cross-namespace session bus, and single-instance
        // behavior is not needed for an editor.
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(|app| {
        if let Err(e) = build_ui(app) {
            eprintln!("error: {e:#}");
            app.quit();
        }
    });
    app.run_with_args::<&str>(&[])
}

struct AppState {
    pipeline: ges::Pipeline,
    project: Option<Project>,
    project_path: Option<PathBuf>,
    mtime: Option<SystemTime>,
    duration: f64,
    selected: Option<String>,
    undo: Vec<String>,
    redo: Vec<String>,
    /// Set while the app itself writes the file, to skip one reload cycle.
    self_write: bool,
}

type Shared = Rc<RefCell<AppState>>;

fn make_pipeline(timeline: &ges::Timeline) -> Result<(ges::Pipeline, gtk::gdk::Paintable)> {
    let pipeline = ges::Pipeline::new();
    pipeline.set_timeline(timeline).context("attaching timeline")?;
    let sink = gst::ElementFactory::make("gtk4paintablesink")
        .build()
        .context("creating gtk4paintablesink")?;
    let paintable = sink.property::<gtk::gdk::Paintable>("paintable");
    let video_sink: gst::Element = match gst::ElementFactory::make("glsinkbin")
        .property("sink", &sink)
        .build()
    {
        Ok(glsink) => glsink,
        Err(_) => sink.clone(),
    };
    pipeline.preview_set_video_sink(Some(&video_sink));
    Ok((pipeline, paintable))
}

fn start_paused(pipeline: &ges::Pipeline) -> Result<()> {
    if pipeline.set_state(gst::State::Paused).is_err() {
        let _ = pipeline.set_state(gst::State::Null);
        if let Ok(fake) = gst::ElementFactory::make("fakesink").build() {
            pipeline.preview_set_audio_sink(Some(&fake));
        }
        pipeline.set_state(gst::State::Paused).context("pausing pipeline")?;
    }
    Ok(())
}

fn compile_project(project: &Project, base_dir: &std::path::Path) -> Result<ges::Timeline> {
    let compiled = mapping::compile(project, base_dir)?;
    for warning in &compiled.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(compiled.timeline)
}

fn seek_to(pipeline: &ges::Pipeline, secs: f64) {
    let _ = pipeline.seek_simple(
        gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
        gst::ClockTime::from_useconds((secs.max(0.0) * 1e6) as u64),
    );
}

/// Widgets the controller refreshes when the document changes.
struct Ui {
    picture: gtk::Picture,
    seek: gtk::Scale,
    strip: gtk::Box,
    inspector: gtk::Box,
    media_grid: gtk::FlowBox,
    media_empty: gtk::Box,
    toasts: adw::ToastOverlay,
    templates_list: gtk::ListBox,
    code_buffer: gtk::TextBuffer,
}

struct Editor {
    state: Shared,
    ui: RefCell<Option<Ui>>,
}

impl Editor {
    fn base_dir(&self) -> PathBuf {
        self.state
            .borrow()
            .project_path
            .as_ref()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Persist the document, push an undo snapshot, rebuild everything.
    fn commit_document(self: &Rc<Self>, project: Project) {
        let (path, prev_json) = {
            let st = self.state.borrow();
            let prev = st.project.as_ref().map(|p| p.to_json());
            (st.project_path.clone(), prev)
        };
        if let Some(prev) = prev_json {
            let mut st = self.state.borrow_mut();
            st.undo.push(prev);
            st.redo.clear();
        }
        match path {
            Some(path) => self.write_and_rebuild(&path, project),
            // Unsaved project: keep everything in memory until Save As.
            None => self.rebuild_in_memory(project),
        }
    }

    fn rebuild_in_memory(self: &Rc<Self>, project: Project) {
        match compile_project(&project, &self.base_dir()) {
            Ok(timeline) => {
                {
                    let st = self.state.borrow();
                    let _ = st.pipeline.set_timeline(&timeline);
                }
                self.state.borrow_mut().project = Some(project);
                self.rebuild_strip();
                self.rebuild_inspector();
                self.rebuild_media();
                self.rebuild_templates();
                self.refresh_code();
            }
            Err(e) => eprintln!("rebuild failed (keeping current timeline): {e:#}"),
        }
    }

    /// Save As: pick a path, write, adopt it for future auto-saves.
    fn save_project_as(self: &Rc<Self>, window: Option<&gtk::Window>) {
        let dialog = gtk::FileDialog::builder().title("Save project").build();
        dialog.set_initial_name(Some("project.json"));
        let this = self.clone();
        dialog.save(window, gtk::gio::Cancellable::NONE, move |res| {
            if let Ok(file) = res
                && let Some(path) = file.path() {
                    let project = this.state.borrow().project.clone();
                    if let Some(project) = project {
                        this.state.borrow_mut().project_path = Some(path.clone());
                        remember_recent(&path);
                        this.write_and_rebuild(&path, project);
                        if let Some(win) = this.window() {
                            win.set_title(Some(&format!(
                                "dualcut — {}",
                                path.file_name().and_then(|n| n.to_str()).unwrap_or("project")
                            )));
                        }
                    }
                }
        });
    }

    /// Transient "X deleted — Undo" toast (deletes are silent otherwise).
    fn toast_undo(self: &Rc<Self>, message: &str) {
        let ui = self.ui.borrow();
        let Some(ui) = ui.as_ref() else { return };
        let toast = adw::Toast::new(message);
        toast.set_button_label(Some("Undo"));
        toast.set_timeout(5);
        let this = self.clone();
        toast.connect_button_clicked(move |_| this.undo());
        ui.toasts.add_toast(toast);
    }

    fn window(&self) -> Option<gtk::Window> {
        let ui = self.ui.borrow();
        ui.as_ref().and_then(|u| u.picture.root().and_downcast::<gtk::Window>())
    }

    /// Open a project file, replacing the current session state.
    fn open_project(self: &Rc<Self>, path: &std::path::Path) {
        match std::fs::read_to_string(path).map_err(anyhow::Error::from).and_then(|json| Project::from_json(&json)) {
            Ok(project) => {
                {
                    let mut st = self.state.borrow_mut();
                    st.project_path = Some(path.to_path_buf());
                    st.undo.clear();
                    st.redo.clear();
                    st.selected = None;
                }
                remember_recent(path);
                if let Some(win) = self.window() {
                    win.set_title(Some(&format!(
                        "dualcut — {}",
                        path.file_name().and_then(|n| n.to_str()).unwrap_or("project")
                    )));
                }
                self.rebuild_in_memory(project);
            }
            Err(e) => eprintln!("open failed: {e:#}"),
        }
    }

    /// Add file paths to the project library (#15); shared by the Import
    /// dialog and window drag-and-drop.
    fn add_to_library(self: &Rc<Self>, paths: &[PathBuf]) {
        let project = self.state.borrow().project.clone();
        let Some(mut project) = project else { return };
        let base = self.base_dir();
        for path in paths {
            let entry = path
                .strip_prefix(&base)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| path.display().to_string());
            if !project.library.contains(&entry) {
                project.library.push(entry);
            }
        }
        self.commit_document(project);
    }

    /// Import media files into the project library (#15).
    fn import_media(self: &Rc<Self>, window: Option<&gtk::Window>) {
        let dialog = gtk::FileDialog::builder().title("Import media").build();
        let this = self.clone();
        dialog.open_multiple(window, gtk::gio::Cancellable::NONE, move |res| {
            let Ok(files) = res else { return };
            let paths: Vec<PathBuf> = (0..files.n_items())
                .filter_map(|i| files.item(i).and_downcast::<gtk::gio::File>()?.path())
                .collect();
            this.add_to_library(&paths);
        });
    }

    fn write_and_rebuild(self: &Rc<Self>, path: &std::path::Path, project: Project) {
        self.state.borrow_mut().self_write = true;
        if let Err(e) = std::fs::write(path, project.to_json()) {
            eprintln!("saving project failed: {e}");
            return;
        }
        {
            let mut st = self.state.borrow_mut();
            st.mtime = path.metadata().ok().and_then(|m| m.modified().ok());
            st.project = Some(project);
        }
        self.rebuild();
    }

    fn undo(self: &Rc<Self>) {
        let (path, snapshot) = {
            let mut st = self.state.borrow_mut();
            let Some(path) = st.project_path.clone() else { return };
            let Some(snapshot) = st.undo.pop() else { return };
            if let Some(cur) = st.project.as_ref() {
                let json = cur.to_json();
                st.redo.push(json);
            }
            (path, snapshot)
        };
        if let Ok(project) = Project::from_json(&snapshot) {
            self.write_and_rebuild(&path, project);
        }
    }

    fn redo(self: &Rc<Self>) {
        let (path, snapshot) = {
            let mut st = self.state.borrow_mut();
            let Some(path) = st.project_path.clone() else { return };
            let Some(snapshot) = st.redo.pop() else { return };
            if let Some(cur) = st.project.as_ref() {
                let json = cur.to_json();
                st.undo.push(json);
            }
            (path, snapshot)
        };
        if let Ok(project) = Project::from_json(&snapshot) {
            self.write_and_rebuild(&path, project);
        }
    }

    /// Rebuild pipeline + strip + inspector from the current document.
    fn rebuild(self: &Rc<Self>) {
        let (project, base_dir) = {
            let st = self.state.borrow();
            (st.project.clone(), self.base_dir())
        };
        let Some(project) = project else { return };
        match compile_project(&project, &base_dir).and_then(|tl| make_pipeline(&tl)) {
            Ok((pipeline, paintable)) => {
                {
                    let ui = self.ui.borrow();
                    let Some(ui) = ui.as_ref() else { return };
                    let old_pos = {
                        let st = self.state.borrow();
                        let pos = st.pipeline.query_position::<gst::ClockTime>();
                        let _ = st.pipeline.set_state(gst::State::Null);
                        pos
                    };
                    ui.picture.set_paintable(Some(&paintable));
                    let duration = project.duration();
                    {
                        let mut st = self.state.borrow_mut();
                        st.pipeline = pipeline;
                        st.duration = duration;
                    }
                    ui.seek.set_range(0.0, duration.max(0.1));
                    let st = self.state.borrow();
                    let _ = start_paused(&st.pipeline);
                    if let Some(pos) = old_pos {
                        let max = gst::ClockTime::from_useconds((duration * 1e6) as u64);
                        seek_to(&st.pipeline, (pos.min(max)).nseconds() as f64 / 1e9);
                    }
                }
                self.rebuild_strip();
                self.rebuild_inspector();
                self.rebuild_media();
                self.rebuild_templates();
                self.refresh_code();
            }
            Err(e) => eprintln!("rebuild failed (keeping current timeline): {e:#}"),
        }
    }

    fn rebuild_strip(self: &Rc<Self>) {
        let ui = self.ui.borrow();
        let Some(ui) = ui.as_ref() else { return };
        while let Some(child) = ui.strip.first_child() {
            ui.strip.remove(&child);
        }
        let project = {
            let st = self.state.borrow();
            st.project.clone()
        };
        let Some(project) = project else { return };
        let cache = self.base_dir().join(".dualcut-cache");
        self.spawn_thumbnail_worker(&project, &cache);

        let scene_row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        for (i, scene) in project.scenes.iter().enumerate() {
            let label = if scene.name.is_empty() { &scene.id } else { &scene.name };
            let button = gtk::Button::with_label(&format!("{label}\n{:.1}s", scene.duration));
            button.set_size_request((scene.duration * SCENE_PX_PER_SEC) as i32, 48);
            button.set_tooltip_text(Some(&scene.id));
            if let Some(thumb) = scene_thumb(&project, scene, &cache, &self.base_dir()) {
                let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                let pic = gtk::Picture::for_filename(&thumb);
                pic.set_size_request(64, 36);
                content.append(&pic);
                let lbl = gtk::Label::new(Some(&format!("{label}\n{:.1}s", scene.duration)));
                content.append(&lbl);
                button.set_child(Some(&content));
            }
            let offset = project.scene_offset(i);
            let this = self.clone();
            let scene_id = scene.id.clone();
            button.connect_clicked(move |_| {
                seek_to(&this.state.borrow().pipeline, offset);
                this.state.borrow_mut().selected = Some(format!("scene:{scene_id}"));
                this.rebuild_inspector();
            });

            // Scene cell: block + reorder arrows.
            let cell = gtk::Box::new(gtk::Orientation::Vertical, 2);
            cell.append(&button);
            let arrows = gtk::Box::new(gtk::Orientation::Horizontal, 2);
            arrows.set_halign(gtk::Align::Center);
            for (glyph, delta) in [("‹", -1i64), ("›", 1i64)] {
                let target = i as i64 + delta;
                if target < 0 || target >= project.scenes.len() as i64 {
                    continue;
                }
                let b = gtk::Button::with_label(glyph);
                b.add_css_class("flat");
                b.set_tooltip_text(Some("Reorder scene"));
                b.update_property(&[gtk::accessible::Property::Label(
                    if delta < 0 { "Move scene earlier" } else { "Move scene later" },
                )]);
                let this = self.clone();
                let project_snapshot = project.clone();
                b.connect_clicked(move |_| {
                    let mut project = project_snapshot.clone();
                    project.scenes.swap(i, target as usize);
                    this.commit_document(project);
                });
                arrows.append(&b);
            }
            cell.append(&arrows);
            scene_row.append(&cell);
        }
        ui.strip.append(&scene_row);

        // Scene-layer lanes: one row per layer slot, clips positioned at
        // absolute time (scene offset + clip start). Dragging retimes the
        // clip within its scene; right-edge drag trims duration.
        let max_layers = project.scenes.iter().map(|s| s.layers.len()).max().unwrap_or(0);
        for li in 0..max_layers {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let tag = gtk::Label::new(Some(&format!("layer {li}")));
            tag.add_css_class("dim-label");
            tag.set_width_chars(14);
            row.append(&tag);
            let lane = gtk::Fixed::new();
            lane.set_size_request((project.duration() * SCENE_PX_PER_SEC) as i32 + 40, 30);
            for (si, scene) in project.scenes.iter().enumerate() {
                let Some(clip) = scene.layers.get(li) else { continue };
                let offset = project.scene_offset(si);
                let duration = if clip.duration > 0.0 {
                    clip.duration
                } else {
                    (scene.duration - clip.start).max(0.1)
                };
                let scene_off = offset;
                let scene_dur = scene.duration;
                self.add_lane_clip(
                    &lane,
                    clip,
                    offset + clip.start,
                    duration,
                    Rc::new(move |raw_abs: f64| (raw_abs - scene_off).clamp(0.0, scene_dur - 0.1)),
                    true,
                    li,
                );
            }
            row.append(&lane);
            ui.strip.append(&row);
        }

        for (ti, track) in project.overlays.iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let name = if track.name.is_empty() { &track.id } else { &track.name };
            let tag = gtk::Label::new(Some(&format!("〜 {name}")));
            tag.add_css_class("dim-label");
            tag.set_width_chars(14);
            row.append(&tag);
            let lane = gtk::Fixed::new();
            lane.set_size_request((project.duration() * SCENE_PX_PER_SEC) as i32 + 40, 30);
            for clip in &track.clips {
                self.add_lane_clip(
                    &lane,
                    clip,
                    clip.start,
                    clip.duration,
                    Rc::new(|raw_abs: f64| raw_abs.max(0.0)),
                    false,
                    max_layers + ti,
                );
            }
            row.append(&lane);
            ui.strip.append(&row);
        }
    }

    /// Shared lane clip: click selects+seeks, body drag retimes (snapped,
    /// `to_start` maps absolute lane seconds to the stored start), edge
    /// drag (last 12px) trims duration.
    #[allow(clippy::too_many_arguments)]
    fn add_lane_clip(
        self: &Rc<Self>,
        lane: &gtk::Fixed,
        clip: &document::Clip,
        abs_start: f64,
        duration: f64,
        to_start: Rc<dyn Fn(f64) -> f64>,
        scene_relative: bool,
        lane_index: usize,
    ) {
        let project = {
            let st = self.state.borrow();
            st.project.clone()
        };
        let Some(project) = project else { return };
        let cache = self.base_dir().join(".dualcut-cache");

        let button = gtk::Button::with_label(&clip.id);
        button.add_css_class("flat");
        button.add_css_class(match &clip.element {
            document::Element::Text { .. } => "clip-text",
            document::Element::Video { .. } => "clip-video",
            document::Element::Audio { .. } => "clip-audio",
            document::Element::Image { .. } => "clip-image",
            document::Element::Shape { .. } => "clip-shape",
            document::Element::CompRef { .. } => "clip-compref",
            document::Element::Test { .. } => "clip-test",
        });
        let width_px = ((duration * SCENE_PX_PER_SEC) as i32).max(30);
        button.set_size_request(width_px, 28);
        button.set_tooltip_text(Some("Drag to move · right edge trims"));
        if let document::Element::Audio { src, .. } = &clip.element
            && let Some(uri) = media_uri(src, &self.base_dir()) {
                let wave = cache.join(format!("wave-{:016x}.png", fx_hash(&uri)));
                if wave.exists() {
                    let pic = gtk::Picture::for_filename(&wave);
                    pic.set_content_fit(gtk::ContentFit::Fill);
                    button.set_child(Some(&pic));
                    button.set_tooltip_text(Some(&clip.id));
                }
            }
        lane.put(&button, abs_start * SCENE_PX_PER_SEC, 1.0);

        {
            let this = self.clone();
            let id = clip.id.clone();
            button.connect_clicked(move |_| {
                {
                    let mut st = this.state.borrow_mut();
                    seek_to(&st.pipeline, abs_start);
                    st.selected = Some(id.clone());
                }
                this.rebuild_inspector();
            });
        }

        let drag = gtk::GestureDrag::new();
        // (orig_x_px, trim_mode)
        let dragmeta = Rc::new(std::cell::Cell::new((abs_start * SCENE_PX_PER_SEC, false)));
        {
            let dragmeta = dragmeta.clone();
            drag.connect_drag_begin(move |_, sx, _| {
                let trim = sx > (width_px as f64) - 12.0;
                dragmeta.set((abs_start * SCENE_PX_PER_SEC, trim));
            });
        }
        {
            let lane = lane.clone();
            let button = button.clone();
            let dragmeta = dragmeta.clone();
            drag.connect_drag_update(move |_, dx, _| {
                let (ox, trim) = dragmeta.get();
                if trim {
                    button.set_size_request(((width_px as f64 + dx) as i32).max(20), 28);
                } else {
                    lane.move_(&button, (ox + dx).max(0.0), 1.0);
                }
            });
        }
        {
            let this = self.clone();
            let id = clip.id.clone();
            let dragmeta = dragmeta.clone();
            let project_snapshot = project.clone();
            drag.connect_drag_end(move |_, dx, dy| {
                if dx.abs() < 2.0 && dy.abs() < 2.0 {
                    return;
                }
                let (ox, trim) = dragmeta.get();
                let mut project = project_snapshot.clone();
                // Vertical drag beyond one row height = move to another lane.
                const LANE_H: f64 = 36.0;
                let lane_delta = (dy / LANE_H).round() as i64;
                if !trim && lane_delta != 0 {
                    let target = (lane_index as i64 + lane_delta)
                        .clamp(0, document::lane_count(&project) as i64 - 1)
                        as usize;
                    let raw_abs = ((ox + dx).max(0.0)) / SCENE_PX_PER_SEC;
                    let snapped = snap_time(&project, raw_abs);
                    match move_clip_to_lane(&mut project, &id, target, snapped) {
                        Ok(()) => this.commit_document(project),
                        Err(e) => eprintln!("move to lane {target}: {e}"),
                    }
                    return;
                }
                if trim {
                    let new_dur = (duration + dx / SCENE_PX_PER_SEC).max(0.1);
                    let snapped_end = if scene_relative {
                        new_dur
                    } else {
                        snap_time(&project, abs_start + new_dur) - abs_start
                    };
                    if let Some(c) = find_clip_mut(&mut project, &id) {
                        c.duration = snapped_end.max(0.1);
                    }
                } else {
                    let raw_abs = ((ox + dx).max(0.0)) / SCENE_PX_PER_SEC;
                    let snapped = snap_time(&project, raw_abs);
                    let new_start = to_start(snapped);
                    if let Some(c) = find_clip_mut(&mut project, &id) {
                        c.start = new_start;
                    }
                }
                this.commit_document(project);
            });
        }
        button.add_controller(drag);
    }

    /// Generate any missing media thumbnails off-thread, then refresh the
    /// strip once so they appear.
    fn spawn_thumbnail_worker(self: &Rc<Self>, project: &Project, cache: &std::path::Path) {
        let base_dir = self.base_dir();
        let mut thumbs: Vec<String> = Vec::new();
        let mut waves: Vec<String> = Vec::new();
        let all_clips = project
            .scenes
            .iter()
            .flat_map(|s| s.layers.iter())
            .chain(project.overlays.iter().flat_map(|t| t.clips.iter()));
        for rel in &project.library {
            if let Some(uri) = media_uri(rel, &base_dir) {
                let is_audio = matches!(
                    rel.rsplit(".").next().unwrap_or("").to_lowercase().as_str(),
                    "ogg" | "mp3" | "wav" | "flac"
                );
                if is_audio {
                    if !cache.join(format!("wave-{:016x}.png", fx_hash(&uri))).exists() {
                        waves.push(uri);
                    }
                } else if !cache.join(format!("thumb-{:016x}.png", fx_hash(&uri))).exists() {
                    thumbs.push(uri);
                }
            }
        }
        for clip in all_clips {
            match &clip.element {
                document::Element::Video { src, .. } | document::Element::Image { src } => {
                    if let Some(uri) = media_uri(src, &base_dir)
                        && !cache.join(format!("thumb-{:016x}.png", fx_hash(&uri))).exists() {
                            thumbs.push(uri);
                        }
                }
                document::Element::Audio { src, .. } => {
                    if let Some(uri) = media_uri(src, &base_dir)
                        && !cache.join(format!("wave-{:016x}.png", fx_hash(&uri))).exists() {
                            waves.push(uri);
                        }
                }
                _ => {}
            }
        }
        let tpl_missing: Vec<String> = project
            .defs
            .iter()
            .filter(|(_, d)| {
                let key = format!(
                    "tpl-{:016x}.png",
                    fx_hash(&serde_json::to_string(d).unwrap_or_default())
                );
                !cache.join(key).exists()
            })
            .map(|(n, _)| n.clone())
            .collect();
        if thumbs.is_empty() && waves.is_empty() && tpl_missing.is_empty() {
            return;
        }
        let cache = cache.to_path_buf();
        let this = self.clone();
        let project_snapshot = project.clone();
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            for uri in thumbs {
                if let Err(e) = dualcut_engine::thumbs::thumbnail_png(&cache, &uri) {
                    eprintln!("thumbnail failed for {uri}: {e:#}");
                }
            }
            for name in tpl_missing {
                if let Err(e) = dualcut_engine::thumbs::template_png(
                    &cache,
                    &project_snapshot,
                    &name,
                    &base_dir,
                ) {
                    eprintln!("template thumb failed for {name}: {e:#}");
                }
            }
            for uri in waves {
                if let Err(e) = dualcut_engine::thumbs::waveform_png(&cache, &uri) {
                    eprintln!("waveform failed for {uri}: {e:#}");
                }
            }
            let _ = tx.send(());
        });
        glib::timeout_add_local(std::time::Duration::from_millis(300), move || {
            match rx.try_recv() {
                Ok(()) => {
                    this.rebuild_strip();
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => glib::ControlFlow::Break,
            }
        });
    }

    /// Library tab: media the user imported (project.library), shown as a
    /// thumbnail grid with a context menu (#15).
    fn rebuild_media(self: &Rc<Self>) {
        let ui = self.ui.borrow();
        let Some(ui) = ui.as_ref() else { return };
        while let Some(child) = ui.media_grid.first_child() {
            ui.media_grid.remove(&child);
        }
        let (library, base) = {
            let st = self.state.borrow();
            (
                st.project.as_ref().map(|p| p.library.clone()).unwrap_or_default(),
                self.base_dir(),
            )
        };
        ui.media_empty.set_visible(library.is_empty());
        let cache = base.join(".dualcut-cache");
        for rel in library {
            let cell = gtk::Box::new(gtk::Orientation::Vertical, 4);
            cell.set_margin_top(4);
            cell.set_margin_bottom(4);
            if let Some(uri) = media_uri(&rel, &base) {
                let thumb = cache.join(format!("thumb-{:016x}.png", fx_hash(&uri)));
                let wave = cache.join(format!("wave-{:016x}.png", fx_hash(&uri)));
                let img = if thumb.exists() { Some(thumb) } else if wave.exists() { Some(wave) } else { None };
                if let Some(img) = img {
                    let pic = gtk::Picture::for_filename(&img);
                    pic.set_size_request(120, 68);
                    pic.set_content_fit(gtk::ContentFit::Cover);
                    cell.append(&pic);
                } else {
                    let icon = gtk::Image::from_icon_name("video-x-generic-symbolic");
                    icon.set_pixel_size(48);
                    cell.append(&icon);
                }
            }
            let label = gtk::Label::new(Some(
                std::path::Path::new(&rel)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&rel),
            ));
            label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            label.set_max_width_chars(14);
            label.set_tooltip_text(Some(&rel));
            cell.append(&label);

            // Right-click context menu: add / remove.
            let gesture = gtk::GestureClick::new();
            gesture.set_button(3);
            {
                let this = self.clone();
                let rel = rel.clone();
                let cell = cell.clone();
                gesture.connect_pressed(move |_, _, x, y| {
                    let pop = gtk::Popover::new();
                    let menu_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    for (label, action) in [("Add to Timeline", 0), ("Remove from Library", 1)] {
                        let b = gtk::Button::with_label(label);
                        b.add_css_class("flat");
                        let this = this.clone();
                        let rel = rel.clone();
                        let pop2 = pop.clone();
                        b.connect_clicked(move |_| {
                            pop2.popdown();
                            if action == 0 {
                                this.insert_media(&rel);
                            } else {
                                let project = this.state.borrow().project.clone();
                                if let Some(mut project) = project {
                                    project.library.retain(|e| e != &rel);
                                    this.commit_document(project);
                                    this.toast_undo("Removed from library");
                                }
                            }
                        });
                        menu_box.append(&b);
                    }
                    pop.set_child(Some(&menu_box));
                    pop.set_parent(&cell);
                    pop.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                    pop.popup();
                });
            }
            cell.add_controller(gesture);

            // Double-click adds to the timeline.
            let dbl = gtk::GestureClick::new();
            dbl.set_button(1);
            {
                let this = self.clone();
                let rel = rel.clone();
                dbl.connect_pressed(move |_, n, _, _| {
                    if n == 2 {
                        this.insert_media(&rel);
                    }
                });
            }
            cell.add_controller(dbl);
            ui.media_grid.insert(&cell, -1);
        }
    }

    /// Insert a media file as a clip in the scene under the playhead.
    fn insert_media(self: &Rc<Self>, rel: &str) {
        let (project, time) = {
            let st = self.state.borrow();
            let time = st
                .pipeline
                .query_position::<gst::ClockTime>()
                .map(|p| p.nseconds() as f64 / 1e9)
                .unwrap_or(0.0);
            (st.project.clone(), time)
        };
        let Some(mut project) = project else { return };
        let ext = rel.rsplit('.').next().unwrap_or("").to_lowercase();
        let element = match ext.as_str() {
            "png" | "jpg" | "jpeg" | "webp" => document::Element::Image { src: rel.to_string() },
            "ogg" | "mp3" | "wav" | "flac" => {
                document::Element::Audio { src: rel.to_string(), offset: 0.0, volume: 1.0 }
            }
            _ => document::Element::Video { src: rel.to_string(), offset: 0.0, volume: 1.0 },
        };
        let mut id_base = rel
            .chars()
            .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
            .collect::<String>();
        id_base.truncate(24);
        let mut id = id_base.clone();
        let mut n = 1;
        while find_clip(&project, &id).is_some() {
            id = format!("{id_base}-{n}");
            n += 1;
        }
        let index = (0..project.scenes.len())
            .rev()
            .find(|&i| time >= project.scene_offset(i))
            .unwrap_or(0);
        let offset = project.scene_offset(index);
        let scene = &mut project.scenes[index];
        let start = (time - offset).clamp(0.0, (scene.duration - 0.1).max(0.0));
        scene.layers.push(document::Clip {
            id: id.clone(),
            start,
            duration: 0.0,
            element,
            transform: Default::default(),
            animations: Vec::new(),
            effects: Vec::new(),
        });
        self.state.borrow_mut().selected = Some(id);
        self.commit_document(project);
    }

    /// Scene form: duration and transition editing.
    fn build_scene_form(self: &Rc<Self>, uiref: &Ui, project: &Project, scene_id: &str) {
        let Some(scene) = project.scenes.iter().find(|s| s.id == scene_id) else { return };
        let form = gtk::Box::new(gtk::Orientation::Vertical, 6);
        form.set_margin_top(8);
        let title = gtk::Label::new(Some(&format!("Scene: {scene_id}")));
        title.add_css_class("heading");
        title.set_halign(gtk::Align::Start);
        form.append(&title);

        let dur_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let dl = gtk::Label::new(Some("Duration"));
        dl.set_width_chars(9);
        dl.set_halign(gtk::Align::Start);
        let dur = gtk::SpinButton::with_range(0.1, 3600.0, 0.1);
        dur.set_value(scene.duration);
        dur.set_hexpand(true);
        dur_row.append(&dl);
        dur_row.append(&dur);
        form.append(&dur_row);
        {
            let this = self.clone();
            let project = project.clone();
            let scene_id = scene_id.to_string();
            dur.connect_value_changed(move |s| {
                let mut project = project.clone();
                if let Some(sc) = project.scenes.iter_mut().find(|x| x.id == scene_id) {
                    sc.duration = s.value();
                }
                this.commit_document(project);
            });
        }

        const KINDS: [&str; 7] =
            ["(none)", "crossfade", "wipe-lr", "wipe-tb", "box-wipe", "iris", "clock"];
        let kind_index = |tr: &Option<document::Transition>| -> u32 {
            match tr {
                None => 0,
                Some(t) => match t.kind {
                    document::TransitionKind::Crossfade => 1,
                    document::TransitionKind::WipeLr => 2,
                    document::TransitionKind::WipeTb => 3,
                    document::TransitionKind::BoxWipe => 4,
                    document::TransitionKind::Iris => 5,
                    document::TransitionKind::Clock => 6,
                },
            }
        };
        let tr_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let tl = gtk::Label::new(Some("Transition"));
        tl.set_width_chars(9);
        tl.set_halign(gtk::Align::Start);
        let dd = gtk::DropDown::from_strings(&KINDS);
        dd.set_selected(kind_index(&scene.transition));
        dd.set_hexpand(true);
        let tdur = gtk::SpinButton::with_range(0.1, 10.0, 0.1);
        tdur.set_value(scene.transition.as_ref().map_or(0.5, |t| t.duration));
        tr_row.append(&tl);
        tr_row.append(&dd);
        tr_row.append(&tdur);
        form.append(&tr_row);
        let is_first = project.scenes.first().map(|s| s.id == scene.id).unwrap_or(false);
        if is_first {
            let hint = gtk::Label::new(Some("(first scene has no incoming transition)"));
            hint.add_css_class("dim-label");
            hint.set_halign(gtk::Align::Start);
            form.append(&hint);
            dd.set_sensitive(false);
            tdur.set_sensitive(false);
        } else {
            let apply = {
                let this = self.clone();
                let project = project.clone();
                let scene_id = scene_id.to_string();
                let dd = dd.clone();
                let tdur = tdur.clone();
                move || {
                    let mut project = project.clone();
                    if let Some(sc) = project.scenes.iter_mut().find(|x| x.id == scene_id) {
                        sc.transition = match dd.selected() {
                            0 => None,
                            k => Some(document::Transition {
                                kind: match k {
                                    1 => document::TransitionKind::Crossfade,
                                    2 => document::TransitionKind::WipeLr,
                                    3 => document::TransitionKind::WipeTb,
                                    4 => document::TransitionKind::BoxWipe,
                                    5 => document::TransitionKind::Iris,
                                    _ => document::TransitionKind::Clock,
                                },
                                duration: tdur.value(),
                            }),
                        };
                    }
                    this.commit_document(project);
                }
            };
            let a2 = apply.clone();
            dd.connect_selected_notify(move |_| apply());
            tdur.connect_value_changed(move |_| a2());
        }
        uiref.inspector.append(&form);
    }

    /// Templates tab: every def, instantiable at the playhead.
    fn rebuild_templates(self: &Rc<Self>) {
        let ui = self.ui.borrow();
        let Some(ui) = ui.as_ref() else { return };
        while let Some(child) = ui.templates_list.first_child() {
            ui.templates_list.remove(&child);
        }
        let defs: Vec<(String, Vec<String>)> = {
            let st = self.state.borrow();
            let Some(project) = st.project.as_ref() else { return };
            project.defs.iter().map(|(n, d)| (n.clone(), d.params.clone())).collect()
        };
        let cache = self.base_dir().join(".dualcut-cache");
        let def_hashes: std::collections::BTreeMap<String, String> = {
            let st = self.state.borrow();
            st.project.as_ref().map_or_else(Default::default, |p| {
                p.defs
                    .iter()
                    .map(|(n, d)| {
                        (n.clone(), format!(
                            "tpl-{:016x}.png",
                            fx_hash(&serde_json::to_string(d).unwrap_or_default())
                        ))
                    })
                    .collect()
            })
        };
        for (name, params) in defs {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
            if let Some(key) = def_hashes.get(&name) {
                let path = cache.join(key);
                if path.exists() {
                    let pic = gtk::Picture::for_filename(&path);
                    pic.set_size_request(-1, 90);
                    pic.set_content_fit(gtk::ContentFit::Contain);
                    row.append(&pic);
                }
            }
            row.set_margin_top(4);
            row.set_margin_bottom(4);
            row.set_margin_start(6);
            row.set_margin_end(6);
            let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let label = gtk::Label::new(Some(&name));
            label.set_halign(gtk::Align::Start);
            label.set_hexpand(true);
            label.add_css_class("heading");
            head.append(&label);
            let insert = gtk::Button::with_label("Insert");
            insert.add_css_class("flat");
            head.append(&insert);
            row.append(&head);
            let mut entries: Vec<(String, gtk::Entry)> = Vec::new();
            for p in &params {
                let entry = gtk::Entry::new();
                entry.set_placeholder_text(Some(p));
                row.append(&entry);
                entries.push((p.clone(), entry));
            }
            {
                let this = self.clone();
                let name = name.clone();
                insert.connect_clicked(move |_| {
                    let args: std::collections::BTreeMap<String, String> = entries
                        .iter()
                        .map(|(p, e)| {
                            let text = e.text().to_string();
                            (p.clone(), if text.is_empty() { p.clone() } else { text })
                        })
                        .collect();
                    this.insert_template(&name, args);
                });
            }
            let lbrow = gtk::ListBoxRow::new();
            lbrow.set_child(Some(&row));
            ui.templates_list.append(&lbrow);
        }
    }

    /// Insert a def instance into the scene under the playhead.
    fn insert_template(
        self: &Rc<Self>,
        name: &str,
        args: std::collections::BTreeMap<String, String>,
    ) {
        let (project, time) = {
            let st = self.state.borrow();
            let time = st
                .pipeline
                .query_position::<gst::ClockTime>()
                .map(|p| p.nseconds() as f64 / 1e9)
                .unwrap_or(0.0);
            (st.project.clone(), time)
        };
        let Some(mut project) = project else { return };
        let mut id = name.to_string();
        let mut n = 1;
        while find_clip(&project, &id).is_some() {
            id = format!("{name}-{n}");
            n += 1;
        }
        let index = (0..project.scenes.len())
            .rev()
            .find(|&i| time >= project.scene_offset(i))
            .unwrap_or(0);
        let offset = project.scene_offset(index);
        let scene = &mut project.scenes[index];
        let start = (time - offset).clamp(0.0, (scene.duration - 0.1).max(0.0));
        scene.layers.insert(
            0,
            document::Clip {
                id: id.clone(),
                start,
                duration: 0.0,
                element: document::Element::CompRef { r#ref: name.to_string(), args },
                transform: Default::default(),
                animations: Vec::new(),
                effects: Vec::new(),
            },
        );
        self.state.borrow_mut().selected = Some(id);
        self.commit_document(project);
    }

    /// Code tab: the document as editable JSON.
    fn refresh_code(self: &Rc<Self>) {
        let ui = self.ui.borrow();
        let Some(ui) = ui.as_ref() else { return };
        let st = self.state.borrow();
        if let Some(project) = st.project.as_ref() {
            ui.code_buffer.set_text(&project.to_json());
        }
    }

    fn rebuild_inspector(self: &Rc<Self>) {
        let (project, selected) = {
            let st = self.state.borrow();
            (st.project.clone(), st.selected.clone())
        };
        let ui = self.ui.borrow();
        let Some(uiref) = ui.as_ref() else { return };
        while let Some(child) = uiref.inspector.first_child() {
            uiref.inspector.remove(&child);
        }
        let Some(project) = project else {
            let hint = gtk::Label::new(Some("No project loaded.\nOpen with: dualcut project.json"));
            hint.add_css_class("dim-label");
            uiref.inspector.append(&hint);
            return;
        };

        // Clip list.
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::Multiple);
        let mut entries: Vec<(String, String)> = Vec::new();
        for scene in &project.scenes {
            for clip in &scene.layers {
                entries.push((format!("{} ▸ {}", scene.id, clip.id), clip.id.clone()));
            }
        }
        for track in &project.overlays {
            for clip in &track.clips {
                entries.push((format!("〜 {} ▸ {}", track.id, clip.id), clip.id.clone()));
            }
        }
        for (label, id) in &entries {
            let row = gtk::ListBoxRow::new();
            let l = gtk::Label::new(Some(label));
            l.set_halign(gtk::Align::Start);
            l.set_margin_top(4);
            l.set_margin_bottom(4);
            l.set_margin_start(8);
            row.set_child(Some(&l));
            list.append(&row);
            if Some(id) == selected.as_ref() {
                list.select_row(Some(&row));
            }
        }
        {
            let this = self.clone();
            let ids: Vec<String> = entries.iter().map(|(_, id)| id.clone()).collect();
            list.connect_selected_rows_changed(move |list| {
                let rows = list.selected_rows();
                let Some(last) = rows.last() else { return };
                let id = ids[last.index() as usize].clone();
                let changed = {
                    let mut st = this.state.borrow_mut();
                    let changed = st.selected.as_ref() != Some(&id) && rows.len() == 1;
                    if rows.len() == 1 {
                        st.selected = Some(id);
                    }
                    changed
                };
                if changed {
                    glib::idle_add_local_once({
                        let this = this.clone();
                        move || this.rebuild_inspector()
                    });
                }
            });
        }
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_child(Some(&list));
        scroll.set_vexpand(true);
        scroll.set_min_content_height(160);
        uiref.inspector.append(&scroll);

        // Multi-select ops (Ctrl/Shift-click rows first).
        let sel_ops = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let del_sel = gtk::Button::with_label("Delete selected");
        {
            let this = self.clone();
            let list = list.clone();
            let ids: Vec<String> = entries.iter().map(|(_, id)| id.clone()).collect();
            let project_snapshot = project.clone();
            del_sel.connect_clicked(move |_| {
                let rows = list.selected_rows();
                if rows.is_empty() {
                    return;
                }
                let mut project = project_snapshot.clone();
                let count = rows.len();
                for row in rows {
                    remove_clip(&mut project, &ids[row.index() as usize]);
                }
                this.state.borrow_mut().selected = None;
                this.commit_document(project);
                this.toast_undo(&format!(
                    "{count} clip{} deleted",
                    if count == 1 { "" } else { "s" }
                ));
            });
        }
        sel_ops.append(&del_sel);

        let tpl_name = gtk::Entry::new();
        tpl_name.set_placeholder_text(Some("template name"));
        tpl_name.set_hexpand(true);
        let save_tpl = gtk::Button::with_label("Save as template");
        {
            let this = self.clone();
            let list = list.clone();
            let ids: Vec<String> = entries.iter().map(|(_, id)| id.clone()).collect();
            let project_snapshot = project.clone();
            let tpl_name = tpl_name.clone();
            save_tpl.connect_clicked(move |_| {
                let selected: Vec<String> = list
                    .selected_rows()
                    .iter()
                    .map(|r| ids[r.index() as usize].clone())
                    .collect();
                let name = tpl_name.text().to_string();
                let mut project = project_snapshot.clone();
                match save_as_def(&mut project, &selected, &name) {
                    Ok(()) => {
                        println!("saved template {name:?} ({} clips)", selected.len());
                        this.commit_document(project);
                    }
                    Err(e) => eprintln!("save template: {e}"),
                }
            });
        }
        sel_ops.append(&tpl_name);
        sel_ops.append(&save_tpl);
        uiref.inspector.append(&sel_ops);

        // Editor form for the selected clip (or scene).
        let Some(selected) = selected else { return };
        if let Some(scene_id) = selected.strip_prefix("scene:") {
            self.build_scene_form(uiref, &project, scene_id);
            return;
        }
        let Some(clip) = find_clip(&project, &selected).cloned() else { return };

        let form = gtk::Box::new(gtk::Orientation::Vertical, 6);
        form.set_margin_top(8);
        let title = gtk::Label::new(Some(&format!("Clip: {}", clip.id)));
        title.add_css_class("heading");
        title.set_halign(gtk::Align::Start);
        form.append(&title);

        let spin = |label: &str, value: f64, max: f64| -> (gtk::Box, gtk::SpinButton) {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let l = gtk::Label::new(Some(label));
            l.set_width_chars(9);
            l.set_halign(gtk::Align::Start);
            let s = gtk::SpinButton::with_range(0.0, max, 0.1);
            s.set_value(value);
            s.set_hexpand(true);
            row.append(&l);
            row.append(&s);
            (row, s)
        };

        let (row_start, spin_start) = spin("Start", clip.start, 3600.0);
        let (row_dur, spin_dur) = spin("Duration", clip.duration, 3600.0);
        let (row_op, spin_op) = spin("Opacity", clip.transform.opacity, 1.0);
        form.append(&row_start);
        form.append(&row_dur);
        form.append(&row_op);

        let text_entry: Option<gtk::Entry> = match &clip.element {
            document::Element::Text { text, .. } => {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let l = gtk::Label::new(Some("Text"));
                l.set_width_chars(9);
                l.set_halign(gtk::Align::Start);
                let entry = gtk::Entry::new();
                entry.set_text(text);
                entry.set_hexpand(true);
                row.append(&l);
                row.append(&entry);
                form.append(&row);
                Some(entry)
            }
            _ => None,
        };

        let apply = gtk::Button::with_label("Apply");
        apply.add_css_class("suggested-action");
        {
            let this = self.clone();
            let id = clip.id.clone();
            let project = project.clone();
            apply.connect_clicked(move |_| {
                let mut project = project.clone();
                if let Some(clip) = find_clip_mut(&mut project, &id) {
                    clip.start = spin_start.value();
                    clip.duration = spin_dur.value();
                    clip.transform.opacity = spin_op.value();
                    if let (document::Element::Text { text, .. }, Some(entry)) =
                        (&mut clip.element, text_entry.as_ref())
                    {
                        *text = entry.text().to_string();
                    }
                }
                this.commit_document(project);
            });
        }
        form.append(&apply);

        // ── Animations ─────────────────────────────────────────
        let anim_head = gtk::Label::new(Some("Animations"));
        anim_head.add_css_class("heading");
        anim_head.set_halign(gtk::Align::Start);
        anim_head.set_margin_top(8);
        form.append(&anim_head);

        const PROPS: [&str; 6] = ["x", "y", "width", "height", "opacity", "volume"];
        const EASINGS: [&str; 4] = ["linear", "easeIn", "easeOut", "easeInOut"];
        let prop_of = |a: &document::AnimProperty| match a {
            document::AnimProperty::X => 0,
            document::AnimProperty::Y => 1,
            document::AnimProperty::Width => 2,
            document::AnimProperty::Height => 3,
            document::AnimProperty::Opacity => 4,
            document::AnimProperty::Volume => 5,
        };
        let ease_of = |e: &document::Easing| match e {
            document::Easing::Linear => 0,
            document::Easing::EaseIn => 1,
            document::Easing::EaseOut => 2,
            document::Easing::EaseInOut => 3,
        };

        for (ai, anim) in clip.animations.iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            let commit_anim = {
                let this = self.clone();
                let project = project.clone();
                let clip_id = clip.id.clone();
                Rc::new(move |mutate: Box<dyn Fn(&mut document::Anim)>| {
                    let mut project = project.clone();
                    if let Some(c) = find_clip_mut(&mut project, &clip_id)
                        && let Some(a) = c.animations.get_mut(ai) {
                            mutate(a);
                        }
                    this.commit_document(project);
                })
            };

            let prop = gtk::DropDown::from_strings(&PROPS);
            prop.set_selected(prop_of(&anim.property) as u32);
            {
                let commit = commit_anim.clone();
                prop.connect_selected_notify(move |dd| {
                    let value = match dd.selected() {
                        0 => document::AnimProperty::X,
                        1 => document::AnimProperty::Y,
                        2 => document::AnimProperty::Width,
                        3 => document::AnimProperty::Height,
                        4 => document::AnimProperty::Opacity,
                        _ => document::AnimProperty::Volume,
                    };
                    commit(Box::new(move |a| a.property = value));
                });
            }
            row.append(&prop);

            let add_spin = |value: f64, lo: f64, hi: f64, set: fn(&mut document::Anim, f64)| {
                let s = gtk::SpinButton::with_range(lo, hi, 0.1);
                s.set_value(value);
                s.set_width_chars(5);
                let commit = commit_anim.clone();
                s.connect_value_changed(move |s| {
                    let v = s.value();
                    commit(Box::new(move |a| set(a, v)));
                });
                row.append(&s);
            };
            add_spin(anim.from, -10000.0, 10000.0, |a, v| a.from = v);
            add_spin(anim.to, -10000.0, 10000.0, |a, v| a.to = v);
            add_spin(anim.start, 0.0, 3600.0, |a, v| a.start = v);
            add_spin(anim.end, 0.0, 3600.0, |a, v| a.end = v);

            let ease = gtk::DropDown::from_strings(&EASINGS);
            ease.set_selected(ease_of(&anim.easing) as u32);
            {
                let commit = commit_anim.clone();
                ease.connect_selected_notify(move |dd| {
                    let value = match dd.selected() {
                        1 => document::Easing::EaseIn,
                        2 => document::Easing::EaseOut,
                        3 => document::Easing::EaseInOut,
                        _ => document::Easing::Linear,
                    };
                    commit(Box::new(move |a| a.easing = value));
                });
            }
            row.append(&ease);

            let del = gtk::Button::from_icon_name("edit-delete-symbolic");
            del.update_property(&[gtk::accessible::Property::Label("Delete animation")]);
            del.add_css_class("flat");
            {
                let this = self.clone();
                let project = project.clone();
                let clip_id = clip.id.clone();
                del.connect_clicked(move |_| {
                    let mut project = project.clone();
                    if let Some(c) = find_clip_mut(&mut project, &clip_id)
                        && ai < c.animations.len() {
                            c.animations.remove(ai);
                        }
                    this.commit_document(project);
                });
            }
            row.append(&del);
            form.append(&row);
        }

        // Presets.
        let presets = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let add_preset = |label: &str, make: fn(&document::Clip) -> document::Anim| {
            let b = gtk::Button::with_label(label);
            let this = self.clone();
            let project = project.clone();
            let clip_id = clip.id.clone();
            b.connect_clicked(move |_| {
                let mut project = project.clone();
                if let Some(c) = find_clip_mut(&mut project, &clip_id) {
                    let anim = make(c);
                    c.animations.push(anim);
                }
                this.commit_document(project);
            });
            presets.append(&b);
        };
        add_preset("+ Fade in", |_| document::Anim {
            property: document::AnimProperty::Opacity,
            from: 0.0, to: 1.0, start: 0.0, end: 0.5,
            easing: document::Easing::EaseOut,
            keyframes: Vec::new(),
        });
        add_preset("+ Fade out", |c| document::Anim {
            property: document::AnimProperty::Opacity,
            from: 1.0, to: 0.0,
            start: (c.duration - 0.5).max(0.0), end: c.duration.max(0.5),
            easing: document::Easing::EaseIn,
            keyframes: Vec::new(),
        });
        add_preset("+ Slide in", |c| document::Anim {
            property: document::AnimProperty::X,
            from: c.transform.x - 300.0, to: c.transform.x,
            start: 0.0, end: 0.6,
            easing: document::Easing::EaseOut,
            keyframes: Vec::new(),
        });
        if matches!(
            clip.element,
            document::Element::Audio { .. } | document::Element::Video { .. }
        ) {
            add_preset("+ Audio in", |_| document::Anim {
                property: document::AnimProperty::Volume,
                from: 0.0, to: 1.0, start: 0.0, end: 1.0,
                easing: document::Easing::EaseOut,
                keyframes: Vec::new(),
            });
            add_preset("+ Audio out", |c| document::Anim {
                property: document::AnimProperty::Volume,
                from: 1.0, to: 0.0,
                start: (c.duration - 1.0).max(0.0), end: c.duration.max(1.0),
                easing: document::Easing::EaseIn,
                keyframes: Vec::new(),
            });
        }
        form.append(&presets);

        // Effects.
        let fx_head = gtk::Label::new(Some("Effects"));
        fx_head.add_css_class("heading");
        fx_head.set_halign(gtk::Align::Start);
        fx_head.set_margin_top(8);
        form.append(&fx_head);
        for (fi, effect) in clip.effects.iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            let commit_fx = {
                let this = self.clone();
                let project = project.clone();
                let clip_id = clip.id.clone();
                Rc::new(move |f: Box<dyn Fn(&mut document::Effect)>| {
                    let mut project = project.clone();
                    if let Some(c) = find_clip_mut(&mut project, &clip_id)
                        && let Some(e) = c.effects.get_mut(fi) {
                            f(e);
                        }
                    this.commit_document(project);
                })
            };
            let fx_spin = |label: &str, value: f64, min: f64, max: f64, step: f64| {
                let l = gtk::Label::new(Some(label));
                l.add_css_class("dim-label");
                let s = gtk::SpinButton::with_range(min, max, step);
                s.set_value(value);
                (l, s)
            };
            match effect {
                document::Effect::Blur { amount } => {
                    let (l, s) = fx_spin("Blur", *amount, 0.0, 50.0, 0.5);
                    row.append(&l);
                    row.append(&s);
                    let commit_fx = commit_fx.clone();
                    s.connect_value_changed(move |s| {
                        let v = s.value();
                        commit_fx(Box::new(move |e| {
                            if let document::Effect::Blur { amount } = e {
                                *amount = v;
                            }
                        }));
                    });
                }
                document::Effect::Color { brightness, contrast, saturation, hue } => {
                    for (name, val, min, max) in [
                        ("Bri", *brightness, -1.0, 1.0),
                        ("Con", *contrast, 0.0, 2.0),
                        ("Sat", *saturation, 0.0, 2.0),
                        ("Hue", *hue, -1.0, 1.0),
                    ] {
                        let (l, s) = fx_spin(name, val, min, max, 0.05);
                        row.append(&l);
                        row.append(&s);
                        let commit_fx = commit_fx.clone();
                        s.connect_value_changed(move |s| {
                            let v = s.value();
                            commit_fx(Box::new(move |e| {
                                if let document::Effect::Color {
                                    brightness, contrast, saturation, hue,
                                } = e
                                {
                                    match name {
                                        "Bri" => *brightness = v,
                                        "Con" => *contrast = v,
                                        "Sat" => *saturation = v,
                                        _ => *hue = v,
                                    }
                                }
                            }));
                        });
                    }
                }
            }
            let rm = gtk::Button::from_icon_name("window-close-symbolic");
            rm.update_property(&[gtk::accessible::Property::Label("Remove effect")]);
            rm.add_css_class("flat");
            {
                let this = self.clone();
                let project = project.clone();
                let clip_id = clip.id.clone();
                rm.connect_clicked(move |_| {
                    let mut project = project.clone();
                    if let Some(c) = find_clip_mut(&mut project, &clip_id)
                        && fi < c.effects.len() {
                            c.effects.remove(fi);
                        }
                    this.commit_document(project);
                });
            }
            row.append(&rm);
            form.append(&row);
        }
        let fx_add = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for (label, make) in [
            ("+ Blur", document::Effect::Blur { amount: 4.0 }),
            ("+ Color", document::Effect::Color {
                brightness: 0.0, contrast: 1.0, saturation: 1.0, hue: 0.0,
            }),
        ] {
            let b = gtk::Button::with_label(label);
            let this = self.clone();
            let project = project.clone();
            let clip_id = clip.id.clone();
            let make = make.clone();
            b.connect_clicked(move |_| {
                let mut project = project.clone();
                if let Some(c) = find_clip_mut(&mut project, &clip_id) {
                    c.effects.push(make.clone());
                }
                this.commit_document(project);
            });
            fx_add.append(&b);
        }
        form.append(&fx_add);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        if matches!(clip.element, document::Element::Video { .. }) {
            let detach = gtk::Button::with_label("Detach audio");
            {
                let this = self.clone();
                let id = clip.id.clone();
                let project = project.clone();
                detach.connect_clicked(move |_| {
                    let mut project = project.clone();
                    if let Some(new) = detach_audio(&mut project, &id) {
                        println!("detached audio -> clip {new:?}");
                        this.commit_document(project);
                    }
                });
            }
            actions.append(&detach);
        }
        let delete = gtk::Button::with_label("Delete clip");
        delete.add_css_class("destructive-action");
        {
            let this = self.clone();
            let id = clip.id.clone();
            let project = project.clone();
            delete.connect_clicked(move |_| {
                let mut project = project.clone();
                remove_clip(&mut project, &id);
                this.state.borrow_mut().selected = None;
                this.commit_document(project);
                this.toast_undo(&format!("Clip {id:?} deleted"));
            });
        }
        actions.append(&delete);
        form.append(&actions);
        uiref.inspector.append(&form);
    }
}

/// Cached thumbnail path for a scene's first media layer, if generated.
fn scene_thumb(
    project: &Project,
    scene: &document::Scene,
    cache: &std::path::Path,
    base_dir: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let _ = project;
    for clip in &scene.layers {
        if let document::Element::Video { src, .. } | document::Element::Image { src } = &clip.element {
            let uri = media_uri(src, base_dir)?;
            let file = cache.join(format!("thumb-{:016x}.png", fx_hash(&uri)));
            if file.exists() {
                return Some(file);
            }
        }
    }
    None
}

/// Composition-space bounding box for a clip (0 width/height = full frame).
fn clip_box(project: &Project, clip: &document::Clip) -> (f64, f64, f64, f64) {
    let t = &clip.transform;
    let w = if t.width > 0.0 { t.width } else { project.meta.width as f64 };
    let h = if t.height > 0.0 { t.height } else { project.meta.height as f64 };
    (t.x, t.y, w, h)
}

/// Clips active at `time` with absolute-time info, topmost first
/// (overlays before scene layers, lower layer index above higher).
fn active_clips_at(project: &Project, time: f64) -> Vec<(String, f64, f64, f64, f64)> {
    let mut out = Vec::new();
    for track in &project.overlays {
        for clip in &track.clips {
            if time >= clip.start && time < clip.start + clip.duration.max(0.01) {
                let (x, y, w, h) = clip_box(project, clip);
                out.push((clip.id.clone(), x, y, w, h));
            }
        }
    }
    for (i, scene) in project.scenes.iter().enumerate() {
        let offset = project.scene_offset(i);
        if time < offset || time >= offset + scene.duration {
            continue;
        }
        for clip in &scene.layers {
            let local = time - offset;
            let duration = if clip.duration > 0.0 { clip.duration } else { scene.duration - clip.start };
            if local >= clip.start && local < clip.start + duration {
                let (x, y, w, h) = clip_box(project, clip);
                out.push((clip.id.clone(), x, y, w, h));
            }
        }
    }
    out
}

/// Map preview-widget coords to composition coords through ContentFit::Contain
/// letterboxing. Returns None outside the video area.
fn widget_to_comp(
    project: &Project,
    widget_w: f64,
    widget_h: f64,
    wx: f64,
    wy: f64,
) -> Option<(f64, f64, f64)> {
    let (cw, ch) = (project.meta.width as f64, project.meta.height as f64);
    let scale = (widget_w / cw).min(widget_h / ch);
    if scale <= 0.0 {
        return None;
    }
    let (vw, vh) = (cw * scale, ch * scale);
    let (ox, oy) = ((widget_w - vw) / 2.0, (widget_h - vh) / 2.0);
    let (cx, cy) = ((wx - ox) / scale, (wy - oy) / scale);
    if cx < 0.0 || cy < 0.0 || cx > cw || cy > ch {
        return None;
    }
    Some((cx, cy, scale))
}

/// Snap a time to scene boundaries or the half-second grid (0.15s window).
fn snap_time(project: &Project, raw: f64) -> f64 {
    const WINDOW: f64 = 0.15;
    let mut candidates: Vec<f64> = (0..=project.scenes.len())
        .map(|i| {
            if i == project.scenes.len() {
                project.duration()
            } else {
                project.scene_offset(i)
            }
        })
        .collect();
    candidates.push((raw * 2.0).round() / 2.0);
    candidates
        .into_iter()
        .filter(|c| (c - raw).abs() <= WINDOW)
        .min_by(|a, b| (a - raw).abs().total_cmp(&(b - raw).abs()))
        .unwrap_or(raw)
        .max(0.0)
}

fn media_uri(src: &str, base_dir: &std::path::Path) -> Option<String> {
    if src.contains("://") {
        return Some(src.to_string());
    }
    base_dir.join(src).canonicalize().ok().map(|p| format!("file://{}", p.display()))
}

fn recents_file() -> PathBuf {
    glib::user_config_dir().join("dualcut").join("recent-projects")
}

fn load_recents() -> Vec<PathBuf> {
    std::fs::read_to_string(recents_file())
        .unwrap_or_default()
        .lines()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .take(8)
        .collect()
}

fn remember_recent(path: &std::path::Path) {
    let mut entries = load_recents();
    entries.retain(|p| p != path);
    entries.insert(0, path.to_path_buf());
    entries.truncate(8);
    let file = recents_file();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(
        &file,
        entries.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("
"),
    );
}

/// Locate the bundled agent skill directory (flatpak install or repo).
fn skill_source_dir() -> Option<PathBuf> {
    ["/app/share/dualcut/skills/dualcut", "../skills/dualcut", "skills/dualcut"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.join("SKILL.md").exists())
}

fn install_skill_to(target_root: &std::path::Path) -> Result<PathBuf> {
    let src = skill_source_dir().context("bundled skill files not found")?;
    let dest = target_root.join("dualcut");
    std::fs::create_dir_all(&dest)?;
    for entry in std::fs::read_dir(&src)?.flatten() {
        if entry.path().is_file() {
            std::fs::copy(entry.path(), dest.join(entry.file_name()))?;
        }
    }
    Ok(dest)
}

fn show_skills_dialog(editor: &Rc<Editor>, window: Option<&gtk::Window>) {
    let dialog = adw::AlertDialog::new(
        Some("Install Agent Skills"),
        Some("Install the dualcut agent skill so coding agents can edit your projects."),
    );
    dialog.add_response("agents", "~/.agents/skills");
    dialog.add_response("claude", "~/.claude/skills");
    dialog.add_response("choose", "Choose directory…");
    dialog.add_response("cancel", "Cancel");
    dialog.set_default_response(Some("claude"));
    dialog.set_close_response("cancel");
    let win = window.cloned();
    let editor = editor.clone();
    dialog.connect_response(None, move |d, response| {
        let home = glib::home_dir();
        let target = match response {
            "agents" => Some(home.join(".agents/skills")),
            "claude" => Some(home.join(".claude/skills")),
            "choose" => {
                let picker = gtk::FileDialog::builder().title("Choose Skill Directory").build();
                let editor = editor.clone();
                picker.select_folder(
                    editor.window().as_ref(),
                    gtk::gio::Cancellable::NONE,
                    move |res| {
                        if let Ok(dir) = res
                            && let Some(path) = dir.path() {
                                match install_skill_to(&path) {
                                    Ok(dest) => println!("skill installed to {}", dest.display()),
                                    Err(e) => eprintln!("skill install failed: {e:#}"),
                                }
                            }
                    },
                );
                None
            }
            _ => None,
        };
        if let Some(target) = target {
            match install_skill_to(&target) {
                Ok(dest) => {
                    let done = adw::AlertDialog::new(
                        Some("Skill installed"),
                        Some(&format!("Installed to {}", dest.display())),
                    );
                    done.add_response("ok", "OK");
                    done.present(win.as_ref());
                }
                Err(e) => eprintln!("skill install failed: {e:#}"),
            }
        }
        d.close();
    });
    dialog.present(window);
}

fn show_about(window: Option<&gtk::Window>) {
    let about = adw::AboutDialog::builder()
        .application_name("Dualcut")
        .application_icon("io.github.hanthor.Dualcut")
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name("James Reilly")
        .developers(["James Reilly (hanthor)", "KiKaraage"])
        .website("https://github.com/hanthor/dualcut")
        .issue_url("https://github.com/hanthor/dualcut/issues")
        .comments("Dual-mode video editor: a GUI for humans and a JSON/TypeScript surface for agents, on one GStreamer Editing Services engine.")
        .build();
    about.present(window);
}

fn fx_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(feature = "scripting")]
fn build_script_panel(editor: &Rc<Editor>) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 6);
    page.set_margin_top(8);
    page.set_margin_start(8);
    page.set_margin_end(8);
    page.set_margin_bottom(8);

    let hint = gtk::Label::new(Some(
        "Automate edits with TypeScript. The Code tab is the document \
itself (JSON); a script here transforms it: it receives the current \
project and the returned project becomes the new document (undoable). \
Must export edit(project: Project): Project. \
Types: engine/schema/dualcut.d.ts",
    ));
    hint.add_css_class("dim-label");
    hint.set_halign(gtk::Align::Start);
    hint.set_wrap(true);
    page.append(&hint);

    let buffer = gtk::TextBuffer::new(None);
    buffer.set_text(
        "export function edit(project) {\n  // e.g. retitle every scene:\n  // project.scenes.forEach((s, i) => s.name = `Scene ${i + 1}`);\n  return project;\n}\n",
    );
    let view = gtk::TextView::with_buffer(&buffer);
    view.set_monospace(true);
    view.set_vexpand(true);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&view));
    scroll.set_vexpand(true);
    page.append(&scroll);

    let status = gtk::Label::new(None);
    status.set_halign(gtk::Align::Start);
    status.set_wrap(true);
    let run = gtk::Button::with_label("Run script");
    run.add_css_class("suggested-action");
    {
        let editor = editor.clone();
        let status = status.clone();
        run.connect_clicked(move |_| {
            let source = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).to_string();
            let project = {
                let st = editor.state.borrow();
                st.project.clone()
            };
            let Some(project) = project else {
                status.set_text("no project loaded");
                return;
            };
            match dualcut_engine::scripting::run_script(&source, &project) {
                Ok(edited) => {
                    status.set_text("✓ applied");
                    editor.commit_document(edited);
                }
                Err(e) => status.set_text(&format!("✗ {e:#}")),
            }
        });
    }
    page.append(&run);
    page.append(&status);
    page
}

fn show_export_dialog(editor: &Rc<Editor>, parent: Option<&gtk::Window>) {
    let (project_json, base_dir, title) = {
        let st = editor.state.borrow();
        let Some(project) = st.project.as_ref() else { return };
        (project.to_json(), editor.base_dir(), project.meta.title.clone())
    };

    let dialog = gtk::Window::builder()
        .title("Export video")
        .modal(true)
        .default_width(420)
        .build();
    if let Some(parent) = parent {
        dialog.set_transient_for(Some(parent));
    }

    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_margin_top(14);
    content.set_margin_bottom(14);
    content.set_margin_start(14);
    content.set_margin_end(14);

    let out_entry = gtk::Entry::new();
    let default_name: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    out_entry.set_text(&base_dir.join(format!("{default_name}.mp4")).display().to_string());
    content.append(&gtk::Label::builder().label("Output file").halign(gtk::Align::Start).build());
    content.append(&out_entry);

    let profile = gtk::DropDown::from_strings(&["mp4 (H.264/AAC)", "webm (VP8/Vorbis)"]);
    content.append(&gtk::Label::builder().label("Format").halign(gtk::Align::Start).build());
    content.append(&profile);

    let status = gtk::Label::new(None);
    status.set_halign(gtk::Align::Start);
    status.set_wrap(true);

    let go = gtk::Button::with_label("Export");
    go.add_css_class("suggested-action");
    {
        let status = status.clone();
        let out_entry = out_entry.clone();
        let profile = profile.clone();
        go.connect_clicked(move |btn| {
            let out = out_entry.text().to_string();
            let prof = if profile.selected() == 1 { "webm" } else { "mp4" }.to_string();
            btn.set_sensitive(false);
            status.set_text("Rendering…");
            let (tx, rx) = std::sync::mpsc::channel::<std::result::Result<(), String>>();
            {
                let project_json = project_json.clone();
                let base_dir = base_dir.clone();
                let out = out.clone();
                std::thread::spawn(move || {
                    let result =
                        dualcut_engine::render_project(&project_json, &base_dir, &out, &prof)
                            .map(|warnings| {
                                for w in warnings {
                                    eprintln!("warning: {w}");
                                }
                            })
                            .map_err(|e| format!("{e:#}"));
                    let _ = tx.send(result);
                });
            }
            let status = status.clone();
            let btn = btn.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
                match rx.try_recv() {
                    Ok(Ok(())) => {
                        status.set_text(&format!("✓ exported {out}"));
                        btn.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                    Ok(Err(e)) => {
                        status.set_text(&format!("✗ {e}"));
                        btn.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(_) => glib::ControlFlow::Break,
                }
            });
        });
    }
    content.append(&go);
    content.append(&status);
    dialog.set_child(Some(&content));
    dialog.present();
}

fn build_ui(app: &adw::Application) -> Result<()> {
    init()?;
    gstgtk4::plugin_register_static().context("registering gtk4paintablesink")?;

    let arg = std::env::args().nth(1);
    let (timeline, project, project_path, duration) = match &arg {
        Some(path) if path.ends_with(".json") => {
            let path = PathBuf::from(path);
            let json = std::fs::read_to_string(&path)?;
            let project = Project::from_json(&json)?;
            let base_dir = path.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
            let timeline = compile_project(&project, &base_dir)?;
            let duration = project.duration();
            (timeline, Some(project), Some(path), duration)
        }
        Some(other) => (build_demo_timeline(Some(other.as_str()))?, None, None, 8.0),
        None => {
            // No file argument: start an unsaved "New Project" (#15).
            let project = dualcut_engine::templates::new_project("New Project");
            let timeline = compile_project(&project, std::path::Path::new("."))?;
            let duration = project.duration();
            (timeline, Some(project), None, duration)
        }
    };

    let (pipeline, paintable) = make_pipeline(&timeline)?;
    let window_title = match &project_path {
        Some(p) => format!(
            "dualcut — {}",
            p.file_name().and_then(|n| n.to_str()).unwrap_or("project")
        ),
        None => "dualcut — New Project (unsaved)".to_string(),
    };
    let mtime = project_path.as_ref().and_then(|p| p.metadata().ok()?.modified().ok());

    // Agent surface: HTTP API over the project file (the mtime watcher
    // makes agent edits appear live in the UI). DUALCUT_API_PORT=0 disables.
    if let Some(path) = project_path.clone() {
        let port: u16 = std::env::var("DUALCUT_API_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(7357);
        if port != 0 {
            std::thread::spawn(move || {
                if let Err(e) = dualcut_engine::api::serve_file_api(path, port) {
                    eprintln!("agent API not available: {e:#}");
                }
            });
        }
    }

    let editor = Rc::new(Editor {
        state: Rc::new(RefCell::new(AppState {
            pipeline: pipeline.clone(),
            project,
            project_path,
            mtime,
            duration,
            selected: None,
            undo: Vec::new(),
            redo: Vec::new(),
            self_write: false,
        })),
        ui: RefCell::new(None),
    });

    let picture = gtk::Picture::builder()
        .paintable(&paintable)
        .content_fit(gtk::ContentFit::Contain)
        .hexpand(true)
        .vexpand(true)
        .build();

    // Selection overlay: draws the selected clip's box; gestures select and
    // move clips directly in the preview.
    let sel_canvas = gtk::DrawingArea::new();
    sel_canvas.set_hexpand(true);
    sel_canvas.set_vexpand(true);
    let preview_overlay = gtk::Overlay::new();
    preview_overlay.set_child(Some(&picture));
    preview_overlay.add_overlay(&sel_canvas);
    {
        let editor = editor.clone();
        sel_canvas.set_draw_func(move |area, cr, w, h| {
            let st = editor.state.borrow();
            let Some(project) = st.project.as_ref() else { return };
            let Some(selected) = st.selected.as_ref() else { return };
            let time = st
                .pipeline
                .query_position::<gst::ClockTime>()
                .map(|p| p.nseconds() as f64 / 1e9)
                .unwrap_or(0.0);
            let _ = area;
            for (id, x, y, bw, bh) in active_clips_at(project, time) {
                if &id != selected {
                    continue;
                }
                let (cw, ch) = (project.meta.width as f64, project.meta.height as f64);
                let scale = (w as f64 / cw).min(h as f64 / ch);
                let (ox, oy) = ((w as f64 - cw * scale) / 2.0, (h as f64 - ch * scale) / 2.0);
                cr.set_source_rgba(0.35, 0.41, 1.0, 0.95);
                cr.set_line_width(2.0);
                cr.rectangle(ox + x * scale, oy + y * scale, bw * scale, bh * scale);
                let _ = cr.stroke();
                // corner handles
                cr.set_source_rgba(0.35, 0.41, 1.0, 1.0);
                for (hx, hy) in [
                    (x, y), (x + bw, y), (x, y + bh), (x + bw, y + bh),
                ] {
                    cr.rectangle(ox + hx * scale - 4.0, oy + hy * scale - 4.0, 8.0, 8.0);
                    let _ = cr.fill();
                }
            }
        });
    }
    {
        // Click to select the topmost/smallest clip under the pointer.
        let editor = editor.clone();
        let canvas = sel_canvas.clone();
        let click = gtk::GestureClick::new();
        click.connect_pressed(move |g, _, wx, wy| {
            let widget = g.widget().unwrap();
            let (w, h) = (widget.width() as f64, widget.height() as f64);
            let hit = {
                let st = editor.state.borrow();
                let Some(project) = st.project.as_ref() else { return };
                let Some((cx, cy, _)) = widget_to_comp(project, w, h, wx, wy) else { return };
                let time = st
                    .pipeline
                    .query_position::<gst::ClockTime>()
                    .map(|p| p.nseconds() as f64 / 1e9)
                    .unwrap_or(0.0);
                active_clips_at(project, time)
                    .into_iter()
                    .filter(|(_, x, y, bw, bh)| cx >= *x && cx <= x + bw && cy >= *y && cy <= y + bh)
                    .min_by(|a, b| (a.3 * a.4).total_cmp(&(b.3 * b.4)))
                    .map(|(id, ..)| id)
            };
            if let Some(id) = hit {
                editor.state.borrow_mut().selected = Some(id);
                editor.rebuild_inspector();
                canvas.queue_draw();
            }
        });
        sel_canvas.add_controller(click);
    }
    {
        // Drag the selected clip to move it (commits on release).
        let editor = editor.clone();
        let canvas = sel_canvas.clone();
        let drag = gtk::GestureDrag::new();
        let orig: Rc<std::cell::Cell<(f64, f64, f64)>> = Rc::new(std::cell::Cell::new((0.0, 0.0, 1.0)));
        {
            let editor = editor.clone();
            let orig = orig.clone();
            drag.connect_drag_begin(move |g, _, _| {
                let widget = g.widget().unwrap();
                let (w, h) = (widget.width() as f64, widget.height() as f64);
                let st = editor.state.borrow();
                let (Some(project), Some(selected)) = (st.project.as_ref(), st.selected.as_ref())
                else {
                    return;
                };
                let scale = {
                    let (cw, ch) = (project.meta.width as f64, project.meta.height as f64);
                    (w / cw).min(h / ch)
                };
                if let Some(clip) = find_clip(project, selected) {
                    orig.set((clip.transform.x, clip.transform.y, scale));
                }
            });
        }
        {
            let editor = editor.clone();
            let orig = orig.clone();
            drag.connect_drag_end(move |_, dx, dy| {
                if dx.abs() < 2.0 && dy.abs() < 2.0 {
                    return;
                }
                let (ox, oy, scale) = orig.get();
                if scale <= 0.0 {
                    return;
                }
                let (project, selected) = {
                    let st = editor.state.borrow();
                    (st.project.clone(), st.selected.clone())
                };
                let (Some(project), Some(selected)) = (project, selected) else { return };
                let mut project = project;
                if let Some(clip) = find_clip_mut(&mut project, &selected) {
                    clip.transform.x = (ox + dx / scale).round();
                    clip.transform.y = (oy + dy / scale).round();
                }
                editor.commit_document(project);
                canvas.queue_draw();
            });
        }
        sel_canvas.add_controller(drag);
    }

    let play = gtk::Button::from_icon_name("media-playback-start-symbolic");
    play.update_property(&[gtk::accessible::Property::Label("Play/Pause")]);
    let time_label = gtk::Label::new(Some("0:00.0"));
    time_label.add_css_class("numeric");
    let seek = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, duration.max(0.1), 0.05);
    seek.set_hexpand(true);
    seek.set_draw_value(false);

    {
        let editor = editor.clone();
        play.connect_clicked(move |btn| {
            let pipeline = editor.state.borrow().pipeline.clone();
            let playing = pipeline.current_state() == gst::State::Playing;
            let next = if playing { gst::State::Paused } else { gst::State::Playing };
            let _ = pipeline.set_state(next);
            btn.set_icon_name(if playing {
                "media-playback-start-symbolic"
            } else {
                "media-playback-pause-symbolic"
            });
        });
    }
    {
        let editor = editor.clone();
        seek.connect_change_value(move |_, _, value| {
            seek_to(&editor.state.borrow().pipeline, value);
            glib::Propagation::Proceed
        });
    }

    // Position updates + external live reload.
    {
        let editor = editor.clone();
        let seek = seek.clone();
        let time_label = time_label.clone();
        let sel_canvas = sel_canvas.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            // Redraw the selection overlay only while something is selected.
            if editor.state.borrow().selected.is_some() {
                sel_canvas.queue_draw();
            }
            {
                let st = editor.state.borrow();
                if let Some(pos) = st.pipeline.query_position::<gst::ClockTime>() {
                    let secs = pos.nseconds() as f64 / 1e9;
                    seek.set_value(secs);
                    time_label.set_text(&format!(
                        "{}:{:04.1} / {}:{:04.1}",
                        (secs / 60.0) as u32,
                        secs % 60.0,
                        (st.duration / 60.0) as u32,
                        st.duration % 60.0
                    ));
                }
            }
            let reload = {
                let mut st = editor.state.borrow_mut();
                match st.project_path.clone() {
                    Some(path) => {
                        let new_mtime = path.metadata().ok().and_then(|m| m.modified().ok());
                        if new_mtime.is_some() && new_mtime != st.mtime {
                            st.mtime = new_mtime;
                            if st.self_write {
                                st.self_write = false;
                                None
                            } else {
                                Some(path)
                            }
                        } else {
                            None
                        }
                    }
                    None => None,
                }
            };
            if let Some(path) = reload {
                match std::fs::read_to_string(&path)
                    .map_err(anyhow::Error::from)
                    .and_then(|j| Project::from_json(&j))
                {
                    Ok(project) => {
                        editor.state.borrow_mut().project = Some(project);
                        editor.rebuild();
                        println!("project reloaded from disk");
                    }
                    Err(e) => eprintln!("reload failed (keeping current timeline): {e:#}"),
                }
            }
            glib::ControlFlow::Continue
        });
    }

    let bar = adw::HeaderBar::new();

    // Open (split button with recents) + Import (#15).
    let open_btn = adw::SplitButton::new();
    open_btn.set_label("Open");
    {
        let editor = editor.clone();
        open_btn.connect_clicked(move |btn| {
            let window = btn.root().and_downcast::<gtk::Window>();
            let dialog = gtk::FileDialog::builder().title("Open project").build();
            let filter = gtk::FileFilter::new();
            filter.add_suffix("json");
            let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));
            let editor = editor.clone();
            dialog.open(window.as_ref(), gtk::gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res
                    && let Some(path) = file.path() {
                        editor.open_project(&path);
                    }
            });
        });
    }
    {
        // Recents popover; placeholder row when there is no history yet.
        let pop = gtk::Popover::new();
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("boxed-list");
        let recents = load_recents();
        if recents.is_empty() {
            let empty = gtk::Label::new(Some("No recent projects"));
            empty.add_css_class("dim-label");
            empty.set_margin_top(8);
            empty.set_margin_bottom(8);
            empty.set_margin_start(12);
            empty.set_margin_end(12);
            let lbrow = gtk::ListBoxRow::new();
            lbrow.set_activatable(false);
            lbrow.set_child(Some(&empty));
            list.append(&lbrow);
        }
        for path in recents {
            let row = gtk::Button::with_label(
                path.file_name().and_then(|n| n.to_str()).unwrap_or("project"),
            );
            row.add_css_class("flat");
            row.set_tooltip_text(path.to_str());
            let editor = editor.clone();
            let pop2 = pop.clone();
            row.connect_clicked(move |_| {
                pop2.popdown();
                editor.open_project(&path);
            });
            let lbrow = gtk::ListBoxRow::new();
            lbrow.set_child(Some(&row));
            list.append(&lbrow);
        }
        pop.set_child(Some(&list));
        open_btn.set_popover(Some(&pop));
    }
    bar.pack_start(&open_btn);
    let import_btn = gtk::Button::with_label("Import");
    import_btn.set_tooltip_text(Some("Import media into the library"));
    {
        let editor = editor.clone();
        import_btn.connect_clicked(move |btn| {
            let window = btn.root().and_downcast::<gtk::Window>();
            editor.import_media(window.as_ref());
        });
    }
    bar.pack_start(&import_btn);
    let left_toggle = gtk::ToggleButton::new();
    left_toggle.set_icon_name("sidebar-show-symbolic");
    left_toggle.update_property(&[gtk::accessible::Property::Label("Toggle left panel")]);
    left_toggle.set_tooltip_text(Some("Toggle left panel (Library / Templates / Code / Script)"));
    left_toggle.set_active(true);
    bar.pack_start(&left_toggle);
    let timeline_toggle = gtk::ToggleButton::new();
    timeline_toggle.set_icon_name("view-continuous-symbolic");
    timeline_toggle.update_property(&[gtk::accessible::Property::Label("Toggle timeline pane")]);
    timeline_toggle.set_tooltip_text(Some("Toggle timeline pane"));
    timeline_toggle.set_active(true);
    bar.pack_start(&timeline_toggle);
    let export = gtk::Button::from_icon_name("document-save-symbolic");
    export.update_property(&[gtk::accessible::Property::Label("Export video")]);
    export.set_tooltip_text(Some("Export video"));
    {
        let editor = editor.clone();
        export.connect_clicked(move |btn| {
            let window = btn.root().and_downcast::<gtk::Window>();
            show_export_dialog(&editor, window.as_ref());
        });
    }
    bar.pack_start(&export);
    let menu = gtk::gio::Menu::new();
    menu.append(Some("New Project"), Some("app.new-project"));
    menu.append(Some("Save Project As…"), Some("app.save-as"));
    menu.append(Some("Install Agent Skills…"), Some("app.install-skills"));
    menu.append(Some("Preferences"), Some("app.preferences"));
    let sec2 = gtk::gio::Menu::new();
    sec2.append(Some("Keyboard Shortcuts"), Some("app.shortcuts"));
    sec2.append(Some("About Dualcut"), Some("app.about"));
    menu.append_section(None, &sec2);
    let burger = gtk::MenuButton::new();
    burger.set_icon_name("open-menu-symbolic");
    burger.update_property(&[gtk::accessible::Property::Label("Main menu")]);
    burger.set_menu_model(Some(&menu));
    bar.pack_end(&burger);
    let right_toggle = gtk::ToggleButton::new();
    right_toggle.set_icon_name("sidebar-show-right-symbolic");
    right_toggle.update_property(&[gtk::accessible::Property::Label("Toggle inspector panel")]);
    right_toggle.set_tooltip_text(Some("Toggle inspector panel"));
    right_toggle.set_active(true);
    bar.pack_end(&right_toggle);

    let transport = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    transport.set_margin_start(12);
    transport.set_margin_end(12);
    transport.append(&play);
    transport.append(&seek);
    transport.append(&time_label);

    // Bottom pane: the multitrack timeline, toggleable (GNOME Builder style).
    let strip = gtk::Box::new(gtk::Orientation::Vertical, 4);
    strip.set_margin_start(12);
    strip.set_margin_end(12);
    strip.set_margin_bottom(8);
    let strip_scroll = gtk::ScrolledWindow::new();
    strip_scroll.set_child(Some(&strip));
    strip_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    strip_scroll.set_min_content_height(150);
    // Bottom pane is a resizable Paned child; the toggle hides it.
    {
        let strip_scroll = strip_scroll.clone();
        timeline_toggle.connect_toggled(move |b| strip_scroll.set_visible(b.is_active()));
    }

    // Center: preview + transport.
    let center = gtk::Box::new(gtk::Orientation::Vertical, 4);
    center.append(&preview_overlay);
    center.append(&transport);

    // Left: Media | Code tabs.
    let media_grid = gtk::FlowBox::new();
    media_grid.set_selection_mode(gtk::SelectionMode::None);
    media_grid.set_min_children_per_line(2);
    media_grid.set_max_children_per_line(3);
    media_grid.set_homogeneous(true);
    let media_empty = gtk::Box::new(gtk::Orientation::Vertical, 8);
    media_empty.set_valign(gtk::Align::Center);
    media_empty.set_margin_top(24);
    let empty_label = gtk::Label::new(Some("No media imported —
add files to import first"));
    empty_label.add_css_class("dim-label");
    empty_label.set_justify(gtk::Justification::Center);
    media_empty.append(&empty_label);
    let empty_import = gtk::Button::with_label("Import…");
    empty_import.set_halign(gtk::Align::Center);
    {
        let editor = editor.clone();
        empty_import.connect_clicked(move |btn| {
            let window = btn.root().and_downcast::<gtk::Window>();
            editor.import_media(window.as_ref());
        });
    }
    media_empty.append(&empty_import);
    let media_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    media_page.append(&media_empty);
    media_page.append(&media_grid);
    let media_scroll = gtk::ScrolledWindow::new();
    media_scroll.set_child(Some(&media_page));
    media_scroll.set_vexpand(true);

    let code_buffer = gtk::TextBuffer::new(None);
    let code_view = gtk::TextView::with_buffer(&code_buffer);
    code_view.set_monospace(true);
    let code_scroll = gtk::ScrolledWindow::new();
    code_scroll.set_child(Some(&code_view));
    code_scroll.set_vexpand(true);
    let code_apply = gtk::Button::with_label("Apply JSON");
    code_apply.add_css_class("suggested-action");
    let code_status = gtk::Label::new(None);
    code_status.set_halign(gtk::Align::Start);
    code_status.set_wrap(true);
    {
        let editor = editor.clone();
        let code_buffer = code_buffer.clone();
        let code_status = code_status.clone();
        code_apply.connect_clicked(move |_| {
            let text = code_buffer
                .text(&code_buffer.start_iter(), &code_buffer.end_iter(), false)
                .to_string();
            match Project::from_json(&text) {
                Ok(project) => {
                    code_status.set_text("✓ applied");
                    editor.commit_document(project);
                }
                Err(e) => code_status.set_text(&format!("✗ {e:#}")),
            }
        });
    }
    let code_page = gtk::Box::new(gtk::Orientation::Vertical, 6);
    code_page.append(&code_scroll);
    code_page.append(&code_apply);
    code_page.append(&code_status);

    let templates_list = gtk::ListBox::new();
    templates_list.add_css_class("boxed-list");
    templates_list.set_selection_mode(gtk::SelectionMode::None);
    let templates_scroll = gtk::ScrolledWindow::new();
    templates_scroll.set_child(Some(&templates_list));
    templates_scroll.set_vexpand(true);

    let left_tabs = gtk::Notebook::new();
    left_tabs.append_page(&media_scroll, Some(&gtk::Label::new(Some("Library"))));
    left_tabs.append_page(&templates_scroll, Some(&gtk::Label::new(Some("Templates"))));
    left_tabs.append_page(&code_page, Some(&gtk::Label::new(Some("Code"))));
    #[cfg(feature = "scripting")]
    {
        let script_page = build_script_panel(&editor);
        left_tabs.append_page(&script_page, Some(&gtk::Label::new(Some("Script"))));
    }
    left_tabs.set_size_request(260, -1);

    // Right: parameters (Inspect | Script), as before.
    let inspector = gtk::Box::new(gtk::Orientation::Vertical, 6);
    inspector.set_margin_top(8);
    inspector.set_margin_start(8);
    inspector.set_margin_end(8);
    inspector.set_size_request(300, -1);

    // Right sidebar is parameters only; scripting lives with the other
    // code views in the left notebook.
    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let inspector_scroll = gtk::ScrolledWindow::new();
    inspector_scroll.set_child(Some(&inspector));
    inspector_scroll.set_vexpand(true);
    inspector_scroll.set_min_content_width(280);
    inspector_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    sidebar.append(&inspector_scroll);

    let inner = gtk::Paned::new(gtk::Orientation::Horizontal);
    inner.set_start_child(Some(&center));
    inner.set_end_child(Some(&sidebar));
    inner.set_resize_start_child(true);
    inner.set_shrink_end_child(false);
    inner.set_position(620);

    let outer = gtk::Paned::new(gtk::Orientation::Horizontal);
    outer.set_start_child(Some(&left_tabs));
    outer.set_end_child(Some(&inner));
    outer.set_resize_end_child(true);
    outer.set_shrink_start_child(false);
    outer.set_position(260);
    outer.set_vexpand(true);

    {
        let left_tabs = left_tabs.clone();
        left_toggle.connect_toggled(move |b| left_tabs.set_visible(b.is_active()));
    }
    {
        let sidebar = sidebar.clone();
        right_toggle.connect_toggled(move |b| sidebar.set_visible(b.is_active()));
    }

    let vpaned = gtk::Paned::new(gtk::Orientation::Vertical);
    vpaned.set_start_child(Some(&outer));
    vpaned.set_end_child(Some(&strip_scroll));
    vpaned.set_resize_start_child(true);
    vpaned.set_shrink_start_child(false);
    vpaned.set_shrink_end_child(false);
    vpaned.set_position(520);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&bar);
    content.append(&vpaned);
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&content));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(&window_title)
        .default_width(1120)
        .default_height(700)
        .content(&toasts)
        .build();

    {
        let controller = gtk::EventControllerKey::new();
        let editor = editor.clone();
        let play_btn = play.clone();
        controller.connect_key_pressed(move |_, key, _, modifier| {
            if modifier.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                && (key == gtk::gdk::Key::y || key == gtk::gdk::Key::Y)
            {
                editor.redo();
                return glib::Propagation::Stop;
            }
            if modifier.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                && (key == gtk::gdk::Key::z || key == gtk::gdk::Key::Z)
            {
                if modifier.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
                    editor.redo();
                } else {
                    editor.undo();
                }
                return glib::Propagation::Stop;
            }
            if modifier.is_empty() {
                let pipeline = editor.state.borrow().pipeline.clone();
                match key {
                    gtk::gdk::Key::space => {
                        let playing = pipeline.current_state() == gst::State::Playing;
                        let next = if playing { gst::State::Paused } else { gst::State::Playing };
                        let _ = pipeline.set_state(next);
                        play_btn.set_icon_name(if playing {
                            "media-playback-start-symbolic"
                        } else {
                            "media-playback-pause-symbolic"
                        });
                        return glib::Propagation::Stop;
                    }
                    gtk::gdk::Key::Left | gtk::gdk::Key::Right => {
                        let fps = editor
                            .state
                            .borrow()
                            .project
                            .as_ref()
                            .map_or(30.0, |p| p.meta.fps as f64);
                        let pos = pipeline
                            .query_position::<gst::ClockTime>()
                            .map(|p| p.nseconds() as f64 / 1e9)
                            .unwrap_or(0.0);
                        let step = if key == gtk::gdk::Key::Left { -1.0 } else { 1.0 } / fps;
                        let _ = pipeline.set_state(gst::State::Paused);
                        seek_to(&pipeline, (pos + step).max(0.0));
                        return glib::Propagation::Stop;
                    }
                    gtk::gdk::Key::Home => {
                        seek_to(&pipeline, 0.0);
                        return glib::Propagation::Stop;
                    }
                    gtk::gdk::Key::End => {
                        let end = editor
                            .state
                            .borrow()
                            .project
                            .as_ref()
                            .map_or(0.0, |p| p.duration());
                        seek_to(&pipeline, (end - 0.05).max(0.0));
                        return glib::Propagation::Stop;
                    }
                    _ => {}
                }
            }
            glib::Propagation::Proceed
        });
        window.add_controller(controller);
    }

    let css = gtk::CssProvider::new();
    css.load_from_data(
        "button.clip-text { background: alpha(#e5a50a, .25); }
         button.clip-video { background: alpha(#3584e4, .25); }
         button.clip-audio { background: alpha(#33d17a, .25); }
         button.clip-image { background: alpha(#9141ac, .25); }
         button.clip-shape { background: alpha(#e66100, .25); }
         button.clip-compref { background: alpha(#986a44, .25); }
         button.clip-test { background: alpha(#77767b, .3); }",
    );
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // App actions for the hamburger menu (#13, #14).
    {
        let make = |name: &str| gtk::gio::SimpleAction::new(name, None);
        let a = make("new-project");
        {
            let editor = editor.clone();
            a.connect_activate(move |_, _| {
                let project = dualcut_engine::templates::new_project("New Project");
                {
                    let mut st = editor.state.borrow_mut();
                    st.project_path = None;
                    st.undo.clear();
                    st.redo.clear();
                    st.selected = None;
                }
                if let Some(win) = editor.window() {
                    win.set_title(Some("dualcut — New Project (unsaved)"));
                }
                editor.rebuild_in_memory(project);
            });
        }
        app.add_action(&a);
        let a = make("save-as");
        {
            let editor = editor.clone();
            a.connect_activate(move |_, _| {
                let win = editor.window();
                editor.save_project_as(win.as_ref());
            });
        }
        app.add_action(&a);
        let a = make("install-skills");
        {
            let editor = editor.clone();
            a.connect_activate(move |_, _| {
                let win = editor.window();
                show_skills_dialog(&editor, win.as_ref());
            });
        }
        app.add_action(&a);
        let a = make("about");
        {
            let editor = editor.clone();
            a.connect_activate(move |_, _| {
                show_about(editor.window().as_ref());
            });
        }
        app.add_action(&a);
        let a = make("shortcuts");
        {
            let editor = editor.clone();
            a.connect_activate(move |_, _| {
                let dialog = adw::AlertDialog::new(
                    Some("Keyboard Shortcuts"),
                    Some(
                        "Space — play / pause\n\
                         ← / → — step one frame\n\
                         Home / End — go to start / end\n\
                         Ctrl+Z — undo\n\
                         Ctrl+Shift+Z or Ctrl+Y — redo\n\
                         Drag clip — move (vertical: change lane)\n\
                         Drag clip right edge — trim\n\
                         Drop files — import into library",
                    ),
                );
                dialog.add_response("ok", "Close");
                dialog.present(editor.window().as_ref());
            });
        }
        app.add_action(&a);
        let a = make("preferences");
        a.set_enabled(false);
        app.add_action(&a);
    }

    // UI smoke-test hook: DUALCUT_TEST_ACTION=<name> activates an app
    // action shortly after startup (headless CI has no reliable pointer).
    if let Ok(name) = std::env::var("DUALCUT_TEST_ACTION") {
        let app = app.clone();
        glib::timeout_add_seconds_local_once(2, move || {
            gtk::prelude::ActionGroupExt::activate_action(&app, &name, None);
        });
    }

    // Walkthrough mode: step through UI states, printing a shot marker
    // after each settles; scripts/walkthrough.sh screenshots per marker.
    // Drives the guide images regenerated on every release.
    if std::env::var("DUALCUT_WALKTHROUGH").is_ok() {
        let untitled = editor.state.borrow().project_path.is_none();
        type Step = (&'static str, Box<dyn Fn()>);
        let mut steps: Vec<Step> = Vec::new();
        if untitled {
            steps.push(("new-project", Box::new(|| {})));
        } else {
            steps.push(("editor-overview", Box::new(|| {})));
            {
                let editor = editor.clone();
                steps.push(("library", Box::new(move || {
                    let project = editor.state.borrow().project.clone();
                    if let Some(mut project) = project {
                        project.library =
                            vec!["assets/ball.mp4".into(), "assets/ticks.ogg".into()];
                        editor.commit_document(project);
                    }
                })));
            }
            {
                let tabs = left_tabs.clone();
                steps.push(("templates", Box::new(move || tabs.set_current_page(Some(1)))));
            }
            {
                let tabs = left_tabs.clone();
                steps.push(("code-view", Box::new(move || tabs.set_current_page(Some(2)))));
            }
            {
                let editor = editor.clone();
                let tabs = left_tabs.clone();
                steps.push(("clip-inspector", Box::new(move || {
                    tabs.set_current_page(Some(0));
                    editor.state.borrow_mut().selected = Some("media-ball".into());
                    editor.rebuild_inspector();
                })));
            }
            {
                let editor = editor.clone();
                steps.push(("scene-form", Box::new(move || {
                    editor.state.borrow_mut().selected = Some("scene:scene-media".into());
                    editor.rebuild_inspector();
                })));
            }
            {
                let app = app.clone();
                steps.push(("about", Box::new(move || {
                    gtk::prelude::ActionGroupExt::activate_action(&app, "about", None);
                })));
            }
        }
        let steps = Rc::new(steps);
        let index = Rc::new(std::cell::Cell::new(0usize));
        let app2 = app.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(3000), move || {
            let i = index.get();
            if i >= steps.len() {
                // Give the driver time to grab the last frame, then exit.
                let app3 = app2.clone();
                glib::timeout_add_local_once(
                    std::time::Duration::from_millis(1500),
                    move || app3.quit(),
                );
                return glib::ControlFlow::Break;
            }
            let (name, run) = &steps[i];
            run();
            let name = *name;
            glib::timeout_add_local_once(std::time::Duration::from_millis(1800), move || {
                use std::io::Write;
                println!("WALKTHROUGH-SHOT {name}");
                let _ = std::io::stdout().flush();
            });
            index.set(i + 1);
            glib::ControlFlow::Continue
        });
    }

    // Drag-and-drop media import: dropping files anywhere on the window
    // adds them to the library.
    {
        let drop = gtk::DropTarget::new(
            gtk::gdk::FileList::static_type(),
            gtk::gdk::DragAction::COPY,
        );
        let editor = editor.clone();
        drop.connect_drop(move |_, value, _, _| {
            let Ok(list) = value.get::<gtk::gdk::FileList>() else { return false };
            let paths: Vec<PathBuf> =
                list.files().iter().filter_map(|f| f.path()).collect();
            if paths.is_empty() {
                return false;
            }
            editor.add_to_library(&paths);
            true
        });
        window.add_controller(drop);
    }

    window.present();
    start_paused(&pipeline)?;

    *editor.ui.borrow_mut() =
        Some(Ui { picture, seek, strip, inspector, media_grid, media_empty, toasts: toasts.clone(), templates_list, code_buffer });
    editor.rebuild_strip();
    editor.rebuild_inspector();
    editor.rebuild_media();
    editor.rebuild_templates();
    editor.refresh_code();
    Ok(())
}
