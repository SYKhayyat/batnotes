//! Types shared between the application state machine and whichever GUI layer
//! is in front of it. Nothing here knows about GTK.

use crate::config::Command;
use gtk4::gdk;
use gtk4::gdk::ModifierType;

pub mod gtk;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Chooser,
    Editor,
    Library,
    Settings,
}

/// A user intent. The GUI layer raises these; [`crate::app::App`] applies them.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    NewNote,
    OpenLibrary,
    /// Editor → library, leaving the draft intact.
    ShowLibrary,
    OpenSettings,
    OpenNote(String),
    ResumeNote(String),
    Back,
    Close,
    Save { close: bool },
    Cancel,
    Delete(String),
    ConfirmDelete(String),
    ConfirmDeleteAccepted(String),
    DismissConfirm,
    Move(String, i32),
    ReorderFocus(i32),
    Select(usize),
    SetQuery(String),
    SetDirection(crate::model::Direction),
    SetDefaultDirection(crate::model::Direction),
    SetPreviewLines(usize),
    SetFocusLoss(crate::config::FocusLoss),
    SetFsync(bool),
    SetUsePortal(bool),
    SetBinding(Command, String),
    ResetBindings,
    ImportNotes,
    ExportNote(String),
    RebuildIndex,
    DismissToast,
    DismissWarning,
    DismissPrompt,
    PromptAction(String),
    OpenNotesDir,
    ApplyHotkey,
}

/// One line of the library list.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    /// Byte offset of the match in the body, when the hit was in the body.
    pub at: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    General,
    Keys,
    Storage,
}

impl Tab {
    pub fn label(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Keys => "Keys",
            Tab::Storage => "Storage",
        }
    }
    pub const ALL: [Tab; 3] = [Tab::General, Tab::Keys, Tab::Storage];
}

pub const PROMPT_FOCUS: &str = "focus";
pub const PROMPT_CANCEL: &str = "cancel";
pub const CONFIRM_DELETE: &str = "delete";
pub const PROMPT_STAY: &str = "stay";
pub const PROMPT_DISCARD: &str = "discard";
pub const PROMPT_SAVE_CLOSE: &str = "save-close";

/// Turn a stored binding into something displayable: `Ctrl+S` -> `Ctrl S`.
pub fn pretty_key(k: &str) -> String {
    k.split('+')
        .map(|part| match part {
            "Escape" => "Esc".into(),
            "Enter" => "\u{21b5}".into(),
            "Delete" => "Del".into(),
            "Comma" => ",".into(),
            "Slash" => "/".into(),
            "BracketLeft" => "[".into(),
            "BracketRight" => "]".into(),
            "Up" => "\u{2191}".into(),
            "Down" => "\u{2193}".into(),
            "Left" => "\u{2190}".into(),
            "Right" => "\u{2192}".into(),
            other => other.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Canonical key name for a keyval, in our binding syntax.
pub fn key_name(key: &gtk4::gdk::Key) -> Option<String> {
    // Named keys first. Several of these (space, Escape, Tab, Return) also have
    // Unicode forms — "\u{1b}" and friends — so matching on `to_unicode` first
    // would store control characters in the binding.
    let named = match *key {
        gdk::Key::space => Some("Space"),
        gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter => Some("Enter"),
        gdk::Key::Escape => Some("Escape"),
        gdk::Key::Tab | gdk::Key::ISO_Left_Tab => Some("Tab"),
        gdk::Key::BackSpace => Some("Backspace"),
        gdk::Key::Delete | gdk::Key::KP_Delete => Some("Delete"),
        gdk::Key::Up => Some("Up"),
        gdk::Key::Down => Some("Down"),
        gdk::Key::Left => Some("Left"),
        gdk::Key::Right => Some("Right"),
        gdk::Key::Home => Some("Home"),
        gdk::Key::End => Some("End"),
        gdk::Key::Page_Up => Some("PageUp"),
        gdk::Key::Page_Down => Some("PageDown"),
        gdk::Key::comma => Some("Comma"),
        gdk::Key::period => Some("Period"),
        gdk::Key::slash => Some("Slash"),
        gdk::Key::backslash => Some("Backslash"),
        gdk::Key::bracketleft => Some("BracketLeft"),
        gdk::Key::bracketright => Some("BracketRight"),
        gdk::Key::minus => Some("Minus"),
        gdk::Key::equal => Some("Equal"),
        gdk::Key::semicolon => Some("Semicolon"),
        gdk::Key::apostrophe => Some("Apostrophe"),
        gdk::Key::grave => Some("Grave"),
        _ => None,
    };
    if let Some(n) = named {
        return Some(n.to_string());
    }
    let c = key.to_unicode()?;
    if c.is_control() {
        return None;
    }
    let mut s = c.to_uppercase().to_string();
    if let Some(first) = s.chars().next() {
        s = first.to_string();
    }
    Some(s)
}

/// Build a binding from a key plus modifier state.
pub fn combo_from(key: &gtk4::gdk::Key, state: &gtk4::gdk::ModifierType) -> Option<String> {
    let name = key_name(key)?;
    let mut mods: Vec<&str> = Vec::new();
    if state.contains(ModifierType::CONTROL_MASK) {
        mods.push("Ctrl");
    }
    if state.contains(ModifierType::ALT_MASK) {
        mods.push("Alt");
    }
    if state.contains(ModifierType::SHIFT_MASK) {
        mods.push("Shift");
    }
    if state.contains(ModifierType::SUPER_MASK) {
        mods.push("Super");
    }
    // A bare modifier press is not a binding. Escape and friends are, so this
    // must test for the modifier keyvals themselves rather than the name's
    // length.
    if matches!(
        *key,
        gdk::Key::Shift_L
            | gdk::Key::Shift_R
            | gdk::Key::Control_L
            | gdk::Key::Control_R
            | gdk::Key::Alt_L
            | gdk::Key::Alt_R
            | gdk::Key::Super_L
            | gdk::Key::Super_R
            | gdk::Key::Meta_L
            | gdk::Key::Meta_R
            | gdk::Key::Caps_Lock
            | gdk::Key::Num_Lock
    ) {
        return None;
    }
    Some(format!("{}{name}", mods.iter().map(|m| format!("{m}+")).collect::<String>()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk4::gdk::{Key, ModifierType};

    #[test]
    fn key_display_is_readable() {
        assert_eq!(pretty_key("Ctrl+S"), "Ctrl S");
        assert_eq!(pretty_key("Escape"), "Esc");
        assert_eq!(pretty_key("Alt+Up"), "Alt \u{2191}");
    }

    #[test]
    fn letter_keys_become_bindings() {
        let m = ModifierType::empty();
        assert_eq!(combo_from(&Key::s, &m).as_deref(), Some("S"));
        let ctrl = ModifierType::CONTROL_MASK;
        assert_eq!(combo_from(&Key::s, &ctrl).as_deref(), Some("Ctrl+S"));
    }

    #[test]
    fn named_keys_map_to_our_syntax() {
        let m = ModifierType::empty();
        assert_eq!(combo_from(&Key::space, &m).as_deref(), Some("Space"));
        assert_eq!(combo_from(&Key::Escape, &m).as_deref(), Some("Escape"));
        assert_eq!(
            combo_from(&Key::comma, &ModifierType::ALT_MASK).as_deref(),
            Some("Alt+Comma")
        );
        assert_eq!(
            combo_from(&Key::Up, &ModifierType::ALT_MASK).as_deref(),
            Some("Alt+Up")
        );
    }

    #[test]
    fn every_default_binding_parses_back() {
        // Whatever we show must be something the recorder could produce.
        for (cmd, b) in crate::config::default_bindings() {
            assert!(
                crate::hotkey::HotkeySpec::parse(&b).is_ok(),
                "{cmd:?} = {b:?} is not a valid binding"
            );
        }
    }
}
