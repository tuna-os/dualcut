//! Video and audio export dialog, render queue, and output file resolution.
//!
//! Extracted from `preview.rs` (dualcut#78): `export_target` builds output paths,
//! `show_export_dialog` manages format profiles, render queues, progress tracking,
//! and overwrite confirmation.

use super::*;

pub(crate) fn export_target(dir: &std::path::Path, name: &str) -> String {
    dir.join(name).display().to_string()
}

/// Kicks off (or queues) one render: (Export button, output path, profile).
pub(crate) type StartRender = Rc<dyn Fn(gtk::Button, String, String)>;

pub(crate) fn show_export_dialog(editor: &Rc<Editor>, parent: Option<&gtk::Window>) {
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

    // Separate directory + file name (#27); name defaults to the project
    // slug with a timestamp suffix so repeated exports never collide.
    let slug: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let stamp = glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.format("%y%m%d_%H%M").ok())
        .map(|s| s.to_string())
        .unwrap_or_default();
    let out_dir = Rc::new(std::cell::RefCell::new(base_dir.clone()));
    let dir_btn = gtk::Button::with_label(&base_dir.display().to_string());
    dir_btn.set_tooltip_text(Some("Choose output directory"));
    {
        let out_dir = out_dir.clone();
        dir_btn.connect_clicked(move |btn| {
            let picker = gtk::FileDialog::builder().title("Choose output directory").build();
            let window = btn.root().and_downcast::<gtk::Window>();
            let out_dir = out_dir.clone();
            let btn = btn.clone();
            picker.select_folder(window.as_ref(), gtk::gio::Cancellable::NONE, move |res| {
                if let Ok(dir) = res
                    && let Some(path) = dir.path()
                {
                    btn.set_label(&path.display().to_string());
                    *out_dir.borrow_mut() = path;
                }
            });
        });
    }

    let dir_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    dir_row.append(&gtk::Label::builder().label("Directory").halign(gtk::Align::Start).build());
    dir_row.append(&dir_btn);
    content.append(&dir_row);

    let default_name = format!("{slug}-{stamp}.mp4");
    let out_entry = gtk::Entry::builder().text(&default_name).hexpand(true).build();
    let name_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    name_row.append(&gtk::Label::builder().label("File name").halign(gtk::Align::Start).build());
    name_row.append(&out_entry);
    content.append(&name_row);

    let profile = gtk::DropDown::from_strings(&[
        "mp4 (H.264/AAC, high quality)",
        "webm (VP8/Vorbis, web friendly)",
        "mp4 (H.265/HEVC, smaller file)",
        "webm (VP9/Opus, modern web)",
        "mp4 (AV1/Opus, best compression)",
        "mov (ProRes 422 HQ, archival)",
        "mkv (FFV1/FLAC lossless)",
        "m4a (AAC audio)",
        "ogg (Opus audio)",
        "flac (audio)",
        "mp3 (audio)",
        "wav (audio)",
    ]);
    {
        let out_entry = out_entry.clone();
        profile.connect_selected_notify(move |dd| {
            let ext = match dd.selected() {
                1 | 3 => "webm",
                5 => "mov",
                6 => "mkv",
                7 => "m4a",
                8 => "ogg",
                9 => "flac",
                10 => "mp3",
                11 => "wav",
                _ => "mp4",
            };
            let text = out_entry.text().to_string();
            if let Some(stem) = text.rsplit_once('.').map(|(s, _)| s.to_string()) {
                out_entry.set_text(&format!("{stem}.{ext}"));
            }
        });
    }
    content.append(&gtk::Label::builder().label("Format").halign(gtk::Align::Start).build());
    content.append(&profile);

    let status = gtk::Label::new(None);
    status.set_selectable(true);
    status.set_wrap(true);
    status.add_css_class("monospace");
    let bar = gtk::ProgressBar::new();
    bar.set_show_text(true);
    bar.set_visible(false);
    status.set_halign(gtk::Align::Start);
    status.set_wrap(true);

    let go = gtk::Button::with_label("Export");
    go.add_css_class("suggested-action");
    {
        let status = status.clone();
        let out_entry = out_entry.clone();
        let profile = profile.clone();
        // Self-referencing so a finished render can start the next queued
        // export (#35); the cell is filled right after construction. The
        // poll closure keeps everything alive even if the dialog closes,
        // so renders (and the queue) survive closing the window.
        let start_render_cell: Rc<RefCell<Option<StartRender>>> = Rc::new(RefCell::new(None));
        let start_render: StartRender = {
            let status = status.clone();
            let bar = bar.clone();
            let project_json = project_json.clone();
            let base_dir = base_dir.clone();
            let editor = editor.clone();
            let cell = start_render_cell.clone();
            Rc::new(move |btn: gtk::Button, out: String, prof: String| {
                // A render is already running (possibly started from an
                // earlier dialog): queue this one instead (#35).
                if editor.exporting.get() {
                    let queued = {
                        let mut q = editor.export_queue.borrow_mut();
                        q.push_back((out, prof));
                        q.len()
                    };
                    status.set_text(&format!("Rendering… ({queued} queued)"));
                    return;
                }
                editor.exporting.set(true);
                status.set_text("Rendering…");
                bar.set_visible(true);
                bar.set_fraction(0.0);
                let (tx, rx) = std::sync::mpsc::channel::<std::result::Result<(), String>>();
                let (ptx, prx) = std::sync::mpsc::channel::<f64>();
                {
                    let project_json = project_json.clone();
                    let base_dir = base_dir.clone();
                    let out = out.clone();
                    std::thread::spawn(move || {
                        let result = dualcut_engine::render_project_with_progress(
                            &project_json,
                            &base_dir,
                            &out,
                            &prof,
                            |p| {
                                let _ = ptx.send(p);
                            },
                        )
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
                let bar = bar.clone();
                let editor = editor.clone();
                let cell = cell.clone();
                glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
                    while let Ok(p) = prx.try_recv() {
                        bar.set_fraction(p);
                    }
                    match rx.try_recv() {
                        Ok(Ok(())) => {
                            status.set_text(&format!("✓ exported {out}"));
                            bar.set_fraction(1.0);
                            editor.toast(&format!("✓ exported {out}"));
                        }
                        Ok(Err(e)) => {
                            // Mirror to the terminal so GUI and console
                            // errors always match (#27).
                            eprintln!("export failed: {e}");
                            status.set_text(&format!("✗ {e}"));
                            editor.toast(&format!("✗ export failed: {e}"));
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            return glib::ControlFlow::Continue
                        }
                        Err(_) => {}
                    }
                    // Render finished (or died): start the next queued one.
                    editor.exporting.set(false);
                    let next = editor.export_queue.borrow_mut().pop_front();
                    if let Some((next_out, next_prof)) = next
                        && let Some(start) = cell.borrow().clone()
                    {
                        start(btn.clone(), next_out, next_prof);
                    }
                    glib::ControlFlow::Break
                });
            })
        };
        *start_render_cell.borrow_mut() = Some(start_render.clone());
        go.connect_clicked(move |btn| {
            let out = export_target(&out_dir.borrow(), out_entry.text().trim());
            let prof = match profile.selected() {
                1 => "webm",
                2 => "h265",
                3 => "vp9",
                4 => "av1",
                5 => "prores",
                6 => "ffv1",
                7 => "m4a",
                8 => "ogg",
                9 => "flac",
                10 => "mp3",
                11 => "wav",
                _ => "mp4",
            }
            .to_string();
            // Never silently clobber an existing file (#27).
            if std::path::Path::new(&out).exists() {
                let confirm = adw::AlertDialog::new(
                    Some("Replace existing file?"),
                    Some(&format!("{out} already exists.")),
                );
                confirm.add_response("cancel", "Cancel");
                confirm.add_response("replace", "Replace");
                confirm.set_response_appearance("replace", adw::ResponseAppearance::Destructive);
                confirm.set_default_response(Some("cancel"));
                let start_render = start_render.clone();
                let btn2 = btn.clone();
                confirm.connect_response(Some("replace"), move |_, _| {
                    start_render(btn2.clone(), out.clone(), prof.clone());
                });
                confirm.present(btn.root().and_downcast::<gtk::Window>().as_ref());
                return;
            }
            start_render(btn.clone(), out, prof);
        });
    }
    content.append(&go);
    content.append(&bar);
    content.append(&status);
    dialog.set_child(Some(&content));
    dialog.present();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_target_joins_dir_and_name() {
        let dir = std::path::Path::new("/tmp/out");
        assert_eq!(export_target(dir, "video.mp4"), "/tmp/out/video.mp4");
    }
}
