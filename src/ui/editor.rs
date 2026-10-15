use egui::{Align, RichText};

use super::kit;
use super::theme::Theme;
use super::Action;
use crate::config::{Command, Config};
use crate::model::Direction;

pub struct Ctx<'a> {
    pub th: &'a Theme,
    pub cfg: &'a Config,
    /// Live buffer. The app owns it; the view reads and writes in place so
    /// nothing has to round-trip through an Action on every keystroke.
    pub text: &'a mut String,
    pub original: &'a str,
    pub direction: &'a mut Direction,
    pub original_direction: Direction,
    pub is_new: bool,
    pub id: Option<String>,
}

impl Ctx<'_> {
    pub fn dirty(&self) -> bool {
        self.text.as_str() != self.original || *self.direction != self.original_direction
    }
}

/// Ruled writing surface, live direction, and an explicit unsaved indicator.
/// Rules are spaced to the text line height so they sit under the glyphs.
pub fn show(ui: &mut egui::Ui, c: &mut Ctx<'_>, out: &mut Vec<Action>) {
    let th = c.th.clone();
    ui.spacing_mut().item_spacing.y = 0.0;

    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let mut dir = *c.direction;
        if kit::direction_toggle(ui, &th, &mut dir) {
            *c.direction = dir;
            out.push(Action::SetDirection(dir));
        }
        ui.label(RichText::new("·").size(11.0).color(th.rule));
        ui.label(
            RichText::new(match &c.id {
                Some(id) => format!("{id}.txt"),
                None => "unsaved draft".to_string(),
            })
            .family(egui::FontFamily::Monospace)
            .size(10.5)
            .color(th.ink_faint),
        );
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            let dirty = c.dirty();
            ui.label(
                RichText::new(if dirty {
                    "Unsaved changes"
                } else if c.is_new {
                    "Ready to write"
                } else {
                    "Saved"
                })
                .size(11.0)
                .color(if dirty { th.clay } else { th.accent }),
            );
            let (r, _) = ui.allocate_exact_size(egui::vec2(6.0, 6.0), egui::Sense::hover());
            ui.painter()
                .circle_filled(r.center(), 3.0, if dirty { th.clay } else { th.accent });
        });
    });

    ui.add_space(12.0);

    let line_h = 22.0;
    let pad = 14.0;
    let height: f32 = 250.0;
    let width = ui.available_width();
    let id = ui.make_persistent_id("batnotes-editor");
    let rtl = *c.direction == Direction::Rtl;

    // Card background is painted by hand: egui 0.29 has no styling hook on
    // TextEdit, and the rules have to sit under the text, not behind a frame.
    let (surface, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    {
        let p = ui.painter();
        for i in 0..((height / line_h).ceil() as usize) {
            let y = surface.top() + pad + (i as f32 + 1.0) * line_h - 1.0;
            if y > surface.bottom() - 4.0 {
                break;
            }
            p.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(surface.left() + pad, y),
                    egui::pos2(surface.right() - pad, y),
                ),
                0.0,
                th.rule.gamma_multiply(0.8),
            );
        }
        p.rect(surface, egui::Rounding::same(8.0), th.paper_raised, egui::Stroke::NONE);
        let focused = ui.memory(|m| m.has_focus(id));
        p.rect(
            surface,
            egui::Rounding::same(8.0),
            if focused { th.accent.gamma_multiply(0.6) } else { th.rule },
            egui::Stroke::new(if focused { 1.4 } else { 1.0 }, if focused { th.accent } else { th.rule }),
        );
    }

    egui::TextEdit::multiline(&mut *c.text)
        .id(id)
        .hint_text(if rtl { "מה תרצו לזכור?" } else { "What would you like to remember?" })
        .font(egui::TextStyle::Body)
        .desired_width(width - 4.0)
        .desired_rows((height / line_h).ceil() as usize)
        .text_color(th.ink)
        .frame(false)
        .margin(egui::Margin::symmetric(pad, pad))
        .show(ui);

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(
            RichText::new(crate::model::title_of(c.text))
                .size(11.5)
                .color(th.ink_soft),
        );
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if c.id.is_some() && kit::ghost(ui, &th, "Delete").clicked() {
                out.push(Action::ConfirmDelete(c.id.clone().unwrap()));
            }
            if kit::ghost(ui, &th, "Cancel").clicked() {
                out.push(Action::Cancel);
            }
            let cap = kit::pretty_binding(&c.cfg.binding(Command::SaveClose));
            if kit::primary(ui, &th, "Save", Some(&cap)).clicked() {
                out.push(Action::Save { close: true });
            }
        });
    });
    ui.add_space(4.0);
    ui.label(
        RichText::new(format!(
            "{} chars · {} words · {} lines{}",
            c.text.chars().count(),
            c.text.split_whitespace().count(),
            c.text.lines().count().max(1),
            if crate::model::is_hebrew(c.text) && !rtl {
                " · Hebrew in an LTR note"
            } else {
                ""
            }
        ))
        .size(10.5)
        .color(th.ink_faint),
    );
    ui.add_space(14.0);
}