use batnotes::{Config, Direction, OrderManifest, Storage, search_notes};
use eframe::egui;
use global_hotkey::{GlobalHotKeyManager, hotkey::{HotKey, Modifiers, Code}};
use global_hotkey::GlobalHotKeyEvent;
use std::path::PathBuf;

#[derive(PartialEq)]
enum View { Chooser, Editor, Library, Settings }

struct App {
    hotkey_manager: Option<GlobalHotKeyManager>,
    hotkey_id: Option<u32>,
    overlay_visible: bool,
    config: Config,
    config_path: PathBuf,
    storage: Storage,
    view: View,
    editor_text: String,
    editor_path: Option<PathBuf>,
    editor_direction: Direction,
    notes: Vec<PathBuf>,
    search_query: String,
    preview_lines: usize,
    selected: Option<usize>,
    error: Option<String>,
}

impl App {
    fn new() -> Self {
        let config_path = dirs::home_dir().unwrap().join(".batnotes/config.toml");
        let config = Config::load(&config_path).unwrap_or_default();
        let _ = config.validate();
        let storage = Storage::new(&config);
        let preview_lines = config.preview_lines;
        let (hotkey_manager, hotkey_id) = match GlobalHotKeyManager::new() {
            Ok(mgr) => {
                let hotkey = HotKey::new(Some(Modifiers::ALT), Code::Space);
                let id = hotkey.id();
                let _ = mgr.register(hotkey);
                (Some(mgr), Some(id))
            }
            Err(_) => (None, None),
        };
        Self {
            hotkey_manager,
            hotkey_id,
            overlay_visible: false,
            config,
            config_path,
            storage,
            view: View::Chooser,
            editor_text: String::new(),
            editor_path: None,
            editor_direction: Direction::Rtl,
            notes: vec![],
            search_query: String::new(),
            preview_lines,
            selected: None,
            error: None,
        }
    }

    fn refresh_notes(&mut self) {
        if let Ok(paths) = self.storage.list_notes() {
            if let Ok(ordered) = self.storage.ordered_notes(paths) {
                self.notes = ordered;
            }
        }
    }
    fn move_note(&mut self, path: &std::path::Path, delta: isize) {
        let Some(pos) = self.notes.iter().position(|p| p == path) else { return; };
        let new_pos = (pos as isize + delta).clamp(0, self.notes.len().saturating_sub(1) as isize) as usize;
        if new_pos == pos { return; }
        let p = self.notes.remove(pos);
        self.notes.insert(new_pos, p);
        let ids: Vec<String> = self.notes.iter().filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string())).collect();
        let _ = self.storage.save_manifest(&OrderManifest { version: 1, order: ids });
    }

    fn save_editor(&mut self) {
        let id = self.editor_path
            .as_ref()
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
            .unwrap_or_else(|| batnotes::new_id());
        match self.storage.save_note(&id, &self.editor_text) {
            Ok(path) => {
                self.editor_path = Some(path);
                self.editor_text.clear();
                self.view = View::Chooser;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let (Some(id), Ok(event)) = (self.hotkey_id, GlobalHotKeyEvent::receiver().try_recv()) {
            if event.id == id { ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true)); self.overlay_visible = true; }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.view = View::Chooser;
            self.editor_text.clear();
        }
        egui::TopBottomPanel::top("title").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("batnotes");
                if self.error.is_some() {
                    ui.colored_label(egui::Color32::RED, self.error.as_ref().unwrap());
                }
            });
        });

        match self.view {
            View::Chooser => {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.vertical_centered(|ui| {
                            if ui.button("A — New note (Alt-A)").clicked() {
                                self.editor_path = None;
                                self.editor_text.clear();
                                self.editor_direction = self.config.default_direction;
                                self.view = View::Editor;
                            }
                            if ui.button("B — My notes (Alt-B)").clicked() {
                                self.refresh_notes();
                                self.view = View::Library;
                            }
                            if ui.button("Settings").clicked() {
                                self.view = View::Settings;
                            }
                        });
                    });
                });
            }
            View::Editor => {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Direction:");
                        egui::ComboBox::from_id_salt("dir")
                            .selected_text(format!("{:?}", self.editor_direction))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.editor_direction, batnotes::Direction::Rtl, "RTL");
                                ui.selectable_value(&mut self.editor_direction, batnotes::Direction::Ltr, "LTR");
                            });
                    });
                    ui.text_edit_multiline(&mut self.editor_text);
                    ui.horizontal(|ui| {
                        if ui.button("Save (Ctrl-S)").clicked() { self.save_editor(); }
                        if ui.button("Cancel (Esc)").clicked() {
                            self.editor_text.clear();
                            self.view = View::Chooser;
                        }
                    });
                });
            }
            View::Library => {
                let filtered: Vec<PathBuf> = search_notes(&self.notes, &self.search_query).into_iter().cloned().collect();
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.text_edit_singleline(&mut self.search_query);
                    ui.separator();
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for path in filtered {
                            let preview = batnotes::Storage::preview_lines(&path, self.preview_lines).unwrap_or_default();
                            ui.group(|ui| {
                                ui.label(path.file_name().unwrap().to_string_lossy());
                                ui.small(preview);
                                if ui.button("Open").clicked() {
                                    let text = self.storage.read_note(&path).unwrap_or_default();
                                    self.editor_text = text;
                                    self.editor_path = Some(path.clone());
                                    self.editor_direction = self.config.default_direction;
                                    self.view = View::Editor;
                                }
                                if ui.button("Delete").clicked() {
                                    let _ = self.storage.delete_note(&path);
                                    self.refresh_notes();
                                }
                                if ui.button("↑").clicked() { self.move_note(&path, -1); }
                                if ui.button("↓").clicked() { self.move_note(&path, 1); }
                            });
                        }
                    });
                    if ui.button("Back").clicked() { self.view = View::Chooser; }
                });
            }
            View::Settings => {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("Settings");
                    ui.add(egui::Slider::new(&mut self.config.preview_lines, 1..=10).text("preview lines"));
                    if ui.button("Save settings").clicked() {
                        let _ = self.config.save(&self.config_path);
                        self.preview_lines = self.config.preview_lines;
                        self.view = View::Chooser;
                    }
                    if ui.button("Back").clicked() { self.view = View::Chooser; }
                });
            }
        }
    }
}

fn main() {
    let options = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_visible(false), ..Default::default() };
    let _ = eframe::run_native("batnotes", options, Box::new(|_cc| Ok(Box::new(App::new()))));
}
