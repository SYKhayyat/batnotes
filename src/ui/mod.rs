pub mod chooser;
pub mod editor;
pub mod kit;
pub mod library;
pub mod settings;
pub mod theme;

use crate::config::Command;
use crate::model::Direction;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Chooser,
    Editor,
    Library,
    Settings,
}

/// A user intent, raised by a view and applied by the app. Views never mutate
/// storage directly, which keeps the state machine in one place.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    NewNote,
    OpenLibrary,
    OpenSettings,
    OpenNote(String),
    ResumeNote(String),
    Back,
    Save { close: bool },
    Cancel,
    Delete(String),
    ConfirmDelete(String),
    Move(String, i32),
    SetQuery(String),
    Select(usize),
    ReorderFocus(i32),
    SetDirection(Direction),
    SetPreviewLines(usize),
    SetFocusLoss(crate::config::FocusLoss),
    SetFsync(bool),
    SetBinding(Command, String),
    ResetBindings,
    ImportNotes,
    ExportNote(String),
    RebuildIndex,
    DismissToast,
    DismissWarning,
    DismissPrompt,
    PromptAction(String),
    ConfirmDeleteAccepted(String),
    DismissConfirm,
    OpenNotesDir,
    ApplyHotkey,
}

pub const PROMPT_FOCUS: &str = "focus";
pub const PROMPT_CANCEL: &str = "cancel";
pub const CONFIRM_DELETE: &str = "delete";
pub const PROMPT_STAY: &str = "stay";
pub const PROMPT_DISCARD: &str = "discard";
pub const PROMPT_SAVE_CLOSE: &str = "save-close";
pub const PROMPT_DELETE_ACCEPT: &str = "delete-accept";
pub const PROMPT_DELETE_KEEP: &str = "delete-keep";
pub const PROMPT_REBUILD: &str = "rebuild";