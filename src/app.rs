use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::{Command, Config, FocusLoss};
use crate::hotkey::{self, HotkeySpec};
use crate::model::{self, Direction, Note};
use crate::search::{Doc, Indexed, Match, Searcher};
use crate::storage::Storage;
use crate::ui::{self, Action, Row, Tab, View};

/// Application state and the state machine.
///
/// Deliberately free of any GUI types: the GTK layer reads this and raises
/// [`Action`]s, which keeps the interesting logic testable without a display.
pub struct App {
    pub config: Config,
    pub config_path: PathBuf,
    pub storage: Storage,
    pub total_bytes: u64,

    pub hotkey: hotkey::Service,
    pub hotkey_spec: HotkeySpec,

    pub index: Arc<Indexed>,
    pub order: Vec<String>,
    searcher: Searcher,

    pub view: View,
    pub prev_view: View,

    pub ed_id: Option<String>,
    pub ed_text: String,
    pub ed_original: String,
    pub ed_dir: Direction,
    pub ed_original_dir: Direction,
    pub ed_is_new: bool,
    /// Where Escape/Cancel returns to.
    pub ed_return: View,

    pub query: String,
    pub rows: Vec<Row>,
    pub selected: usize,
    pub last_query: String,

    pub recording: Option<Command>,
    pub settings_tab: Tab,
    pub conflicts: HashSet<Command>,

    pub prompt: Option<String>,
    pub confirm_delete: Option<String>,
    pub toast: Option<(String, Instant)>,
    pub config_warning: Option<String>,
    pub storage_error: Option<String>,

    pub visible: bool,
    /// Set by the GUI layer when Escape/close was requested.
    pub closing: bool,
}

impl App {
    pub fn new() -> Result<App> {
        let config_path = crate::config::config_path();
        let loaded = Config::load(&config_path)?;
        let config = loaded.config;

        let storage = Storage::new(&config);
        let (index, order, bytes) = Self::load_index(&storage);

        let invoke = HotkeySpec::parse(&config.binding(Command::Invoke))
            .unwrap_or_else(|_| HotkeySpec::parse("Meta+Shift+Space").expect("literal"));
        let hotkey = hotkey::start(invoke.clone(), config.use_portal);

        let searcher = Searcher::new();
        searcher.rebuild(index.clone());

        let conflicts = config.binding_conflicts().into_iter().collect();

        Ok(App {
            config,
            config_path,
            storage,
            total_bytes: bytes,
            hotkey,
            hotkey_spec: invoke,
            index,
            order,
            searcher,
            view: View::Chooser,
            prev_view: View::Chooser,
            ed_id: None,
            ed_text: String::new(),
            ed_original: String::new(),
            ed_dir: Direction::Rtl,
            ed_original_dir: Direction::Rtl,
            ed_is_new: true,
            ed_return: View::Chooser,
            query: String::new(),
            rows: Vec::new(),
            selected: 0,
            last_query: String::new(),
            recording: None,
            settings_tab: Tab::General,
            conflicts,
            prompt: None,
            confirm_delete: None,
            toast: None,
            config_warning: loaded.warning,
            storage_error: None,
            visible: false,
            closing: false,
        })
    }

    fn load_index(storage: &Storage) -> (Arc<Indexed>, Vec<String>, u64) {
        let notes = storage.list_notes().unwrap_or_default();
        let ordered = storage.ordered(notes).unwrap_or_default();
        let bytes = ordered.iter().map(|n| n.size).sum();
        let docs: Vec<Doc> = ordered
            .iter()
            .filter_map(|n| {
                n.read()
                    .ok()
                    .map(|c| Doc::new(n.id.clone(), n.path.clone(), c, n.direction))
            })
            .collect();
        (
            Arc::new(Indexed::build(docs)),
            ordered.iter().map(|n| n.id.clone()).collect(),
            bytes,
        )
    }

    pub fn refresh(&mut self) {
        let (index, order, bytes) = Self::load_index(&self.storage);
        self.index = index;
        self.order = order;
        self.total_bytes = bytes;
        self.searcher.rebuild(self.index.clone());
        self.recompute_rows();
    }

    pub fn dirty(&self) -> bool {
        self.ed_text != self.ed_original || self.ed_dir != self.ed_original_dir
    }

    pub fn hebrew_count(&self) -> usize {
        self.index
            .ids()
            .iter()
            .filter(|id| {
                self.index
                    .get(id)
                    .map(|d| d.direction == Direction::Rtl)
                    .unwrap_or(false)
            })
            .count()
    }

    fn recompute_rows(&mut self) {
        if self.query.trim().is_empty() {
            self.rows = self
                .order
                .iter()
                .map(|id| Row { id: id.clone(), at: None })
                .collect();
        } else {
            self.rows = self
                .index
                .ids()
                .into_iter()
                .map(|id| Row { id, at: None })
                .collect();
        }
        if self.selected >= self.rows.len() {
            self.selected = self.rows.len().saturating_sub(1);
        }
    }

    fn apply_hits(&mut self, hits: Vec<Match>) {
        if self.query.trim().is_empty() {
            self.recompute_rows();
            return;
        }
        let by_id: HashMap<&str, &Match> = hits.iter().map(|h| (h.id.as_str(), h)).collect();
        self.rows = self
            .order
            .iter()
            .filter_map(|id| by_id.get(id.as_str()).map(|h| Row { id: id.clone(), at: h.at }))
            .collect();
        if self.selected >= self.rows.len() {
            self.selected = 0;
        }
    }

    // ---- visibility --------------------------------------------------

    pub fn show(&mut self) {
        self.visible = true;
        self.closing = false;
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.view = View::Chooser;
        self.ed_id = None;
        self.ed_text.clear();
        self.ed_original.clear();
        self.prompt = None;
        self.confirm_delete = None;
        self.recording = None;
        self.query.clear();
        self.recompute_rows();
    }

    pub fn toggle(&mut self) {
        if self.visible {
            self.hide();
        } else {
            self.show();
        }
    }

    /// Called every frame by the GUI layer.
    pub fn pump(&mut self) {
        while self.hotkey.rx.try_recv().is_ok() || hotkey::take_show_request() {
            self.toggle();
        }
        if let Some(hits) = self.searcher.poll() {
            self.apply_hits(hits);
        }
        if let Some((_, at)) = self.toast.clone() {
            if at.elapsed() > Duration::from_millis(2600) {
                self.toast = None;
            }
        }
    }

    pub fn toast(&mut self, m: impl Into<String>) {
        self.toast = Some((m.into(), Instant::now()));
    }

    // ---- focus loss ---------------------------------------------------

    /// SRS 4.1: never silently discard work.
    pub fn on_focus_loss(&mut self) {
        match self.config.focus_loss {
            FocusLoss::Never => {}
            FocusLoss::AutoHide => {
                if self.dirty() {
                    self.prompt = Some(ui::PROMPT_FOCUS.into());
                } else {
                    self.hide();
                }
            }
            FocusLoss::Prompt => {
                if self.dirty() {
                    self.prompt = Some(ui::PROMPT_FOCUS.into());
                }
            }
        }
    }

    // ---- notes ----------------------------------------------------------

    pub fn open_note(&mut self, id: &str) {
        let Some(doc) = self.index.get(id) else { return };
        self.ed_text = doc.content.as_ref().clone();
        self.ed_original = self.ed_text.clone();
        self.ed_dir = doc.direction;
        self.ed_original_dir = doc.direction;
        self.ed_id = Some(id.to_string());
        self.ed_is_new = false;
        self.ed_return = View::Library;
        self.view = View::Editor;
        self.prompt = None;
    }

    pub fn new_draft(&mut self) {
        self.ed_text.clear();
        self.ed_original.clear();
        self.ed_dir = self.config.default_direction;
        self.ed_original_dir = self.ed_dir;
        self.ed_id = None;
        self.ed_is_new = true;
        self.ed_return = View::Chooser;
        self.view = View::Editor;
        self.prompt = None;
    }

    pub fn save(&mut self, close: bool) {
        if self.ed_text.trim().is_empty() && self.ed_is_new {
            if self.config.delete_empty_draft {
                self.toast("Empty draft discarded");
                if close {
                    self.hide();
                } else {
                    self.view = View::Chooser;
                    self.ed_id = None;
                }
            } else {
                self.toast("Write something first, or cancel to discard");
            }
            return;
        }

        let result = match self.ed_id.clone() {
            Some(id) => self
                .storage
                .save(&id, &self.ed_text, self.ed_dir)
                .and_then(|n| self.storage.rename_to_match_content(&n, &self.ed_text)),
            None => {
                let id = model::new_id();
                self.storage.save(&id, &self.ed_text, self.ed_dir)
            }
        };

        match result {
            Ok(n) => {
                self.ed_original = self.ed_text.clone();
                self.ed_original_dir = self.ed_dir;
                self.ed_id = Some(n.id.clone());
                self.ed_is_new = false;
                if !self.order.contains(&n.id) {
                    self.order.insert(0, n.id);
                    let _ = self.storage.write_order(&self.order);
                }
                self.refresh();
                self.toast(if close { "Saved" } else { "Saved · still open" });
                if close {
                    self.hide();
                }
            }
            Err(e) => self.storage_error = Some(format!("Could not save: {e}")),
        }
    }

    fn leave_editor(&mut self) {
        let back = self.ed_return;
        self.view = back;
        self.ed_id = None;
        self.ed_text.clear();
        self.ed_original.clear();
        self.ed_original_dir = self.ed_dir;
    }

    pub fn delete_note(&mut self, id: &str) {
        if let Some(n) = self.note_by_id(id) {
            if let Err(e) = self.storage.delete(&n) {
                self.storage_error = Some(format!("Could not delete: {e}"));
                return;
            }
        }
        self.order.retain(|x| x != id);
        let _ = self.storage.write_order(&self.order);
        if self.ed_id.as_deref() == Some(id) {
            self.ed_id = None;
            self.ed_text.clear();
        }
        self.confirm_delete = None;
        self.refresh();
        self.toast("Note deleted");
        if self.view == View::Editor {
            self.view = View::Library;
        }
    }

    fn note_by_id(&self, id: &str) -> Option<Note> {
        self.index.get(id).map(|d| Note {
            id: id.to_string(),
            path: d.path.clone(),
            direction: d.direction,
            modified_at: std::time::SystemTime::UNIX_EPOCH,
            size: 0,
        })
    }

    pub fn move_note(&mut self, id: &str, delta: i32) {
        let Some(i) = self.order.iter().position(|x| x == id) else { return };
        let j = (i as i32 + delta).clamp(0, self.order.len() as i32 - 1) as usize;
        if i == j {
            return;
        }
        let v = self.order.remove(i);
        self.order.insert(j, v);
        let _ = self.storage.write_order(&self.order);
        self.toast("Order saved");
        self.selected = j;
    }

    pub fn move_selected(&mut self, delta: i32) {
        if let Some(row) = self.rows.get(self.selected) {
            let id = row.id.clone();
            self.move_note(&id, delta);
        }
    }

    pub fn open_selected(&mut self) {
        if let Some(row) = self.rows.get(self.selected) {
            let id = row.id.clone();
            self.open_note(&id);
        }
    }

    // ---- dispatch ---------------------------------------------------------

    pub fn dispatch(&mut self, a: Action) {
        match a {
            Action::NewNote => self.new_draft(),
            Action::OpenLibrary => {
                self.refresh();
                self.query.clear();
                self.last_query.clear();
                self.recompute_rows();
                self.ed_return = View::Library;
                self.view = View::Library;
            }
            // Leave the draft intact when going to the library.
            Action::ShowLibrary => {
                self.refresh();
                self.view = View::Library;
            }
            Action::OpenSettings => {
                self.prev_view = self.view;
                self.settings_tab = Tab::General;
                self.recording = None;
                self.view = View::Settings;
            }
            Action::OpenNote(id) | Action::ResumeNote(id) => self.open_note(&id),
            Action::Back => match self.view {
                View::Settings => self.view = self.prev_view,
                View::Editor => {
                    if self.dirty() {
                        self.prompt = Some(ui::PROMPT_CANCEL.into());
                    } else {
                        self.leave_editor();
                    }
                }
                View::Library => {
                    self.view = View::Chooser;
                    self.query.clear();
                    self.last_query.clear();
                    self.recompute_rows();
                }
                View::Chooser => self.hide(),
            },
            Action::Save { close } => self.save(close),
            Action::Cancel => {
                if self.dirty() {
                    self.prompt = Some(ui::PROMPT_CANCEL.into());
                } else {
                    self.leave_editor();
                }
            }
            Action::Delete(id) | Action::ConfirmDeleteAccepted(id) => self.delete_note(&id),
            Action::ConfirmDelete(id) => self.confirm_delete = Some(id),
            Action::DismissConfirm => self.confirm_delete = None,
            Action::Move(id, d) => self.move_note(&id, d),
            Action::ReorderFocus(d) => {
                if !self.rows.is_empty() {
                    self.selected =
                        (self.selected as i32 + d).clamp(0, self.rows.len() as i32 - 1) as usize;
                }
            }
            Action::Select(i) => self.selected = i,
            Action::SetQuery(q) => {
                if q != self.query {
                    self.query = q.clone();
                    self.last_query = q;
                    self.searcher.submit(self.index.clone(), &self.query);
                    self.selected = 0;
                    self.recompute_rows();
                }
            }
            Action::SetDirection(d) => {
                self.ed_dir = d;
            }
            Action::SetDefaultDirection(d) => {
                self.config.default_direction = d;
                let _ = self.config.save(&self.config_path);
            }
            Action::SetPreviewLines(n) => {
                self.config.preview_lines = n;
                let _ = self.config.save(&self.config_path);
            }
            Action::SetFocusLoss(f) => {
                self.config.focus_loss = f;
                let _ = self.config.save(&self.config_path);
            }
            Action::SetFsync(b) => {
                self.config.fsync = b;
                let _ = self.config.save(&self.config_path);
            }
            Action::SetUsePortal(b) => {
                self.config.use_portal = b;
                if !b {
                    hotkey::forget_portal_failure();
                }
                let _ = self.config.save(&self.config_path);
                self.toast(if b {
                    "Portal enabled — restart to try it. It may be unstable on some desktops."
                } else {
                    "Portal disabled"
                });
            }
            Action::SetBinding(cmd, binding) => {
                self.config.bindings.insert(cmd, binding.clone());
                self.recording = None;
                self.conflicts = self.config.binding_conflicts().into_iter().collect();
                let _ = self.config.save(&self.config_path);
                self.toast(format!("{} → {}", cmd.label(), crate::ui::pretty_key(&binding)));
            }
            Action::ResetBindings => {
                self.config.bindings = crate::config::default_bindings();
                self.recording = None;
                self.conflicts = self.config.binding_conflicts().into_iter().collect();
                let _ = self.config.save(&self.config_path);
            }
            Action::ImportNotes => self.import_notes(),
            Action::ExportNote(id) => self.export_note(&id),
            Action::RebuildIndex => {
                self.refresh();
                self.toast(format!("Index rebuilt · {} files rescanned", self.index.len()));
            }
            Action::DismissToast => self.toast = None,
            Action::DismissWarning => self.config_warning = None,
            Action::DismissPrompt => self.prompt = None,
            Action::PromptAction(id) => match id.as_str() {
                ui::PROMPT_SAVE_CLOSE => {
                    self.prompt = None;
                    self.save(true);
                }
                ui::PROMPT_DISCARD => {
                    self.prompt = None;
                    self.ed_text = self.ed_original.clone();
                    self.ed_dir = self.ed_original_dir;
                    self.hide();
                }
                ui::PROMPT_STAY => self.prompt = None,
                ui::CONFIRM_DELETE => {
                    if let Some(id) = self.confirm_delete.clone() {
                        self.delete_note(&id);
                    }
                }
                _ => self.prompt = None,
            },
            Action::OpenNotesDir => {
                let _ = std::process::Command::new("xdg-open")
                    .arg(&self.config.notes_dir)
                    .spawn();
            }
            Action::ApplyHotkey => match hotkey::install_desktop_shortcut(&self.hotkey_spec) {
                Ok(()) => self.toast(format!(
                    "Desktop shortcut written · {}",
                    self.hotkey_spec.to_kde_accelerator()
                )),
                Err(e) => self.storage_error = Some(format!("Could not install shortcut: {e}")),
            },
            Action::Close => self.hide(),
        }
    }

    fn import_notes(&mut self) {
        let Some(parent) = self.storage.notes_dir.parent().map(|p| p.to_path_buf()) else {
            return;
        };
        let dir = parent.join("inbox");
        let _ = std::fs::create_dir_all(&dir);
        let mut n = 0;
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                if e.path().extension().and_then(|s| s.to_str()) == Some("txt")
                    && self.storage.import_file(&e.path()).is_ok()
                {
                    n += 1;
                }
            }
        }
        self.refresh();
        self.toast(format!("Imported {n} note{}", if n == 1 { "" } else { "s" }));
    }

    fn export_note(&mut self, id: &str) {
        let Some(doc) = self.index.get(id) else { return };
        let target = self.storage.export_path(doc.content.as_str());
        match std::fs::write(&target, doc.content.as_bytes()) {
            Ok(()) => self.toast(format!("Exported to {}", target.display())),
            Err(e) => self.storage_error = Some(format!("Export failed: {e}")),
        }
    }

    pub fn hints(&self) -> Vec<(Command, &'static str)> {
        let mut v: Vec<(Command, &'static str)> = vec![];
        match self.view {
            View::Chooser => {
                v.push((Command::NewNote, "new"));
                v.push((Command::OpenLibrary, "notes"));
            }
            View::Editor => {
                v.push((Command::Save, "save"));
                v.push((Command::SaveClose, "save & close"));
                v.push((Command::ShowLibrary, "notes"));
                v.push((Command::ToggleDirection, "RTL/LTR"));
            }
            View::Library => {
                v.push((Command::MoveUp, "move up"));
                v.push((Command::OpenLibrary, "back"));
            }
            View::Settings => {}
        }
        v.push((Command::Settings, "settings"));
        v.push((
            Command::Back,
            if self.view == View::Chooser { "close" } else { "back" },
        ));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app(dir: &std::path::Path) -> App {
        let mut cfg = Config::default();
        cfg.notes_dir = dir.join("notes");
        let storage = Storage::new(&cfg);
        let (index, order, bytes) = App::load_index(&storage);
        App {
            config: cfg,
            config_path: dir.join("config.toml"),
            storage,
            total_bytes: bytes,
            hotkey: hotkey::Service {
                rx: std::sync::mpsc::channel().1,
                status: hotkey::Status {
                    backend: hotkey::Backend::None,
                    trigger_confirmed: false,
                    detail: String::new(),
                },
            },
            hotkey_spec: HotkeySpec::parse("Meta+Shift+Space").unwrap(),
            index,
            order,
            searcher: Searcher::new(),
            view: View::Chooser,
            prev_view: View::Chooser,
            ed_id: None,
            ed_text: String::new(),
            ed_original: String::new(),
            ed_dir: Direction::Rtl,
            ed_original_dir: Direction::Rtl,
            ed_is_new: true,
            ed_return: View::Chooser,
            query: String::new(),
            rows: Vec::new(),
            selected: 0,
            last_query: String::new(),
            recording: None,
            settings_tab: Tab::General,
            conflicts: HashSet::new(),
            prompt: None,
            confirm_delete: None,
            toast: None,
            config_warning: None,
            storage_error: None,
            visible: false,
            closing: false,
        }
    }

    #[test]
    fn dirty_tracks_text_and_direction() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        a.new_draft();
        assert!(!a.dirty());
        a.ed_text = "שלום".into();
        assert!(a.dirty());
        a.ed_text = a.ed_original.clone();
        assert!(!a.dirty());
        a.ed_dir = Direction::Ltr;
        assert!(a.dirty(), "direction change alone is a real edit");
    }

    #[test]
    fn focus_loss_never_discards_work() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        a.config.focus_loss = FocusLoss::AutoHide;
        a.new_draft();
        a.ed_text = "unsaved".into();
        a.visible = true;
        a.on_focus_loss();
        assert!(a.visible, "must not close with unsaved text");
        assert_eq!(a.prompt.as_deref(), Some(ui::PROMPT_FOCUS));
    }

    #[test]
    fn focus_loss_hides_when_clean() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        a.config.focus_loss = FocusLoss::AutoHide;
        a.visible = true;
        a.on_focus_loss();
        assert!(!a.visible);
    }

    #[test]
    fn save_creates_a_note_and_clears_dirty() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        a.new_draft();
        a.ed_text = "hello".into();
        a.save(false);
        assert!(!a.dirty());
        assert_eq!(a.order.len(), 1);
    }

    #[test]
    fn empty_draft_is_discarded_without_touching_disk() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        a.new_draft();
        a.save(false);
        assert!(a.order.is_empty());
    }

    #[test]
    fn back_from_editor_returns_to_library() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        a.new_draft();
        a.ed_text = "x".into();
        a.save(false);
        a.dispatch(Action::OpenLibrary);
        a.open_selected();
        assert_eq!(a.view, View::Editor);
        a.dispatch(Action::Back);
        assert_eq!(a.view, View::Library, "Back must return to the library, not the chooser");
    }

    #[test]
    fn delete_requires_confirmation() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        a.new_draft();
        a.ed_text = "gone soon".into();
        a.save(false);
        let id = a.order[0].clone();
        a.dispatch(Action::ConfirmDelete(id.clone()));
        assert_eq!(a.order.len(), 1, "confirming must not delete yet");
        a.dispatch(Action::ConfirmDeleteAccepted(id));
        assert!(a.order.is_empty());
    }

    #[test]
    fn direction_survives_a_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        a.new_draft();
        a.ed_dir = Direction::Rtl;
        a.ed_text = "רשימה\nחלה, יין".into();
        a.save(false);
        let id = a.order[0].clone();
        a.dispatch(Action::OpenNote(id));
        assert_eq!(a.ed_dir, Direction::Rtl, "F8: direction is preserved");
        assert_eq!(a.ed_text, "רשימה\nחלה, יין");
    }

    #[test]
    fn hints_are_live_for_the_current_view() {
        let d = tempfile::tempdir().unwrap();
        let a = test_app(d.path());
        assert!(a.hints().iter().any(|(c, _)| *c == Command::NewNote));
    }
}