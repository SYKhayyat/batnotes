use egui::{Align, Color32, RichText};
use std::collections::HashSet;

use super::kit;
use super::theme::Theme;
use super::Action;
use crate::config::{Command, Config, ALL_COMMANDS};
use crate::hotkey::HotkeySpec;

#[derive(Clone, Copy, PartialEq, Eq)]
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

pub struct Ctx<'a> {
    pub th: &'a Theme,
    pub cfg: &'a Config,
    pub tab: &'a mut Tab,
    pub recording: &'a mut Option<Command>,
    pub conflicts: &'a HashSet<Command>,
    pub hotkey_backend: &'a str,
    pub hotkey_detail: &'a str,
    pub hotkey_ok: bool,
    pub note_count: usize,
    pub total_bytes: u64,
    pub storage_error: Option<String>,
    pub serif_loaded: bool,
    pub hebrew_loaded: bool,
}

pub fn show(ui: &mut egui::Ui, c: &mut Ctx<'_>, out: &mut Vec<Action>) {
    let th = c.th.clone();
    ui.spacing_mut().item_spacing.y = 0.0;

    ui.add_space(14.0);
    ui.horizontal(|ui| {
        kit::eyebrow(ui, &th, "Make it yours");
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if kit::ghost(ui, &th, "Done").clicked() {
                out.push(Action::Back);
            }
        });
    });
    ui.add_space(6.0);
    ui.label(
        RichText::new("Settings")
            .font(th.styles.heading.clone())
            .color(th.ink),
    );

    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for t in Tab::ALL {
            let on = *c.tab == t;
            let bg = if on { th.accent } else { Color32::TRANSPARENT };
            let fg = if on { Color32::WHITE } else { th.ink_faint };
            if ui
                .add(
                    egui::Button::new(
                        RichText::new(t.label())
                            .size(12.0)
                            .strong()
                            .color(fg),
                    )
                    .fill(bg)
                    .stroke(egui::Stroke::new(1.0, if on { th.accent } else { th.rule }))
                    .rounding(6.0)
                    .min_size(egui::vec2(84.0, 28.0)),
                )
                .clicked()
            {
                *c.tab = t;
                *c.recording = None;
            }
        }
    });
    kit::rule(ui, &th);

    let scroll = egui::ScrollArea::vertical().max_height(300.0).auto_shrink([false, false]);
    scroll.show(ui, |ui| match *c.tab {
        Tab::General => general(ui, c, out),
        Tab::Keys => keys(ui, c, out),
        Tab::Storage => storage(ui, c, out),
    });
    ui.add_space(12.0);
}

fn general(ui: &mut egui::Ui, c: &mut Ctx<'_>, out: &mut Vec<Action>) {
    let th = c.th.clone();
    ui.add_space(14.0);

    ui.add_space(4.0);
    row(ui, &th, "Preview lines", "How much of each note the library shows.", |ui| {
        let mut n = c.cfg.preview_lines as i32;
        if ui.small_button("−").clicked() {
            n -= 1;
        }
        ui.label(RichText::new(format!("{n}")).size(12.5).color(th.ink));
        if ui.small_button("+").clicked() {
            n += 1;
        }
        let n = n.clamp(1, 8) as usize;
        if n != c.cfg.preview_lines {
            out.push(Action::SetPreviewLines(n));
        }
    });

    ui.add_space(10.0);
    row(ui, &th, "Default direction", "Starting direction for new notes.", |ui| {
        let mut d = c.cfg.default_direction;
        if kit::direction_toggle(ui, &th, &mut d) {
            out.push(Action::SetDirection(d));
        }
    });

    ui.add_space(10.0);
    row(ui, &th, "When focus is lost", "What happens when you click away.", |ui| {
        let mut f = c.cfg.focus_loss;
        for opt in [
            crate::config::FocusLoss::AutoHide,
            crate::config::FocusLoss::Prompt,
            crate::config::FocusLoss::Never,
        ] {
            let on = f == opt;
            let label = opt.label();
            if ui
                .add(
                    egui::Button::new(
                        RichText::new(label)
                            .size(11.0)
                            .strong()
                            .color(if on { Color32::WHITE } else { th.ink_faint }),
                    )
                    .fill(if on { th.accent } else { Color32::TRANSPARENT })
                    .stroke(egui::Stroke::new(1.0, if on { th.accent } else { th.rule }))
                    .rounding(5.0)
                    .min_size(egui::vec2(0.0, 26.0)),
                )
                .clicked()
            {
                f = opt;
            }
        }
        if f != c.cfg.focus_loss {
            out.push(Action::SetFocusLoss(f));
        }
    });

    // Toggles raise Actions; the view never writes config directly.
    for (label, sub, current) in [
        (
            "fsync before atomic rename",
            "Slower saves, no data loss on power cut.",
            c.cfg.fsync,
        ),
    ] {
        ui.add_space(10.0);
        let mut on = current;
        if row_hit(ui, &th, label, sub, |ui| toggle(ui, &th, &mut on).clicked()) {
            out.push(Action::SetFsync(on));
        }
    }
    ui.add_space(12.0);
}

fn keys(ui: &mut egui::Ui, c: &mut Ctx<'_>, out: &mut Vec<Action>) {
    let th = c.th.clone();
    ui.add_space(4.0);
    for (i, cmd) in ALL_COMMANDS.iter().copied().enumerate() {
        let conflict = c.conflicts.contains(&cmd);
        let bg = if conflict {
            th.danger.gamma_multiply(0.10)
        } else if i % 2 == 1 {
            th.wash
        } else {
            Color32::TRANSPARENT
        };
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::click());
        ui.painter().rect(rect, egui::Rounding::ZERO, bg, egui::Stroke::NONE);
        let p = ui.painter();
        p.text(
            egui::pos2(rect.left() + 6.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            cmd.label(),
            th.styles.small.clone(),
            th.ink,
        );
        if conflict {
            p.text(
                egui::pos2(rect.left() + 6.0 + cmd.label().len() as f32 * 6.6 + 8.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                "conflict",
                th.styles.tiny.clone(),
                th.danger,
            );
        }

        // Recording state
        let rec = *c.recording == Some(cmd);
        let cap_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 70.0, rect.center().y),
            egui::vec2(104.0, 24.0),
        );
        let r = ui.interact(cap_rect, ui.make_persistent_id(("rec", cmd)), egui::Sense::click());
        if rec {
            p.rect(cap_rect, egui::Rounding::same(5.0), th.clay.gamma_multiply(0.18), egui::Stroke::NONE);
            p.text(
                cap_rect.center(),
                egui::Align2::CENTER_CENTER,
                "press keys…",
                th.styles.tiny.clone(),
                th.clay,
            );
        } else {
            let binding = c.cfg.binding(cmd);
            let valid = HotkeySpec::parse(&binding).is_ok();
            p.rect(
                cap_rect,
                egui::Rounding::same(5.0),
                th.paper_raised,
                egui::Stroke::new(1.0, if valid { th.rule } else { th.danger }),
                
            );
            let mut inner = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(cap_rect.shrink(4.0))
                    .layout(egui::Layout::left_to_right(Align::Center)),
            );
            kit::kbd(&mut inner, &th, &binding, false);
        }
        let cap_clicked = r.clicked();
        let row_clicked = response.clicked() && !cap_clicked;
        if r.hovered() {
            let _ = r.on_hover_text(if rec { "Escape cancels" } else { "Record a new combination" });
        }
        if cap_clicked {
            *c.recording = if rec { None } else { Some(cmd) };
        } else if row_clicked {
            *c.recording = Some(cmd);
        }
    }

    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui
            .add(egui::Button::new(RichText::new("Reset all bindings").size(11.5).color(th.ink_faint)))
            .clicked()
        {
            out.push(Action::ResetBindings);
        }
    });

    ui.add_space(16.0);
    kit::rule(ui, &th);
    ui.add_space(10.0);
    ui.label(
        RichText::new("Global hotkey backend")
            .size(11.5)
            .color(th.ink),
    );
    ui.add_space(4.0);
    let tone = if c.hotkey_ok { th.accent } else { th.clay };
    ui.label(
        RichText::new(format!("{} — {}", c.hotkey_backend, c.hotkey_detail))
            .size(11.0)
            .color(tone),
    );
    ui.add_space(10.0);
    // Off by default on purpose: on some desktops the XDG GlobalShortcuts
    // portal crashes the portal daemon outright. Never enable it implicitly.
    let mut portal = c.cfg.use_portal;
    if row_hit(
        ui,
        &th,
        "Use the XDG GlobalShortcuts portal",
        "Off by default — it crashes the portal daemon on some desktops. Restart to apply.",
        |ui| toggle(ui, &th, &mut portal).clicked(),
    ) {
        out.push(Action::SetUsePortal(portal));
    }

    if !c.hotkey_ok {
        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "No global hotkey is currently bound. Press Apply to write the desktop shortcut entry, or bind it by hand in your desktop's shortcut settings.",
            )
            .size(11.0)
            .color(th.ink_faint),
        );
        ui.add_space(8.0);
        if kit::ghost(ui, &th, "Apply desktop shortcut").clicked() {
            out.push(Action::ApplyHotkey);
        }
    }
    ui.add_space(16.0);
}

fn storage(ui: &mut egui::Ui, c: &mut Ctx<'_>, out: &mut Vec<Action>) {
    let th = c.th.clone();
    ui.add_space(14.0);
    row(ui, &th, "Notes directory", "Plain UTF-8 .txt files.", |ui| {
        ui.label(
            RichText::new(c.cfg.notes_dir.display().to_string())
                .family(egui::FontFamily::Monospace)
                .size(10.5)
                .color(th.ink_faint),
        );
        if kit::ghost(ui, &th, "Open").clicked() {
            out.push(Action::OpenNotesDir);
        }
    });

    ui.add_space(12.0);
    ui.horizontal(|ui| {
        let cells = [
            (c.note_count.to_string(), "note files"),
            (format!("{:.1} KB", c.total_bytes as f64 / 1024.0), "on disk"),
            (c.cfg.preview_lines.to_string(), "preview lines"),
        ];
        for (v, l) in cells {
            let w = (ui.available_width() - 16.0) / 3.0;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 42.0), egui::Sense::hover());
            let p = ui.painter();
            p.rect(rect, egui::Rounding::same(6.0), th.wash, egui::Stroke::NONE);
            p.text(
                rect.center() - egui::vec2(0.0, 8.0),
                egui::Align2::CENTER_CENTER,
                v,
                th.styles.body.clone(),
                th.ink,
            );
            p.text(
                rect.center() + egui::vec2(0.0, 8.0),
                egui::Align2::CENTER_CENTER,
                l,
                th.styles.tiny.clone(),
                th.ink_faint,
            );
        }
    });

    ui.add_space(12.0);
    ui.label(
        RichText::new("order.toml holds the order. The .txt files are the truth.")
            .size(10.5)
            .color(th.ink_faint),
    );
    if let Some(err) = &c.storage_error {
        ui.add_space(6.0);
        ui.label(RichText::new(err).size(11.0).color(th.danger));
    }

    ui.add_space(10.0);
    // Say what is actually loaded rather than implying a theme was applied.
    ui.label(
        RichText::new(match (c.serif_loaded, c.hebrew_loaded) {
            (true, true) => "serif + Hebrew fonts loaded",
            (true, false) => "serif loaded, no Hebrew font found",
            _ => "built-in fonts — set heading_font in config.toml",
        })
        .size(10.5)
        .color(th.ink_faint),
    );

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if kit::ghost(ui, &th, "Import .txt").clicked() {
            out.push(Action::ImportNotes);
        }
        if kit::ghost(ui, &th, "Rebuild index").clicked() {
            out.push(Action::RebuildIndex);
        }
    });
    ui.add_space(16.0);
}

fn row<F: FnOnce(&mut egui::Ui)>(ui: &mut egui::Ui, th: &Theme, title: &str, sub: &str, controls: F) {
    row_hit(ui, th, title, sub, |ui| {
        controls(ui);
        false
    });
}

/// A settings row whose control reports whether it was activated.
fn row_hit<F: FnOnce(&mut egui::Ui) -> bool>(
    ui: &mut egui::Ui,
    th: &Theme,
    title: &str,
    sub: &str,
    controls: F,
) -> bool {
    let mut out = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).size(12.5).color(th.ink));
            ui.label(RichText::new(sub).size(11.0).color(th.ink_faint));
        });
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            out = controls(ui);
        });
    });
    ui.add_space(8.0);
    out
}

fn toggle(ui: &mut egui::Ui, th: &Theme, on: &mut bool) -> egui::Response {
    ui.add(
        egui::Button::new("")
            .fill(if *on { th.accent } else { th.rule })
            .rounding(11.0)
            .min_size(egui::vec2(38.0, 22.0)),
    )
}