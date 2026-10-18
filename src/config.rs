use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::model::Direction;

/// Every command the app exposes. Order matters: it is the order the settings
/// screen and the footer hint bar render in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    Invoke,
    ToggleOverlay,
    NewNote,
    OpenLibrary,
    /// Editor to library, without losing the buffer.
    ShowLibrary,
    FocusSearch,
    Save,
    SaveClose,
    Back,
    Settings,
    DeleteNote,
    MoveUp,
    MoveDown,
    ToggleDirection,
    TogglePreviewLines,
}

pub const ALL_COMMANDS: &[Command] = &[
    Command::Invoke,
    Command::ToggleOverlay,
    Command::NewNote,
    Command::OpenLibrary,
    Command::FocusSearch,
    Command::Save,
    Command::SaveClose,
    Command::Back,
    Command::Settings,
    Command::DeleteNote,
    Command::MoveUp,
    Command::MoveDown,
    Command::ToggleDirection,
    Command::TogglePreviewLines,
    Command::ShowLibrary,
];

impl Command {
    pub fn label(self) -> &'static str {
        match self {
            Command::Invoke => "Invoke / dismiss overlay",
            Command::ToggleOverlay => "Show overlay",
            Command::NewNote => "Chooser → New note",
            Command::OpenLibrary => "Chooser → My notes",
            Command::ShowLibrary => "Editor → My notes",
            Command::FocusSearch => "Focus library search",
            Command::Save => "Save note (stay open)",
            Command::SaveClose => "Save note and close",
            Command::Back => "Back / close",
            Command::Settings => "Open settings",
            Command::DeleteNote => "Delete selected note",
            Command::MoveUp => "Move note up",
            Command::MoveDown => "Move note down",
            Command::ToggleDirection => "Toggle RTL / LTR",
            Command::TogglePreviewLines => "More / fewer preview lines",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Command::Invoke => "Global. Works from any application.",
            Command::ToggleOverlay => "Global. Toggles the overlay.",
            Command::NewNote => "From the chooser.",
            Command::OpenLibrary => "From the chooser.",
            Command::ShowLibrary => "From the editor, without discarding the draft.",
            Command::FocusSearch => "In the library.",
            Command::Save => "Keeps you in the editor.",
            Command::SaveClose => "Returns you to the chooser.",
            Command::Back => "Respects the unsaved-changes guard.",
            Command::Settings => "From any view.",
            Command::DeleteNote => "In the library. Always confirmed.",
            Command::MoveUp => "In the library.",
            Command::MoveDown => "In the library.",
            Command::ToggleDirection => "In the editor.",
            Command::TogglePreviewLines => "In the library.",
        }
    }

    /// Only these can ever be global. The rest are meaningless without a
    /// focused window, and advertising them as global would be a lie.
    pub fn can_be_global(self) -> bool {
        matches!(self, Command::Invoke | Command::ToggleOverlay)
    }
}

pub type Bindings = BTreeMap<Command, String>;

fn defaults() -> Vec<(Command, &'static str)> {
    vec![
        (Command::Invoke, "Meta+Shift+Space"),
        (Command::ToggleOverlay, "Meta+Shift+Space"),
        (Command::NewNote, "A"),
        (Command::OpenLibrary, "B"),
        (Command::ShowLibrary, "Alt+B"),
        (Command::FocusSearch, "Slash"),
        (Command::Save, "Ctrl+S"),
        (Command::SaveClose, "Ctrl+Enter"),
        (Command::Back, "Escape"),
        (Command::Settings, "Ctrl+Comma"),
        (Command::DeleteNote, "Delete"),
        (Command::MoveUp, "Alt+Up"),
        (Command::MoveDown, "Alt+Down"),
        (Command::ToggleDirection, "Ctrl+Shift+D"),
        (Command::TogglePreviewLines, "Alt+BracketRight"),
    ]
}

pub fn default_bindings() -> Bindings {
    defaults().into_iter().map(|(k, v)| (k, v.to_string())).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FocusLoss {
    /// Close immediately when the editor is clean.
    AutoHide,
    /// Keep the overlay open and offer save / discard / stay.
    Prompt,
    /// Never close on focus loss; only the hotkey or Escape closes it.
    Never,
}

impl FocusLoss {
    pub fn label(self) -> &'static str {
        match self {
            FocusLoss::AutoHide => "Hide when clean",
            FocusLoss::Prompt => "Keep open and ask",
            FocusLoss::Never => "Never close",
        }
    }
}

impl Default for FocusLoss {
    fn default() -> Self {
        FocusLoss::Prompt
    }
}

/// Light or dark. Not the desktop's setting: batnotes picks its own so the
/// window is the same on a dark KDE session as on a light one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    pub const ALL: [Theme; 2] = [Theme::Light, Theme::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Theme::Light => "Light",
            Theme::Dark => "Dark",
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::Light
    }
}

/// A named colour, so the palette is data rather than code and the user can
/// retint the whole app without a rebuild.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Palette {
    pub paper: String,
    pub paper_raised: String,
    pub ink: String,
    pub ink_soft: String,
    pub ink_faint: String,
    pub accent: String,
    /// Hover shade for the accent, kept configurable so the pair stays in step.
    pub accent_deep: String,
    pub clay: String,
    pub rule: String,
    pub wash: String,
    pub danger: String,
    pub danger_deep: String,
}

impl Default for Palette {
    fn default() -> Self {
        Palette {
            paper: "#fdfdf9".into(),
            paper_raised: "#ffffff".into(),
            ink: "#2b3b30".into(),
            // These two carry most of the small text in the app. The previous
            // values sat near 2:1 on the paper background and were unreadable.
            ink_soft: "#5c6759".into(),
            ink_faint: "#6d7768".into(),
            accent: "#31573e".into(),
            accent_deep: "#244731".into(),
            clay: "#9a6a3c".into(),
            rule: "#e4e9e0".into(),
            wash: "#f4f7f1".into(),
            danger: "#aa5e52".into(),
            danger_deep: "#914d44".into(),
        }
    }
}

impl Palette {
    /// The same palette read on a dark surface. Green ink on near-black keeps
    /// the accent that carries the app; the greys are lifted so secondary text
    /// still clears 4.5:1.
    pub fn dark() -> Palette {
        Palette {
            paper: "#1c201d".into(),
            paper_raised: "#252a26".into(),
            ink: "#e6ece4".into(),
            ink_soft: "#a3ada2".into(),
            ink_faint: "#8b958a".into(),
            accent: "#7fc79b".into(),
            accent_deep: "#9ad9b1".into(),
            clay: "#d9a469".into(),
            rule: "#333a34".into(),
            wash: "#2b312c".into(),
            danger: "#e08878".into(),
            danger_deep: "#f0a396".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub notes_dir: PathBuf,
    pub preview_lines: usize,
    pub default_direction: Direction,
    pub focus_loss: FocusLoss,
    pub delete_empty_draft: bool,
    pub fsync: bool,
    /// Opt-in only. Calling CreateSession on the GlobalShortcuts portal crashes
    /// xdg-desktop-portal on some desktops (KDE Plasma 6 segfaults in
    /// handle_create_session), so it must never run unasked.
    pub use_portal: bool,
    /// Keep the overlay above other windows while it is open, so typing in
    /// the window underneath does not bury the thing you are writing.
    pub keep_above: bool,
    /// Second global key: opens the note list directly. The invoke key is
    /// capture-only, so review needs its own entry point.
    pub notes_hotkey: String,
    pub ui_font: Option<PathBuf>,
    pub heading_font: Option<PathBuf>,
    pub theme: Theme,
    pub palette: Palette,
    pub dark_palette: Palette,
    #[serde(default)]
    pub bindings: Bindings,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            notes_dir: default_notes_dir(),
            preview_lines: 3,
            default_direction: Direction::Rtl,
            focus_loss: FocusLoss::Prompt,
            delete_empty_draft: true,
            fsync: true,
            use_portal: false,
            keep_above: true,
            notes_hotkey: "Meta+Shift+L".to_string(),
            ui_font: None,
            heading_font: None,
            theme: Theme::Light,
            palette: Palette::default(),
            dark_palette: Palette::dark(),
            bindings: default_bindings(),
        }
    }
}

/// XDG data dir rather than a dotfolder in $HOME.
pub fn default_notes_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("batnotes/notes")
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("batnotes/config.toml")
}

pub struct Loaded {
    pub config: Config,
    /// Set when an existing config was unreadable and defaults were substituted.
    pub warning: Option<String>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Loaded> {
        if !path.exists() {
            let cfg = Config::default();
            let _ = cfg.save(path);
            return Ok(Loaded { config: cfg, warning: None });
        }
        let text = std::fs::read_to_string(path)?;
        match toml::from_str::<Config>(&text) {
            Ok(mut cfg) => {
                // Merge in any binding added since the file was written, so a
                // config from an older version keeps working.
                let d = default_bindings();
                for (k, v) in d {
                    cfg.bindings.entry(k).or_insert(v);
                }
                let warnings = cfg.validate();
                Ok(Loaded { config: cfg, warning: (!warnings.is_empty()).then(|| warnings.join("; ")) })
            }
            Err(e) => {
                // DESIGN §10: keep the corrupt file, load defaults, say so.
                let bak = path.with_extension("toml.bak");
                let _ = std::fs::copy(path, &bak);
                let cfg = Config::default();
                let _ = cfg.save(path);
                Ok(Loaded {
                    config: cfg,
                    warning: Some(format!(
                        "Settings could not be read ({}). Defaults were restored; your old file is at {}",
                        first_line(&e.to_string()),
                        bak.display()
                    )),
                })
            }
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        crate::storage::atomic_write(path, text.as_bytes(), self.fsync)
    }

    /// Returns human-readable problems. Empty means valid.
    pub fn validate(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.preview_lines == 0 || self.preview_lines > 8 {
            out.push(format!("preview_lines must be between 1 and 8 (is {})", self.preview_lines));
        }
        for c in ALL_COMMANDS.iter().copied() {
            let Some(b) = self.bindings.get(&c) else {
                out.push(format!("{} has no binding", c.label()));
                continue;
            };
            if crate::hotkey::HotkeySpec::parse(b).is_err() {
                out.push(format!("{} has an invalid binding: {b:?}", c.label()));
            }
        }
        for c in self.binding_conflicts() {
            out.push(format!("{} conflicts with another command", c.label()));
        }
        out
    }

    /// Commands whose binding collides with another command's. Both sides are
    /// reported so the settings screen can flag the whole family.
    ///
    /// `Invoke` and `ToggleOverlay` share a key by design — they are two names
    /// for the same global action — so they are exempt.
    pub fn binding_conflicts(&self) -> Vec<Command> {
        let exempt = |c: &Command| matches!(c, Command::Invoke | Command::ToggleOverlay);
        let mut counts: BTreeMap<String, Vec<Command>> = BTreeMap::new();
        for (cmd, b) in &self.bindings {
            if exempt(cmd) {
                continue;
            }
            if let Ok(spec) = crate::hotkey::HotkeySpec::parse(b) {
                counts.entry(spec.normalised()).or_default().push(*cmd);
            }
        }
        counts
            .into_values()
            .filter(|v| v.len() > 1)
            .flatten()
            .collect()
    }

    pub fn binding(&self, c: Command) -> String {
        self.bindings
            .get(&c)
            .cloned()
            .unwrap_or_else(|| "—".to_string())
    }

    /// The palette the UI should draw with. Every colour the app renders comes
    /// from here — nothing is inherited from the desktop theme.
    pub fn palette(&self) -> &Palette {
        match self.theme {
            Theme::Dark => &self.dark_palette,
            Theme::Light => &self.palette,
        }
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("parse error").to_string()
}

/// `#rrggbb` or `#rrggbbaa` into components in 0.0..=1.0, which is what both
/// GTK and CSS want.
pub fn parse_colour(hex: &str) -> Option<Colour> {
    let h = hex.trim().trim_start_matches('#');
    let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    let (r, g, b) = (byte(0)?, byte(2)?, byte(4)?);
    let a = if h.len() >= 8 { byte(6)? } else { 255 };
    Some(Colour {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: a as f32 / 255.0,
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colour {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Colour {
    /// CSS `rgba(...)`, which is what GTK's parser accepts.
    pub fn css(&self) -> String {
        format!(
            "rgba({},{},{},{})",
            (self.r * 255.0).round() as u8,
            (self.g * 255.0).round() as u8,
            (self.b * 255.0).round() as u8,
            (self.a * 255.0).round() as u8
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.toml");
        let cfg = Config::default();
        cfg.save(&p).unwrap();
        let loaded = Config::load(&p).unwrap();
        assert_eq!(loaded.config.preview_lines, 3);
        assert_eq!(loaded.config.bindings.len(), ALL_COMMANDS.len());
        assert!(loaded.warning.is_none());
    }

    #[test]
    fn new_binding_appears_for_old_config() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.toml");
        std::fs::write(&p, "preview_lines = 4\n").unwrap();
        let loaded = Config::load(&p).unwrap();
        // serde requires the non-default fields, so this asserts we tolerate a
        // partial file rather than silently dropping user settings.
        assert_eq!(loaded.config.preview_lines, 4);
        assert_eq!(loaded.config.bindings.len(), ALL_COMMANDS.len());
    }

    #[test]
    fn conflicts_are_detected_on_both_sides() {
        let mut cfg = Config::default();
        cfg.bindings.insert(Command::NewNote, "Ctrl+S".into());
        let c = cfg.binding_conflicts();
        assert!(c.contains(&Command::NewNote));
        assert!(c.contains(&Command::Save));
    }

    #[test]
    fn no_conflicts_by_default() {
        // Invoke and ToggleOverlay intentionally share a key.
        let cfg = Config::default();
        let only_real = cfg
            .binding_conflicts()
            .into_iter()
            .filter(|c| !matches!(c, Command::Invoke | Command::ToggleOverlay))
            .collect::<Vec<_>>();
        assert!(only_real.is_empty(), "unexpected conflicts: {only_real:?}");
    }

    #[test]
    fn corrupt_config_is_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.toml");
        std::fs::write(&p, "this is not = = toml").unwrap();
        let loaded = Config::load(&p).unwrap();
        assert!(loaded.warning.is_some());
        assert!(dir.path().join("config.toml.bak").exists());
    }

    #[test]
    fn validate_flags_zero_preview_lines() {
        let mut cfg = Config::default();
        cfg.preview_lines = 0;
        assert!(!cfg.validate().is_empty());
    }

    #[test]
    fn colour_parsing() {
        let white = parse_colour("#ffffff").expect("valid hex");
        assert_eq!((white.r, white.g, white.b, white.a), (1.0, 1.0, 1.0, 1.0));
        assert_eq!(parse_colour("nope"), None);
        assert_eq!(parse_colour("#ff0000").unwrap().css(), "rgba(255,0,0,255)");
    }
}