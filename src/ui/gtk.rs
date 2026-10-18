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
use crate::ui::{self, Action, Row, View};

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
    /// Widgets that outlive a rebuild.
    ///
    /// Rebuilding the whole tree is fine for static chrome, but it destroys
    /// whatever had keyboard focus. That is what made clicking "New note"
    /// close the window: the button was torn out from under the caret, GTK
    /// dropped focus, and the focus-out handler read it as "the user left".
    /// The editor is worse — it was destroyed on every keystroke.
    editor_view: RefCell<Option<gtk4::TextView>>,
    editor_status: RefCell<Option<gtk4::Label>>,
    editor_count: RefCell<Option<gtk4::Label>>,
    search_entry: RefCell<Option<gtk4::SearchEntry>>,
    /// Set while we are deliberately taking focus, so the focus-out handler
    /// does not mistake it for the user clicking away.
    refocus: RefCell<bool>,
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
        current_view: RefCell::new(View::Library),
        editor_view: RefCell::new(None),
        editor_status: RefCell::new(None),
        editor_count: RefCell::new(None),
        search_entry: RefCell::new(None),
        refocus: RefCell::new(false),
    });
    let rc2 = rc.clone();
    rc2.init();
    rc
}

impl Ui {
    fn init(self: &Rc<Self>) {
        let w = &self.window;
        w.set_title(Some("batnotes"));
        // A titlebar and square corners are what made this read as a dialog
        // instead of something that floats above your work.
        w.set_decorated(false);
        // An undecorated toplevel will map but never take keyboard focus, so
        // every binding silently did nothing.
        w.set_focusable(true);
        w.set_can_focus(true);
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

        self.root.add_css_class("root");
        self.root.set_spacing(0);
        w.set_child(Some(&self.root));

        self.toast.add_css_class("toast");
        self.toast.set_visible(false);

        self.clone().wire_keys();
        self.clone().wire_focus();
        self.clone().render_all();
        // current_view is seeded with Editor, so the change-gated resize in
        // rebuild() never fires for the view we open in.
        self.apply_size(View::Chooser);
    }

    // ---- window control -------------------------------------------------

    /// Capture wants a small surface; reviewing needs room for a list. One
    /// large window for both means a jot takes more of the screen than a jot
    /// should, which is itself a distraction.
    fn apply_size(&self, view: View) {
        let (w, h) = match view {
            View::Chooser => (640, 440),
            View::Editor => (640, 520),
            View::Library | View::Settings => (700, 600),
        };
        self.window.set_default_size(w, h);
    }

    pub fn show(&self) {
        self.window.present();
        // present() asks the compositor for focus; asking the window as well
        // covers the case where it is already mapped and merely re-shown.
        self.window.grab_focus();

        // Re-assert keep-above every time the overlay opens. Wayland clients
        // cannot set this themselves, so the compositor has to be asked again
        // — and only *after* the surface is mapped, because the script walks
        // the list of existing windows. Once mapped, the compositor may still
        // be catching up, hence the second attempt.
        if self.app.borrow().config.keep_above {
            crate::platform::apply_keep_above("batnotes");
            glib::timeout_add_local_once(Duration::from_millis(250), || {
                crate::platform::apply_keep_above("batnotes");
            });
        }
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
        let app_view = app.view;
        let confirm_id = app.confirm_delete.clone();
        // Capture stays bare. The list and settings can afford a key legend.
        let show_hints = matches!(app.view, View::Library | View::Settings);
        if *self.current_view.borrow() != app.view {
            *self.current_view.borrow_mut() = app.view;
            self.apply_size(app.view);
        }
        match app.view {
            View::Chooser => self.build_chooser(&app),
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

        // Put the caret back where the user expects it. Without this the
        // window opens focused on nothing and typing goes nowhere.
        let want = match app_view {
            View::Editor => self.editor_view.borrow().is_some(),
            View::Library | View::Settings | View::Chooser => false,
        };
        if want {
            *self.refocus.borrow_mut() = true;
            if let Some(v) = self.editor_view.borrow().as_ref() {
                v.grab_focus();
            }
        }
    }

    /// Rebuild only when something visible changed.
    fn render_all(self: Rc<Self>) {
        let sig = self.signature();
        if *self.rendered.borrow() == sig {
            return;
        }
        self.clone().rebuild();
        *self.rendered.borrow_mut() = sig;
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
            // A signal can fire this while a borrow is already held; the
            // action is simply dropped rather than aborting the daemon.
            if this.app.try_borrow_mut().is_err() {
                return;
            }
            this.app.borrow_mut().dispatch(a);
            this.clone().render_all();
        }));
        if outcome.is_err() {
            // Do not touch the widget tree here. Disposing it re-enters GTK
            // signal handlers, and a second panic across the extern "C"
            // boundary aborts the process rather than unwinding — which turns
            // a cosmetic fault into a dead daemon.
            eprintln!("batnotes: a widget handler panicked; recovered");
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
    /// Update the editor's status line and counters without touching the tree.
    fn refresh_editor_chrome(self: &Rc<Self>) {
        let (text, words, lines) = {
            let a = self.app.borrow();
            (
                a.ed_text.len(),
                a.ed_text.split_whitespace().count(),
                a.ed_text.lines().count(),
            )
        };
        if let Some(l) = self.editor_count.borrow().as_ref() {
            l.set_text(&format!("{text} chars \u{00b7} {words} words \u{00b7} {lines} lines"));
        }
    }

    /// A padded content column. The card keeps the rounded corners; the
    /// content stays clear of them.
    fn page(&self) -> gtk4::Box {
        let b = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        b.set_margin_top(22);
        b.set_margin_bottom(18);
        b.set_margin_start(28);
        b.set_margin_end(28);
        b
    }

    /// The opening screen. Two choices, no more: capture something, or go
    /// look at what you already have. One hotkey can only mean one thing,
    /// and the point of the overlay is to get out of the way again.
    fn build_chooser(self: &Rc<Self>, app: &App) {
        let col = self.page();

        let over = gtk4::Label::new(Some("CAPTURE OR REVIEW"));
        over.add_css_class("overline");
        over.set_xalign(0.0);
        col.append(&over);

        let h = gtk4::Label::new(Some("What do you want to do?"));
        h.add_css_class("display");
        h.set_xalign(0.0);
        set_direction(&h, app.config.default_direction);
        col.append(&h);

        let sub = gtk4::Label::new(Some(
            "Notes are plain text files on disk. Nothing leaves this machine.",
        ));
        sub.add_css_class("lede");
        sub.set_xalign(0.0);
        sub.set_wrap(true);
        col.append(&sub);

        let gap = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        gap.set_margin_top(14);
        col.append(&gap);

        col.append(&self.clone().choice_row(
            "\u{270f}",
            "New note",
            "Blank page, cursor ready. Type, save, gone.",
            &ui::pretty_key(&app.config.binding(Command::NewNote)),
            Action::NewNote,
        ));

        let hebrew = app
            .order
            .iter()
            .filter(|id| {
                app.index
                    .get(id)
                    .map(|d| d.direction == Direction::Rtl)
                    .unwrap_or(false)
            })
            .count();
        let detail = if app.order.is_empty() {
            "Nothing here yet \u{2014} start with a new note".to_string()
        } else {
            format!(
                "{} note{} \u{00b7} {} Hebrew \u{00b7} search, reopen, delete",
                app.order.len(),
                if app.order.len() == 1 { "" } else { "s" },
                hebrew
            )
        };
        col.append(&self.clone().choice_row(
            "\u{25a6}",
            "My notes",
            &detail,
            &ui::pretty_key(&app.config.binding(Command::OpenLibrary)),
            Action::ShowLibrary,
        ));

        let foot = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        let hint = gtk4::Label::new(Some(&format!(
            "Press {} any time to dismiss",
            ui::pretty_key(&app.config.binding(Command::Invoke))
        )));
        hint.add_css_class("chooser-foot");
        hint.set_xalign(0.0);
        hint.set_hexpand(true);
        foot.append(&hint);

        let close = gtk4::Button::with_label("\u{2715}  Close");
        close.add_css_class("ghost");
        close.set_tooltip_text(Some(&format!(
            "{}",
            ui::pretty_key(&app.config.binding(Command::Back))
        )));
        close.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::Close)
        });
        foot.append(&close);
        col.append(&foot);

        self.root.append(&col);
    }

    /// One full-width destination row. Deliberately not a grid of cards: a
    /// list reads as a menu you can move through, which is faster than
    /// hunting for the right tile.
    fn choice_row(
        self: &Rc<Self>,
        icon: &str,
        title: &str,
        detail: &str,
        key: &str,
        action: Action,
    ) -> gtk4::Button {
        let row = gtk4::Button::new();
        row.add_css_class("choice-row");

        let box_ = gtk4::Box::new(gtk4::Orientation::Horizontal, 16);
        let ic = gtk4::Label::new(Some(icon));
        ic.add_css_class("choice-icon");
        box_.append(&ic);

        let texts = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        let t = gtk4::Label::new(Some(title));
        t.add_css_class("choice-title");
        t.set_xalign(0.0);
        texts.append(&t);
        let d = gtk4::Label::new(Some(detail));
        d.add_css_class("choice-count");
        d.set_xalign(0.0);
        d.set_wrap(true);
        texts.append(&d);
        box_.append(&texts);

        let kbd = gtk4::Label::new(Some(key));
        kbd.add_css_class("kbd");
        box_.append(&kbd);

        let arrow = gtk4::Label::new(Some("\u{2192}"));
        arrow.add_css_class("choice-arrow");
        box_.append(&arrow);

        row.set_child(Some(&box_));
        row.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(action.clone())
        });
        row
    }

    fn build_editor(self: &Rc<Self>, app: &App) {
        let col = self.page();

        let head = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        // The title, not the filename. The id is opaque and meaningless to a
        // person; in the list it read as noise.
        let label: String = match &app.ed_id {
            Some(id) => app
                .index
                .get(id)
                .map(|d| d.title.as_str().to_string())
                .unwrap_or_else(|| "Note".to_string()),
            None => "New note".to_string(),
        };
        let name = gtk4::Label::new(Some(&label));
        name.add_css_class("section-title");
        name.set_xalign(0.0);
        name.set_hexpand(true);
        name.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        set_direction(&name, app.ed_dir);
        head.append(&name);

        // Deliberately no "saving… / saved" label. The note saves itself; a
        // status light is one more thing to look at while trying to think.
        col.append(&head);
        col.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

        // Reuse the live TextView. A new one per keystroke destroyed the
        // buffer the user was typing into and lost the caret every character.
        let view = match self.editor_view.borrow_mut().take() {
            Some(v) => v,
            None => {
                let v = gtk4::TextView::new();
                v.set_wrap_mode(gtk4::WrapMode::WordChar);
                v.set_hexpand(true);
                v.set_vexpand(true);
                v.add_css_class("note");
                v.add_css_class("writing");
                v.set_top_margin(14);
                v.set_bottom_margin(14);
                v.set_left_margin(16);
                v.set_right_margin(16);

                let buffer = v.buffer();
                buffer.connect_changed({
                    let ui = self.clone();
                    move |b| {
                        let text = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
                        // try_borrow, not borrow: this signal also fires from
                        // set_text during a rebuild, which is already holding
                        // the borrow that produced the text.
                        let Ok(mut app) = ui.app.try_borrow_mut() else {
                            return;
                        };
                        if app.ed_text == text {
                            return;
                        }
                        app.ed_text = text;
                        app.note_edited();
                        drop(app);
                        // Update in place. Rebuilding here would destroy the
                        // view this very handler belongs to.
                        ui.refresh_editor_chrome();
                    }
                });
                v
            }
        };
        let buffer = view.buffer();
        if buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).to_string() != app.ed_text {
            buffer.set_text(&app.ed_text);
        }
        set_direction(&view, app.ed_dir);
        col.append(&view);

        let foot = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        // Take before matching: a temporary borrow in the match scrutinee is
        // held for the whole match, so touching the cell inside an arm
        // panics with "already borrowed".
        let cached = self.editor_count.borrow_mut().take();
        let count = match cached {
            Some(l) => l,
            None => {
                let l = gtk4::Label::new(None);
                l.add_css_class("subtitle");
                l
            }
        };
        count.set_xalign(0.0);
        count.set_hexpand(true);
        *self.editor_count.borrow_mut() = Some(count.clone());
        foot.append(&count);

        // Keyboard alone is not an interface. Capture needs a visible way to
        // reach the list and a visible way out.
        let notes = gtk4::Button::with_label("\u{25a6}  My notes");
        notes.add_css_class("ghost");
        notes.set_tooltip_text(Some(&ui::pretty_key(&app.config.binding(Command::ShowLibrary))));
        notes.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::ShowLibrary)
        });
        foot.append(&notes);

        let close = gtk4::Button::with_label("\u{2715}");
        close.add_css_class("flat");
        close.set_tooltip_text(Some(&format!(
            "Close ({})",
            ui::pretty_key(&app.config.binding(Command::Back))
        )));
        close.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::Close)
        });
        foot.append(&close);
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
        let col = self.page();
        col.set_spacing(8);

        let h = gtk4::Label::new(Some("My notes"));
        h.add_css_class("display");
        h.set_xalign(0.0);
        col.append(&h);
        let count = gtk4::Label::new(Some(&format!(
            "{} note{} on disk \u{00b7} search by title, file or content",
            app.order.len(),
            if app.order.len() == 1 { "" } else { "s" }
        )));
        count.add_css_class("lede");
        count.set_xalign(0.0);
        col.append(&count);

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

        // Same reason as the text view: rebuilding on every keystroke of the
        // search box destroyed the entry mid-search.
        let cached = self.search_entry.borrow_mut().take();
        let search = match cached {
            Some(e) => e,
            None => {
                let e = gtk4::SearchEntry::new();
                e.set_placeholder_text(Some("Search titles and content\u{2026}"));
                e.connect_search_changed({
                    let ui = self.clone();
                    move |ent| {
                        // Same hazard as the text view: set_text during a
                        // rebuild re-enters this handler.
                        if ui.app.try_borrow_mut().is_err() {
                            return;
                        }
                        ui.clone().dispatch(Action::SetQuery(ent.text().to_string()));
                    }
                });
                e
            }
        };
        if search.text().to_string() != app.query {
            search.set_text(&app.query);
        }
        *self.search_entry.borrow_mut() = Some(search.clone());
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
        let new = gtk4::Button::with_label("+  New note");
        new.add_css_class("primary");
        new.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::NewNote)
        });
        footer.append(&new);

        let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        footer.append(&spacer);

        let back = gtk4::Button::with_label("Back");
        back.add_css_class("ghost");
        back.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::Back)
        });
        footer.append(&back);

        let close = gtk4::Button::with_label("\u{2715}");
        close.add_css_class("flat");
        close.connect_clicked({
            let ui = self.clone();
            move |_| ui.clone().dispatch(Action::Close)
        });
        footer.append(&close);

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

    fn note_row(self: Rc<Self>, app: &App, _i: usize, row: &Row, doc: &crate::search::Doc) -> gtk4::Widget {
        let r = gtk4::ListBoxRow::new();
        let inner = gtk4::Box::new(gtk4::Orientation::Vertical, 2);

        let head = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        // No direction, no badge. Telling the reader which language a note is
        // in is not information they asked for, and rendering each row
        // right-to-left on its own made the list look like two different
        // lists stacked on top of each other. Direction is applied where it
        // matters: in the editor, when the note is opened.
        let title = gtk4::Label::new(Some(doc.title.as_str()));
        title.add_css_class("note-title");
        title.set_xalign(0.0);
        title.set_hexpand(true);
        title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        head.append(&title);

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
        inner.append(&p);

        let meta = gtk4::Label::new(Some(&format!(
            "{} \u{00b7} {} chars",
            pretty_ago(&doc.path),
            doc.content.len()
        )));
        meta.add_css_class("subtitle");
        meta.set_xalign(0.0);
        inner.append(&meta);

        r.set_child(Some(&inner));
        r.upcast::<gtk4::Widget>()
    }

    fn build_settings(self: &Rc<Self>, app: &App) {
        let page = self.page();
        page.set_spacing(4);
        let scroll = gtk4::ScrolledWindow::builder()
            .child(&self.settings_page(app))
            .vexpand(true)
            .build();
        page.append(&scroll);
        self.root.append(&page);
    }

    fn settings_page(self: &Rc<Self>, app: &App) -> gtk4::Box {
        let b = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
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
            "Keep the overlay above other windows",
            "So typing underneath does not bury what you are writing",
            {
                let ui = self.clone();
                let s = gtk4::Switch::new();
                s.set_active(app.config.keep_above);
                let uic = ui.clone();
                s.connect_state_set(move |_, v| {
                    uic.clone().dispatch(Action::SetKeepAbove(v));
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
                keylog(&format!(
                    "raw key={:?} code={keycode} state={:?}",
                    key, state
                ));
                // Recording a new binding takes priority over everything.
                let recording = ui.app.try_borrow().ok().and_then(|a| a.recording);
                if let Some(cmd) = recording {
                    if key == gdk::Key::Escape {
                        if let Ok(mut a) = ui.app.try_borrow_mut() {
                            a.recording = None;
                        }
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
                keylog(&format!("  combo={:?} action={:?}", combo, combo.as_deref().and_then(|c| action_for(&ui, c))));
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
                    // A rebuild briefly drops focus while we move it back.
                    // Treating that as "the user left" closed the overlay the
                    // instant a button was clicked.
                    if *ui2.refocus.borrow() {
                        *ui2.refocus.borrow_mut() = false;
                        return;
                    }
                    if let Ok(mut app) = ui2.app.try_borrow_mut() {
                        if !app.visible {
                            return;
                        }
                        app.on_focus_loss();
                        drop(app);
                        ui2.clone().render_all();
                    }
                });
            }
        });
    }

    // ---- styling ----------------------------------------------------------------

    /// The visual language. Sizes are quiet and surfaces are flat, but every
    /// interactive edge is rounded and every state change is animated: that
    /// is the difference between a dialog and something that floats.
    fn css(&self) -> String {
        let p = &self.app.borrow().config.palette;
        let c = |s: &str| config::parse_colour(s).map(|c| c.css()).unwrap_or_default();
        let wash = |s: &str, a: f64| {
            config::parse_colour(s)
                .map(|x| {
                    format!(
                        "rgba({},{},{},{a})",
                        (x.r * 255.0) as u8,
                        (x.g * 255.0) as u8,
                        (x.b * 255.0) as u8
                    )
                })
                .unwrap_or_default()
        };
        format!(
            r#"
/* The window itself is the rounded card. Transparency lets the corners be
   real; the compositor does the soft shadow. */
window {{ background: transparent; }}
.root {{
  background: {paper};
  border: 1px solid {rule};
  border-radius: 16px;
  box-shadow: 0 18px 46px {shade}, 0 2px 8px {shade2};
}}

.brand {{ font-weight: 700; font-size: 14px; color: {ink}; letter-spacing: -0.01em; }}
.subtitle {{ color: {soft}; font-size: 12px; }}
.mono {{ font-family: monospace; font-size: 11px; color: {soft}; }}
.warn {{ color: {danger}; font-size: 11.5px; }}
.ok {{ color: {accent}; font-size: 11.5px; }}

/* --- chooser --- */
.overline {{
  font-size: 10.5px; font-weight: 750; color: {accent}; letter-spacing: 0.16em;
  margin-top: 6px;
}}
.display {{
  font-family: {serif}; font-weight: 400; font-size: 34px;
  letter-spacing: -0.03em; color: {ink}; margin-top: 10px;
}}
.lede {{ color: {soft}; font-size: 13px; margin-top: 8px; }}

.choice-row {{
  background: transparent; border: none; border-bottom: 1px solid {rule};
  border-radius: 0; padding: 15px 6px; min-height: 76px;
  transition: padding 180ms ease, background 180ms ease;
}}
.choice-row:hover {{ background: {wash}; padding-left: 14px; padding-right: 10px; }}
.choice-icon {{
  font-size: 19px; color: {accent}; background: {wash};
  min-width: 44px; min-height: 44px; border-radius: 12px;
}}
.choice-title {{ font-size: 16px; font-weight: 650; color: {ink}; letter-spacing: -0.02em; }}
.choice-count {{ color: {soft}; font-size: 12.5px; }}
.choice-arrow {{ color: {faint}; font-size: 15px; transition: transform 180ms ease; }}
.choice-row:hover .choice-arrow {{ transform: translateX(4px); color: {accent}; }}

.resume {{
  background: transparent; border: 1px dashed {rule}; border-radius: 10px;
  padding: 10px 12px; margin-top: 16px; transition: border-color 160ms ease, background 160ms ease;
}}
.resume:hover {{ border-color: {accent}; background: {wash}; }}
.resume-title {{ font-size: 12.5px; font-weight: 600; color: {ink}; }}
.resume-meta {{ font-size: 11px; color: {soft}; }}
.resume-icon {{ color: {soft}; font-size: 14px; }}
.resume-arrow {{ color: {soft}; font-size: 13px; }}
.chooser-foot {{ color: {soft}; font-size: 11.5px; margin-top: 18px; }}

.kbd {{
  font-family: monospace; font-size: 10.5px; padding: 4px 7px;
  border-radius: 5px; background: {wash}; color: {ink};
  border: 1px solid {rule}; box-shadow: 0 1px 0 {rule};
}}
.kbd.conflict {{ border-color: {danger}; color: {danger}; }}

/* --- editor --- */
.note {{ background: transparent; color: {ink}; font-size: 15px; caret-color: {accent}; }}
.writing {{
  background: transparent;
  background-image: repeating-linear-gradient(to bottom, transparent 0, transparent 35px, {rule} 35px, {rule} 36px);
  border: 1px solid {rule}; border-radius: 10px; transition: border-color 160ms ease;
}}
.writing:focus {{ border-color: {accent}; }}
.section-title {{ font-family: {serif}; font-weight: 400; font-size: 26px; color: {ink}; letter-spacing: -0.02em; }}

/* --- library --- */
.note-row {{
  padding: 13px 10px; border-radius: 10px; border-bottom: 1px solid {rule};
  transition: background 150ms ease, padding 150ms ease;
}}
.note-row:hover {{ background: {wash}; padding-left: 14px; }}
.note-row.selected {{ background: {wash}; border-left: 2px solid {accent}; }}
.note-title {{ font-weight: 650; font-size: 14px; color: {ink}; }}
.note-preview {{ color: {soft}; font-size: 12.5px; }}
.badge-rtl, .badge-ltr {{ font-size: 9.5px; font-weight: 700; padding: 2px 7px; border-radius: 6px; }}
.badge-rtl {{ background: {claywash}; color: {clay}; }}
.badge-ltr {{ background: {wash}; color: {soft}; }}

.hintbar {{ margin-top: 10px; padding-top: 9px; border-top: 1px solid {rule}; }}
.hintlabel {{ color: {soft}; font-size: 11px; }}

.banner {{ padding: 10px 12px; border-radius: 9px; background: {wash}; }}
.banner-warn {{ background: {claywash}; }}
.banner-danger {{ background: {dangerwash}; }}
.primary {{
  background: {accent}; color: #ffffff; border: none; border-radius: 8px;
  padding: 9px 15px; font-weight: 620; transition: background 150ms ease, transform 150ms ease;
}}
.primary:hover {{ background: {accentdeep}; }}
.primary:active {{ transform: translateY(1px); }}
.ghost, .flat {{
  background: transparent; border: 1px solid {rule}; color: {ink};
  border-radius: 8px; padding: 8px 12px; transition: background 150ms ease, border-color 150ms ease;
}}
.flat {{ border: none; }}
.ghost:hover, .flat:hover {{ background: {wash}; border-color: {accent}; }}
.danger {{ background: {danger}; color: #ffffff; border: none; border-radius: 8px; padding: 9px 15px; font-weight: 620; }}
.danger:hover {{ background: {dangerdeep}; }}
.toast {{
  background: {accent}; color: #ffffff; border-radius: 16px;
  padding: 9px 16px; font-size: 12px; margin-top: 10px;
}}
.conflict {{ background: {dangerwash}; }}
.on {{ background: {accent}; color: #ffffff; }}

.scrim {{ background: {scrim}; border-radius: 16px; }}
.confirm-panel {{
  background: {raised}; border: 1px solid {rule}; border-radius: 14px;
  padding: 20px; margin: 40px; box-shadow: 0 14px 34px {shade};
}}
.dialog-title {{ font-family: {serif}; font-weight: 400; font-size: 22px; color: {ink}; }}
"#,
            paper = c(&p.paper),
            raised = c(&p.paper_raised),
            ink = c(&p.ink),
            soft = c(&p.ink_soft),
            faint = c(&p.ink_faint),
            accent = c(&p.accent),
            accentdeep = c(&p.accent_deep),
            clay = c(&p.clay),
            rule = c(&p.rule),
            wash = c(&p.wash),
            danger = c(&p.danger),
            claywash = wash(&p.clay, 0.14),
            dangerdeep = c(&p.danger_deep),
            dangerwash = wash(&p.danger, 0.11),
            scrim = wash("#1a2a1d", 0.30),
            shade = wash("#1a2a1d", 0.16),
            shade2 = wash("#1a2a1d", 0.07),
            serif = "Georgia, 'DejaVu Serif', 'Noto Serif', serif".to_string(),
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
/// Append a line to the key log, if it is switched on.
///
/// The switch is a marker file rather than an environment variable: on Nix
/// the process restarts itself with a rebuilt environment, and the variable
/// was being dropped before it reached the stage that does the logging.
fn keylog(line: &str) {
    let Some(dir) = dirs::state_dir() else { return };
    let dir = dir.join("batnotes");
    if !dir.join("keylog").exists() {
        return;
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("keylog.txt"))
    {
        let _ = writeln!(f, "{line}");
    }
}

/// Relative time from the file's mtime. Notes are files, so that is the only
/// timestamp there is — and it is the one the filesystem agrees on.
fn pretty_ago(path: &std::path::Path) -> String {
    let modified = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
    let secs = modified.map(|d| d.as_secs()).unwrap_or(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(secs);
    match now.saturating_sub(secs) {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{}m ago", (now - secs) / 60),
        3600..=86399 => format!("{}h ago", (now - secs) / 3600),
        _ => format!("{}d ago", (now - secs) / 86400),
    }
}

/// Which command, if any, a key combination names.
///
/// Split out from the action mapping so it can be tested against the user's
/// real configuration without a display.
pub fn command_for_combo(
    bindings: &std::collections::BTreeMap<crate::config::Command, String>,
    combo: &str,
) -> Option<crate::config::Command> {
    let norm = crate::hotkey::HotkeySpec::parse(combo).ok()?.normalised();
    bindings.iter().find_map(|(cmd, binding)| {
        let b = crate::hotkey::HotkeySpec::parse(binding).ok()?.normalised();
        (b == norm).then_some(*cmd)
    })
}

fn action_for(ui: &Ui, combo: &str) -> Option<Action> {
    let app = ui.app.borrow();
    let cmd = command_for_combo(&app.config.bindings, combo)?;
    let has_selection = app.view == View::Library && !app.rows.is_empty();
    Some(match cmd {
        Command::Invoke | Command::ToggleOverlay => Action::Close,
        Command::NewNote => Action::NewNote,
        Command::OpenLibrary => match app.view {
            View::Chooser => Action::ShowLibrary,
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
#[cfg(test)]
mod key_tests {
    use super::command_for_combo;
    use crate::config::{Command, Config};

    fn resolve(cfg: &Config, combo: &str) -> Option<Command> {
        command_for_combo(&cfg.bindings, combo)
    }

    /// Every binding must name a command, round-tripping through the syntax
    /// the config is written in.
    #[test]
    fn every_shipped_binding_resolves() {
        let cfg = Config::default();
        for (cmd, binding) in &cfg.bindings {
            let spec = crate::hotkey::HotkeySpec::parse(binding)
                .unwrap_or_else(|_| panic!("{binding:?} should parse"));
            // Invoke and ToggleOverlay deliberately share a binding and both
            // close the overlay, so either is the right answer here.
            assert!(
                resolve(&cfg, &spec.normalised()).is_some(),
                "{cmd:?} should resolve from {binding:?}"
            );
        }
    }

    #[test]
    fn the_essentials_resolve_from_what_gtk_reports_for_those_keys() {
        let cfg = Config::default();
        // Escape, New note, Save & close, My notes.
        assert_eq!(resolve(&cfg, "Escape"), Some(Command::Back));
        assert_eq!(resolve(&cfg, "A"), Some(Command::NewNote));
        assert_eq!(resolve(&cfg, "Ctrl+Enter"), Some(Command::SaveClose));
        assert_eq!(resolve(&cfg, "Ctrl+S"), Some(Command::Save));
        assert!(resolve(&cfg, "B").is_some(), "B reaches the library");
    }

    #[test]
    fn the_live_user_config_resolves() {
        let path = crate::config::config_path();
        let Ok(loaded) = Config::load(&path) else {
            return;
        };
        let cfg = loaded.config;
        for (cmd, binding) in &cfg.bindings {
            let spec = crate::hotkey::HotkeySpec::parse(binding)
                .unwrap_or_else(|_| panic!("{binding:?} should parse"));
            // Invoke and ToggleOverlay deliberately share a binding and both
            // close the overlay, so the first match is the correct answer.
            let got = resolve(&cfg, &spec.normalised());
            assert!(got.is_some(), "{cmd:?} should resolve from {binding:?}");
        }
    }
}
