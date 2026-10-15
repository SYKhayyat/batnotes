//! GTK4 front end.
//!
//! GTK is used specifically for its text stack. egui maps codepoints through a
//! font's `cm��`: it does no shaping and no bidi, which renders Hebrew with
//! isolated letterforms and puts embedded numbers in the wrong place. Pango
//! does both correctly, so every piece of note text goes through `GtkTextView`
//! rather than a hand-drawn label.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::glib;

use crate::app::App;
use crate::config::{self, Command, FocusLoss};
use crate::model::Direction;
use crate::ui::{self, Action, Row, Tab, View};

/// Shared between the widget tree and the callbacks, which GTK invokes from
/// arbitrary signal handlers.
pub struct Ui {
    pub app: Rc<RefCell<App>>,
    window: gtk4::Window,
    /// Rebuilt on every state change; simplest correct thing for a tree this
    /// small, and it keeps state in one place instead of across widgets.
    root: gtk4::Box,
    toast: gtk4::Label,
    last_toast: RefCell<Option<String>>,
    /// Fingerprint of the state the current tree was built from, so the tree is
    /// only rebuilt when something visible has changed.
    rendered: RefCell<String>,
}

/// `window` is moved in and kept: GTK must not collect the only reference to
/// the overlay's window while the daemon is still running.
pub fn build(app: App, window: gtk4::Window) -> Rc<Ui> {
    let rc = Rc::new(Ui {
        app: Rc::new(RefCell::new(app)),
        window,
        root: gtk4::Box::new(gtk4::Orientation::Vertical, 0),
        toast: gtk4::Label::new(None),
        last_toast: RefCell::new(None),
        rendered: RefCell::new(String::new()),
    });
    let rc2 = rc.clone();
    rc2.init();
    rc
}

impl Ui {
    fn init(self: &Rc<Self>) {
        let w = &self.window;
        w.set_title(Some("batnotes"));
        w.set_decorated(false);
        // Keep the overlay above ordinary windows without becoming a panel
        // that steals input from whatever the user is actually doing.
        w.set_modal(false);
        w.set_default_size(620, 470);
        w.set_resizable(true);
        w.set_visible(false);

        let css = self.clone().css();
        let provider = gtk4::CssProvider::new();
        provider.load_from_data(&css);
        if let Some(display) = gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        self.root.set_margin_top(14);
        self.root.set_margin_bottom(14);
        self.root.set_margin_start(18);
        self.root.set_margin_end(18);
        self.root.set_spacing(10);
        w.set_child(Some(&self.root));

        self.toast.add_css_class("toast");
        self.toast.set_visible(false);

        self.clone().wire_keys();
        self.clone().wire_focus();
        self.clone().render_all();
    }

    // ---- window control -------------------------------------------------

    pub fn show(&self) {
        self.window.present();
    }

    pub fn hide(&self) {
        self.window.set_visible(false);
    }

    pub fn is_visible(&self) -> bool {
        self.window.is_visible()
    }

    /// Called on a timer by the bootstrap so the daemon keeps ticking while
    /// hidden, which is what lets the hotkey show it again.
    pub fn tick(self: &Rc<Self>) {
        let mut app = self.app.borrow_mut();
        app.pump();
        let want_visible = app.visible;
        let want_toast = app.toast.as_ref().map(|(m, _)| m.clone());
        drop(app);

        if want_visible && !self.window.is_visible() {
            self.show();
        } else if !want_visible && self.window.is_visible() {
            self.hide();
        }

        if want_toast != *self.last_toast.borrow() {
            *self.last_toast.borrow_mut() = want_toast.clone();
            if let Some(m) = want_toast {
                self.toast.set_text(&m);
                self.toast.set_visible(true);
            } else {
                self.toast.set_visible(false);
            }
        }

        // Only rebuild when something the UI shows actually changed.
        if self.signature() != *self.rendered.borrow() {
            self.clone().render_all();
        }
    }

    /// Cheap fingerprint of everything the tree depends on.
    fn signature(&self) -> String {
        let a = self.app.borrow();
        format!(
            "{:?}|{}|{}|{}|{}|{}|{}|{:?}|{:?}|{}|{:?}",
            a.view,
            a.ed_id.clone().unwrap_or_default(),
            a.ed_text.len(),
            a.ed_dir == Direction::Rtl,
            a.dirty(),
            a.query,
            a.rows.len(),
            a.prompt,
            a.confirm_delete,
            a.conflicts.len(),
            a.recording,
        )
    }

    fn rebuild(self: &Rc<Self>) {
        while let Some(c) = self.root.first_child() {
            self.root.remove(&c);
        }
        let app = self.app.borrow();
        match app.view {
            View::Chooser => self.build_chooser(&app),
            View::Editor => self.build_editor(&app),
            View::Library => self.build_library(&app),
            View::Settings => self.build_settings(&app),
        }
        drop(app);
        self.root.append(&self.clone().hints_bar());
        if self.last_toast.borrow().is_some() {
            self.root.append(&self.toast.clone());
        }
    }

    fn render_all(self: Rc<Self>) {
        self.clone().rebuild();
        *self.rendered.borrow_mut() = self.signature();
    }

    // ---- actions ---------------------------------------------------------

    fn dispatch(self: Rc<Self>, a: Action) {
        self.app.borrow_mut().dispatch(a);
        self.clone().render_all();
    }

    // ---- chrome ----------------------------------------------------------

    fn header(self: Rc<Self>, subtitle: String) -> gtk4::Box {
        let bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let left = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        left.append(&gtk4::Label::builder().label("batnotes").css_classes(["brand"]).build());
        bar.append(&left);

        let sub = gtk4::Label::builder()
            .label(&subtitle)
            .css_classes(["subtitle"])
            .hexpand(true)
            .xalign(0.0)
            .build();
        bar.append(&sub);

        let settings = gtk4::Button::with_label("⚙");
        settings.add_css_class("flat");
        settings.set_tooltip_text(Some("Settings"));
        settings.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::OpenSettings)
        });
        bar.append(&settings);

        let close = gtk4::Button::with_label("✕");
        close.add_css_class("flat");
        close.set_tooltip_text(Some("Close"));
        close.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::Close)
        });
        bar.append(&close);
        bar
    }

    fn hints_bar(self: Rc<Self>) -> gtk4::Box {
        let bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 14);
        bar.add_css_class("hintbar");
        let app = self.app.borrow();
        for (cmd, label) in app.hints() {
            let b = keycap(&app.config.binding(cmd));
            bar.append(&b);
            let l = gtk4::Label::new(Some(label));
            l.add_css_class("hintlabel");
            bar.append(&l);
        }
        bar
    }

    // ---- chooser ---------------------------------------------------------

    fn build_chooser(self: &Rc<Self>, app: &App) {
        let col = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        col.append(&self.clone().header(if app.order.is_empty() {
            "new note or browse".to_string()
        } else {
            format!("{} notes", app.order.len())
        }));
        col.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

        col.append(&self.clone().choice_row(app, "＋", "New note", Command::NewNote, Action::NewNote, None));
        col.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
        let count = if app.order.is_empty() {
            None
        } else {
            Some(app.order.len().to_string())
        };
        col.append(&self.clone().choice_row(
            app,
            "≡",
            "My notes",
            Command::OpenLibrary,
            Action::OpenLibrary,
            count,
        ));
        col.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

        // Resume: the most recent note, one click away.
        if let Some(id) = app.index.ids().first().cloned() {
            if let Some(doc) = app.index.get(&id) {
                let b = gtk4::Button::new();
                b.add_css_class("resume");
                let inner = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                let t = gtk4::Label::new(Some(doc.title.as_str()));
                t.add_css_class("resume-title");
                t.set_xalign(0.0);
                t.set_hexpand(true);
                t.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                inner.append(&t);
                let hint = gtk4::Label::new(Some("resume"));
                hint.add_css_class("subtitle");
                inner.append(&hint);
                b.set_child(Some(&inner));
                b.connect_clicked({
                    let ui = self.clone();
                    move |_| ui.clone().dispatch(Action::ResumeNote(id.clone()))
                });
                col.append(&b);
            }
        }

        if let Some(w) = &app.config_warning {
            col.append(&self.clone().banner(w, gtk4::Orientation::Horizontal));
        }
        if let Some(p) = &app.prompt {
            col.append(&self.clone().prompt_bar(p.clone()));
        }
        if !app.conflicts.is_empty() {
            let l = gtk4::Label::new(Some(&format!(
                "{} bindings conflict — check Settings",
                app.conflicts.len()
            )));
            l.add_css_class("warn");
            col.append(&l);
        }
        self.root.append(&col);
    }

    fn choice_row(self: Rc<Self>,
        app: &App,
        glyph: &str,
        title: &str,
        cmd: Command,
        action: Action,
        count: Option<String>,
    ) -> gtk4::Button {
        let b = gtk4::Button::new();
        b.add_css_class("choice");
        let inner = gtk4::Box::new(gtk4::Orientation::Horizontal, 14);

        let icon = gtk4::Label::new(Some(glyph));
        icon.add_css_class("choice-icon");
        inner.append(&icon);

        let t = gtk4::Label::new(Some(title));
        t.add_css_class("choice-title");
        t.set_xalign(0.0);
        t.set_hexpand(true);
        inner.append(&t);

        if let Some(c) = count {
            let n = gtk4::Label::new(Some(&c));
            n.add_css_class("choice-count");
            inner.append(&n);
        }
        inner.append(&keycap(&app.config.binding(cmd)));

        b.set_child(Some(&inner));
        b.connect_clicked(move |_| self.clone().dispatch(action.clone()));
        b
    }

    fn banner(self: Rc<Self>, text: &str, dir: gtk4::Orientation) -> gtk4::Box {
        let bar = gtk4::Box::new(dir, 8);
        bar.add_css_class("banner");
        let l = gtk4::Label::new(Some(text));
        l.set_wrap(true);
        l.set_xalign(0.0);
        l.set_hexpand(true);
        bar.append(&l);
        bar
    }

    fn prompt_bar(self: Rc<Self>, which: String) -> gtk4::Box {
        let (text, save_label, tone) = match which.as_str() {
            ui::PROMPT_FOCUS => (
                "Unsaved changes — the overlay stays open so you don't lose capture.",
                "Save & close",
                "banner-warn",
            ),
            _ => ("Discard these changes?", "Save instead", "banner-danger"),
        };
        let bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        bar.add_css_class(tone);
        let l = gtk4::Label::new(Some(text));
        l.set_wrap(true);
        l.set_xalign(0.0);
        l.set_hexpand(true);
        bar.append(&l);

        let save = gtk4::Button::with_label(save_label);
        save.add_css_class("primary");
        save.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::PromptAction(ui::PROMPT_SAVE_CLOSE.into()))
        });
        bar.append(&save);

        let discard = gtk4::Button::with_label("Discard");
        discard.add_css_class("ghost");
        discard.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::PromptAction(ui::PROMPT_DISCARD.into()))
        });
        bar.append(&discard);

        let stay = gtk4::Button::with_label("Stay");
        stay.add_css_class("ghost");
        stay.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::PromptAction(ui::PROMPT_STAY.into()))
        });
        bar.append(&stay);
        bar
    }

    // ---- editor ------------------------------------------------------------

    fn build_editor(self: &Rc<Self>, app: &App) {
        let col = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        col.append(&self.clone().header(if app.ed_is_new {
            "new note".to_string()
        } else {
            format!("{}.txt", app.ed_id.clone().unwrap_or_default())
        }));

        let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);

        // Direction: a real Pango attribute, not a mirror.
        let seg = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        seg.add_css_class("segmented");
        for (d, label) in [(Direction::Ltr, "LTR"), (Direction::Rtl, "RTL")] {
            let b = gtk4::ToggleButton::with_label(label);
            b.set_active(app.ed_dir == d);
            if app.ed_dir == d {
                b.add_css_class("on");
            }
            b.connect_clicked({
                let ui = self.clone();
                move |_| ui.clone().dispatch(Action::SetDirection(d))
            });
            seg.append(&b);
        }
        toolbar.append(&seg);

        let notes = gtk4::Button::with_label("My notes");
        notes.add_css_class("ghost");
        notes.set_tooltip_text(Some("Browse without leaving this draft"));
        notes.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::ShowLibrary)
        });
        toolbar.append(&notes);

        let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        toolbar.append(&spacer);

        let status = gtk4::Label::new(Some(if app.dirty() {
            "Unsaved changes"
        } else if app.ed_is_new {
            "Ready to write"
        } else {
            "Saved"
        }));
        status.add_css_class(if app.dirty() { "warn" } else { "ok" });
        toolbar.append(&status);
        col.append(&toolbar);

        // The one widget that must do the text work.
        let view = gtk4::TextView::new();
        view.set_wrap_mode(gtk4::WrapMode::WordChar);
        view.set_hexpand(true);
        view.set_vexpand(true);
        view.add_css_class("note");
        view.set_top_margin(12);
        view.set_bottom_margin(12);
        view.set_left_margin(14);
        view.set_right_margin(14);

        let buffer = view.buffer();
        buffer.set_text(&app.ed_text);
        apply_direction(&buffer, app.ed_dir);

        buffer.connect_changed({
            let ui = self.clone();
            move |b| {
                let text = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
                let mut app = ui.app.borrow_mut();
                if app.ed_text != text {
                    app.ed_text = text;
                    drop(app);
                    ui.clone().render_all();
                }
            }
        });

        let scroller = gtk4::ScrolledWindow::builder()
            .child(&view)
            .hexpand(true)
            .vexpand(true)
            .build();
        scroller.add_css_class("note-surface");
        col.append(&scroller);

        let footer = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let stats = gtk4::Label::new(Some(&self.stats(app)));
        stats.add_css_class("subtitle");
        stats.set_xalign(0.0);
        stats.set_hexpand(true);
        footer.append(&stats);

        if app.ed_id.is_some() {
            let del = gtk4::Button::with_label("Delete");
            del.add_css_class("ghost");
            let id = app.ed_id.clone().unwrap();
            del.connect_clicked({
                let ui = self.clone();
                move |_| ui.clone().dispatch(Action::ConfirmDelete(id.clone()))
            });
            footer.append(&del);
        }
        let cancel = gtk4::Button::with_label("Cancel");
        cancel.add_css_class("ghost");
        cancel.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::Cancel)
        });
        footer.append(&cancel);

        let save = gtk4::Button::with_label(&format!(
            "Save  {}",
            ui::pretty_key(&app.config.binding(Command::SaveClose))
        ));
        save.add_css_class("primary");
        save.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::Save { close: true })
        });
        footer.append(&save);
        col.append(&footer);

        if app.prompt.is_some() {
            col.append(&self.clone().prompt_bar(app.prompt.clone().unwrap()));
        }
        self.root.append(&col);

        // Put the caret in the text immediately; otherwise the first keystroke
        // goes nowhere, which is exactly the bug this rewrite had to fix.
        view.grab_focus();
    }

    fn stats(&self, app: &App) -> String {
        format!(
            "{} chars · {} words · {} lines",
            app.ed_text.chars().count(),
            app.ed_text.split_whitespace().count(),
            app.ed_text.lines().count().max(1)
        )
    }

    // ---- library -------------------------------------------------------------

    fn build_library(self: &Rc<Self>, app: &App) {
        let col = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        col.append(&self.clone().header(format!("{} notes", app.order.len())));

        let search = gtk4::SearchEntry::new();
        search.set_text(&app.query);
        search.set_placeholder_text(Some("Search titles, filenames & content…"));
        search.connect_search_changed({
            let ui = self.clone();
            move |e| ui.clone().dispatch(Action::SetQuery(e.text().to_string()))
        });
        col.append(&search);

        let list = gtk4::ListBox::new();
        list.set_selection_mode(gtk4::SelectionMode::Single);
        list.add_css_class("library");
        for (i, row) in app.rows.iter().enumerate() {
            let Some(doc) = app.index.get(&row.id) else { continue };
            list.append(&self.clone().note_row(app, i, row, doc));
        }
        let scroller = gtk4::ScrolledWindow::builder()
            .child(&list)
            .vexpand(true)
            .hexpand(true)
            .build();
        scroller.add_css_class("note-surface");
        col.append(&scroller);

        if app.rows.is_empty() {
            let empty = gtk4::Label::new(Some(if app.query.is_empty() {
                "No notes yet"
            } else {
                "Nothing matches — search is Unicode-aware, Hebrew works too"
            }));
            empty.add_css_class("subtitle");
            col.append(&empty);
        }

        let footer = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let back = gtk4::Button::with_label("Back");
        back.add_css_class("ghost");
        back.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::Back)
        });
        footer.append(&back);

        let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        footer.append(&spacer);

        let up = gtk4::Button::with_label("↑ move");
        up.add_css_class("ghost");
        up.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::ReorderFocus(-1))
        });
        footer.append(&up);
        let down = gtk4::Button::with_label("↓ move");
        down.add_css_class("ghost");
        down.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::ReorderFocus(1))
        });
        footer.append(&down);
        col.append(&footer);

        self.root.append(&col);
    }

    fn note_row(self: Rc<Self>, app: &App, i: usize, row: &Row, doc: &crate::search::Doc) -> gtk4::ListBoxRow {
        let r = gtk4::ListBoxRow::new();
        let inner = gtk4::Box::new(gtk4::Orientation::Vertical, 2);

        let head = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        // The title carries the note's own direction, so Hebrew titles read
        // right-to-left inside an otherwise left-to-right list.
        let title = gtk4::Label::new(Some(doc.title.as_str()));
        title.add_css_class("note-title");
        title.set_xalign(0.0);
        title.set_hexpand(true);
        title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        set_widget_direction(&title, doc.direction);
        head.append(&title);

        let badge = gtk4::Label::new(Some(if doc.direction == Direction::Rtl {
            "HE · RTL"
        } else {
            "EN · LTR"
        }));
        badge.add_css_class(if doc.direction == Direction::Rtl {
            "badge-rtl"
        } else {
            "badge-ltr"
        });
        head.append(&badge);
        inner.append(&head);

        let preview = match row.at {
            Some(at) if !app.query.is_empty() => app.index.snippet(&row.id, at, 28, 42),
            _ => app
                .index
                .preview(&row.id, app.config.preview_lines)
                .replace('\n', "  "),
        };
        let p = gtk4::Label::new(Some(&preview));
        p.add_css_class("note-preview");
        p.set_xalign(0.0);
        p.set_wrap(true);
        p.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        p.set_max_width_chars(90);
        set_widget_direction(&p, doc.direction);
        inner.append(&p);

        let meta = gtk4::Label::new(Some(&format!(
            "{} · {} chars",
            doc.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            doc.content.chars().count()
        )));
        meta.add_css_class("subtitle");
        meta.set_xalign(0.0);
        inner.append(&meta);

        r.set_child(Some(&inner));
        r.add_css_class("note-row");
        if i == app.selected {
            r.add_css_class("selected");
        }
        let id = row.id.clone();
        r.set_activatable(true);
        r.connect_activate({
            let ui = self.clone();
            let id = id.clone();
            move |_| ui.clone().dispatch(Action::OpenNote(id.clone()))
        });
        r
    }

    // ---- settings --------------------------------------------------------------

    fn build_settings(self: &Rc<Self>, app: &App) {
        let col = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        col.append(&self.clone().header("settings".to_string()));

        let tabs = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        for t in Tab::ALL {
            let b = gtk4::ToggleButton::with_label(t.label());
            if t == app.settings_tab {
                b.set_active(true);
                b.add_css_class("on");
            }
            b.connect_clicked({
                let ui = self.clone();
                move |_| {
                    ui.app.borrow_mut().settings_tab = t;
                    ui.clone().render_all();
                }
            });
            tabs.append(&b);
        }
        col.append(&tabs);

        let page: gtk4::Widget = match app.settings_tab {
            Tab::General => self.clone().general_page(app).upcast(),
            Tab::Keys => self.clone().keys_page(app).upcast(),
            Tab::Storage => self.clone().storage_page(app).upcast(),
        };
        let scroller = gtk4::ScrolledWindow::builder()
            .child(&page)
            .vexpand(true)
            .hexpand(true)
            .build();
        col.append(&scroller);
        self.root.append(&col);
    }

    fn general_page(self: Rc<Self>, app: &App) -> gtk4::Box {
        let b = gtk4::Box::new(gtk4::Orientation::Vertical, 10);

        b.append(&setting_row(
            "Preview lines",
            &format!("{} lines of each note in the library", app.config.preview_lines),
            {
                let ui = self.clone();
                let uim = ui.clone();
                let minus = gtk4::Button::with_label("−");
                minus.add_css_class("ghost");
                minus.connect_clicked(move |_| {
                    let ui = uim.clone();
                    let cur = ui.app.borrow().config.preview_lines;
                    ui.clone().dispatch(Action::SetPreviewLines(cur.saturating_sub(1).max(1)));
                });
                let plus = gtk4::Button::with_label("+");
                plus.add_css_class("ghost");
                plus.connect_clicked(move |_| {
                    let cur = ui.app.borrow().config.preview_lines;
                    ui.clone().dispatch(Action::SetPreviewLines((cur + 1).min(8)));
                });
                let box_ = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
                box_.append(&minus);
                box_.append(&plus);
                box_.upcast::<gtk4::Widget>()
            },
        ));

        b.append(&setting_row(
            "Default direction",
            "Starting direction for new notes",
            {
                let ui = self.clone();
                let stack = gtk4::Stack::new();
                for (d, label) in [(Direction::Ltr, "LTR"), (Direction::Rtl, "RTL")] {
                    let t = gtk4::ToggleButton::with_label(label);
                    t.set_active(app.config.default_direction == d);
                    if app.config.default_direction == d {
                        t.add_css_class("on");
                    }
                    let uic = ui.clone();
                    t.connect_clicked(move |_| uic.clone().dispatch(Action::SetDefaultDirection(d)));
                    stack.add_named(&t, Some(label));
                }
                stack.set_visible_child_name(if app.config.default_direction == Direction::Rtl {
                    "RTL"
                } else {
                    "LTR"
                });
                stack.upcast::<gtk4::Widget>()
            },
        ));

        b.append(&setting_row(
            "When focus is lost",
            "Unsaved writing is never discarded silently",
            {
                let ui = self.clone();
                let stack = gtk4::Stack::new();
                for opt in [FocusLoss::AutoHide, FocusLoss::Prompt, FocusLoss::Never] {
                    let t = gtk4::ToggleButton::with_label(opt.label());
                    t.set_active(app.config.focus_loss == opt);
                    if app.config.focus_loss == opt {
                        t.add_css_class("on");
                    }
                    let uic = ui.clone();
                    t.connect_clicked(move |_| uic.clone().dispatch(Action::SetFocusLoss(opt)));
                    stack.add_named(&t, Some(opt.label()));
                }
                stack.set_visible_child_name(app.config.focus_loss.label());
                stack.upcast::<gtk4::Widget>()
            },
        ));

        b.append(&setting_row(
            "fsync before atomic rename",
            "Slower saves, no data loss on power cut",
            {
                let ui = self.clone();
                let s = gtk4::Switch::new();
                s.set_active(app.config.fsync);
                let uic = ui.clone();
                s.connect_state_set(move |_, v| {
                    uic.clone().dispatch(Action::SetFsync(v));
                    glib::Propagation::Proceed
                });
                s.upcast::<gtk4::Widget>()
            },
        ));

        b.append(&setting_row(
            "Use the XDG GlobalShortcuts portal",
            "Off by default — it crashes the portal daemon on some desktops",
            {
                let ui = self.clone();
                let s = gtk4::Switch::new();
                s.set_active(app.config.use_portal);
                let uic = ui.clone();
                s.connect_state_set(move |_, v| {
                    uic.clone().dispatch(Action::SetUsePortal(v));
                    glib::Propagation::Proceed
                });
                s.upcast::<gtk4::Widget>()
            },
        ));
        b
    }

    fn keys_page(self: Rc<Self>, app: &App) -> gtk4::Box {
        let b = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        let list = gtk4::ListBox::new();
        list.add_css_class("keys");
        for cmd in config::ALL_COMMANDS.iter().copied() {
            let conflict = app.conflicts.contains(&cmd);
            let row = gtk4::ListBoxRow::new();
            let inner = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);

            let l = gtk4::Label::new(Some(cmd.label()));
            l.set_xalign(0.0);
            l.set_hexpand(true);
            inner.append(&l);

            let cap = if app.recording == Some(cmd) {
                gtk4::Label::new(Some("press keys…"))
            } else {
                keycap(&app.config.binding(cmd))
            };
            if conflict {
                cap.add_css_class("conflict");
            }
            inner.append(&cap);

            let rec = gtk4::Button::with_label("Record");
            rec.add_css_class("ghost");
            rec.connect_clicked({
                let ui = self.clone();
                move |_| {
                    ui.app.borrow_mut().recording = Some(cmd);
                    ui.clone().render_all();
                }
            });
            inner.append(&rec);

            row.set_child(Some(&inner));
            if conflict {
                row.add_css_class("conflict");
            }
            list.append(&row);
        }
        b.append(&list);

        let reset = gtk4::Button::with_label("Reset all bindings");
        reset.add_css_class("ghost");
        reset.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::ResetBindings)
        });
        b.append(&reset);

        let ok = app.hotkey.status.trigger_confirmed;
        let detail = gtk4::Label::new(Some(&format!(
            "{} — {}",
            app.hotkey.status.backend.label(),
            app.hotkey.status.detail
        )));
        detail.add_css_class(if ok { "ok" } else { "warn" });
        detail.set_wrap(true);
        detail.set_xalign(0.0);
        b.append(&detail);

        if !ok {
            let apply = gtk4::Button::with_label("Apply desktop shortcut");
            apply.add_css_class("primary");
            apply.connect_clicked({
                let ui = self.clone();
                move |_| ui.clone().dispatch(Action::ApplyHotkey)
            });
            b.append(&apply);
        }
        b
    }

    fn storage_page(self: Rc<Self>, app: &App) -> gtk4::Box {
        let b = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
        let path = gtk4::Label::new(Some(&app.config.notes_dir.display().to_string()));
        path.add_css_class("mono");
        path.set_xalign(0.0);
        path.set_selectable(true);
        b.append(&path);

        let stats = gtk4::Label::new(Some(&format!(
            "{} note files · {:.1} KB on disk",
            app.order.len(),
            app.total_bytes as f64 / 1024.0
        )));
        stats.add_css_class("subtitle");
        stats.set_xalign(0.0);
        b.append(&stats);

        let note = gtk4::Label::new(Some(
            "order.toml holds the order. The .txt files are the truth.",
        ));
        note.add_css_class("subtitle");
        note.set_wrap(true);
        note.set_xalign(0.0);
        b.append(&note);

        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let imp = gtk4::Button::with_label("Import .txt");
        imp.add_css_class("ghost");
        imp.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::ImportNotes)
        });
        row.append(&imp);
        let reb = gtk4::Button::with_label("Rebuild index");
        reb.add_css_class("ghost");
        reb.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::RebuildIndex)
        });
        row.append(&reb);
        let open = gtk4::Button::with_label("Open folder");
        open.add_css_class("ghost");
        open.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::OpenNotesDir)
        });
        row.append(&open);
        b.append(&row);
        b
    }

    // ---- input ---------------------------------------------------------------

    fn wire_keys(self: Rc<Self>) {
        let controller = gtk4::EventControllerKey::new();
        self.window.add_controller(controller.clone());

        // Recording takes priority: capture the next combination.
        controller.connect_key_pressed({
            let ui = self.clone();
            move |_, key, _keycode, state| -> glib::Propagation {
                let recording = ui.app.borrow().recording;
                let Some(cmd) = recording else {
                    return glib::Propagation::Proceed;
                };
                if key == gdk::Key::Escape {
                    ui.app.borrow_mut().recording = None;
                    ui.clone().render_all();
                    return glib::Propagation::Stop;
                }
                match ui::combo_from(&key, &state) {
                    Some(combo) => {
                        ui.clone().dispatch(Action::SetBinding(cmd, combo));
                        glib::Propagation::Stop
                    }
                    None => glib::Propagation::Proceed,
                }
            }
        });

        // Ordinary commands fire on press. Always return Proceed so the
        // focused widget — above all the note editor — still receives typing.
        controller.connect_key_pressed({
            let ui = self.clone();
            move |_, key, _keycode, state| {
                let recording = ui.app.borrow().recording;
                if let Some(cmd) = recording {
                    if key == gdk::Key::Escape {
                        ui.app.borrow_mut().recording = None;
                        ui.clone().render_all();
                        return glib::Propagation::Stop;
                    }
                    if let Some(combo) = ui::combo_from(&key, &state) {
                        ui.clone().dispatch(Action::SetBinding(cmd, combo));
                        return glib::Propagation::Stop;
                    }
                    return glib::Propagation::Proceed;
                }
                if let Some(action) = ui::combo_from(&key, &state).and_then(|c| action_for(&ui, &c)) {
                    ui.clone().dispatch(action);
                }
                glib::Propagation::Proceed
            }
        });
    }

    fn wire_focus(self: Rc<Self>) {
        let focus = gtk4::EventControllerFocus::new();
        self.window.add_controller(focus.clone());
        focus.connect_leave({
            let ui = self.clone();
            move |_| {
                // Give the click that caused the blur time to land, otherwise
                // clicking a control closes the overlay before it is pressed.
                let ui2 = ui.clone();
                glib::timeout_add_local_once(Duration::from_millis(60), move || {
                    if ui2.app.borrow().visible {
                        ui2.app.borrow_mut().on_focus_loss();
                        ui2.clone().render_all();
                    }
                });
            }
        });
    }

    // ---- styling ----------------------------------------------------------------

    fn css(&self) -> String {
        let p = &self.app.borrow().config.palette;
        let c = |s: &str| config::parse_colour(s).map(|c| c.css()).unwrap_or_default();
        format!(
            r#"
window {{ background: {paper}; }}
.brand {{ font-weight: 700; font-size: 15px; color: {ink}; }}
.subtitle {{ color: {faint}; font-size: 11px; }}
.mono {{ font-family: monospace; font-size: 11px; color: {soft}; }}
.warn {{ color: {clay}; font-size: 11px; }}
.ok {{ color: {accent}; font-size: 11px; }}
.segmented {{ }}
.choice {{ padding: 14px 16px; }}
.choice:hover {{ background: {wash}; }}
.choice-icon {{ font-size: 17px; color: {accent}; }}
.choice-title {{ font-size: 15px; font-weight: 600; color: {ink}; }}
.choice-count {{ color: {faint}; font-size: 12px; }}
.resume {{ padding: 8px 12px; margin-top: 4px; }}
.resume:hover {{ background: {wash}; }}
.resume-title {{ font-size: 12px; color: {soft}; }}
.kbd {{
  font-family: monospace; font-size: 10px; padding: 2px 6px; margin: 0 1px;
  border-radius: 4px; background: {wash}; color: {soft};
  border: 1px solid {rule};
}}
.kbd.conflict {{ border-color: {danger}; color: {danger}; }}
.note-surface {{ background: {raised}; border: 1px solid {rule}; border-radius: 8px; }}
.note {{ background: {raised}; color: {ink}; font-size: 15px; }}
.library, .keys {{ background: transparent; }}
.note-row {{ padding: 9px 10px; border-radius: 8px; }}
.note-row.selected {{ background: {wash}; }}
.note-title {{ font-weight: 700; font-size: 13.5px; color: {ink}; }}
.note-preview {{ color: {soft}; font-size: 12px; }}
.badge-rtl, .badge-ltr {{
  font-size: 9px; font-weight: 700; padding: 1px 6px; border-radius: 7px;
}}
.badge-rtl {{ background: {claywash}; color: {clay}; }}
.badge-ltr {{ background: {wash}; color: {faint}; }}
.hintbar {{ margin-top: 10px; padding-top: 8px; border-top: 1px solid {rule}; }}
.hintlabel {{ color: {faint}; font-size: 10.5px; }}
.banner {{ padding: 9px 11px; border-radius: 7px; background: {wash}; }}
.banner-warn {{ background: {claywash}; }}
.banner-danger {{ background: {dangerwash}; }}
.primary {{ background: {accent}; color: #ffffff; border: none; border-radius: 7px; padding: 7px 13px; font-weight: 600; }}
.ghost, .flat {{ background: transparent; border: 1px solid {rule}; color: {soft}; border-radius: 7px; padding: 6px 11px; }}
.flat {{ border: none; }}
.ghost:hover, .flat:hover {{ background: {wash}; color: {ink}; }}
.toast {{ background: {ink}; color: {paper}; border-radius: 15px; padding: 7px 15px; font-size: 12px; }}
.conflict {{ background: {dangerwash}; }}
on {{ background: {accent}; color: #ffffff; }}
"#,
            paper = c(&p.paper),
            raised = c(&p.paper_raised),
            ink = c(&p.ink),
            soft = c(&p.ink_soft),
            faint = c(&p.ink_faint),
            accent = c(&p.accent),
            clay = c(&p.clay),
            rule = c(&p.rule),
            wash = c(&p.wash),
            danger = c(&p.danger),
            claywash = config::parse_colour(&p.clay)
                .map(|x| format!("rgba({},{},{},0.13)", (x.r * 255.0) as u8, (x.g * 255.0) as u8, (x.b * 255.0) as u8))
                .unwrap_or_default(),
            dangerwash = config::parse_colour(&p.danger)
                .map(|x| format!("rgba({},{},{},0.10)", (x.r * 255.0) as u8, (x.g * 255.0) as u8, (x.b * 255.0) as u8))
                .unwrap_or_default(),
        )
    }
}

// ---- helpers ---------------------------------------------------------------

fn keycap(binding: &str) -> gtk4::Label {
    let l = gtk4::Label::new(Some(&ui::pretty_key(binding)));
    l.add_css_class("kbd");
    l
}

fn setting_row(title: &str, sub: &str, control: gtk4::Widget) -> gtk4::Box {
    let b = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    let text = gtk4::Box::new(gtk4::Orientation::Vertical, 1);
    let t = gtk4::Label::new(Some(title));
    t.add_css_class("choice-title");
    t.set_xalign(0.0);
    text.append(&t);
    let s = gtk4::Label::new(Some(sub));
    s.add_css_class("subtitle");
    s.set_xalign(0.0);
    text.append(&s);
    text.set_hexpand(true);
    b.append(&text);
    b.append(&control);
    b
}

/// Pango paragraph direction on the whole buffer. This is the piece egui could
/// not do: it is what makes Hebrew right-aligned and correctly reordered.

/// Set the base paragraph direction.
///
/// GTK4 dropped `pango_attr_direction_new`; base direction is a property on
/// the buffer, and it is what drives Pango's bidi reordering. This is exactly
/// what egui had no equivalent of.
fn apply_direction(buffer: &gtk4::TextBuffer, dir: Direction) {
    // Set through the property so this keeps working regardless of whether the
    // bindings expose a typed setter for it.
    buffer.set_property(
        "direction",
        match dir {
            Direction::Ltr => gtk4::TextDirection::Ltr,
            Direction::Rtl => gtk4::TextDirection::Rtl,
        },
    );
}

fn set_widget_direction(w: &impl IsA<gtk4::Widget>, dir: Direction) {
    let ok = match dir {
        Direction::Ltr => w.set_direction(gtk4::TextDirection::Ltr),
        Direction::Rtl => w.set_direction(gtk4::TextDirection::Rtl),
    };
    let _ = ok;
}

/// Which command (if any) the released key combination means.
fn action_for(ui: &Ui, combo: &str) -> Option<Action> {
    let spec = crate::hotkey::HotkeySpec::parse(combo).ok()?;
    let norm = spec.normalised();
    let app = ui.app.borrow();
    let mut found = None;
    for (cmd, binding) in &app.config.bindings {
        if crate::hotkey::HotkeySpec::parse(binding)
            .map(|s| s.normalised())
            .ok()
            .as_deref()
            == Some(norm.as_str())
        {
            found = Some(*cmd);
        }
    }
    let cmd = found?;
    let has_selection = app.view == View::Library && !app.rows.is_empty();
    Some(match cmd {
        Command::Invoke | Command::ToggleOverlay => Action::Close,
        Command::NewNote => Action::NewNote,
        Command::OpenLibrary => match app.view {
            View::Editor => Action::ShowLibrary,
            _ => Action::OpenLibrary,
        },
        Command::ShowLibrary => Action::ShowLibrary,
        Command::Settings => Action::OpenSettings,
        Command::Save => Action::Save { close: false },
        Command::SaveClose => Action::Save { close: true },
        Command::Back => Action::Back,
        Command::FocusSearch => Action::SetQuery(app.query.clone()),
        Command::ToggleDirection => Action::SetDirection(app.ed_dir.other()),
        Command::TogglePreviewLines => {
            Action::SetPreviewLines(if app.config.preview_lines >= 8 {
                1
            } else {
                app.config.preview_lines + 1
            })
        }
        Command::DeleteNote if has_selection => {
            Action::ConfirmDelete(app.rows.get(app.selected)?.id.clone())
        }
        Command::MoveUp if has_selection => Action::ReorderFocus(-1),
        Command::MoveDown if has_selection => Action::ReorderFocus(1),
        _ => return None,
    })
}

use std::time::Duration;