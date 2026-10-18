use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long typing must pause before the note is written to disk.
const AUTOSAVE_DELAY: Duration = Duration::from_millis(700);

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
    /// Set while recording the second global key, which is a config field and
    /// not one of the rebindable commands.
    pub recording_global: bool,
    pub settings_tab: Tab,
    pub conflicts: HashSet<Command>,
    /// Set when a global trigger has been changed but the compositor is still
    /// holding the old one. The global key is registered once at startup, so
    /// until the next run the UI would be showing a shortcut that does not
    /// work — the UI says so rather than quietly lying.
    pub trigger_restart_pending: bool,

    pub prompt: Option<String>,
    pub confirm_delete: Option<String>,
    pub toast: Option<(String, Instant)>,
    pub config_warning: Option<String>,
    pub storage_error: Option<String>,

    /// When the text last changed, for autosave debouncing.
    pub last_edit: Option<Instant>,
    /// True once the user has typed in the current capture, so autosave does
    /// not write empty notes.
    pub touched: bool,
    pub visible: bool,
    /// Set by the GUI layer when Escape/close was requested.
    pub closing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hint {
    /// Set when the key comes from a rebindable command.
    pub cmd: Option<Command>,
    /// Set when the key is not a command at all — Enter in the list, or the
    /// arrow keys GTK handles on its own.
    pub literal: Option<&'static str>,
    pub label: &'static str,
}

impl Hint {
    pub fn key(cmd: Command, label: &'static str) -> Hint {
        Hint { cmd: Some(cmd), literal: None, label }
    }

    pub fn plain(key: &'static str, label: &'static str) -> Hint {
        Hint { cmd: None, literal: Some(key), label }
    }
}

/// The other half of the pair that shares one global key.
///
/// `Invoke` and `ToggleOverlay` are two names for the same action, so editing
/// either has to edit both. Editing one and not the other leaves a binding in
/// the settings list that nothing ever listens for.
fn other_global(cmd: Command) -> Command {
    match cmd {
        Command::Invoke => Command::ToggleOverlay,
        _ => Command::Invoke,
    }
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
        let notes_hotkey = HotkeySpec::parse(&config.notes_hotkey)
            .unwrap_or_else(|_| HotkeySpec::parse("Meta+Shift+L").expect("literal"));
        let hotkey = hotkey::start(invoke.clone(), notes_hotkey, config.use_portal);

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
            ed_return: View::Library,
            query: String::new(),
            rows: Vec::new(),
            selected: 0,
            last_query: String::new(),
            recording: None,
            recording_global: false,
            settings_tab: Tab::General,
            conflicts,
            trigger_restart_pending: false,
            prompt: None,
            confirm_delete: None,
            toast: None,
            config_warning: loaded.warning,
            storage_error: None,
            last_edit: None,
            touched: false,
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
        self.resync_rows();
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

    /// Re-derive the visible rows from the current order *without* throwing a
    /// filtered list away.
    ///
    /// `recompute_rows` resets a search back to every note, which is right when
    /// the query itself is changing and wrong for everything else: reordering,
    /// saving and deleting all call it, and each of those used to make a search
    /// silently become the full list.
    fn resync_rows(&mut self) {
        if self.query.trim().is_empty() {
            self.recompute_rows();
            return;
        }
        let mut rows = std::mem::take(&mut self.rows);
        rows.sort_by_key(|r| self.order.iter().position(|id| *id == r.id).unwrap_or(usize::MAX));
        self.rows = rows;
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

    /// Invoke goes straight into an empty editor.
    ///
    /// There is deliberately no chooser on the hot path. Asking "new note or
    /// Open on the chooser. The hotkey has to mean one thing, and "capture"
    /// is not it — half the time the intent is "what did I write down?".
    pub fn show(&mut self) {
        self.visible = true;
        self.closing = false;
        self.view = View::Chooser;
    }

    /// Close the overlay. Anything typed is saved first: the whole premise is
    /// that a captured thought survives, so hiding is a save.
    pub fn hide(&mut self) {
        if self.view == View::Editor && self.has_content() && self.dirty() {
            self.save_quiet(true);
        }
        self.visible = false;
        self.view = View::Chooser;
        self.ed_id = None;
        self.ed_text.clear();
        self.ed_original.clear();
        self.prompt = None;
        self.confirm_delete = None;
        self.recording = None;
        self.query.clear();
        self.touched = false;
        self.last_edit = None;
        self.recompute_rows();
    }

    /// The invoke key always means "capture". Pressing it again closes the
    /// overlay, which is the same muscle memory as any quick-capture tool.
    pub fn toggle(&mut self) {
        if self.visible {
            self.hide();
        } else {
            self.show();
        }
    }

    /// Second global entry point: go straight to the list, skipping capture.
    pub fn show_library(&mut self) {
        self.visible = true;
        self.closing = false;
        self.dispatch(Action::OpenLibrary);
    }

    /// Called every frame by the GUI layer.
    pub fn pump(&mut self) {
        let mut toggle = false;
        let mut library = false;
        while self.hotkey.rx.try_recv().is_ok() {
            toggle = true;
        }
        match hotkey::take_show_request() {
            Some(hotkey::ShowRequest::Toggle) => toggle = true,
            Some(hotkey::ShowRequest::Library) => library = true,
            None => {}
        }
        if library {
            self.show_library();
        } else if toggle {
            self.toggle();
        }
        if self.autosave() {
            return;
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

    /// Called by the GUI whenever the buffer changes.
    pub fn note_edited(&mut self) {
        self.last_edit = Some(Instant::now());
        self.touched = true;
    }

    /// Is there anything worth writing to disk?
    pub fn has_content(&self) -> bool {
        !self.ed_text.trim().is_empty()
    }

    /// Autosave after a pause in typing.
    ///
    /// A quick capture must never depend on the user remembering to save.
    /// Nothing is written for an empty buffer, so no junk files appear.
    pub fn autosave(&mut self) -> bool {
        if self.view != View::Editor || !self.has_content() || !self.dirty() {
            return false;
        }
        let Some(at) = self.last_edit else { return false };
        if at.elapsed() < AUTOSAVE_DELAY {
            return false;
        }
        // `save` assigns `ed_id` from the note it just wrote, so subsequent
        // autosaves keep updating the same file rather than spawning new ones.
        self.save_quiet(false);
        true
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
        self.touched = false;
        self.last_edit = None;
    }

    pub fn new_draft(&mut self) {
        self.ed_text.clear();
        self.ed_original.clear();
        self.ed_dir = self.config.default_direction;
        self.ed_original_dir = self.ed_dir;
        self.ed_id = None;
        self.ed_is_new = true;
        // A new note was started from the chooser, so that is where saving
        // should land. Opening one from the list returns to the list.
        self.ed_return = View::Chooser;
        self.view = View::Editor;
        self.prompt = None;
        self.touched = false;
        self.last_edit = None;
    }

    /// Save without any feedback. Used by autosave and by closing the overlay,
    /// where a confirmation toast is pure noise.
    pub fn save_quiet(&mut self, close: bool) {
        let saved_toast = self.toast.take();
        self.save(close);
        self.toast = saved_toast;
    }

    pub fn save(&mut self, close: bool) {
        if self.ed_text.trim().is_empty() && self.ed_is_new {
            if self.config.delete_empty_draft {
                self.toast("Empty draft discarded");
                if close {
                    self.hide();
                } else {
                    self.hide();
                }
            } else {
                self.toast("Write something first, or cancel to discard");
            }
            return;
        }

        let result = match self.ed_id.clone() {
            // Filenames stay stable. Renaming a note as its first line is
            // typed produces a burst of junk files during exactly the flow
            // this app is built for, and the title is derived from content
            // for display anyway.
            Some(id) => self.storage.save(&id, &self.ed_text, self.ed_dir),
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
        // The visible list has to follow the new order, or the arrows appear to
        // do nothing until the overlay is reopened.
        self.resync_rows();
        self.selected = self.rows.iter().position(|r| r.id == id).unwrap_or(self.selected);
        self.toast("Order saved");
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
            Action::OpenNote(id) => self.open_note(&id),
            // Back closes the overlay rather than navigating a menu. There is
            // no menu: Back is "I'm done here".
            Action::Back => match self.view {
                View::Settings => self.view = self.prev_view,
                View::Library => self.view = View::Chooser,
                // Back means "one step up", not "shut the app". Closing is
                // what Back does at the top of the stack, and what the invoke
                // key does anywhere.
                View::Editor => {
                    // Persist, but stay open: we are stepping up a level,
                    // not shutting down.
                    self.save_quiet(false);
                    self.view = self.ed_return;
                    self.ed_id = None;
                    self.ed_text.clear();
                    self.ed_original.clear();
                    self.prompt = None;
                }
                View::Chooser => self.hide(),
            },
            Action::Save { close } => self.save(close),
            Action::Cancel => {
                if self.dirty() {
                    self.prompt = Some(ui::PROMPT_CANCEL.into());
                } else {
                    self.hide();
                }
            }
            Action::Delete(id) | Action::ConfirmDeleteAccepted(id) => self.delete_note(&id),
            Action::ConfirmDelete(id) => self.confirm_delete = Some(id),
            Action::DismissConfirm => self.confirm_delete = None,
            Action::Move(id, d) => self.move_note(&id, d),
            Action::MoveSelected(d) => {
                let id = self.rows.get(self.selected).map(|r| r.id.clone());
                if let Some(id) = id {
                    self.move_note(&id, d);
                }
            }
            Action::Select(i) => {
                if i < self.rows.len() {
                    self.selected = i;
                }
            }
            Action::FocusSearch => {}
            Action::SetQuery(q) => {
                if q != self.query {
                    self.query = q.clone();
                    self.last_query = q;
                    self.searcher.submit(self.index.clone(), &self.query);
                    self.selected = 0;
                    // Deliberately *not* recompute_rows: the previous results
                    // stay on screen until the worker answers, so the list does
                    // not flash the entire library on every keystroke.
                    if self.query.trim().is_empty() {
                        self.recompute_rows();
                    }
                }
            }
            Action::SelectTab(t) => self.settings_tab = t,
            Action::SetDirection(d) => {
                self.ed_dir = d;
            }
            Action::SetDefaultDirection(d) => {
                self.config.default_direction = d;
                let _ = self.config.save(&self.config_path);
            }
            Action::SetTheme(t) => {
                self.config.theme = t;
                let _ = self.config.save(&self.config_path);
            }
            Action::SetPreviewLines(n) => {
                self.config.preview_lines = n.clamp(1, 8);
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
                self.recording_global = false;
                self.config.bindings.insert(cmd, binding.clone());
                // Invoke and ToggleOverlay are two names for the same global
                // action and deliberately share a key. Setting only one left
                // the other showing a shortcut that nothing listens for.
                if matches!(cmd, Command::Invoke | Command::ToggleOverlay) {
                    self.config
                        .bindings
                        .insert(other_global(cmd), binding.clone());
                    self.trigger_restart_pending = true;
                }
                self.recording = None;
                self.conflicts = self.config.binding_conflicts().into_iter().collect();
                let _ = self.config.save(&self.config_path);
                self.toast(format!("{} → {}", cmd.label(), crate::ui::pretty_key(&binding)));
            }
            Action::SetNotesHotkey(binding) => {
                self.recording = None;
                self.recording_global = false;
                if self.config.notes_hotkey != binding {
                    self.config.notes_hotkey = binding.clone();
                    self.trigger_restart_pending = true;
                    let _ = self.config.save(&self.config_path);
                    self.toast(format!("My notes → {}", crate::ui::pretty_key(&binding)));
                }
            }
            Action::ResetBindings => {
                self.config.bindings = crate::config::default_bindings();
                self.config.notes_hotkey = "Meta+Shift+L".to_string();
                self.trigger_restart_pending = true;
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
            // A failed save has to stay on screen until it is acknowledged:
            // silently swallowing it is how a note looks captured and is not.
            Action::DismissError => self.storage_error = None,
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
            Action::ApplyHotkey => {
                let notes = HotkeySpec::parse(&self.config.notes_hotkey)
                    .unwrap_or_else(|_| HotkeySpec::parse("Meta+Shift+L").expect("literal"));
                match hotkey::install_desktop_shortcut(&self.hotkey_spec, &notes) {
                    Ok(()) => self.toast(format!(
                        "Shortcuts written · {} capture, {} notes",
                        self.hotkey_spec.to_kde_accelerator(),
                        notes.to_kde_accelerator()
                    )),
                    Err(e) => {
                        self.storage_error = Some(format!("Could not install shortcut: {e}"))
                    }
                }
            },
            Action::SetKeepAbove(v) => {
                self.config.keep_above = v;
                let _ = self.config.save(&self.config_path);
                if v {
                    crate::platform::apply_keep_above("batnotes");
                }
            }
            Action::DeleteFocused => {
                if let Some(id) = self.rows.get(self.selected).map(|r| r.id.clone()) {
                    self.confirm_delete = Some(id);
                }
            }
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

    /// Bindings that are worth spelling out in the current view.
    ///
    /// Every other action already carries its key on the button that performs
    /// it, so listing those again just fills the footer with "Del delete" and
    /// "Ctrl , settings". What belongs here is what the buttons cannot say:
    /// shortcuts with no button behind them.
    pub fn hints(&self) -> Vec<Hint> {
        let mut v: Vec<Hint> = vec![];
        match self.view {
            View::Chooser => {
                // Both destinations are on screen with their keys attached.
            }
            View::Editor => {
                // Ctrl+S saves without closing; the button is save *and* close.
                v.push(Hint::key(Command::Save, "save, stay here"));
            }
            View::Library => {
                v.push(Hint::key(Command::FocusSearch, "search"));
                v.push(Hint::plain("Enter", "open"));
                v.push(Hint::plain("↑ ↓", "move selection"));
            }
            View::Settings => {}
        }
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
            ed_return: View::Library,
            query: String::new(),
            rows: Vec::new(),
            selected: 0,
            last_query: String::new(),
            recording: None,
            recording_global: false,
            settings_tab: Tab::General,
            conflicts: HashSet::new(),
            trigger_restart_pending: false,
            prompt: None,
            confirm_delete: None,
            toast: None,
            config_warning: None,
            storage_error: None,
            last_edit: None,
            touched: false,
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
    fn back_walks_up_one_level_and_saves_on_the_way() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        a.show();
        a.dispatch(Action::NewNote);
        a.ed_text = "something".into();
        a.note_edited();
        a.hide();
        a.show();
        a.dispatch(Action::OpenLibrary);
        a.open_selected();
        assert_eq!(a.view, View::Editor, "picking a note opens the editor");
        a.visible = true;
        a.dispatch(Action::Back);
        assert_eq!(a.view, View::Library, "Back returns to the list you came from");
        assert!(a.visible, "going back must not close the overlay");
        a.dispatch(Action::Back);
        assert_eq!(a.view, View::Chooser);
        a.dispatch(Action::Back);
        assert!(!a.visible, "Back at the top of the stack closes");
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
    fn invoke_offers_capture_or_review_and_new_note_is_empty() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        a.show();
        assert_eq!(
            a.view,
            View::Chooser,
            "the hotkey must not guess between capturing and reviewing"
        );
        a.dispatch(Action::NewNote);
        assert_eq!(a.view, View::Editor);
        assert!(a.ed_id.is_none());
        assert!(a.ed_text.is_empty());
        a.dispatch(Action::Back);
        assert_eq!(a.view, View::Chooser, "Back from the editor returns to the choice");
    }

    #[test]
    fn closing_saves_the_capture_rather_than_dropping_it() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        a.show();
        a.dispatch(Action::NewNote);
        a.ed_text = "a passing thought".into();
        a.note_edited();
        a.hide();
        assert_eq!(a.order.len(), 1, "closing must not discard a capture");
        let id = a.order[0].clone();
        assert_eq!(
            a.storage.read_note(&a.index.get(&id).unwrap().path).unwrap(),
            "a passing thought"
        );
    }

    #[test]
    fn autosave_writes_after_a_pause_without_a_keypress() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        a.show();
        a.dispatch(Action::NewNote);
        a.ed_text = "no key pressed".into();
        a.note_edited();
        assert!(!a.autosave(), "must wait for the debounce");
        a.last_edit = Some(Instant::now() - Duration::from_secs(2));
        assert!(a.autosave());
        assert_eq!(a.order.len(), 1);
    }

    #[test]
    fn autosave_ignores_an_untouched_buffer() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        a.show();
        a.dispatch(Action::NewNote);
        a.last_edit = Some(Instant::now() - Duration::from_secs(5));
        assert!(!a.autosave());
        assert!(a.order.is_empty(), "no note may be written for an empty buffer");
    }

    #[test]
    fn autosave_keeps_writing_to_the_same_note() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        a.show();
        a.dispatch(Action::NewNote);
        a.ed_text = "one".into();
        a.note_edited();
        a.last_edit = Some(Instant::now() - Duration::from_secs(2));
        a.autosave();
        let first = a.order[0].clone();

        a.ed_text = "one two".into();
        a.note_edited();
        a.last_edit = Some(Instant::now() - Duration::from_secs(2));
        a.autosave();

        assert_eq!(a.order.len(), 1, "autosave must not create a note per pause");
        assert_eq!(a.order[0], first);
        // The file is renamed as the first line changes, so assert on whatever
        // single .txt exists rather than on a remembered path.
        let files: Vec<_> = std::fs::read_dir(d.path().join("notes"))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("txt"))
            .collect();
        assert_eq!(files.len(), 1, "one note file, no churn");
        assert_eq!(std::fs::read_to_string(&files[0]).unwrap(), "one two");
    }

    #[test]
    fn every_capture_starts_a_fresh_draft() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        a.show();
        a.dispatch(Action::NewNote);
        a.ed_text = "first thought".into();
        a.note_edited();
        a.hide();
        a.show();
        a.dispatch(Action::NewNote);
        assert!(a.ed_text.is_empty(), "each capture is a new note");
        assert!(a.ed_id.is_none(), "a new draft is not editing the last note");
        assert_eq!(a.order.len(), 1, "the first thought was still captured");
    }

    #[test]
    fn reordering_moves_the_note_and_keeps_it_selected() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        for body in ["first", "second", "third"] {
            a.new_draft();
            a.ed_text = body.into();
            a.save(false);
        }
        // Newest first, so the order is third, second, first.
        assert_eq!(a.order.len(), 3);
        a.dispatch(Action::OpenLibrary);
        assert_eq!(a.rows.len(), 3);
        let last = a.rows[a.rows.len() - 1].id.clone();
        a.dispatch(Action::Select(2));
        a.dispatch(Action::MoveSelected(-1));
        assert_eq!(a.rows[1].id, last, "the note actually changed place");
        assert_eq!(a.selected, 1, "and stayed selected so it can be moved again");
    }

    #[test]
    fn reordering_persists_across_a_reload() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        for body in ["one", "two"] {
            a.new_draft();
            a.ed_text = body.into();
            a.save(false);
        }
        let moved = a.order[1].clone();
        a.dispatch(Action::Select(1));
        a.dispatch(Action::MoveSelected(-1));
        assert_eq!(a.order.first(), Some(&moved), "the note moved to the front");
        // refresh() is what the app does on the next visit: it re-reads the
        // manifest from disk, so the order has to have been written.
        a.refresh();
        assert_eq!(a.order.first(), Some(&moved));
    }

    #[test]
    fn a_search_survives_a_save_instead_of_becoming_the_full_list() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        for body in ["shopping list", "meeting notes"] {
            a.new_draft();
            a.ed_text = body.into();
            a.save(false);
        }
        a.dispatch(Action::OpenLibrary);
        a.dispatch(Action::SetQuery("shopping".into()));
        // The worker is asynchronous; ask for the same thing it would deliver.
        a.apply_hits(a.index.search("shopping"));
        assert_eq!(a.rows.len(), 1);

        a.new_draft();
        a.ed_text = "shopping again".into();
        a.save(false);
        assert!(
            a.rows.len() <= 2,
            "saving must not silently replace the filtered list with everything"
        );
    }

    #[test]
    fn selecting_past_the_end_of_the_list_is_ignored() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        a.new_draft();
        a.ed_text = "only note".into();
        a.save(false);
        a.dispatch(Action::OpenLibrary);
        a.dispatch(Action::Select(99));
        assert_eq!(a.selected, 0);
    }

    #[test]
    fn rebinding_the_invoke_key_rebinds_its_other_name_too() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        a.dispatch(Action::SetBinding(Command::Invoke, "Ctrl+Alt+N".into()));
        // Invoke and ToggleOverlay are two names for one global action. Leaving
        // the other showing the old key meant the settings screen advertised a
        // shortcut nothing was listening for.
        assert_eq!(a.config.binding(Command::ToggleOverlay), "Ctrl+Alt+N");
        assert!(a.conflicts.is_empty(), "the pair is exempt from conflict checks");
    }

    #[test]
    fn changing_a_global_key_says_it_needs_a_restart() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        assert!(!a.trigger_restart_pending);
        a.dispatch(Action::SetBinding(Command::Invoke, "Ctrl+Alt+N".into()));
        assert!(
            a.trigger_restart_pending,
            "the compositor is still holding the old key and the UI must say so"
        );
        // It stays set: the compositor is still holding the old global key, and
        // only a restart clears that. An in-app binding changing meanwhile does
        // not make the global one real again.
        a.dispatch(Action::SetBinding(Command::Save, "Ctrl+Shift+S".into()));
        assert!(a.trigger_restart_pending);
    }

    #[test]
    fn the_second_global_key_is_saved_and_reported() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        a.dispatch(Action::SetNotesHotkey("Ctrl+Alt+L".into()));
        assert_eq!(a.config.notes_hotkey, "Ctrl+Alt+L");
        assert!(a.trigger_restart_pending);
        assert!(a.recording.is_none(), "recording ends when the key is set");
    }

    #[test]
    fn resetting_bindings_restores_the_defaults() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        a.dispatch(Action::SetBinding(Command::Save, "Ctrl+Alt+Q".into()));
        a.dispatch(Action::SetNotesHotkey("Ctrl+Alt+L".into()));
        a.dispatch(Action::ResetBindings);
        assert_eq!(a.config.binding(Command::Save), "Ctrl+S");
        assert_eq!(a.config.notes_hotkey, "Meta+Shift+L");
    }

    #[test]
    fn hints_are_live_for_the_current_view() {
        let d = tempfile::tempdir().unwrap();
        let mut a = test_app(d.path());
        a.show();
        // The chooser puts both destinations on screen with their keys, so it
        // has nothing to add in a legend.
        assert!(a.hints().is_empty());
        a.dispatch(Action::NewNote);
        assert!(a.hints().iter().any(|h| h.cmd == Some(Command::Save)));
        a.dispatch(Action::OpenLibrary);
        assert!(a.hints().iter().any(|h| h.cmd == Some(Command::FocusSearch)));
        assert!(
            a.hints().iter().any(|h| h.literal == Some("Enter")),
            "the list must say that Enter opens a note"
        );
    }
}