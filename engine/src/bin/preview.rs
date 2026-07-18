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
use document::{detach_audio, find_clip, find_clip_mut, remove_clip};
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

        let scene_row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        for (i, scene) in project.scenes.iter().enumerate() {
            let label = if scene.name.is_empty() { &scene.id } else { &scene.name };
            let button = gtk::Button::with_label(&format!("{label}\n{:.1}s", scene.duration));
            button.set_size_request((scene.duration * SCENE_PX_PER_SEC) as i32, 48);
            button.set_tooltip_text(Some(&scene.id));
            let offset = project.scene_offset(i);
            let this = self.clone();
            button.connect_clicked(move |_| {
                seek_to(&this.state.borrow().pipeline, offset);
            });
            scene_row.append(&button);
        }
        ui.strip.append(&scene_row);

        for track in &project.overlays {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
            let name = if track.name.is_empty() { &track.id } else { &track.name };
            let tag = gtk::Label::new(Some(&format!("〜 {name}")));
            tag.add_css_class("dim-label");
            row.append(&tag);
            for clip in &track.clips {
                let button = gtk::Button::with_label(&clip.id);
                button.add_css_class("flat");
                let start = clip.start;
                let this = self.clone();
                let id = clip.id.clone();
                button.connect_clicked(move |_| {
                    {
                        let mut st = this.state.borrow_mut();
                        seek_to(&st.pipeline, start);
                        st.selected = Some(id.clone());
                    }
                    this.rebuild_inspector();
                });
                row.append(&button);
            }
            ui.strip.append(&row);
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
            list.connect_row_selected(move |_, row| {
                let Some(row) = row else { return };
                let id = ids[row.index() as usize].clone();
                let changed = {
                    let mut st = this.state.borrow_mut();
                    let changed = st.selected.as_ref() != Some(&id);
                    st.selected = Some(id);
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
        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
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
    left.append(&picture);
    left.append(&transport);
    left.append(&strip_scroll);

    let inspector = gtk::Box::new(gtk::Orientation::Vertical, 6);
    inspector.set_margin_top(8);
    inspector.set_margin_start(8);
    inspector.set_margin_end(8);
    inspector.set_size_request(300, -1);

    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    paned.set_start_child(Some(&left));
    paned.set_end_child(Some(&inspector));
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
