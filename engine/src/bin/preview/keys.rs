//! Keyboard shortcut system: Combo parsing, Action mapping, presets,
//! and prefs persistence.
//!
//! Extracted from `preview.rs` (dualcut#78): `Action` enumerates bindable
//! commands, `Combo` parses and formats shortcut combos, `Preset` matches
//! popular NLE keyboard layouts, and `binding`/`set_binding`/`apply_preset`
//! persist customizations to the prefs file.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    PlayPause,
    StepBack,
    StepForward,
    GoStart,
    GoEnd,
    Split,
    RippleDelete,
    Undo,
    Redo,
}

impl Action {
    pub const ALL: [Action; 9] = [
        Action::PlayPause,
        Action::StepBack,
        Action::StepForward,
        Action::GoStart,
        Action::GoEnd,
        Action::Split,
        Action::RippleDelete,
        Action::Undo,
        Action::Redo,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Action::PlayPause => "play_pause",
            Action::StepBack => "step_back",
            Action::StepForward => "step_forward",
            Action::GoStart => "go_start",
            Action::GoEnd => "go_end",
            Action::Split => "split",
            Action::RippleDelete => "ripple_delete",
            Action::Undo => "undo",
            Action::Redo => "redo",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Action::PlayPause => "Play / Pause",
            Action::StepBack => "Step one frame back",
            Action::StepForward => "Step one frame forward",
            Action::GoStart => "Go to start",
            Action::GoEnd => "Go to end",
            Action::Split => "Split clip at playhead",
            Action::RippleDelete => "Delete selected clip (ripple)",
            Action::Undo => "Undo",
            Action::Redo => "Redo",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Combo {
    pub key: gtk::gdk::Key,
    pub ctrl: bool,
    pub shift: bool,
}

impl Combo {
    pub fn new(key: gtk::gdk::Key, ctrl: bool, shift: bool) -> Self {
        Combo { key, ctrl, shift }
    }

    /// Parse a combo string like `"ctrl+shift+z"` or `"space"`.
    /// Case-insensitive; `+`-separated; last segment is the key name.
    pub fn parse(s: &str) -> Option<Combo> {
        let mut ctrl = false;
        let mut shift = false;
        let mut key_name = "";
        for part in s.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "cmd" | "primary" => ctrl = true,
                "shift" => shift = true,
                _ => key_name = part,
            }
        }
        let key = gtk::gdk::Key::from_name(key_name)
            .or_else(|| gtk::gdk::Key::from_name(key_name.to_ascii_lowercase()))?;
        // Normalize letter keys to their lowercase form so two
        // differently-cased spellings of the same combo (e.g. "ctrl+z"
        // vs "Ctrl+Z", or a Shift-modified capture) compare equal and
        // format identically -- `shift` alone (not the key itself)
        // carries whether Shift is part of the combo.
        Some(Combo::new(key.to_lower(), ctrl, shift))
    }

    /// Render back to the same string format `parse` accepts.
    pub fn format(self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("ctrl+");
        }
        if self.shift {
            s.push_str("shift+");
        }
        s.push_str(self.key.name().as_deref().unwrap_or("?"));
        s
    }

    /// Human-readable form for display, e.g. "Ctrl+Shift+Z".
    pub fn display(self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        let name = self.key.name().map(|n| n.to_string()).unwrap_or_else(|| "?".into());
        parts.push(match name.as_str() {
            "space" => "Space".into(),
            "Left" | "Right" | "Home" | "End" | "Delete" => name,
            short if short.len() == 1 => short.to_uppercase(),
            other => other.to_string(),
        });
        parts.join("+")
    }

    /// GTK accelerator syntax for `adw::ShortcutsItem`, e.g. "<Ctrl>Z".
    pub fn gtk_accel(self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("<Ctrl>");
        }
        if self.shift {
            s.push_str("<Shift>");
        }
        let name = self.key.name().map(|n| n.to_string()).unwrap_or_default();
        let display_name = if name.len() == 1 { name.to_uppercase() } else { name };
        s.push_str(&display_name);
        s
    }

    pub fn matches(self, key: gtk::gdk::Key, modifier: gtk::gdk::ModifierType) -> bool {
        let ctrl = modifier.contains(gtk::gdk::ModifierType::CONTROL_MASK);
        let shift = modifier.contains(gtk::gdk::ModifierType::SHIFT_MASK);
        // Letter keys arrive as lower/uppercase depending on Shift;
        // normalize both sides to lowercase so "S" and "s" match the
        // same binding regardless of how Shift was involved.
        key.to_lower() == self.key.to_lower() && ctrl == self.ctrl && shift == self.shift
    }
}

/// dualcut's own defaults -- also the fallback for any action a preset
/// (e.g. iMovie) doesn't have a real equivalent for.
pub const DEFAULT: [(Action, &str); 9] = [
    (Action::PlayPause, "space"),
    (Action::StepBack, "Left"),
    (Action::StepForward, "Right"),
    (Action::GoStart, "Home"),
    (Action::GoEnd, "End"),
    (Action::Split, "s"),
    (Action::RippleDelete, "Delete"),
    (Action::Undo, "ctrl+z"),
    (Action::Redo, "ctrl+shift+z"),
];

/// One preset per major NLE, covering only the actions dualcut has an
/// equivalent for (verified against each app's own docs, not guessed):
/// Premiere/Resolve's plain Delete does something dualcut has no
/// equivalent for (leave a gap), so their Shift+Delete -- the actual
/// ripple delete -- maps to dualcut's only delete action. Final Cut
/// Pro and iMovie's plain Delete already IS a ripple delete, matching
/// dualcut's default exactly. Split key is each app's real "blade"/
/// "add edit" shortcut, with Cmd read as Ctrl (no Cmd key on Linux).
/// iMovie has no dedicated go-to-start/end shortcut, so those two
/// fall back to dualcut's own Home/End rather than leaving them unset.
pub struct Preset {
    pub name: &'static str,
    pub bindings: &'static [(Action, &'static str)],
}

pub const PRESETS: [Preset; 5] = [
    Preset { name: "Dualcut (default)", bindings: &DEFAULT },
    Preset {
        name: "Adobe Premiere Pro",
        bindings: &[
            (Action::PlayPause, "space"),
            (Action::StepBack, "Left"),
            (Action::StepForward, "Right"),
            (Action::GoStart, "Home"),
            (Action::GoEnd, "End"),
            (Action::Split, "ctrl+k"),
            (Action::RippleDelete, "shift+Delete"),
            (Action::Undo, "ctrl+z"),
            (Action::Redo, "ctrl+shift+z"),
        ],
    },
    Preset {
        name: "DaVinci Resolve",
        bindings: &[
            (Action::PlayPause, "space"),
            (Action::StepBack, "Left"),
            (Action::StepForward, "Right"),
            (Action::GoStart, "Home"),
            (Action::GoEnd, "End"),
            (Action::Split, "ctrl+b"),
            (Action::RippleDelete, "shift+Delete"),
            (Action::Undo, "ctrl+z"),
            (Action::Redo, "ctrl+shift+z"),
        ],
    },
    Preset {
        name: "Final Cut Pro",
        bindings: &[
            (Action::PlayPause, "space"),
            (Action::StepBack, "Left"),
            (Action::StepForward, "Right"),
            (Action::GoStart, "Home"),
            (Action::GoEnd, "End"),
            (Action::Split, "ctrl+b"),
            (Action::RippleDelete, "Delete"),
            (Action::Undo, "ctrl+z"),
            (Action::Redo, "ctrl+shift+z"),
        ],
    },
    Preset {
        name: "iMovie",
        bindings: &[
            (Action::PlayPause, "space"),
            (Action::StepBack, "Left"),
            (Action::StepForward, "Right"),
            (Action::GoStart, "Home"),
            (Action::GoEnd, "End"),
            (Action::Split, "ctrl+b"),
            (Action::RippleDelete, "Delete"),
            (Action::Undo, "ctrl+z"),
            (Action::Redo, "ctrl+shift+z"),
        ],
    },
];

/// The active binding for one action: a custom override from prefs if
/// set, else dualcut's own default.
pub fn binding(action: Action) -> Combo {
    let prefs_key = format!("key.{}", action.key());
    std::fs::read_to_string(prefs_file())
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                let (k, v) = l.split_once('=')?;
                (k.trim() == prefs_key).then(|| Combo::parse(v.trim())).flatten()
            })
        })
        .or_else(|| DEFAULT.iter().find(|(a, _)| *a == action).and_then(|(_, s)| Combo::parse(s)))
        .unwrap_or(Combo::new(gtk::gdk::Key::VoidSymbol, false, false))
}

pub fn set_binding(action: Action, combo: Combo) {
    prefs_set(&format!("key.{}", action.key()), &combo.format());
}

/// Apply every binding in a preset as an override, including actions
/// the preset doesn't mention (reset to dualcut's own default) -- so
/// switching presets never leaves a stale custom binding behind.
pub fn apply_preset(preset: &Preset) {
    for action in Action::ALL {
        let combo = preset
            .bindings
            .iter()
            .find(|(a, _)| *a == action)
            .or_else(|| DEFAULT.iter().find(|(a, _)| *a == action))
            .and_then(|(_, s)| Combo::parse(s));
        if let Some(combo) = combo {
            set_binding(action, combo);
        }
    }
}

/// Look up which action (if any) a keypress triggers, given the
/// current bindings.
pub fn action_for(key: gtk::gdk::Key, modifier: gtk::gdk::ModifierType) -> Option<Action> {
    Action::ALL.into_iter().find(|&a| binding(a).matches(key, modifier))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_format_roundtrip() {
        for s in ["space", "Left", "ctrl+z", "ctrl+shift+z", "shift+Delete", "ctrl+k"] {
            let combo = Combo::parse(s).unwrap_or_else(|| panic!("failed to parse {s:?}"));
            let reparsed = Combo::parse(&combo.format())
                .unwrap_or_else(|| panic!("failed to reparse {:?}", combo.format()));
            assert_eq!(combo, reparsed, "roundtrip mismatch for {s:?}");
        }
    }

    #[test]
    fn parse_is_case_insensitive_for_modifiers() {
        let a = Combo::parse("ctrl+z").unwrap();
        let b = Combo::parse("Ctrl+Z").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn matches_requires_exact_modifier_state() {
        let combo = Combo::parse("ctrl+z").unwrap();
        assert!(combo.matches(gtk::gdk::Key::z, gtk::gdk::ModifierType::CONTROL_MASK));
        assert!(combo.matches(gtk::gdk::Key::Z, gtk::gdk::ModifierType::CONTROL_MASK));
        assert!(!combo.matches(gtk::gdk::Key::z, gtk::gdk::ModifierType::empty()));
        assert!(!combo.matches(
            gtk::gdk::Key::z,
            gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK
        ));
    }

    #[test]
    fn bare_key_does_not_match_with_modifiers_held() {
        let combo = Combo::parse("space").unwrap();
        assert!(combo.matches(gtk::gdk::Key::space, gtk::gdk::ModifierType::empty()));
        assert!(!combo.matches(gtk::gdk::Key::space, gtk::gdk::ModifierType::CONTROL_MASK));
    }

    #[test]
    fn every_default_and_preset_binding_parses() {
        for (action, s) in DEFAULT {
            assert!(Combo::parse(s).is_some(), "DEFAULT[{action:?}] = {s:?} failed to parse");
        }
        for preset in PRESETS {
            for (action, s) in preset.bindings {
                assert!(
                    Combo::parse(s).is_some(),
                    "preset {:?}[{action:?}] = {s:?} failed to parse",
                    preset.name
                );
            }
        }
    }

    #[test]
    fn every_preset_covers_every_action() {
        // Not a hard requirement (apply_preset falls back to DEFAULT
        // for anything missing), but every preset here should be
        // fully specified -- a silent fallback would mean a typo'd
        // Action variant went unnoticed.
        for preset in PRESETS {
            for action in Action::ALL {
                assert!(
                    preset.bindings.iter().any(|(a, _)| *a == action),
                    "preset {:?} has no binding for {action:?}",
                    preset.name
                );
            }
        }
    }

    #[test]
    fn display_and_gtk_accel_are_nonempty_for_every_default() {
        for (action, s) in DEFAULT {
            let combo = Combo::parse(s).unwrap();
            assert!(!combo.display().is_empty(), "{action:?} has empty display()");
            assert!(!combo.gtk_accel().is_empty(), "{action:?} has empty gtk_accel()");
        }
    }
}
