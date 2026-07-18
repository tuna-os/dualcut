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
use document::{detach_audio, find_clip, find_clip_mut, remove_clip, save_as_def};
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
            let Some(path) = st.project_path.clone() else { return };
            let prev = st.project.as_ref().map(|p| p.to_json());
            (path, prev)
        };
        if let Some(prev) = prev_json {
            let mut st = self.state.borrow_mut();
            st.undo.push(prev);
            st.redo.clear();
        }
        self.write_and_rebuild(&path, project);
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
            button.connect_clicked(move |_| {
                seek_to(&this.state.borrow().pipeline, offset);
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
                );
            }
            row.append(&lane);
            ui.strip.append(&row);
        }

        for track in &project.overlays {
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
    ) {
        let project = {
            let st = self.state.borrow();
            st.project.clone()
        };
        let Some(project) = project else { return };
        let cache = self.base_dir().join(".dualcut-cache");

        let button = gtk::Button::with_label(&clip.id);
        button.add_css_class("flat");
        let width_px = ((duration * SCENE_PX_PER_SEC) as i32).max(30);
        button.set_size_request(width_px, 28);
        button.set_tooltip_text(Some("Drag to move · right edge trims"));
        if let document::Element::Audio { src, .. } = &clip.element {
            if let Some(uri) = media_uri(src, &self.base_dir()) {
                let wave = cache.join(format!("wave-{:016x}.png", fx_hash(&uri)));
                if wave.exists() {
                    let pic = gtk::Picture::for_filename(&wave);
                    pic.set_content_fit(gtk::ContentFit::Fill);
                    button.set_child(Some(&pic));
                    button.set_tooltip_text(Some(&clip.id));
                }
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
            drag.connect_drag_end(move |_, dx, _| {
                if dx.abs() < 2.0 {
                    return;
                }
                let (ox, trim) = dragmeta.get();
                let mut project = project_snapshot.clone();
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
        for clip in all_clips {
            match &clip.element {
                document::Element::Video { src, .. } | document::Element::Image { src } => {
                    if let Some(uri) = media_uri(src, &base_dir) {
                        if !cache.join(format!("thumb-{:016x}.png", fx_hash(&uri))).exists() {
                            thumbs.push(uri);
                        }
                    }
                }
                document::Element::Audio { src, .. } => {
                    if let Some(uri) = media_uri(src, &base_dir) {
                        if !cache.join(format!("wave-{:016x}.png", fx_hash(&uri))).exists() {
                            waves.push(uri);
                        }
                    }
                }
                _ => {}
            }
        }
        if thumbs.is_empty() && waves.is_empty() {
            return;
        }
        let cache = cache.to_path_buf();
        let this = self.clone();
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            for uri in thumbs {
                if let Err(e) = dualcut_engine::thumbs::thumbnail_png(&cache, &uri) {
                    eprintln!("thumbnail failed for {uri}: {e:#}");
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
                for row in rows {
                    remove_clip(&mut project, &ids[row.index() as usize]);
                }
                this.state.borrow_mut().selected = None;
                this.commit_document(project);
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

        // Editor form for the selected clip.
        let Some(selected) = selected else { return };
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

        const PROPS: [&str; 3] = ["x", "y", "opacity"];
        const EASINGS: [&str; 4] = ["linear", "easeIn", "easeOut", "easeInOut"];
        let prop_of = |a: &document::AnimProperty| match a {
            document::AnimProperty::X => 0,
            document::AnimProperty::Y => 1,
            document::AnimProperty::Opacity => 2,
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
                    if let Some(c) = find_clip_mut(&mut project, &clip_id) {
                        if let Some(a) = c.animations.get_mut(ai) {
                            mutate(a);
                        }
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
                        _ => document::AnimProperty::Opacity,
                    };
                    commit(Box::new(move |a| a.property = value));
                });
            }
            row.append(&prop);

            let mut add_spin = |value: f64, lo: f64, hi: f64, set: fn(&mut document::Anim, f64)| {
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
            del.add_css_class("flat");
            {
                let this = self.clone();
                let project = project.clone();
                let clip_id = clip.id.clone();
                del.connect_clicked(move |_| {
                    let mut project = project.clone();
                    if let Some(c) = find_clip_mut(&mut project, &clip_id) {
                        if ai < c.animations.len() {
                            c.animations.remove(ai);
                        }
                    }
                    this.commit_document(project);
                });
            }
            row.append(&del);
            form.append(&row);
        }

        // Presets.
        let presets = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let mut add_preset = |label: &str, make: fn(&document::Clip) -> document::Anim| {
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
        });
        add_preset("+ Fade out", |c| document::Anim {
            property: document::AnimProperty::Opacity,
            from: 1.0, to: 0.0,
            start: (c.duration - 0.5).max(0.0), end: c.duration.max(0.5),
            easing: document::Easing::EaseIn,
        });
        add_preset("+ Slide in", |c| document::Anim {
            property: document::AnimProperty::X,
            from: c.transform.x - 300.0, to: c.transform.x,
            start: 0.0, end: 0.6,
            easing: document::Easing::EaseOut,
        });
        form.append(&presets);

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
        "TypeScript — must export edit(project: Project): Project.\nTypes: engine/schema/dualcut.d.ts",
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
        other => (build_demo_timeline(other.as_deref())?, None, None, 8.0),
    };

    let (pipeline, paintable) = make_pipeline(&timeline)?;
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
        let seek = ();
        let _ = seek;
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
            sel_canvas.queue_draw();
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
    bar.pack_start(&play);
    let export = gtk::Button::from_icon_name("document-save-symbolic");
    export.set_tooltip_text(Some("Export video"));
    {
        let editor = editor.clone();
        export.connect_clicked(move |btn| {
            let window = btn.root().and_downcast::<gtk::Window>();
            show_export_dialog(&editor, window.as_ref());
        });
    }
    bar.pack_start(&export);
    bar.pack_end(&time_label);

    let transport = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    transport.set_margin_start(12);
    transport.set_margin_end(12);
    transport.append(&seek);

    let strip = gtk::Box::new(gtk::Orientation::Vertical, 4);
    strip.set_margin_start(12);
    strip.set_margin_end(12);
    strip.set_margin_bottom(8);
    let strip_scroll = gtk::ScrolledWindow::new();
    strip_scroll.set_child(Some(&strip));
    strip_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
    strip_scroll.set_min_content_height(110);

    let left = gtk::Box::new(gtk::Orientation::Vertical, 4);
    left.append(&preview_overlay);
    left.append(&transport);
    left.append(&strip_scroll);

    let inspector = gtk::Box::new(gtk::Orientation::Vertical, 6);
    inspector.set_margin_top(8);
    inspector.set_margin_start(8);
    inspector.set_margin_end(8);
    inspector.set_size_request(300, -1);

    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let stack = gtk::Stack::new();
    stack.add_titled(&inspector, Some("inspect"), "Inspect");
    #[cfg(feature = "scripting")]
    {
        let script_page = build_script_panel(&editor);
        stack.add_titled(&script_page, Some("script"), "Script");
    }
    let switcher = gtk::StackSwitcher::new();
    switcher.set_stack(Some(&stack));
    switcher.set_halign(gtk::Align::Center);
    switcher.set_margin_top(6);
    sidebar.append(&switcher);
    sidebar.append(&stack);
    stack.set_vexpand(true);

    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    paned.set_start_child(Some(&left));
    paned.set_end_child(Some(&sidebar));
    paned.set_resize_start_child(true);
    paned.set_shrink_end_child(false);
    paned.set_position(760);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&bar);
    content.append(&paned);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("dualcut")
        .default_width(1120)
        .default_height(700)
        .content(&content)
        .build();

    {
        let controller = gtk::EventControllerKey::new();
        let editor = editor.clone();
        controller.connect_key_pressed(move |_, key, _, modifier| {
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
            glib::Propagation::Proceed
        });
        window.add_controller(controller);
    }

    window.present();
    start_paused(&pipeline)?;

    *editor.ui.borrow_mut() = Some(Ui { picture, seek, strip, inspector });
    editor.rebuild_strip();
    editor.rebuild_inspector();
    Ok(())
}
