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
    /// View the current tree was built for, so resizing happens on change.
    current_view: RefCell<View>,
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
        current_view: RefCell::new(View::Editor),
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
        // current_view is seeded with Editor, so the change-gated resize in
        // rebuild() never fires for the view we open in.
        self.apply_size(View::Editor);
    }

    // ---- window control -------------------------------------------------

    /// Capture wants a small surface; reviewing needs room for a list. One
    /// large window for both means a jot takes more of the screen than a jot
    /// should, which is itself a distraction.
    fn apply_size(&self, view: View) {
        let (w, h) = match view {
            View::Editor => (560, 240),
            View::Library | View::Settings => (660, 560),
        };
        self.window.set_default_size(w, h);
    }

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
        let confirm_id = app.confirm_delete.clone();
        // Capture stays bare. The list and settings can afford a key legend.
        let show_hints = matches!(app.view, View::Library | View::Settings);
        if *self.current_view.borrow() != app.view {
            *self.current_view.borrow_mut() = app.view;
            self.apply_size(app.view);
        }
        match app.view {
            View::Editor => self.build_editor(&app),
            View::Library => self.build_library(&app),
            View::Settings => self.build_settings(&app),
        }
        if let Some(p) = app.prompt.clone() {
            let ui = self.clone();
            self.root.append(&ui.prompt_bar(p));
        }
        drop(app);

        if show_hints {
            self.root.append(&self.clone().hints_bar());
        }
        if self.last_toast.borrow().is_some() {
            self.root.append(&self.toast.clone());
        }
        // Delete is irreversible, so it is confirmed. Drawn inside this window:
        // a separate window can land behind the overlay and, with no
        // always-on-top, the user would never see it.
        if let Some(id) = confirm_id {
            let ui = self.clone();
            self.root.append(&ui.delete_overlay(id));
        }
    }

    fn render_all(self: Rc<Self>) {
        self.clone().rebuild();
        *self.rendered.borrow_mut() = self.signature();
    }

    // ---- actions ---------------------------------------------------------

    /// Apply an action and redraw.
    ///
    /// GTK calls signal handlers across an `extern "C"` boundary, so a panic
    /// here cannot unwind — it aborts the process. This guard turns a widget bug
    /// into a message instead of a dead daemon.
    fn dispatch(self: Rc<Self>, a: Action) {
        let this = self.clone();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            this.app.borrow_mut().dispatch(a);
            this.clone().render_all();
        }));
        if outcome.is_err() {
            eprintln!("batnotes: a widget handler panicked; recovered");
            if let Some(w) = self.window.child() {
                self.window.set_child(None::<&gtk4::Widget>);
                drop(w);
            }
        }
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





    // ---- editor ------------------------------------------------------------

    /// Unsaved-changes guard. Non-modal by design: it must not read as an
    /// error, because the user is mid-thought.
    fn prompt_bar(self: Rc<Self>, which: String) -> gtk4::Box {
        let (text, save_label) = match which.as_str() {
            ui::PROMPT_FOCUS => (
                "Unsaved changes — the overlay stays open so you don't lose capture.",
                "Save & close",
            ),
            _ => ("Discard these changes?", "Save instead"),
        };
        let bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        bar.add_css_class("banner-warn");

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

        let stay = gtk4::Button::with_label("Keep writing");
        stay.add_css_class("ghost");
        stay.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::PromptAction(ui::PROMPT_STAY.into()))
        });
        bar.append(&stay);
        bar
    }

    /// Delete confirmation, drawn over the current view inside the overlay.
    /// A separate window can land behind the overlay and, with no
    /// always-on-top, would never be seen.
    fn delete_overlay(self: Rc<Self>, id: String) -> gtk4::Box {
        let title = self
            .app
            .borrow()
            .index
            .get(&id)
            .map(|d| d.title.as_str())
            .unwrap_or("this note")
            .to_string();

        let layer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        layer.set_vexpand(true);
        layer.add_css_class("scrim");

        let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
        panel.add_css_class("confirm-panel");
        panel.set_halign(gtk4::Align::Center);
        panel.set_valign(gtk4::Align::Center);

        let heading = gtk4::Label::new(Some("Delete this note?"));
        heading.add_css_class("dialog-title");
        heading.set_xalign(0.0);
        panel.append(&heading);

        let body = gtk4::Label::new(Some(&format!(
            "\u{201c}{title}\u{201d} will be removed from disk. This cannot be undone."
        )));
        body.add_css_class("subtitle");
        body.set_wrap(true);
        body.set_xalign(0.0);
        panel.append(&body);

        let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        buttons.set_halign(gtk4::Align::End);

        let keep = gtk4::Button::with_label("Keep it");
        keep.add_css_class("ghost");
        keep.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::DismissConfirm)
        });
        buttons.append(&keep);

        let del = gtk4::Button::with_label("Delete file");
        del.add_css_class("danger");
        del.connect_clicked({
            let ui = self.clone();
            let target = id.clone();
            move |_| ui.clone().dispatch(Action::ConfirmDeleteAccepted(target.clone()))
        });
        buttons.append(&del);

        panel.append(&buttons);
        layer.append(&panel);
        layer
    }

    /// Capture. The default surface and deliberately almost empty: the text,
    /// one quiet line of status, and nothing that asks for a decision. Every
    /// control here is something the user has to *not* think about while
    /// trying to hold onto a thought.
    fn build_editor(self: &Rc<Self>, app: &App) {
        let col = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        let head = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let label: String = match &app.ed_id {
            Some(id) => format!("{id}.txt"),
            None => "new note".to_string(),
        };
        let name = gtk4::Label::new(Some(&label));
        name.add_css_class("subtitle");
        name.set_xalign(0.0);
        name.set_hexpand(true);
        set_direction(&name, app.ed_dir);
        head.append(&name);

        let state = gtk4::Label::new(Some(if app.dirty() {
            "saving\u{2026}"
        } else if app.touched {
            "saved"
        } else {
            ""
        }));
        state.add_css_class(if app.dirty() { "warn" } else { "ok" });
        head.append(&state);
        col.append(&head);
        col.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

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
        set_direction(&view, app.ed_dir);

        buffer.connect_changed({
            let ui = self.clone();
            move |b| {
                let text = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
                let mut app = ui.app.borrow_mut();
                if app.ed_text != text {
                    app.ed_text = text;
                    app.note_edited();
                    drop(app);
                    ui.clone().render_all();
                }
            }
        });
        col.append(&view);

        let foot = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let count = gtk4::Label::new(Some(&self.stats(app)));
        count.add_css_class("subtitle");
        count.set_xalign(0.0);
        count.set_hexpand(true);
        foot.append(&count);

        let hint = gtk4::Label::new(Some(&format!(
            "{} save & close   \u{00b7}   {} close",
            ui::pretty_key(&app.config.binding(Command::SaveClose)),
            ui::pretty_key(&app.config.binding(Command::Back))
        )));
        hint.add_css_class("subtitle");
        foot.append(&hint);
        col.append(&foot);

        self.root.append(&col);
        // The caret must be in the note the instant the overlay appears.
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

        // Adding from the list: the review flow is also a place you notice
        // you need another note, and there is no capture key to hand.
        let add = gtk4::Button::with_label("+  New note");
        add.add_css_class("primary");
        add.set_halign(gtk4::Align::Start);
        add.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::NewNote)
        });
        col.append(&add);

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
        list.set_vexpand(true);
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
        set_direction(&title, doc.direction);
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

        // Every row carries its own delete, rather than hiding it behind a
        // hover state that is easy to miss.
        let del = gtk4::Button::with_label("\u{2715}");
        del.add_css_class("flat");
        del.set_tooltip_text(Some("Delete this note"));
        del.set_valign(gtk4::Align::Start);
        del.connect_clicked({
            let ui = self.clone();
            let target = row.id.clone();
            move |_| ui.clone().dispatch(Action::ConfirmDelete(target.clone()))
        });
        head.append(&del);
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
        set_direction(&p, doc.direction);
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
        // Capture phase is required: in the bubble phase GtkTextView consumes
        // Ctrl+S as text handling, so Save never ran while the editor had
        // focus.
        controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
        self.window.add_controller(controller.clone());

        controller.connect_key_pressed({
            let ui = self.clone();
            move |_, key, keycode, state| -> glib::Propagation {
                // Recording a new binding takes priority over everything.
                if let Some(cmd) = ui.app.borrow().recording {
                    if key == gdk::Key::Escape {
                        ui.app.borrow_mut().recording = None;
                        ui.clone().render_all();
                        return glib::Propagation::Stop;
                    }
                    if let Some(combo) = ui::combo_from(&key, keycode, &state) {
                        ui.clone().dispatch(Action::SetBinding(cmd, combo));
                        return glib::Propagation::Stop;
                    }
                    return glib::Propagation::Proceed;
                }

                // Hardware keycode, not the keysym: with a Hebrew layout GTK
                // reports Ctrl+S as `hebrew_dalet`, so shortcuts taken from the
                // keysym stop matching the moment you switch layouts.
                let combo = ui::combo_from(&key, keycode, &state);
                if let Some(action) = combo.as_deref().and_then(|c| action_for(&ui, c)) {
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
.note {{ background: transparent; color: {ink}; font-size: 15px; }}
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
/// Set text direction on a widget that has the property.
fn set_direction(w: &impl IsA<gtk4::Widget>, dir: Direction) {
    w.set_direction(match dir {
        Direction::Ltr => gtk4::TextDirection::Ltr,
        Direction::Rtl => gtk4::TextDirection::Rtl,
    });
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