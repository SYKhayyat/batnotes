use anyhow::Result;
use egui::{Context, Key};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::{Command, Config, FocusLoss};
use crate::hotkey::{self, HotkeySpec};
use crate::model::{self, Direction, Note};
use crate::search::{Doc, Indexed, Match, Searcher};
use crate::storage::Storage;
use crate::ui::library::Row;
use crate::ui::settings::Tab;
use crate::ui::{self, kit, theme::Fonts, theme::Theme, Action, View};

/// What the keyboard produced this frame.
struct Keys {
    combo: Option<String>,
    pressed: bool,
    escape: bool,
    enter: bool,
    tab: bool,
    up: bool,
    down: bool,
    /// A real (not bare-modifier) key while recording.
    recorded: Option<String>,
}

impl Default for Keys {
    fn default() -> Self {
        Keys { combo: None, pressed: false, escape: false, enter: false, tab: false, up: false, down: false, recorded: None }
    }
}

pub struct App {
    config: Config,
    config_path: PathBuf,
    storage: Storage,
    theme: Theme,
    fonts: Fonts,

    hotkey: hotkey::Service,
    hotkey_spec: HotkeySpec,

    index: Arc<Indexed>,
    order: Vec<String>,
    searcher: Searcher,

    view: View,
    prev_view: View,

    ed_id: Option<String>,
    ed_text: String,
    ed_original: String,
    ed_dir: Direction,
    ed_original_dir: Direction,
    ed_is_new: bool,
    /// Where Escape/Cancel should go when the editor closes.
    ed_return: View,

    query: String,
    rows: Vec<Row>,
    selected: usize,
    hover: Option<usize>,
    dragging: Option<usize>,
    drag_over: Option<usize>,

    recording: Option<Command>,
    settings_tab: Tab,
    conflicts: HashSet<Command>,

    prompt: Option<String>,
    confirm_delete: Option<String>,
    toast: Option<(String, Instant)>,
    config_warning: Option<String>,
    storage_error: Option<String>,

    focus_editor: bool,
    opened_at: Option<Instant>,
    open_ms: Option<f32>,
    visible: bool,
    was_focused: bool,
    total_bytes: u64,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Result<Self> {
        let config_path = crate::config::config_path();
        let loaded = Config::load(&config_path)?;
        let config = loaded.config;

        let fonts = Fonts::load(&cc.egui_ctx, config.heading_font.clone(), config.ui_font.clone());
        let theme = Theme::from_config(&config);
        theme.apply(&cc.egui_ctx);
        let _ = &fonts;

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
            theme,
            fonts,
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
            hover: None,
            dragging: None,
            drag_over: None,
            recording: None,
            settings_tab: Tab::General,
            conflicts,
            prompt: None,
            confirm_delete: None,
            toast: None,
            config_warning: loaded.warning,
            storage_error: None,
            focus_editor: false,
            opened_at: None,
            open_ms: None,
            visible: false,
            was_focused: false,
            total_bytes: bytes,
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

    fn refresh(&mut self) {
        let (index, order, bytes) = Self::load_index(&self.storage);
        self.index = index;
        self.order = order;
        self.total_bytes = bytes;
        self.searcher.rebuild(self.index.clone());
        self.recompute_rows();
    }

    fn dirty(&self) -> bool {
        self.ed_text != self.ed_original || self.ed_dir != self.ed_original_dir
    }

    fn recompute_rows(&mut self) {
        if self.query.trim().is_empty() {
            self.rows = self.order.iter().map(|id| Row { id: id.clone(), at: None }).collect();
        } else {
            self.rows = self.index.ids().into_iter().map(|id| Row { id, at: None }).collect();
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

    // ---- visibility -----------------------------------------------------

    fn show(&mut self, ctx: &Context) {
        self.opened_at = Some(Instant::now());
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        self.visible = true;
    }

    fn hide(&mut self, ctx: &Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
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

    fn toast(&mut self, m: impl Into<String>) {
        self.toast = Some((m.into(), Instant::now()));
    }

    // ---- notes ----------------------------------------------------------

    fn open_note(&mut self, id: &str) {
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
        self.focus_editor = true;
    }

    fn new_draft(&mut self) {
        self.ed_text.clear();
        self.ed_original.clear();
        self.ed_dir = self.config.default_direction;
        self.ed_original_dir = self.ed_dir;
        self.ed_id = None;
        self.ed_is_new = true;
        self.ed_return = View::Chooser;
        self.view = View::Editor;
        self.prompt = None;
        self.focus_editor = true;
    }

    fn save(&mut self, close: bool) {
        if self.ed_text.trim().is_empty() && self.ed_is_new {
            if self.config.delete_empty_draft {
                self.toast("Empty draft discarded");
                if close {
                    self.hide_nohide();
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
                    self.hide_nohide();
                }
            }
            Err(e) => self.storage_error = Some(format!("Could not save: {e}")),
        }
    }

    /// Reset the buffer without touching window visibility.
    fn hide_nohide(&mut self) {
        self.view = View::Chooser;
        self.ed_id = None;
        self.ed_text.clear();
        self.ed_original.clear();
        self.confirm_delete = None;
        self.prompt = None;
    }

    fn delete_note(&mut self, id: &str) {
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

    fn move_note(&mut self, id: &str, delta: i32) {
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

    // ---- dispatch --------------------------------------------------------

    fn dispatch(&mut self, a: Action, ctx: &Context) {
        match a {
            Action::NewNote => self.new_draft(),
            Action::OpenLibrary => {
                self.refresh();
                self.query.clear();
                self.recompute_rows();
                self.ed_return = View::Library;
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
                        let back = self.ed_return;
                        self.view = back;
                        self.ed_id = None;
                        self.ed_text.clear();
                    }
                }
                View::Library => {
                    self.view = View::Chooser;
                    self.query.clear();
                    self.recompute_rows();
                }
                View::Chooser => self.hide(ctx),
            },
            Action::Save { close } => self.save(close),
            Action::Cancel => {
                if self.dirty() {
                    self.prompt = Some(ui::PROMPT_CANCEL.into());
                } else {
                    let back = self.ed_return;
                    self.view = back;
                    self.ed_id = None;
                    self.ed_text.clear();
                }
            }
            // Jump from the editor straight to the library, and back again.
            Action::ShowLibrary => {
                self.ed_id = None;
                self.ed_text.clear();
                if self.ed_original.is_empty() {
                    self.ed_original = String::new();
                }
                self.refresh();
                self.query.clear();
                self.recompute_rows();
                self.ed_return = View::Library;
                self.view = View::Library;
            }
            Action::Delete(id) | Action::ConfirmDeleteAccepted(id) => self.delete_note(&id),
            Action::ConfirmDelete(id) => self.confirm_delete = Some(id),
            Action::DismissConfirm => self.confirm_delete = None,
            Action::Move(id, d) => self.move_note(&id, d),
            Action::SetQuery(q) => {
                self.query = q;
                self.searcher.submit(self.index.clone(), &self.query);
                self.selected = 0;
                self.recompute_rows();
            }
            Action::Select(i) => self.selected = i,
            Action::ReorderFocus(d) => {
                if !self.rows.is_empty() {
                    self.selected =
                        (self.selected as i32 + d).clamp(0, self.rows.len() as i32 - 1) as usize;
                }
            }
            Action::SetDirection(d) => {
                self.ed_dir = d;
                if self.view != View::Editor {
                    self.config.default_direction = d;
                    let _ = self.config.save(&self.config_path);
                }
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
            Action::SetBinding(cmd, binding) => {
                self.config.bindings.insert(cmd, binding.clone());
                self.recording = None;
                self.conflicts = self.config.binding_conflicts().into_iter().collect();
                let _ = self.config.save(&self.config_path);
                self.toast(format!("{} → {}", cmd.label(), kit::pretty_binding(&binding)));
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
                    self.hide(ctx);
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
            Action::SetUsePortal(b) => {
                self.config.use_portal = b;
                if !b {
                    hotkey::forget_portal_failure();
                }
                let _ = self.config.save(&self.config_path);
                self.toast(if b {
                    "Portal enabled — restart batnotes to try it. It may be unstable on this desktop."
                } else {
                    "Portal disabled"
                });
            }
            Action::ApplyHotkey => match hotkey::install_desktop_shortcut(&self.hotkey_spec) {
                Ok(()) => self.toast(format!(
                    "Desktop shortcut written · {}",
                    self.hotkey_spec.to_kde_accelerator()
                )),
                Err(e) => self.storage_error = Some(format!("Could not install shortcut: {e}")),
            },
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

    // ---- keyboard --------------------------------------------------------

    fn read_keys(ctx: &Context, recording: bool) -> Keys {
        let mut state = Keys::default();
        ctx.input(|i| {
            state.pressed = !i.events.is_empty();
            state.escape = i.key_pressed(Key::Escape);
            state.enter = i.key_pressed(Key::Enter);
            state.tab = i.key_pressed(Key::Tab);
            state.up = i.key_pressed(Key::ArrowUp);
            state.down = i.key_pressed(Key::ArrowDown);
            for ev in &i.events {
                let egui::Event::Key { key, pressed, repeat, .. } = ev else { continue };
                if !pressed || *repeat {
                    continue;
                }
                let name = key.name().to_string();
                if recording {
                    if matches!(
                        name.as_str(),
                        "Shift" | "Control" | "Alt" | "Super" | "Meta"
                    ) {
                        continue;
                    }
                    state.recorded = Some(combo_from_event(i.modifiers, &name));
                } else if !matches!(name.as_str(), "Shift" | "Control" | "Alt" | "Super" | "Meta") {
                    state.combo = Some(combo_from_event(i.modifiers, &name));
                }
            }
        });
        state
    }

    fn action_for(&mut self, combo: &str) -> Option<Action> {
        let spec = HotkeySpec::parse(combo).ok()?;
        let norm = spec.normalised();
        let mut hit = None;
        for (cmd, binding) in &self.config.bindings {
            if HotkeySpec::parse(binding).map(|s| s.normalised()).ok().as_deref() == Some(norm.as_str()) {
                hit = Some(*cmd);
            }
        }
        let cmd = hit?;
        let library_needs_selection = self.view == View::Library && !self.rows.is_empty();
        let selected_id = || self.rows.get(self.selected).map(|r| r.id.clone());

        Some(match cmd {
            Command::Invoke | Command::ToggleOverlay => {
                // Handled by the global-hotkey channel, not here.
                return None;
            }
            Command::NewNote => Action::NewNote,
            Command::OpenLibrary => Action::OpenLibrary,
            Command::ShowLibrary if self.view == View::Editor => Action::ShowLibrary,
            Command::Settings => Action::OpenSettings,
            Command::Save => Action::Save { close: false },
            Command::SaveClose => Action::Save { close: true },
            Command::Back => Action::Back,
            Command::FocusSearch => {
                self.view = View::Library;
                Action::Back
            }
            Command::ToggleDirection => Action::SetDirection(self.ed_dir.other()),
            Command::TogglePreviewLines => {
                let n = self.config.preview_lines;
                Action::SetPreviewLines(if n >= 8 { 1 } else { n + 1 })
            }
            Command::DeleteNote if library_needs_selection => {
                Action::ConfirmDelete(selected_id()?)
            }
            Command::MoveUp if library_needs_selection => Action::Move(selected_id()?, -1),
            Command::MoveDown if library_needs_selection => Action::Move(selected_id()?, 1),
            _ => return None,
        })
    }

    fn on_focus_loss(&mut self, ctx: &Context) {
        match self.config.focus_loss {
            FocusLoss::Never => {}
            FocusLoss::AutoHide => {
                if !self.dirty() {
                    self.hide(ctx);
                } else {
                    self.prompt = Some(ui::PROMPT_FOCUS.into());
                }
            }
            FocusLoss::Prompt => {
                if self.dirty() {
                    self.prompt = Some(ui::PROMPT_FOCUS.into());
                }
            }
        }
    }

    // ---- paint -----------------------------------------------------------

    fn paint(&mut self, ctx: &Context) {
        let mut out: Vec<Action> = Vec::new();
        let th = self.theme.clone();

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(th.paper).inner_margin(egui::Margin::symmetric(26.0, 20.0)))
            .show(ctx, |ui: &mut egui::Ui| {
                // Header
                ui.horizontal(|ui| {
                    kit::brand(ui, &th, 24.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if kit::icon_button(ui, &th, "✕", "Close").clicked() {
                            out.push(Action::Back);
                        }
                        if kit::icon_button(ui, &th, "⚙", "Settings").clicked() {
                            out.push(Action::OpenSettings);
                        }
                    });
                });
                ui.add_space(10.0);

                // Warnings
                if let Some(w) = self.config_warning.clone() {
                    if kit::banner(ui, &th, kit::Tone::Warn, &w, vec![kit::BannerAction::quiet("dismiss", "Dismiss")]).is_some() {
                        out.push(Action::DismissWarning);
                    }
                    ui.add_space(8.0);
                }
                if let Some(e) = self.storage_error.clone() {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(e).size(11.0).color(th.danger));
                }

                // Focus-loss banner, non-modal per DESIGN 4.1
                if self.prompt.as_deref() == Some(ui::PROMPT_FOCUS) {
                    let actions = vec![
                        kit::BannerAction::primary(ui::PROMPT_SAVE_CLOSE, "Save & close"),
                        kit::BannerAction::quiet(ui::PROMPT_DISCARD, "Discard"),
                        kit::BannerAction::quiet(ui::PROMPT_STAY, "Stay"),
                    ];
                    if let Some(id) = kit::banner(
                        ui,
                        &th,
                        kit::Tone::Warn,
                        "Unsaved changes — the overlay stays open so you don't lose capture.",
                        actions,
                    ) {
                        out.push(Action::PromptAction(id));
                    }
                    ui.add_space(8.0);
                }
                if self.prompt.as_deref() == Some(ui::PROMPT_CANCEL) {
                    let actions = vec![
                        kit::BannerAction::primary(ui::PROMPT_SAVE_CLOSE, "Save instead"),
                        kit::BannerAction::quiet(ui::PROMPT_DISCARD, "Discard"),
                        kit::BannerAction::quiet(ui::PROMPT_STAY, "Keep editing"),
                    ];
                    if let Some(id) = kit::banner(
                        ui,
                        &th,
                        kit::Tone::Danger,
                        "Discard these changes?",
                        actions,
                    ) {
                        out.push(Action::PromptAction(id));
                    }
                    ui.add_space(8.0);
                }

                ui.add_space(2.0);

                // Body
                match self.view {
                    View::Chooser => {
                        let idx = self.index.clone();
                        let c = ui::chooser::Ctx {
                            th: &th,
                            cfg: &self.config,
                            index: &idx,
                            conflicts: &self.conflicts,
                            note_count: self.order.len(),
                            view: View::Chooser,
                        };
                        ui::chooser::show(ui, &c, &mut out);
                    }
                    View::Editor => {
                        let mut dir = self.ed_dir;
                        let mut text = std::mem::take(&mut self.ed_text);
                        let mut c = ui::editor::Ctx {
                            th: &th,
                            cfg: &self.config,
                            text: &mut text,
                            original: &self.ed_original.clone(),
                            direction: &mut dir,
                            original_direction: self.ed_original_dir,
                            is_new: self.ed_is_new,
                            id: self.ed_id.clone(),
                        };
                        ui::editor::show(ui, &mut c, &mut out);
                        self.ed_text = text;
                        self.ed_dir = dir;
                    }
                    View::Library => {
                        let rows = self.rows.clone();
                        let idx = self.index.clone();
                        let mut query = self.query.clone();
                        let mut c = ui::library::Ctx {
                            th: &th,
                            cfg: &self.config,
                            index: &idx,
                            query: &mut query,
                            rows: &rows,
                            selected: self.selected,
                            hover: self.hover,
                            dragging: self.dragging,
                            drag_over: self.drag_over,
                            rebuilt: self.open_ms.is_some(),
                        };
                        ui::library::show(ui, &mut c, &mut out);
                        if query != self.query {
                            self.query = query;
                        }
                    }
                    View::Settings => {
                        let mut tab = self.settings_tab;
                        let mut c = ui::settings::Ctx {
                            th: &th,
                            cfg: &self.config,
                            tab: &mut tab,
                            recording: &mut self.recording,
                            conflicts: &self.conflicts,
                            hotkey_backend: self.hotkey.status.backend.label(),
                            hotkey_detail: &self.hotkey.status.detail,
                            hotkey_ok: self.hotkey.status.backend != hotkey::Backend::None,
                            note_count: self.order.len(),
                            total_bytes: self.total_bytes,
                            storage_error: self.storage_error.clone(),
                            serif_loaded: self.fonts.serif_loaded,
                            hebrew_loaded: self.fonts.hebrew_loaded,
                        };
                        ui::settings::show(ui, &mut c, &mut out);
                        self.settings_tab = tab;
                    }
                }

                // Footer hints, always live
                ui.add_space(6.0);
                let hints: Vec<(String, String)> = self
                    .hints()
                    .into_iter()
                    .map(|(c, l)| (self.config.binding(c), String::from(l)))
                    .collect();
                let refs: Vec<(&str, &str)> =
                    hints.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
                kit::hint_bar(ui, &th, &refs);
            });

        // Delete confirmation over a scrim
        if let Some(id) = self.confirm_delete.clone() {
            let title = self
                .index
                .get(&id)
                .map(|d| d.title.as_str())
                .unwrap_or("this note")
                .to_string();
            let body = format!("“{title}” will be removed from disk. This cannot be undone — it is a real file, not a trash bin.");
            let mut hit = false;
            let mut keep = false;
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut actions: Vec<kit::BannerAction> = Vec::new();
                let _ = &mut actions;
                hit = kit::confirm_card(
                    ui,
                    &th,
                    360.0,
                    "Delete this note?",
                    &body,
                    "Delete file",
                    "Keep it",
                    true,
                    |ui| {
                        ui.allocate_space(egui::vec2(0.0, 0.0));
                    },
                );
                keep = false;
            });
            if hit {
                out.push(Action::PromptAction(ui::CONFIRM_DELETE.into()));
            }
            let _ = keep;
            // Keep the id alive across the borrow.
            self.confirm_delete = Some(id);
        }

        // Toast
        if let Some((msg, _)) = self.toast.clone() {
            egui::TopBottomPanel::bottom("toast").show(ctx, |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    kit::toast(ui, &th, &msg);
                });
            });
        }

        for a in out {
            self.dispatch(a, ctx);
        }
    }

    fn hints(&self) -> Vec<(Command, &'static str)> {
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
        v.push((Command::Back, if self.view == View::Chooser { "close" } else { "back" }));
        v
    }
}

fn combo_from_event(mods: egui::Modifiers, key: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    if mods.ctrl {
        parts.push("Ctrl".into());
    }
    if mods.alt {
        parts.push("Alt".into());
    }
    if mods.shift {
        parts.push("Shift".into());
    }
    if mods.command {
        parts.push("Super".into());
    }
    let named = match key {
        " " => "Space",
        "Escape" => "Escape",
        "Enter" => "Enter",
        "ArrowUp" => "Up",
        "ArrowDown" => "Down",
        "ArrowLeft" => "Left",
        "ArrowRight" => "Right",
        "," => "Comma",
        "/" => "Slash",
        "[" => "BracketLeft",
        "]" => "BracketRight",
        k if k.chars().count() == 1 => {
            let mut s = k.to_uppercase();
            if let Some(c) = s.chars().next() {
                s = c.to_string();
            }
            let s = s;
            parts.push(s);
            return parts.join("+");
        }
        _ => return String::new(),
    };
    parts.push(named.to_string());
    parts.join("+")
}

impl eframe::App for App {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        if hotkey::take_show_request() {
            if self.visible {
                self.hide(ctx);
            } else {
                self.show(ctx);
            }
        }

        while self.hotkey.rx.try_recv().is_ok() {
            if self.visible {
                self.hide(ctx);
            } else {
                self.show(ctx);
            }
        }

        if let Some(t) = self.opened_at {
            if t.elapsed() > Duration::from_millis(220) && self.open_ms.is_none() {
                self.open_ms = Some(t.elapsed().as_secs_f32() * 1000.0);
            }
        }
        if let Some(hits) = self.searcher.poll() {
            self.apply_hits(hits);
        }
        if let Some((_, at)) = self.toast.clone() {
            if at.elapsed() > Duration::from_millis(2600) {
                self.toast = None;
            }
        }

        let keys = Self::read_keys(ctx, self.recording.is_some());

        if let Some(cmd) = self.recording {
            if keys.escape {
                self.recording = None;
            } else if let Some(combo) = keys.recorded {
                if HotkeySpec::parse(&combo).is_ok() {
                    self.dispatch(Action::SetBinding(cmd, combo), ctx);
                } else {
                    self.toast("Not a usable combination");
                }
            }
        } else if self.visible {
            if let Some(combo) = keys.combo.clone() {
                if let Some(a) = self.action_for(&combo) {
                    self.dispatch(a, ctx);
                }
            }
            if keys.up {
                self.dispatch(Action::ReorderFocus(-1), ctx);
            }
            if keys.down {
                self.dispatch(Action::ReorderFocus(1), ctx);
            }
            if keys.enter {
                if let Some(id) = self.rows.get(self.selected).map(|r| r.id.clone()) {
                    self.open_note(&id);
                }
            }

            // Focus loss only counts after the window has been focused once,
            // otherwise the first frame after showing would close it again.
            let focused = ctx.input(|i| i.focused);
            if focused {
                self.was_focused = true;
            } else if self.was_focused && self.visible {
                self.was_focused = false;
                self.on_focus_loss(ctx);
            }
        }

        // A panic while painting must not take the daemon down: the whole point
        // of this app is that it is always there for the next capture. Catch
        // it, record it, and fall back to a usable view.
        let painted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.paint(ctx)));
        if painted.is_err() {
            self.storage_error = Some(
                "The last screen could not be drawn — see crash.log. The app is still running.".into(),
            );
            self.view = View::Chooser;
            self.ed_id = None;
            self.ed_text.clear();
            self.confirm_delete = None;
        }
        ctx.request_repaint_after(Duration::from_millis(80));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = std::fs::remove_file(hotkey::socket_path());
    }
}