use egui::{Color32, FontFamily, FontId, Stroke, TextStyle};
use std::path::PathBuf;
use std::sync::Arc;

use crate::config::{parse_colour, Config};

/// Custom font families. egui ships Proportional and Monospace only, so the
/// serif used for display headings is registered by name.
pub fn serif() -> FontFamily {
    FontFamily::Name(Arc::from("batnotes-serif"))
}

pub fn mono() -> FontFamily {
    FontFamily::Name(Arc::from("batnotes-mono"))
}

/// Resolved colours, built once at startup from `Config::palette` so the whole
/// app can be retinted without touching code.
#[derive(Clone)]
pub struct Theme {
    pub paper: Color32,
    pub paper_raised: Color32,
    pub ink: Color32,
    pub ink_soft: Color32,
    pub ink_faint: Color32,
    pub accent: Color32,
    pub clay: Color32,
    pub rule: Color32,
    pub wash: Color32,
    pub danger: Color32,
    pub scrim: Color32,
    pub styles: Styles,
}

/// Type scale, in points.
#[derive(Clone)]
pub struct Styles {
    pub display: FontId,
    pub title: FontId,
    pub heading: FontId,
    pub body: FontId,
    pub small: FontId,
    pub tiny: FontId,
    pub kbd: FontId,
    pub mono: FontId,
}

impl Default for Styles {
    fn default() -> Self {
        Styles {
            display: FontId::new(38.0, serif()),
            title: FontId::new(26.0, serif()),
            heading: FontId::new(22.0, serif()),
            body: FontId::new(13.5, FontFamily::Proportional),
            small: FontId::new(11.5, FontFamily::Proportional),
            tiny: FontId::new(10.5, FontFamily::Proportional),
            kbd: FontId::new(11.0, FontFamily::Proportional),
            mono: FontId::new(10.5, mono()),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::from_config(&Config::default())
    }
}

impl Theme {
    pub fn from_config(cfg: &Config) -> Theme {
        let p = &cfg.palette;
        let c = |s: &str, fallback: Color32| parse_colour(s).unwrap_or(fallback);
        Theme {
            paper: c(&p.paper, Color32::from_rgb(0xfd, 0xfd, 0xf9)),
            paper_raised: c(&p.paper_raised, Color32::WHITE),
            ink: c(&p.ink, Color32::from_rgb(0x25, 0x3d, 0x31)),
            ink_soft: c(&p.ink_soft, Color32::from_rgb(0x7b, 0x81, 0x79)),
            ink_faint: c(&p.ink_faint, Color32::from_rgb(0xa6, 0xaf, 0xa4)),
            accent: c(&p.accent, Color32::from_rgb(0x4b, 0x7a, 0x5c)),
            clay: c(&p.clay, Color32::from_rgb(0xc2, 0x8b, 0x69)),
            rule: c(&p.rule, Color32::from_rgb(0xe4, 0xe9, 0xe0)),
            wash: c(&p.wash, Color32::from_rgb(0xf4, 0xf7, 0xf1)),
            danger: c(&p.danger, Color32::from_rgb(0xaa, 0x5e, 0x52)),
            scrim: Color32::from_black_alpha(56),
            styles: Styles::default(),
        }
    }

    pub fn apply(&self, ctx: &egui::Context) {
        let mut style = (*ctx.style()).clone();

        style.visuals.panel_fill = self.paper;
        style.visuals.window_fill = self.paper;
        style.visuals.extreme_bg_color = self.paper;
        style.visuals.faint_bg_color = self.wash;
        style.visuals.code_bg_color = self.wash;
        style.visuals.dark_mode = false;

        style.visuals.widgets.inactive.bg_fill = self.paper_raised;
        style.visuals.widgets.inactive.weak_bg_fill = self.paper_raised;
        style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, self.rule);
        style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, self.ink_soft);
        style.visuals.widgets.inactive.rounding = egui::Rounding::same(7.0);

        style.visuals.widgets.hovered.bg_fill = self.wash;
        style.visuals.widgets.hovered.weak_bg_fill = self.wash;
        style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, self.accent);
        style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, self.ink);
        style.visuals.widgets.hovered.rounding = egui::Rounding::same(7.0);

        style.visuals.widgets.active.bg_fill = self.accent;
        style.visuals.widgets.active.weak_bg_fill = self.accent;
        style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, self.accent);
        style.visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
        style.visuals.widgets.active.rounding = egui::Rounding::same(7.0);

        style.visuals.widgets.open.bg_fill = self.wash;
        style.visuals.widgets.open.weak_bg_fill = self.wash;
        style.visuals.widgets.open.fg_stroke = Stroke::new(1.0, self.ink);
        style.visuals.widgets.open.rounding = egui::Rounding::same(7.0);

        style.visuals.selection.bg_fill = self.accent.gamma_multiply(0.20);
        style.visuals.selection.stroke = Stroke::new(1.0, self.accent);
        style.visuals.hyperlink_color = self.accent;
        style.visuals.error_fg_color = self.danger;
        style.visuals.warn_fg_color = self.clay;

        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 7.0);
        style.spacing.window_margin = egui::Margin::same(0.0);
        style.spacing.menu_margin = egui::Margin::same(6.0);
        style.animation_time = 0.16;
        style.interaction.selectable_labels = false;
        style.interaction.tooltip_delay = 0.3;

        style.text_styles.insert(TextStyle::Heading, self.styles.heading.clone());
        style.text_styles.insert(TextStyle::Body, self.styles.body.clone());
        style.text_styles.insert(TextStyle::Button, self.styles.body.clone());
        style.text_styles.insert(TextStyle::Small, self.styles.small.clone());
        style.text_styles.insert(TextStyle::Monospace, self.styles.mono.clone());

        ctx.set_style(style);
    }
}

/// Which families were actually loaded, so the UI can tell the truth about
/// typography instead of silently falling back.
pub struct Fonts {
    pub serif_loaded: bool,
    pub hebrew_loaded: bool,
}

impl Fonts {
    /// Build the font set: the default proportional stack, a serif family for
    /// display headings, a monospace family for paths, and Hebrew registered as
    /// a fallback in both so no glyph silently disappears.
    pub fn load(
        ctx: &egui::Context,
        heading_override: Option<PathBuf>,
        mono_override: Option<PathBuf>,
    ) -> Fonts {
        let mut defs = egui::FontDefinitions::default();
        let mut serif_loaded = false;
        let mut hebrew_loaded = false;

        let mut serif_stack: Vec<String> = Vec::new();
        if let Some(p) = heading_override.or_else(|| find_font(&["NotoSerif-Regular", "DejaVuSerif"])) {
            if let Some(name) = insert(&mut defs, &p, "serif-face") {
                serif_stack.push(name);
                serif_loaded = true;
            }
        }

        if let Some(p) = find_font(&["NotoSansHebrew-Regular", "NotoSansHebrew"]) {
            if let Some(name) = insert(&mut defs, &p, "hebrew-face") {
                hebrew_loaded = true;
                serif_stack.push(name.clone());
                let prop = defs.families.entry(FontFamily::Proportional).or_default();
                if !prop.contains(&name) {
                    prop.push(name);
                }
            }
        }

        if !serif_stack.is_empty() {
            defs.families.insert(serif(), serif_stack);
        }

        if let Some(p) = mono_override.or_else(|| find_font(&["NotoSansMono-Regular", "DejaVuSansMono"])) {
            if let Some(name) = insert(&mut defs, &p, "mono-face") {
                defs.families.insert(mono(), vec![name]);
            }
        }

        ctx.set_fonts(defs);
        Fonts { serif_loaded, hebrew_loaded }
    }
}

fn insert(defs: &mut egui::FontDefinitions, path: &PathBuf, name: &str) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    defs.font_data
        .insert(name.to_string(), egui::FontData::from_owned(bytes));
    Some(name.to_string())
}

/// Search the usual font directories for the first family that matches. NixOS
/// keeps fonts under hashed store prefixes, so there is a /nix/store sweep too.
pub fn find_font(want: &[&str]) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(d) = std::env::var("XDG_DATA_DIRS") {
        roots.extend(d.split(':').map(PathBuf::from));
    }
    if let Some(d) = dirs::data_dir() {
        roots.push(d);
    }
    roots.push(PathBuf::from("/run/current-system/sw/share/fonts"));
    roots.push(PathBuf::from("/usr/share/fonts"));

    for root in roots {
        if root.exists() {
            if let Some(hit) = search_dir(&root, want, 6000) {
                return Some(hit);
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir("/nix/store") {
        let mut dirs: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let n = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                n.contains("noto-fonts") || n.contains("dejavu") || n.contains("liberation")
            })
            .collect();
        dirs.sort();
        for d in dirs {
            let share = d.join("share/fonts");
            if share.exists() {
                if let Some(hit) = search_dir(&share, want, 6000) {
                    return Some(hit);
                }
            }
        }
    }
    None
}

fn search_dir(dir: &PathBuf, want: &[&str], budget: i32) -> Option<PathBuf> {
    let mut stack = vec![dir.clone()];
    let mut budget = budget;
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            if budget <= 0 {
                return None;
            }
            budget -= 1;
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                if want.iter().any(|w| stem.eq_ignore_ascii_case(w)) {
                    return Some(p);
                }
            }
        }
    }
    None
}