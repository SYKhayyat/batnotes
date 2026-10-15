use egui::RichText;

use super::kit;
use super::theme::Theme;
use super::{Action, View};
use crate::config::{Command, Config};
use crate::search::Indexed;
use std::sync::Arc;

pub struct Ctx<'a> {
    pub th: &'a Theme,
    pub cfg: &'a Config,
    pub index: &'a Arc<Indexed>,
    pub conflicts: &'a std::collections::HashSet<Command>,
    pub note_count: usize,
    pub view: View,
}

/// Two choices, nothing else. Everything the prototypes used to sell the app —
/// the display headline, the value propositions, the trust strip — is cut,
/// because an overlay you open fifty times a day has nothing to prove.
pub fn show(ui: &mut egui::Ui, c: &Ctx<'_>, out: &mut Vec<Action>) {
    let th = c.th;
    ui.spacing_mut().item_spacing.y = 0.0;

    ui.add_space(6.0);
    kit::rule(ui, th);
    row(ui, c, out, "＋", "New note", Command::NewNote, Action::NewNote);
    kit::hairline(ui, th);
    row(
        ui,
        c,
        out,
        "≡",
        "My notes",
        Command::OpenLibrary,
        Action::OpenLibrary,
    );
    kit::rule(ui, th);

    // Resume: one line, only when there is something to resume.
    if let Some(id) = c.index.ids().first().cloned() {
        if let Some(doc) = c.index.get(&id) {
            ui.add_space(2.0);
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), egui::Sense::click());
            let p = ui.painter();
            let hovered = resp.hovered();
            if hovered {
                p.rect(rect, egui::Rounding::same(6.0), th.wash, egui::Stroke::NONE);
            }
            p.text(
                egui::pos2(rect.left() + 4.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                doc.title.as_str(),
                th.styles.small.clone(),
                if hovered { th.accent } else { th.ink_soft },
            );
            p.text(
                egui::pos2(rect.right() - 4.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                "resume",
                th.styles.tiny.clone(),
                th.ink_faint,
            );
            if resp.clicked() {
                out.push(Action::ResumeNote(id));
            }
        }
    }

    if !c.conflicts.is_empty() {
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!(
                "{} bindings conflict — check Settings",
                c.conflicts.len()
            ))
            .size(10.5)
            .color(th.clay),
        );
    }
    ui.add_space(10.0);
}

fn row(
    ui: &mut egui::Ui,
    c: &Ctx<'_>,
    out: &mut Vec<Action>,
    glyph: &str,
    title: &str,
    command: Command,
    action: Action,
) {
    let th = c.th;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 52.0), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect(rect, egui::Rounding::same(7.0), th.wash, egui::Stroke::NONE);
    }

    let center = rect.center();
    let tile = egui::Rect::from_center_size(center + egui::vec2(-(rect.width() - 116.0) / 2.0 + 15.0, 0.0), egui::vec2(26.0, 26.0));
    p.text(tile.center(), egui::Align2::CENTER_CENTER, glyph, th.styles.body.clone(), th.accent);

    let x = tile.right() + 14.0;
    p.text(
        egui::pos2(x, center.y),
        egui::Align2::LEFT_CENTER,
        title,
        th.styles.body.clone(),
        th.ink,
    );
    if title == "My notes" && c.note_count > 0 {
        let tw = p
            .layout(
                title.to_string(),
                th.styles.body.clone(),
                th.ink,
                rect.width(),
            )
            .size()
            .x;
        p.text(
            egui::pos2(x + tw + 8.0, center.y),
            egui::Align2::LEFT_CENTER,
            c.note_count.to_string(),
            th.styles.small.clone(),
            th.ink_faint,
        );
    }

    let cap_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 18.0, center.y),
        egui::vec2(80.0, 20.0),
    );
    let mut cap = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(cap_rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    kit::kbd(&mut cap, th, &c.cfg.binding(command), false);

    if resp.clicked() {
        out.push(action);
    }
}