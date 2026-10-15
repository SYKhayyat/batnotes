//! Shared UI pieces. These are the parts that make the overlay read as one
//! deliberate surface rather than a pile of default widgets.

use egui::{Align, Color32, Rounding, Layout, Rect, Response, RichText, Sense, Stroke, Ui, Vec2};

use super::theme::Theme;

/// Split `Ctrl+Shift+Space` into individual caps, drawn as keycaps.
///
/// This is the component that carries "every action shows its current binding"
/// from the SRS — so it renders from config on every frame and cannot drift.
pub fn kbd(ui: &mut Ui, th: &Theme, binding: &str, on_accent: bool) {
    let parts: Vec<&str> = binding.split('+').collect();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (i, p) in parts.iter().enumerate() {
            if i > 0 {
                ui.label(
                    RichText::new("+")
                        .size(9.0)
                        .color(if on_accent { Color32::from_white_alpha(140) } else { th.ink_faint }),
                );
            }
            let label = pretty_key(p);
            let galley = ui.painter().layout_no_wrap(
                label.clone().into(),
                th.styles.kbd.clone(),
                Color32::PLACEHOLDER,
            );
            let w = galley.size().x + 12.0;
            let h = 19.0;
            let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
            let (fill, stroke, fg) = if on_accent {
                (
                    Color32::from_black_alpha(38),
                    Color32::from_white_alpha(40),
                    Color32::from_white_alpha(225),
                )
            } else {
                (th.wash, th.rule, th.ink_faint)
            };
            ui.painter()
                .rect(rect, Rounding::same(4.0), fill, Stroke::new(1.0, stroke), );
            // The lower bevel is what makes a flat rect read as a keycap.
            ui.painter().rect(
                Rect::from_min_max(
                    rect.min + Vec2::new(1.0, 0.0),
                    rect.max - Vec2::new(1.0, 0.0),
                ),
                Rounding::same(4.0),
                Color32::TRANSPARENT,
                Stroke::new(1.0, Color32::from_black_alpha(14)),
                
            );
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                th.styles.kbd.clone(),
                fg,
            );
        }
    });
}

fn pretty_key(k: &str) -> String {
    match k {
        "Escape" => "Esc".into(),
        "Enter" => "↵".into(),
        "Delete" => "Del".into(),
        "Space" => "Space".into(),
        "Up" => "↑".into(),
        "Down" => "↓".into(),
        "Left" => "←".into(),
        "Right" => "→".into(),
        "Comma" => ",".into(),
        "Slash" => "/".into(),
        "BracketLeft" => "[".into(),
        "BracketRight" => "]".into(),
        other => other.to_string(),
    }
}

/// Small all-caps label with a short rule beside it. Borrowed from prototype B;
/// two elements that give the whole surface a voice.
pub fn eyebrow(ui: &mut Ui, th: &Theme, text: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 9.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(17.0, 1.0), Sense::hover());
        ui.painter().rect_filled(rect, 0.0, th.accent.gamma_multiply(0.6));
        ui.label(
            RichText::new(text.to_uppercase())
                .size(10.5)
                .strong()
                .color(th.accent)
                .extra_letter_spacing(2.0),
        );
    });
}

/// A full-width row separated by hairlines instead of being drawn as a box.
/// Reads as a list, not a dashboard.
pub fn hairline(ui: &mut Ui, th: &Theme) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, th.rule);
}

pub fn rule(ui: &mut Ui, th: &Theme) {
    ui.add_space(1.0);
    hairline(ui, th);
    ui.add_space(1.0);
}

/// Three stacked, slightly rotated pages — prototype B's mark.
pub fn brand_mark(ui: &mut Ui, th: &Theme, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let p = ui.painter();
    let unit = size / 29.0;
    for (i, (rot, tint)) in [(-10.0, th.paper), (2.0, th.wash), (10.0, th.accent.gamma_multiply(0.55))]
        .into_iter()
        .enumerate()
    {
        let s = 19.0 * unit;
        let h = 23.0 * unit;
        let c = rect.center() + Vec2::new(((i as f32) - 1.0) * 3.0 * unit, 0.0);
        let pts = rotated_rect(Rect::from_center_size(c, Vec2::new(s, h)), rot);
        p.add(egui::Shape::convex_polygon(
            pts.to_vec(),
            tint,
            Stroke::new(1.4 * unit, th.accent.gamma_multiply(0.85)),
        ));
    }
}

fn rotated_rect(r: Rect, deg: f32) -> [egui::Pos2; 4] {
    let rad = deg.to_radians();
    let (s, c) = rad.sin_cos();
    let rot = |p: egui::Pos2| {
        let d = p - r.center();
        egui::Pos2::new(
            r.center().x + d.x * c - d.y * s,
            r.center().y + d.x * s + d.y * c,
        )
    };
    [
        rot(r.left_top()),
        rot(r.right_top()),
        rot(r.right_bottom()),
        rot(r.left_bottom()),
    ]
}

pub fn brand(ui: &mut Ui, th: &Theme, size: f32) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        brand_mark(ui, th, size);
        ui.label(
            RichText::new("batnotes")
                .font(th.styles.title.clone())
                .color(th.ink)
                .extra_letter_spacing(-1.0),
        );
    });
}

/// Small dot plus label. Green when clean, clay when there is unsaved work.
pub fn status_dot(ui: &mut Ui, th: &Theme, dirty: bool, saved_label: &str, dirty_label: &str) {
    let (col, text) = if dirty { (th.clay, dirty_label) } else { (th.accent, saved_label) };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (r, _) = ui.allocate_exact_size(Vec2::splat(6.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 3.0, col);
        ui.label(RichText::new(text).size(11.0).color(col));
    });
}

pub fn pill(ui: &mut Ui, th: &Theme, text: &str, fg: Color32, bg: Color32) {
    let galley = ui.painter().layout_no_wrap(
        text.to_string().into(),
        th.styles.tiny.clone(),
        Color32::PLACEHOLDER,
    );
    let w = galley.size().x + 14.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 18.0), Sense::hover());
    ui.painter().rect(rect, Rounding::same(9.0), bg, egui::Stroke::NONE);
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text.to_string(), th.styles.tiny.clone(), fg);
}

/// Absolutely-positioned pill, for painting badges onto a card we also need to
/// hit-test separately.
pub fn pill_at(p: &egui::Painter, th: &Theme, x: f32, y: f32, text: &str, fg: Color32, bg: Color32) {
    let galley = p.layout_no_wrap(text.to_string().into(), th.styles.tiny.clone(), Color32::PLACEHOLDER);
    let w = galley.size().x + 12.0;
    let r = Rect::from_min_size(egui::pos2(x, y), Vec2::new(w, 15.0));
    p.rect(r, Rounding::same(7.0), bg, egui::Stroke::NONE);
    p.text(r.center(), egui::Align2::CENTER_CENTER, text.to_string(), th.styles.tiny.clone(), fg);
}

/// Solid primary action.
pub fn primary(ui: &mut Ui, th: &Theme, label: &str, binding: Option<&str>) -> Response {
    let r = ui.add_enabled(
        true,
        egui::Button::new(RichText::new(label).size(13.0).strong().color(Color32::WHITE))
            .fill(th.accent)
            .rounding(7.0)
            .min_size(Vec2::new(0.0, 32.0)),
    );
    if let Some(b) = binding {
        let r = r.rect;
        ui.painter().text(
            egui::pos2(r.right() - 10.0, r.center().y),
            egui::Align2::RIGHT_CENTER,
            pretty_binding(b),
            th.styles.tiny.clone(),
            Color32::from_white_alpha(150),
        );
    }
    r
}

/// Quiet secondary action.
pub fn ghost(ui: &mut Ui, th: &Theme, label: &str) -> Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(12.5).color(th.ink_soft))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, th.rule))
            .rounding(7.0)
            .min_size(Vec2::new(0.0, 32.0)),
    )
}

pub fn danger_button(ui: &mut Ui, th: &Theme, label: &str) -> Response {
    ui.add(
        egui::Button::new(
            RichText::new(label).size(13.0).strong().color(Color32::WHITE),
        )
        .fill(th.danger)
        .rounding(7.0)
        .min_size(Vec2::new(0.0, 32.0)),
    )
}

/// Icon-only square button used for the per-note action stack.
pub fn icon_button(ui: &mut Ui, th: &Theme, glyph: &str, tip: &str) -> Response {
    let r = ui.add(
        egui::Button::new(RichText::new(glyph).size(13.0).color(th.ink_faint))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, th.rule))
            .rounding(5.0)
            .min_size(Vec2::splat(26.0)),
    );
    r.on_hover_text(tip)
}

pub fn pretty_binding(b: &str) -> String {
    b.split('+').map(pretty_key).collect::<Vec<_>>().join("+")
}

/// Segmented LTR / RTL control.
pub fn direction_toggle(ui: &mut Ui, th: &Theme, current: &mut crate::model::Direction) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (d, label) in [
            (crate::model::Direction::Ltr, "LTR"),
            (crate::model::Direction::Rtl, "RTL"),
        ] {
            let on = *current == d;
            let bg = if on { th.accent } else { Color32::TRANSPARENT };
            let fg = if on { Color32::WHITE } else { th.ink_faint };
            if ui
                .add(
                    egui::Button::new(RichText::new(label).size(11.0).strong().color(fg))
                        .fill(bg)
                        .stroke(Stroke::new(1.0, if on { th.accent } else { th.rule }))
                        .rounding(5.0)
                        .min_size(Vec2::new(48.0, 26.0)),
                )
                .clicked()
            {
                if !on {
                    *current = d;
                    changed = true;
                }
            }
        }
        if *current == crate::model::Direction::Rtl {
            ui.add_space(4.0);
            ui.label(RichText::new("עברית").size(11.0).color(th.ink_faint));
        }
    });
    changed
}

/// Non-modal strip for the unsaved-changes guard, per DESIGN §4.1.
pub fn banner(ui: &mut Ui, th: &Theme, tone: Tone, text: &str, actions: Vec<BannerAction>) -> Option<String> {
    let (bg, fg) = match tone {
        Tone::Warn => (th.clay.gamma_multiply(0.13), th.clay.gamma_multiply(0.95)),
        Tone::Danger => (th.danger.gamma_multiply(0.10), th.danger),
        Tone::Info => (th.wash, th.ink_soft),
    };
    let text = text.to_string();
    let height = 38.0;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    ui.painter().rect(rect, Rounding::same(7.0), bg, egui::Stroke::NONE);
    let mut clicked = None;
    let mut area = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink(10.0))
            .layout(Layout::left_to_right(Align::Center)),
    );
    area.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.label(RichText::new(text).size(12.0).color(fg));
        for a in actions {
            let hit = if a.primary {
                primary(ui, th, &a.label, None).clicked()
            } else {
                ui.add(
                    egui::Button::new(RichText::new(&a.label).size(11.5).color(fg))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE),
                )
                .clicked()
            };
            if hit {
                clicked = Some(a.id);
            }
        }
    });
    clicked
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Warn,
    Danger,
    Info,
}

pub struct BannerAction {
    pub id: String,
    pub label: String,
    pub primary: bool,
}

impl BannerAction {
    pub fn primary(id: &str, label: &str) -> Self {
        BannerAction { id: id.into(), label: label.into(), primary: true }
    }
    pub fn quiet(id: &str, label: &str) -> Self {
        BannerAction { id: id.into(), label: label.into(), primary: false }
    }
}

/// Centred confirm dialog drawn over a scrim.
pub fn confirm_card<F: FnOnce(&mut Ui)>(
    ui: &mut Ui,
    th: &Theme,
    width: f32,
    title: &str,
    body: &str,
    confirm_label: &str,
    cancel_label: &str,
    confirm_danger: bool,
    build: F,
) -> bool {
    let full = ui.ctx().screen_rect();
    let painter = ui.painter();
    painter.rect_filled(full, 0.0, th.scrim);

    let height = 196.0;
    let rect = egui::Rect::from_center_size(
        full.center(),
        Vec2::new(width.min(full.width() - 48.0), height),
    );
    // egui has no blur, so the shadow is stacked translucent rects.
    for i in (0..6).rev() {
        painter.rect(
            rect.expand((i as f32) * 2.0),
            Rounding::same(14.0 + i as f32),
            Color32::from_black_alpha(6),
            egui::Stroke::NONE,
        );
    }
    painter.rect(rect, Rounding::same(14.0), th.paper_raised, egui::Stroke::NONE);
    painter.rect(rect, Rounding::same(14.0), th.paper_raised, Stroke::new(1.0, th.rule));

    let mut out = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink(24.0))
            .layout(Layout::top_down(Align::Min)),
    );
    out.add_space(2.0);
    out.label(RichText::new(title).font(th.styles.heading.clone()).color(th.ink));
    out.add_space(6.0);
    out.add(egui::Label::new(RichText::new(body).size(12.5).color(th.ink_soft)).wrap());
    out.add_space(16.0);
    let mut confirmed = false;
    out.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let b = if confirm_danger {
            danger_button(ui, th, confirm_label)
        } else {
            primary(ui, th, confirm_label, None)
        };
        confirmed = b.clicked();
        ghost(ui, th, cancel_label);
    });
    out.add_space(14.0);
    build(&mut out);
    confirmed
}

/// Transient confirmation, bottom-centre. egui has no timers, so the caller
/// owns the deadline and passes `None` once it should vanish.
pub fn toast(ui: &mut Ui, th: &Theme, text: &str) {
    let galley = ui.painter().layout_no_wrap(
        text.to_string().into(),
        th.styles.small.clone(),
        Color32::PLACEHOLDER,
    );
    let w = galley.size().x + 52.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 32.0), Sense::hover());
    let c = rect.center();
    ui.painter().rect(rect, Rounding::same(16.0), th.ink, egui::Stroke::NONE);
    ui.painter().rect(rect, Rounding::same(16.0), th.ink, Stroke::new(1.0, Color32::from_black_alpha(40)));
    ui.painter().circle_filled(egui::pos2(c.x - w / 2.0 + 16.0, c.y), 7.0, th.accent.gamma_multiply(1.5));
    ui.painter().text(
        egui::pos2(c.x + 6.0, c.y),
        egui::Align2::LEFT_CENTER,
        text.to_string(),
        th.styles.small.clone(),
        Color32::from_white_alpha(235),
    );
}

/// Footer strip. Contents are chosen per view from live bindings.
pub fn hint_bar(ui: &mut Ui, th: &Theme, hints: &[(&str, &str)]) {
    let height = 34.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    ui.painter().rect(rect, Rounding::ZERO, th.wash, egui::Stroke::NONE);
    ui.painter()
        .rect(rect, Rounding::ZERO, th.wash, Stroke::new(1.0, th.rule));
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink(10.0))
            .layout(Layout::left_to_right(Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = 14.0;
    for (binding, label) in hints {
        inner.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            kbd(ui, th, binding, false);
            ui.label(RichText::new(*label).size(10.5).color(th.ink_faint));
        });
    }
}

/// A drop target used for drag-to-reorder.
pub fn card(ui: &mut Ui, th: &Theme, selected: bool, hovered: bool, dragging: bool) -> Response {
    let bg = if selected {
        th.paper_raised
    } else if hovered {
        th.wash
    } else {
        th.paper_raised
    };
    let stroke = if selected || hovered {
        th.accent
    } else {
        th.rule
    };
    let alpha = if dragging { 0.45 } else { 1.0 };
    ui.add(
        egui::Button::new("")
            .fill(bg.gamma_multiply(alpha))
            .stroke(Stroke::new(if selected { 1.4 } else { 1.0 }, stroke))
            .rounding(9.0)
            .sense(Sense::click()),
    )
}

/// Centred text-align helper for the two-line captions.
pub fn caption(ui: &mut Ui, th: &Theme, text: &str) {
    ui.add(
        egui::Label::new(RichText::new(text).size(12.5).color(th.ink_soft)).wrap(),
    );
}
