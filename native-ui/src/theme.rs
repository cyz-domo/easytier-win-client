use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Painter,
    Pos2, Rect, Stroke, StrokeKind, Vec2,
};
use std::sync::Arc;

pub const BG: Color32 = Color32::from_rgb(0x17, 0x19, 0x1D);
pub const SIDEBAR: Color32 = Color32::from_rgb(0x1E, 0x21, 0x26);
pub const CARD: Color32 = Color32::from_rgb(0x23, 0x26, 0x2C);
pub const CARD_HI: Color32 = Color32::from_rgb(0x2B, 0x2F, 0x36);
pub const INPUT: Color32 = Color32::from_rgb(0x1A, 0x1D, 0x22);
pub const BORDER: Color32 = Color32::from_rgb(0x32, 0x37, 0x40);
pub const TEXT: Color32 = Color32::from_rgb(0xE8, 0xEA, 0xEE);
pub const MUTED: Color32 = Color32::from_rgb(0x8B, 0x92, 0x9E);
pub const DIM: Color32 = Color32::from_rgb(0x6F, 0x77, 0x83);
pub const OK: Color32 = Color32::from_rgb(0x5A, 0xC8, 0x7E);
pub const RX: Color32 = Color32::from_rgb(0x5B, 0x9B, 0xFF);
pub const TX: Color32 = Color32::from_rgb(0x5A, 0xC8, 0x7E);
pub const WARN: Color32 = Color32::from_rgb(0xE0, 0xA4, 0x58);
pub const ERR: Color32 = Color32::from_rgb(0xE0, 0x6C, 0x75);
pub const ACCENT: Color32 = Color32::from_rgb(0x4C, 0x8D, 0xFF);

pub fn setup(ctx: &egui::Context) -> (bool, egui::Style) {
    let loaded = install_cjk_font(ctx);
    (loaded, build_style())
}

pub fn build_style() -> egui::Style {
    let mut style = egui::Style::default();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = Color32::TRANSPARENT;
    style.visuals.window_fill = BG;
    style.visuals.extreme_bg_color = INPUT;
    style.visuals.faint_bg_color = CARD;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.selection.bg_fill = Color32::from_rgb(0x2C, 0x4A, 0x7A);
    style.visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);

    let w = &mut style.visuals.widgets;
    w.noninteractive.bg_fill = CARD;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.inactive.bg_fill = INPUT;
    w.inactive.weak_bg_fill = CARD_HI;
    w.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    w.hovered.bg_fill = INPUT;
    w.hovered.weak_bg_fill = Color32::from_rgb(0x33, 0x38, 0x40);
    w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x44, 0x4B, 0x57));
    w.active.bg_fill = INPUT;
    w.active.weak_bg_fill = Color32::from_rgb(0x3A, 0x40, 0x49);
    w.open.weak_bg_fill = CARD_HI;

    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(10.0, 5.0);
    style.spacing.interact_size = Vec2::new(36.0, 26.0);
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(13.5));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::proportional(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, FontId::proportional(11.0));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, FontId::proportional(18.0));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, FontId::monospace(12.0));

    style
}

/// Loads a system CJK font so Chinese glyphs render without bundling a font file.
pub fn install_cjk_font(ctx: &egui::Context) -> bool {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_owned());
    let mut fonts = FontDefinitions::default();
    let mut loaded = false;

    for file in ["msyh.ttc", "msyhbd.ttc", "Deng.ttf", "simhei.ttf", "simsun.ttc"] {
        let path = std::path::Path::new(&windir).join("Fonts").join(file);
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let mut data = FontData::from_owned(bytes);
        data.index = 0;
        fonts.font_data.insert("cjk".to_owned(), Arc::new(data));
        // Keep egui's crisp Latin faces first; YaHei only fills CJK glyphs.
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            if let Some(list) = fonts.families.get_mut(&family) {
                list.push("cjk".to_owned());
            }
        }
        loaded = true;
        break;
    }

    ctx.set_fonts(fonts);
    loaded
}

pub fn text_w(s: &str, size: f32) -> f32 {
    s.chars()
        .map(|c| {
            let wide = matches!(c as u32, 0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6);
            if wide {
                size
            } else {
                size * 0.56
            }
        })
        .sum()
}

pub fn txt(
    p: &Painter,
    pos: Pos2,
    anchor: Align2,
    text: impl ToString,
    size: f32,
    color: Color32,
) {
    p.text(pos, anchor, text, FontId::proportional(size), color);
}

pub fn mono(
    p: &Painter,
    pos: Pos2,
    anchor: Align2,
    text: impl ToString,
    size: f32,
    color: Color32,
) {
    p.text(pos, anchor, text, FontId::monospace(size), color);
}

pub fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::same(16))
}

pub fn card(p: &Painter, r: Rect) {
    p.rect_filled(r, CornerRadius::same(10), CARD);
    p.rect_stroke(
        r,
        CornerRadius::same(10),
        Stroke::new(1.0, BORDER),
        StrokeKind::Inside,
    );
}

pub fn pill(p: &Painter, left: Pos2, h: f32, label: &str, fg: Color32, bg: Color32) -> f32 {
    let w = text_w(label, 11.5) + 16.0;
    let r = Rect::from_min_size(left, Vec2::new(w, h));
    p.rect_filled(r, CornerRadius::same((h / 2.0) as u8), bg);
    p.text(
        r.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(11.5),
        fg,
    );
    w
}
