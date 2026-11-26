use crate::sensors::Kind;
use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, Visuals};
use std::sync::Arc;

pub const BACK: Color32 = Color32::from_rgb(17, 19, 23);
pub const CARD: Color32 = Color32::from_rgb(26, 29, 35);
pub const ROW: Color32 = Color32::from_rgb(33, 37, 45);
pub const LINE: Color32 = Color32::from_rgb(44, 48, 58);
pub const TEXT: Color32 = Color32::from_rgb(228, 230, 235);
pub const MUTED: Color32 = Color32::from_rgb(139, 145, 156);
pub const FAINT: Color32 = Color32::from_rgb(98, 104, 116);
pub const ACCENT: Color32 = Color32::from_rgb(122, 162, 247);
pub const POSITIVE: Color32 = Color32::from_rgb(158, 206, 106);
pub const NEGATIVE: Color32 = Color32::from_rgb(247, 118, 142);
pub const WARNING: Color32 = Color32::from_rgb(224, 175, 104);
pub const INK: Color32 = Color32::from_rgb(17, 19, 23);

const SEMI: &str = "semibold";

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn semi(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMI.into()))
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

pub fn kind_color(kind: Kind) -> Color32 {
    match kind {
        Kind::Cpu => Color32::from_rgb(255, 158, 100),
        Kind::Gpu => Color32::from_rgb(187, 154, 247),
        Kind::Memory => Color32::from_rgb(125, 207, 255),
        Kind::Storage => Color32::from_rgb(115, 218, 202),
        Kind::Board => Color32::from_rgb(154, 165, 206),
        Kind::Calibration => MUTED,
    }
}

pub fn kind_short(kind: Kind) -> &'static str {
    match kind {
        Kind::Cpu => "CPU",
        Kind::Gpu => "GPU",
        Kind::Memory => "RAM",
        Kind::Storage => "SSD",
        Kind::Board => "MB",
        Kind::Calibration => "±",
    }
}

pub fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Cpu => "CPU",
        Kind::Gpu => "GPU",
        Kind::Memory => "Memory",
        Kind::Storage => "Storage",
        Kind::Board => "Board",
        Kind::Calibration => "Offset",
    }
}

fn system_font(file: &str) -> Option<Arc<FontData>> {
    let dir = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| r"C:\Windows".into());
    std::fs::read(dir.join("Fonts").join(file)).ok().map(|b| Arc::new(FontData::from_owned(b)))
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut plain_fonts = Vec::new();
    let mut semi_fonts = Vec::new();

    // Segoe UI is already on every Windows box, no reason to ship it
    for (key, file, list) in [("segoe", "segoeui.ttf", &mut plain_fonts), ("segoe-semibold", "seguisb.ttf", &mut semi_fonts)] {
        if let Some(data) = system_font(file) {
            fonts.font_data.insert(key.into(), data);
            list.push(key.to_string());
        }
    }
    if let Some(data) = system_font("seguisym.ttf") {
        fonts.font_data.insert("segoe-symbol".into(), data);
        plain_fonts.push("segoe-symbol".into());
        semi_fonts.push("segoe-symbol".into());
    }

    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    for (i, name) in plain_fonts.iter().enumerate() {
        proportional.insert(i, name.clone());
    }
    let fallback = proportional.clone();
    let mut named = semi_fonts;
    named.extend(fallback);
    fonts.families.insert(FontFamily::Name(SEMI.into()), named);
    ctx.set_fonts(fonts);

    let mut v = Visuals::dark();
    v.panel_fill = BACK;
    v.window_fill = CARD;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.extreme_bg_color = ROW;
    v.faint_bg_color = ROW;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, INK);
    v.hyperlink_color = ACCENT;
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.bg_stroke = Stroke::new(1.0, LINE);
        w.corner_radius = 8.into();
    }
    v.widgets.inactive.weak_bg_fill = ROW;
    v.widgets.inactive.bg_fill = ROW;
    v.widgets.hovered.weak_bg_fill = mix(ROW, Color32::WHITE, 0.1);
    v.widgets.hovered.bg_fill = mix(ROW, Color32::WHITE, 0.1);
    v.widgets.active.weak_bg_fill = mix(ROW, Color32::WHITE, 0.16);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    ctx.set_visuals(v);

    ctx.global_style_mut(|s| {
        s.text_styles.insert(egui::TextStyle::Body, regular(14.0));
        s.text_styles.insert(egui::TextStyle::Button, semi(14.0));
        s.text_styles.insert(egui::TextStyle::Heading, semi(18.0));
        s.text_styles.insert(egui::TextStyle::Small, regular(12.0));
        s.spacing.item_spacing = egui::vec2(10.0, 8.0);
        s.spacing.button_padding = egui::vec2(14.0, 6.0);
    });
}
