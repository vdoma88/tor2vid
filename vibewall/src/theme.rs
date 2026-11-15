//! Оформление по DESIGN.md (Linear): почти чёрный холст, лестница
//! поверхностей, волосяные рамки, один акцент — лавандовый.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke,
    TextStyle,
};

pub const CANVAS: Color32 = Color32::from_rgb(0x01, 0x01, 0x02);
pub const SURFACE_1: Color32 = Color32::from_rgb(0x0f, 0x10, 0x11);
pub const SURFACE_2: Color32 = Color32::from_rgb(0x14, 0x15, 0x16);
pub const SURFACE_3: Color32 = Color32::from_rgb(0x18, 0x19, 0x1a);
pub const HAIRLINE: Color32 = Color32::from_rgb(0x23, 0x25, 0x2a);
pub const HAIRLINE_STRONG: Color32 = Color32::from_rgb(0x34, 0x34, 0x3a);
pub const INK: Color32 = Color32::from_rgb(0xf7, 0xf8, 0xf8);
pub const INK_MUTED: Color32 = Color32::from_rgb(0xd0, 0xd6, 0xe0);
pub const INK_SUBTLE: Color32 = Color32::from_rgb(0x8a, 0x8f, 0x98);
pub const INK_TERTIARY: Color32 = Color32::from_rgb(0x62, 0x66, 0x6d);
pub const PRIMARY: Color32 = Color32::from_rgb(0x5e, 0x6a, 0xd2);
pub const PRIMARY_HOVER: Color32 = Color32::from_rgb(0x82, 0x8f, 0xff);
pub const SUCCESS: Color32 = Color32::from_rgb(0x27, 0xa6, 0x44);
pub const DANGER: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);

pub const SEMIBOLD: &str = "semibold";

/// Segoe UI из системы: в нём есть кириллица, и это родной шрифт Windows.
fn load_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    let read = |name: &str| std::fs::read(format!("{dir}\\Fonts\\{name}")).ok();
    if let Some(regular) = read("segoeui.ttf") {
        fonts
            .font_data
            .insert("segoe".into(), FontData::from_owned(regular).into());
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "segoe".into());
    }
    let semibold = read("seguisb.ttf").map(|b| ("segoe-sb", b));
    let mut sb_family = Vec::new();
    if let Some((name, bytes)) = semibold {
        fonts
            .font_data
            .insert(name.into(), FontData::from_owned(bytes).into());
        sb_family.push(name.to_string());
    }
    sb_family.extend(
        fonts
            .families
            .get(&FontFamily::Proportional)
            .cloned()
            .unwrap_or_default(),
    );
    fonts
        .families
        .insert(FontFamily::Name(SEMIBOLD.into()), sb_family);
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context) {
    load_fonts(ctx);
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.text_styles = [
            (
                TextStyle::Heading,
                FontId::new(22.0, FontFamily::Name(SEMIBOLD.into())),
            ),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Small, FontId::proportional(12.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        let s = &mut style.spacing;
        s.item_spacing = egui::vec2(8.0, 8.0);
        s.button_padding = egui::vec2(14.0, 8.0);
        s.interact_size.y = 34.0;
        s.window_margin = Margin::same(24);
        s.slider_width = 140.0;

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.panel_fill = CANVAS;
        v.window_fill = SURFACE_1;
        v.window_stroke = Stroke::new(1.0, HAIRLINE);
        v.extreme_bg_color = SURFACE_1;
        v.faint_bg_color = SURFACE_2;
        v.code_bg_color = SURFACE_2;
        v.hyperlink_color = PRIMARY_HOVER;
        v.override_text_color = None;
        v.selection.bg_fill = PRIMARY.gamma_multiply(0.55);
        v.selection.stroke = Stroke::new(1.0, INK);
        v.window_corner_radius = CornerRadius::same(12);
        v.menu_corner_radius = CornerRadius::same(8);
        v.window_shadow = egui::epaint::Shadow::NONE;
        v.popup_shadow = egui::epaint::Shadow::NONE;
        v.error_fg_color = DANGER;
        v.warn_fg_color = INK_MUTED;

        let r = CornerRadius::same(8);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = SURFACE_1;
        w.noninteractive.weak_bg_fill = SURFACE_1;
        w.noninteractive.bg_stroke = Stroke::new(1.0, HAIRLINE);
        w.noninteractive.fg_stroke = Stroke::new(1.0, INK_MUTED);
        w.noninteractive.corner_radius = r;

        w.inactive.bg_fill = SURFACE_2;
        w.inactive.weak_bg_fill = SURFACE_1;
        w.inactive.bg_stroke = Stroke::new(1.0, HAIRLINE);
        w.inactive.fg_stroke = Stroke::new(1.0, INK);
        w.inactive.corner_radius = r;

        w.hovered.bg_fill = SURFACE_3;
        w.hovered.weak_bg_fill = SURFACE_2;
        w.hovered.bg_stroke = Stroke::new(1.0, HAIRLINE_STRONG);
        w.hovered.fg_stroke = Stroke::new(1.0, INK);
        w.hovered.corner_radius = r;
        w.hovered.expansion = 0.0;

        w.active.bg_fill = SURFACE_3;
        w.active.weak_bg_fill = SURFACE_3;
        w.active.bg_stroke = Stroke::new(1.0, PRIMARY);
        w.active.fg_stroke = Stroke::new(1.0, INK);
        w.active.corner_radius = r;
        w.active.expansion = 0.0;

        w.open = w.active;
    });
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}
