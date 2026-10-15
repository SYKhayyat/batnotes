use egui::{Align, Color32, RichText, Sense};
use std::sync::Arc;

use super::kit;
use super::theme::Theme;
use super::Action;
use crate::config::Config;
use crate::model::Direction;
use crate::search::Indexed;

#[derive(Clone)]
pub struct Row {
    pub id: String,
    /// Byte offset of the match in the body, when the hit was in the body.
    pub at: Option<usize>,
}

pub struct Ctx<'a> {
    pub th: &'a Theme,
    pub cfg: &'a Config,
    pub index: &'a Arc<Indexed>,
    pub query: &'a mut String,
    pub rows: &'a [Row],
    pub selected: usize,
    pub hover: Option<usize>,
    pub dragging: Option<usize>,
    pub drag_over: Option<usize>,
    pub rebuilt: bool,
}

pub fn show(ui: &mut egui::Ui, c: &mut Ctx<'_>, out: &mut Vec<Action>) {
    let th = c.th.clone();
    ui.spacing_mut().item_spacing.y = 0.0;

    // ---- header ----
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        kit::eyebrow(ui, &th, "The collection");
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if kit::ghost(ui, &th, "New note").clicked() {
                out.push(Action::NewNote);
            }
        });
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("My notes")
                .font(th.styles.heading.clone())
                .color(th.ink),
        );
        kit::pill(ui, &th, &c.rows.len().to_string(), th.accent, th.wash);
    });

    ui.add_space(12.0);

    // ---- search ----
    let query = c.query.clone();
    let search_response = egui::TextEdit::singleline(&mut *c.query)
        .id(ui.make_persistent_id("batnotes-search"))
        .hint_text(if query.is_empty() {
            "Search titles, filenames & content…".to_string()
        } else {
            format!("{} result{}", c.rows.len(), if c.rows.len() == 1 { "" } else { "s" })
        })
        .desired_width(ui.available_width())
        .text_color(th.ink)
        .frame(false)
        .margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui);
    let _ = search_response;
    if *c.query != query {
        out.push(Action::SetQuery(c.query.clone()));
    }

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(if c.query.is_empty() {
                "Drag to reorder · ↑↓ to navigate · Enter opens"
            } else {
                "title matches first"
            })
            .size(10.5)
            .color(th.ink_faint),
        );
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            let cap = format!("{} line preview", c.cfg.preview_lines);
            ui.label(RichText::new(cap).size(10.5).color(th.ink_faint));
        });
    });
    kit::rule(ui, &th);

    // ---- rows ----
    let scroll = egui::ScrollArea::vertical().max_height(300.0).auto_shrink([false, false]);
    scroll.show(ui, |ui| {
        if c.rows.is_empty() {
            ui.add_space(28.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(
                        if c.query.is_empty() {
                            "No notes yet".to_string()
                        } else {
                            format!("Nothing matches “{}”", c.query)
                        },
                    )
                    .size(13.0)
                    .color(th.ink_soft),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Search is case-insensitive and Unicode-aware — Hebrew works too.")
                        .size(11.0)
                        .color(th.ink_faint),
                );
                ui.add_space(12.0);
                if kit::primary(ui, &th, "Write it instead", None).clicked() {
                    out.push(Action::NewNote);
                }
            });
            ui.add_space(28.0);
            return;
        }

        let n = c.rows.len();
        for i in 0..n {
            let row = Row { id: c.rows[i].id.clone(), at: c.rows[i].at };
            let Some(doc) = c.index.get(&row.id) else { continue };
            let selected = i == c.selected;
            let hovered = c.hover == Some(i);
            let dragging = c.dragging == Some(i);
            let is_drop_target = c.drag_over == Some(i) && c.dragging != Some(i);

            ui.add_space(6.0);
            card(ui, c, &th, i, row, doc, selected, hovered, dragging, is_drop_target, out);
        }
        ui.add_space(10.0);
    });
}

#[allow(clippy::too_many_arguments)]
fn card(
    ui: &mut egui::Ui,
    c: &mut Ctx<'_>,
    th: &Theme,
    i: usize,
    row: Row,
    doc: &crate::search::Doc,
    selected: bool,
    hovered: bool,
    dragging: bool,
    drop_target: bool,
    out: &mut Vec<Action>,
) {
    let rtl = doc.direction == Direction::Rtl;
    let bg = if selected {
        th.paper_raised
    } else if hovered {
        th.wash
    } else {
        th.paper_raised
    };
    let border = if drop_target {
        th.accent
    } else if selected || hovered {
        th.accent.gamma_multiply(0.8)
    } else {
        th.rule
    };

    let response = ui.add(
        egui::Button::new("")
            .fill(bg.gamma_multiply(if dragging { 0.4 } else { 1.0 }))
            .stroke(egui::Stroke::new(if selected { 1.4 } else { 1.0 }, border))
            .rounding(9.0)
            .sense(Sense::click_and_drag())
            .min_size(egui::vec2(ui.available_width(), 62.0)),
    );

    let rect = response.rect;
    let p = ui.painter();

    // Grip
    p.text(
        rect.left_center() + egui::vec2(11.0, 0.0),
        egui::Align2::CENTER_CENTER,
        "⠿",
        th.styles.body.clone(),
        th.rule,
    );

    let text_x = rect.left() + 26.0;
    let right_pad = if hovered { 40.0 } else { 12.0 };

    // Title + direction badge
    p.text(
        egui::pos2(text_x, rect.top() + 15.0),
        egui::Align2::LEFT_TOP,
        doc.title.as_str(),
        th.styles.body.clone(),
        th.ink,
    );
    let title_w = p
        .layout(
            doc.title.as_str().to_string(),
            th.styles.body.clone(),
            th.ink,
            rect.width() * 0.5,
        )
        .size()
        .x;
    let badge_x = (text_x + 10.0 + title_w).min(rect.right() - 90.0);
    if rtl {
        kit::pill_at(
            p,
            th,
            badge_x,
            rect.top() + 7.0,
            "HE · RTL",
            th.clay,
            th.clay.gamma_multiply(0.14),
        );
    } else {
        kit::pill_at(p, th, badge_x, rect.top() + 7.0, "EN · LTR", th.ink_faint, th.wash);
    }

    // Preview, or a snippet centred on the match.
    let preview = match row.at {
        Some(at) if !c.query.is_empty() => c.index.snippet(&row.id, at, 28, 42),
        _ => c.index.preview(&row.id, c.cfg.preview_lines).replace('\n', "  "),
    };
    let preview_text: String = preview.chars().take(160).collect();
    let galley = p.layout(
        preview_text,
        th.styles.small.clone(),
        th.ink_faint,
        (rect.width() - 40.0 - right_pad).max(40.0),
    );
    p.galley(egui::pos2(text_x, rect.top() + 30.0), galley, th.ink_faint);

    // Footer: filename and size.
    let meta = format!(
        "{} · {} chars",
        doc.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        doc.content.chars().count()
    );
    p.text(
        egui::pos2(text_x, rect.bottom() - 12.0),
        egui::Align2::LEFT_BOTTOM,
        meta,
        th.styles.tiny.clone(),
        th.ink_faint,
    );

    // Hover action stack. These are real interactions so the hover state and
    // the click are both genuine rather than reconstructed from rect maths.
    if hovered {
        let bx = rect.right() - 8.0;
        let mut y = rect.center().y - 22.0;
        for (glyph, tip, make) in [
            ("↑", "Move up", 0u8),
            ("↓", "Move down", 1),
            ("🗑", "Delete", 2),
        ] {
            let r = egui::Rect::from_min_size(egui::pos2(bx - 24.0, y), egui::vec2(24.0, 21.0));
            let resp = ui.interact(
                r,
                ui.make_persistent_id(("note-action", row.id.clone(), make)),
                Sense::click(),
            );
            let hov = resp.hovered();
            p.rect(
                r,
                egui::Rounding::same(5.0),
                if hov { th.danger.gamma_multiply(0.10) } else { Color32::TRANSPARENT },
                egui::Stroke::new(1.0, th.rule),
            );
            p.text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                glyph,
                th.styles.small.clone(),
                if hov { th.danger } else { th.ink_faint },
            );
            if resp.clicked() {
                let _ = resp.on_hover_text(tip);
                match make {
                    0 => out.push(Action::Move(row.id.clone(), -1)),
                    1 => out.push(Action::Move(row.id.clone(), 1)),
                    _ => out.push(Action::ConfirmDelete(row.id.clone())),
                }
            }
            y += 24.0;
        }
    }

    if response.clicked() {
        out.push(Action::Select(i));
    }
    if response.double_clicked() {
        out.push(Action::OpenNote(row.id.clone()));
    }

}