//! egui style built from `spec/ui_theme.toml`: the Windows light system theme.

use egui::{Color32, CornerRadius, FontId, Stroke, TextStyle, Theme, vec2};
use mt_spec::{Rgb, UiTheme};

pub(crate) fn rgb(c: Rgb) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Colors and sizes resolved once from the spec.
#[derive(Debug, Clone)]
pub struct Theme32 {
    pub window: Color32,
    pub menu_bar: Color32,
    pub toolbar: Color32,
    pub text: Color32,
    pub text_disabled: Color32,
    pub hover: Color32,
    pub hover_border: Color32,
    pub pressed: Color32,
    pub pressed_border: Color32,
    pub menu_highlight: Color32,
    pub separator: Color32,
    pub border: Color32,
    pub tray: Color32,
    pub tray_title: Color32,
    pub panel_header: Color32,
    pub panel_body: Color32,
    pub status_bar: Color32,
    pub field: Color32,
    pub selection: Color32,
    pub sizes: mt_spec::UiSizes,
}

impl Theme32 {
    pub fn from_spec(t: &UiTheme) -> Self {
        let c = &t.colors;
        Self {
            window: rgb(c.window),
            menu_bar: rgb(c.menu_bar),
            toolbar: rgb(c.toolbar),
            text: rgb(c.text),
            text_disabled: rgb(c.text_disabled),
            hover: rgb(c.hover),
            hover_border: rgb(c.hover_border),
            pressed: rgb(c.pressed),
            pressed_border: rgb(c.pressed_border),
            menu_highlight: rgb(c.menu_highlight),
            separator: rgb(c.separator),
            border: rgb(c.border),
            tray: rgb(c.tray),
            tray_title: rgb(c.tray_title),
            panel_header: rgb(c.panel_header),
            panel_body: rgb(c.panel_body),
            status_bar: rgb(c.status_bar),
            field: rgb(c.field),
            selection: rgb(c.selection),
            sizes: t.sizes.clone(),
        }
    }
}

/// Switches `ctx` to the light theme and restyles it like SketchUp on Windows:
/// light greys, flat square widgets, small spacing, 12 px text.
pub fn apply(ctx: &egui::Context, theme: &UiTheme) {
    let t = Theme32::from_spec(theme);
    ctx.set_theme(Theme::Light);
    ctx.style_mut_of(Theme::Light, |style| {
        let font = t.sizes.font;
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(font - 2.0)),
            (TextStyle::Body, FontId::proportional(font)),
            (TextStyle::Button, FontId::proportional(font)),
            (TextStyle::Heading, FontId::proportional(font + 3.0)),
            (TextStyle::Monospace, FontId::monospace(font)),
        ]
        .into();

        let s = &mut style.spacing;
        s.item_spacing = vec2(6.0, 3.0);
        s.button_padding = vec2(6.0, 2.0);
        s.menu_margin = egui::Margin::same(2);
        s.interact_size = vec2(20.0, 18.0);
        s.window_margin = egui::Margin::same(4);
        s.indent = 14.0;

        let v = &mut style.visuals;
        v.dark_mode = false;
        v.override_text_color = None;
        v.panel_fill = t.window;
        v.window_fill = t.menu_bar;
        v.window_stroke = Stroke::new(1.0, t.border);
        v.window_corner_radius = CornerRadius::ZERO;
        v.menu_corner_radius = CornerRadius::ZERO;
        v.window_shadow = egui::Shadow {
            offset: [2, 2],
            blur: 4,
            spread: 0,
            color: Color32::from_black_alpha(40),
        };
        v.popup_shadow = v.window_shadow;
        v.extreme_bg_color = t.field;
        v.text_edit_bg_color = Some(t.field);
        v.faint_bg_color = t.window;
        v.selection.bg_fill = t.selection;
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        v.collapsing_header_frame = false;
        v.indent_has_left_vline = false;
        v.button_frame = true;
        v.interact_cursor = None;

        let w = &mut v.widgets;
        for wv in [
            &mut w.noninteractive,
            &mut w.inactive,
            &mut w.hovered,
            &mut w.active,
            &mut w.open,
        ] {
            wv.corner_radius = CornerRadius::ZERO;
            wv.expansion = 0.0;
            wv.fg_stroke = Stroke::new(1.0, t.text);
        }
        w.noninteractive.bg_fill = t.window;
        w.noninteractive.weak_bg_fill = t.window;
        w.noninteractive.bg_stroke = Stroke::new(1.0, t.separator);
        w.inactive.bg_fill = t.field;
        w.inactive.weak_bg_fill = Color32::TRANSPARENT;
        w.inactive.bg_stroke = Stroke::NONE;
        w.hovered.bg_fill = t.hover;
        w.hovered.weak_bg_fill = t.hover;
        w.hovered.bg_stroke = Stroke::new(1.0, t.hover_border);
        w.active.bg_fill = t.pressed;
        w.active.weak_bg_fill = t.pressed;
        w.active.bg_stroke = Stroke::new(1.0, t.pressed_border);
        w.open.bg_fill = t.menu_highlight;
        w.open.weak_bg_fill = t.menu_highlight;
        w.open.bg_stroke = Stroke::NONE;
    });
}
