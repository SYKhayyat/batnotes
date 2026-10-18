//! Types shared between the application state machine and whichever GUI layer
//! is in front of it. Nothing here knows about GTK.

use crate::config::Command;
use gtk4::gdk;
use gtk4::gdk::ModifierType;

pub mod gtk;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The overlay is always either *capturing* or *reviewing*.
///
/// There is no "chooser" view on purpose: pressing the hotkey must land the
/// caret in an empty note, not in a menu asking what to do next.
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
    Back,
    Close,
    Save { close: bool },
    Cancel,
    Delete(String),
    ConfirmDelete(String),
    ConfirmDeleteAccepted(String),
    DismissConfirm,
    Move(String, i32),
    /// Move the note under the selection up or down and keep it selected.
    MoveSelected(i32),
    Select(usize),
    SetQuery(String),
    /// Put the caret in the library's search box.
    FocusSearch,
    SetDirection(crate::model::Direction),
    SetDefaultDirection(crate::model::Direction),
    SetTheme(crate::config::Theme),
    SetKeepAbove(bool),
    DeleteFocused,
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
    DismissError,
    DismissPrompt,
    PromptAction(String),
    OpenNotesDir,
    ApplyHotkey,
    SelectTab(Tab),
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

/// Canonical key name from a **hardware keycode**.
///
/// This is the only layout-independent way to identify a key. GTK reports the
/// keysym for the active layout, so Ctrl+S arrives as `hebrew_dalet` when the
/// Hebrew layout is selected and `Ctrl+Shift+S` would never match. Users
/// expect a shortcut to mean the physical key they pressed.
///
/// Codes are the standard Linux evdev values.
/// GTK hands us the X11 keycode; Linux tables are written in evdev terms.
pub fn x11_to_evdev(code: u32) -> u32 {
    code.saturating_sub(8)
}

/// Modifier keys, by evdev code. Catches AltGr and the ISO_* spellings that a
/// keysym check misses.
fn is_modifier_keycode(code: u32) -> bool {
    matches!(
        code,
        29 | 42 // Control_L, Shift_L
            | 54 | 56 // Shift_R, Alt_L
            | 97 | 100 // Ctrl_R, Alt_R
            | 125 | 126 | 130 // Super_L, Super_R, Menu
    )
}

pub fn key_name_from_keycode(code: u32) -> Option<&'static str> {
    Some(match code {
        1 => "Escape",
        2..=11 => return Some(LETTERS_DIGITS[(code - 2) as usize]), // 1..0
        12 => "Minus",
        13 => "Equal",
        14 => "Backspace",
        15 => "Tab",
        16..=25 => return Some(LETTERS[(code - 16) as usize]), // q..p
        26 => "BracketLeft",
        27 => "BracketRight",
        28 => "Enter",
        30..=38 => return Some(LETTERS[(code - 30 + 10) as usize]), // a..l
        39 => "Semicolon",
        40 => "Apostrophe",
        41 => "Grave",
        42 => "Backslash",
        44..=50 => return Some(LETTERS[(code - 44 + 19) as usize]), // z..m
        51 => "Comma",
        52 => "Period",
        53 => "Slash",
        57 => "Space",
        59..=68 => return Some(FUNCTION[(code - 59) as usize]),
        71..=74 => return Some(NUMPAD[(code - 71) as usize]),
        75 => "Insert",
        76 => "End",
        77 => "Down",
        78 => "PageDown",
        79 => "Left",
        82 => "Home",
        83 => "Up",
        84 => "PageUp",
        85 => "Right",
        87 => "Insert",
        88 => "End",
        89 => "Down",
        90 => "PageDown",
        91 => "Left",
        92 => "Up",
        93 => "PageUp",
        94 => "Right",
        102 => "Home",
        103 => "Up",
        104 => "PageUp",
        105 => "Left",
        106 => "Right",
        107 => "End",
        108 => "Down",
        109 => "PageDown",
        110 => "Insert",
        111 => "Delete",
        _ => return None,
    })
}

/// q w e r t y u i o p a s d f g h j k l z x c v b n m
const LETTERS: [&str; 26] = [
    "Q", "W", "E", "R", "T", "Y", "U", "I", "O", "P", "A", "S", "D", "F", "G", "H", "J", "K", "L", "Z",
    "X", "C", "V", "B", "N", "M",
];
/// 1 2 3 4 5 6 7 8 9 0
const LETTERS_DIGITS: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
const FUNCTION: [&str; 10] = [
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10",
];
const NUMPAD: [&str; 4] = ["Numpad7", "Numpad8", "Numpad9", "NumpadAdd"];

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
pub fn combo_from(
    key: &gtk4::gdk::Key,
    keycode: u32,
    state: &gtk4::gdk::ModifierType,
) -> Option<String> {
    // Prefer the hardware keycode; fall back to the keysym only when the
    // keycode is not one we know.
    //
    // GTK reports the X11 keycode. On Linux that is the evdev code plus
    // eight, and reading it as raw evdev shifted every key by eight: `a`
    // resolved to `L` and Escape matched nothing at all.
    let name = match key_name_from_keycode(x11_to_evdev(keycode)) {
        Some(n) => n.to_string(),
        None => key_name(key)?,
    };
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
    // Modifier keys arrive under several spellings depending on layout —
    // Shift_L, but also ISO_Level3_Shift for AltGr. Identifying them by
    // hardware code covers every spelling at once.
    if is_modifier_keycode(x11_to_evdev(keycode)) {
        return None;
    }
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
        // X11 keycode for S is 39 (evdev 31 + 8). Passing the raw evdev code
        // used to resolve to a different letter entirely, which is how `A`
        // became `L` and Escape matched nothing.
        assert_eq!(combo_from(&Key::s, 39, &m).as_deref(), Some("S"));
        let ctrl = ModifierType::CONTROL_MASK;
        assert_eq!(combo_from(&Key::s, 39, &ctrl).as_deref(), Some("Ctrl+S"));
    }

    #[test]
    fn bindings_follow_the_hardware_key_not_the_layout() {
        // With the Hebrew layout, GTK reports the S key as hebrew_dalet. A
        // shortcut must still be Ctrl+S, because that is the physical key.
        let ctrl = ModifierType::CONTROL_MASK;
        let combo = combo_from(&Key::hebrew_dalet, 31 + 8, &ctrl);
        assert_eq!(combo.as_deref(), Some("Ctrl+S"));
    }

    #[test]
    fn keycode_table_matches_the_evdev_layout() {
        assert_eq!(key_name_from_keycode(30), Some("A"));
        assert_eq!(key_name_from_keycode(31), Some("S"));
        assert_eq!(key_name_from_keycode(44), Some("Z"));
        assert_eq!(key_name_from_keycode(57), Some("Space"));
        assert_eq!(key_name_from_keycode(111), Some("Delete"));
        assert_eq!(key_name_from_keycode(28), Some("Enter"));
        assert_eq!(key_name_from_keycode(103), Some("Up"));
        assert_eq!(key_name_from_keycode(2), Some("1"));
        assert_eq!(key_name_from_keycode(9999), None);
    }

    #[test]
    fn named_keys_map_to_our_syntax() {
        let m = ModifierType::empty();
        // A bare letter must resolve to the binding's spelling, case
        // included, or New note ("A") and Back never fire.
        let a = combo_from(&Key::a, 38, &ModifierType::empty());
        assert_eq!(a.as_deref(), Some("A"));
        assert_eq!(
            crate::hotkey::HotkeySpec::parse(a.as_deref().unwrap())
                .map(|s| s.normalised())
                .unwrap_or_default(),
            crate::hotkey::HotkeySpec::parse("A")
                .map(|s| s.normalised())
                .unwrap_or_default(),
            "pressing A must match the New note binding"
        );

        assert_eq!(combo_from(&Key::space, 65, &m).as_deref(), Some("Space"));
        assert_eq!(combo_from(&Key::Escape, 9, &m).as_deref(), Some("Escape"));
        // evdev 51 is comma, 103 is Up.
        assert_eq!(
            combo_from(&Key::comma, 59, &ModifierType::ALT_MASK).as_deref(),
            Some("Alt+Comma")
        );
        assert_eq!(
            combo_from(&Key::Up, 111, &ModifierType::ALT_MASK).as_deref(),
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
