//! Agent skills installer and update detection dialog.
//!
//! Extracted from `preview.rs` (dualcut#78): discovers bundled skill assets,
//! copies them to target agents/claude skill roots, and presents the GTK install dialog.

use super::*;

/// Locate the bundled agent skill directory (flatpak install or repo).
pub(crate) fn skill_source_dir() -> Option<PathBuf> {
    ["/app/share/dualcut/skills/dualcut", "../skills/dualcut", "skills/dualcut"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.join("SKILL.md").exists())
}

pub(crate) fn copy_dir_recursive(src: &std::path::Path, dest: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)?.flatten() {
        let path = entry.path();
        let target = dest.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else {
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

pub(crate) fn install_skill_to(target_root: &std::path::Path) -> Result<PathBuf> {
    let src = skill_source_dir().context("bundled skill files not found")?;
    let dest = target_root.join("dualcut");
    // references/ (schema + types) travels with the skill so it stays
    // self-contained wherever it's installed (#49).
    copy_dir_recursive(&src, &dest)?;
    prefs_set("skill_install_dir", &target_root.display().to_string());
    Ok(dest)
}

/// Bundled skill differs from what's installed at the recorded location
/// (#49) -- returns the install root (parent of the `dualcut/` skill dir)
/// to reinstall to, if an update is available.
pub(crate) fn skill_update_available() -> Option<PathBuf> {
    let target_root = std::fs::read_to_string(prefs_file()).ok().and_then(|s| {
        s.lines().find_map(|l| l.trim().strip_prefix("skill_install_dir=").map(PathBuf::from))
    })?;
    let src = skill_source_dir()?;
    let bundled = std::fs::read_to_string(src.join("SKILL.md")).ok()?;
    let installed =
        std::fs::read_to_string(target_root.join("dualcut").join("SKILL.md")).ok()?;
    (bundled != installed).then_some(target_root)
}

pub(crate) fn show_skills_dialog(editor: &Rc<Editor>, window: Option<&gtk::Window>) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_dir_recursive_copies_nested_structure() {
        let tmp = std::env::temp_dir().join("dualcut-skills-test-copy");
        let src = tmp.join("src");
        let dest = tmp.join("dest");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("file1.txt"), b"hello").unwrap();
        std::fs::write(src.join("sub").join("file2.txt"), b"world").unwrap();

        copy_dir_recursive(&src, &dest).expect("recursive copy succeeds");
        assert_eq!(std::fs::read_to_string(dest.join("file1.txt")).unwrap(), "hello");
        assert_eq!(std::fs::read_to_string(dest.join("sub").join("file2.txt")).unwrap(), "world");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
