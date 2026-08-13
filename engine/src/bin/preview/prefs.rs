//! Preferences and recents — the flat `key=value` prefs file and the
//! recent-projects list.
//!
//! Extracted from `preview.rs` (dualcut#78): `prefs_*` read/write the
//! glib user-config prefs file, `preview_scale` / font-family / proxy /
//! script helpers wrap it, and `recents_*` manage the MRU project list.
//! The `keys` module and the skills installer consume `prefs_set` /
//! `prefs_file` through this module.

use super::*;

pub(crate) fn preview_scale() -> f64 {
    std::fs::read_to_string(prefs_file())
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| l.trim().strip_prefix("preview_scale=").map(str::to_string))
        })
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.5)
}

pub(crate) fn prefs_set_preview_scale(value: f64) {
    prefs_set("preview_scale", &value.to_string());
}

/// Font family used for new text objects and built-in templates (#61).
/// Every built-in font spec (starter templates, `Element::Text` defaults)
/// is a Pango description with family "Sans" -- swapping just that prefix
/// is enough without needing a real Pango-description parser.
pub(crate) fn prefs_default_font_family() -> String {
    std::fs::read_to_string(prefs_file())
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| l.trim().strip_prefix("default_font_family=").map(str::to_string))
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Sans".to_string())
}

pub(crate) fn prefs_set_default_font_family(value: &str) {
    prefs_set("default_font_family", value);
}

/// Rewrite a Pango font description's family to the configured default,
/// e.g. "Sans Bold 32" -> "Verdana Bold 32". No-op for specs that don't
/// start with the "Sans" family this codebase's built-in fonts all use.
pub(crate) fn apply_default_font_family(spec: &str) -> String {
    let family = prefs_default_font_family();
    if family == "Sans" {
        return spec.to_string();
    }
    match spec.strip_prefix("Sans") {
        Some(rest) => format!("{family}{rest}"),
        None => spec.to_string(),
    }
}

/// Preview-only proxy media (960px MJPEG transcodes) — on unless disabled.
pub(crate) fn prefs_use_proxies() -> bool {
    std::fs::read_to_string(prefs_file())
        .map(|s| !s.lines().any(|l| l.trim() == "use_proxies=false"))
        .unwrap_or(true)
}

pub(crate) fn prefs_set_use_proxies(value: bool) {
    prefs_set("use_proxies", &value.to_string());
}

/// Rewrite one `key=value` line in the prefs file, preserving every other key.
pub(crate) fn prefs_set(key: &str, value: &str) {
    let file = prefs_file();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let prefix = format!("{key}=");
    let mut lines: Vec<String> = std::fs::read_to_string(&file)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim().starts_with(&prefix))
        .map(str::to_string)
        .collect();
    lines.push(format!("{key}={value}"));
    let _ = std::fs::write(&file, lines.join("\n") + "\n");
}

pub(crate) fn prefs_file() -> PathBuf {
    glib::user_config_dir().join("dualcut").join("prefs")
}

pub(crate) fn prefs_show_script() -> bool {
    std::fs::read_to_string(prefs_file())
        .map(|s| s.lines().any(|l| l.trim() == "show_script=true"))
        .unwrap_or(false)
}

pub(crate) fn prefs_set_show_script(value: bool) {
    prefs_set("show_script", &value.to_string());
}

pub(crate) fn recents_file() -> PathBuf {
    glib::user_config_dir().join("dualcut").join("recent-projects")
}

pub(crate) fn load_recents() -> Vec<PathBuf> {
    std::fs::read_to_string(recents_file())
        .unwrap_or_default()
        .lines()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .take(8)
        .collect()
}

pub(crate) fn remember_recent(path: &std::path::Path) {
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
