//! Styles panel extras (FEATURES.md item 65) in the app: the model's
//! [`mt_kernel::style_extras`] settings shown in the Styles panel, applied from its
//! events and passed to the renderer. Edge colors and modeling colors are used where
//! the scene is built (`document.rs`) and where selections are highlighted.

use std::sync::Arc;

use glam::DVec3;
use mt_kernel::Model;
use mt_kernel::style_extras::{EdgeColorMode, TransparencyQuality, Watermark, WatermarkMode};
use mt_ui::{Event, StyleExtras, WatermarkRow};

use crate::App;

fn rgb(c: [u8; 3]) -> [f32; 3] {
    c.map(|x| f32::from(x) / 255.0)
}

/// sRGB 0..1 with alpha 1.
pub(crate) fn rgba(c: [u8; 3]) -> [f32; 4] {
    let [r, g, b] = rgb(c);
    [r, g, b, 1.0]
}

/// The positioned watermark anchors of [`mt_ui::WATERMARK_ANCHORS`] (row by row from
/// the top left) as 0..1 across and down.
fn anchor_of(index: usize) -> [f32; 2] {
    let i = index.min(8);
    [(i % 3) as f32 / 2.0, (i / 3) as f32 / 2.0]
}

fn anchor_index(a: [f32; 2]) -> usize {
    let k = |x: f32| ((x.clamp(0.0, 1.0) * 2.0).round() as usize).min(2);
    k(a[1]) * 3 + k(a[0])
}

/// The Styles panel row of a watermark.
pub(crate) fn watermark_row(w: &Watermark) -> WatermarkRow {
    let (mode, lock_aspect, scale, anchor) = match w.mode {
        WatermarkMode::Stretched { lock_aspect } => (0, lock_aspect, 1.0, 4),
        WatermarkMode::Tiled { scale } => (1, true, scale, 4),
        WatermarkMode::Positioned { anchor, scale } => (2, true, scale, anchor_index(anchor)),
    };
    WatermarkRow {
        name: w.name.clone(),
        overlay: w.overlay,
        opacity: (w.opacity.clamp(0.0, 1.0) * 100.0).round() as u8,
        mode,
        lock_aspect,
        scale: (scale * 100.0).round().clamp(10.0, 400.0) as u16,
        anchor,
        mask: w.mask,
    }
}

/// Applies a panel row's settings to a watermark (its image stays).
pub(crate) fn apply_watermark_row(w: &mut Watermark, r: &WatermarkRow) {
    w.overlay = r.overlay;
    w.mask = r.mask;
    w.opacity = f32::from(r.opacity.min(100)) / 100.0;
    let scale = f32::from(r.scale.clamp(10, 400)) / 100.0;
    w.mode = match r.mode {
        0 => WatermarkMode::Stretched {
            lock_aspect: r.lock_aspect,
        },
        1 => WatermarkMode::Tiled { scale },
        _ => WatermarkMode::Positioned {
            anchor: anchor_of(r.anchor),
            scale,
        },
    };
}

/// The panel's extras from the model.
pub(crate) fn tray_extras(m: &Model) -> (StyleExtras, Vec<WatermarkRow>) {
    let s = &m.style;
    let e = &s.edge_look;
    let bg = &s.background;
    let ao = &s.ambient_occlusion;
    let mc = &s.modeling_colors;
    let percent = |x: f32| (x.clamp(0.0, 1.0) * 100.0).round() as u8;
    let extras = StyleExtras {
        edge_color_mode: EdgeColorMode::ALL
            .iter()
            .position(|&x| x == e.color_mode)
            .unwrap_or(0),
        edge_color: e.color,
        endpoints: e.endpoints,
        endpoint_length: e.endpoint_length.round().clamp(1.0, 99.0) as u8,
        jitter: e.jitter,
        front_color: m.front_color,
        back_color: m.back_color,
        transparency: !s.face_look.transparency_off,
        nicer: s.face_look.quality == TransparencyQuality::Nicer,
        ambient_occlusion: ao.enabled,
        ao_intensity: percent(ao.intensity),
        ao_distance: ao.distance.round().clamp(1.0, 9999.0) as u16,
        ao_use_color: ao.use_color,
        ao_color: ao.color,
        background: bg.color,
        sky: bg.sky,
        sky_color: bg.sky_color,
        horizon_color: bg.horizon_color,
        // Panel 1..100 is the sky gradient exponent 0.01..1 (50: the default 0.5).
        horizon_blend: (bg.horizon_blend * 100.0).round().clamp(1.0, 100.0) as u8,
        ground: bg.ground,
        ground_color: bg.ground_color,
        ground_transparency: percent(bg.ground_transparency),
        ground_from_below: bg.ground_from_below,
        selected: mc.selected,
        locked: mc.locked,
        guides: mc.guides,
        section_active: mc.section_active,
        section_inactive: mc.section_inactive,
        section_cut: mc.section_cut,
        hidden_geometry: s.modeling_extras.hidden_geometry,
        inference: {
            let i = &s.modeling_extras.inference;
            [i.endpoint, i.midpoint, i.on_edge, i.on_face]
        },
    };
    (extras, s.watermarks.iter().map(watermark_row).collect())
}

/// Applies the panel's extras to the model.
pub(crate) fn apply_extras(m: &mut Model, x: &StyleExtras) {
    m.front_color = x.front_color;
    m.back_color = x.back_color;
    let s = &mut m.style;
    let e = &mut s.edge_look;
    e.color_mode = EdgeColorMode::ALL[x.edge_color_mode.min(2)];
    e.color = x.edge_color;
    e.endpoints = x.endpoints;
    e.endpoint_length = f32::from(x.endpoint_length.max(1));
    e.jitter = x.jitter;
    s.face_look.transparency_off = !x.transparency;
    s.face_look.quality = if x.nicer {
        TransparencyQuality::Nicer
    } else {
        TransparencyQuality::Faster
    };
    let ao = &mut s.ambient_occlusion;
    ao.enabled = x.ambient_occlusion;
    ao.intensity = f32::from(x.ao_intensity.min(100)) / 100.0;
    ao.distance = f32::from(x.ao_distance.max(1));
    ao.use_color = x.ao_use_color;
    ao.color = x.ao_color;
    let bg = &mut s.background;
    bg.color = x.background;
    bg.sky = x.sky;
    bg.sky_color = x.sky_color;
    bg.horizon_color = x.horizon_color;
    bg.horizon_blend = f32::from(x.horizon_blend.clamp(1, 100)) / 100.0;
    bg.ground = x.ground;
    bg.ground_color = x.ground_color;
    bg.ground_transparency = f32::from(x.ground_transparency.min(100)) / 100.0;
    bg.ground_from_below = x.ground_from_below;
    let mc = &mut s.modeling_colors;
    mc.selected = x.selected;
    mc.locked = x.locked;
    mc.guides = x.guides;
    mc.section_active = x.section_active;
    mc.section_inactive = x.section_inactive;
    mc.section_cut = x.section_cut;
    let me = &mut s.modeling_extras;
    me.hidden_geometry = x.hidden_geometry;
    [
        me.inference.endpoint,
        me.inference.midpoint,
        me.inference.on_edge,
        me.inference.on_face,
    ] = x.inference;
}

/// The renderer's style from the model's extras (on top of the spec defaults and the
/// View-menu settings already in `style`).
pub(crate) fn apply_render(m: &Model, style: &mut mt_render::Style) {
    let s = &m.style;
    let bg = &s.background;
    style.background = rgb(bg.color);
    style.sky = bg.sky.then(|| (rgb(bg.sky_color), rgb(bg.horizon_color)));
    style.horizon_blend = bg.horizon_blend.max(0.01);
    style.ground = bg.ground.then(|| rgb(bg.ground_color));
    style.ground_opacity = 1.0 - bg.ground_transparency.clamp(0.0, 1.0);
    style.ground_from_below = bg.ground_from_below;
    let e = &s.edge_look;
    style.endpoints = e.endpoints.then_some(e.endpoint_length);
    style.jitter = e.jitter;
    style.transparency = match (s.face_look.transparency_off, s.face_look.quality) {
        (true, _) => mt_render::Transparency::Off,
        (false, TransparencyQuality::Faster) => mt_render::Transparency::Faster,
        (false, TransparencyQuality::Nicer) => mt_render::Transparency::Nicer,
    };
    let ao = &s.ambient_occlusion;
    style.ambient_occlusion = ao.enabled.then(|| mt_render::AmbientOcclusion {
        intensity: ao.intensity,
        distance: ao.distance,
        color: if ao.use_color {
            rgb(ao.color)
        } else {
            [0.0; 3]
        },
    });
    style.watermarks = s
        .watermarks
        .iter()
        .map(|w| mt_render::Watermark {
            image: w.image.clone(),
            overlay: w.overlay,
            opacity: w.opacity,
            // A mask shows the background color.
            mask: w.mask.then_some(bg.color),
            placement: match w.mode {
                WatermarkMode::Stretched { lock_aspect } => {
                    mt_render::WatermarkPlacement::Stretched { lock_aspect }
                }
                WatermarkMode::Tiled { scale } => mt_render::WatermarkPlacement::Tiled { scale },
                WatermarkMode::Positioned { anchor, scale } => {
                    mt_render::WatermarkPlacement::Positioned { anchor, scale }
                }
            },
        })
        .collect();
    style.selection_color = rgb(s.modeling_colors.selected);
}

/// Edge Settings > Color for a model edge running along `dir` (world space).
/// `material_color`: the color of the material of the edge's first face (`None`
/// without faces or material; Meshtard edges carry no material of their own). Edges
/// without a material or axis match take the All same color.
pub(crate) fn edge_color(m: &Model, dir: DVec3, material_color: Option<[f32; 4]>) -> [f32; 4] {
    let e = &m.style.edge_look;
    let same = rgba(e.color);
    match e.color_mode {
        EdgeColorMode::AllSame => same,
        EdgeColorMode::ByMaterial => material_color.map_or(same, |c| [c[0], c[1], c[2], 1.0]),
        EdgeColorMode::ByAxis => {
            let d = dir.normalize_or_zero();
            // Drawing axes (Axes tool) in world space; their colors as drawn.
            let axes = [m.axes.x_axis, m.axes.y_axis, m.axes.z_axis]
                .map(|a| a.truncate().normalize_or_zero());
            const COLORS: [[f32; 4]; 3] = [
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 0.7, 0.0, 1.0],
                [0.0, 0.0, 1.0, 1.0],
            ];
            axes.iter()
                .position(|a| a.dot(d).abs() > 1.0 - 1e-9)
                .map_or(same, |k| COLORS[k])
        }
    }
}

impl App {
    /// Styles panel extras and watermark events. Display settings like the rest of
    /// the Styles panel: not undo steps. Returns whether to rebuild the scene.
    pub(crate) fn style_extras_event(&mut self, event: Event) -> bool {
        match event {
            Event::StyleExtrasChanged(x) => {
                let Some(doc) = self.document.as_mut() else {
                    return false;
                };
                apply_extras(&mut doc.model, &x);
                true
            }
            Event::WatermarkAdd => {
                let filters: [(&str, &[&str]); 1] =
                    [("Images (*.png, *.jpg)", &["png", "jpg", "jpeg"])];
                let Some(path) = crate::filedialog::open_in(
                    crate::filedialog::Folder::WatermarkImages,
                    &filters,
                ) else {
                    return false;
                };
                let data = match std::fs::read(&path) {
                    Ok(d) => d,
                    Err(e) => {
                        self.file_error("open", &path, e);
                        return false;
                    }
                };
                if let Err(e) = mt_kernel::images::decode(&data) {
                    self.file_error("open", &path, e);
                    return false;
                }
                let Some(doc) = self.document.as_mut() else {
                    return false;
                };
                let name = path
                    .file_name()
                    .map_or_else(|| "Watermark".into(), |n| n.to_string_lossy().into_owned());
                // SketchUp's wizard asks background or overlay, blend and placement;
                // here a new watermark starts as a half-blended stretched background
                // (edited on the page afterwards).
                doc.model.style.watermarks.push(Watermark {
                    name,
                    image: Arc::from(data),
                    overlay: false,
                    opacity: 0.5,
                    mode: WatermarkMode::Stretched { lock_aspect: true },
                    mask: false,
                });
                true
            }
            Event::WatermarkMove(i, up) => {
                let Some(doc) = self.document.as_mut() else {
                    return false;
                };
                Watermark::reorder(&mut doc.model.style.watermarks, i, up).is_some()
            }
            Event::WatermarkRemove(i) => {
                let Some(doc) = self.document.as_mut() else {
                    return false;
                };
                if i < doc.model.style.watermarks.len() {
                    doc.model.style.watermarks.remove(i);
                    return true;
                }
                false
            }
            Event::WatermarkChanged(i, row) => {
                let Some(doc) = self.document.as_mut() else {
                    return false;
                };
                match doc.model.style.watermarks.get_mut(i) {
                    Some(w) => {
                        apply_watermark_row(w, &row);
                        true
                    }
                    None => false,
                }
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_round_trip_and_render_style() {
        let mut m = Model::default();
        let (mut x, _) = tray_extras(&m);
        assert_eq!(
            x,
            StyleExtras::default(),
            "panel defaults match the model's"
        );
        x.edge_color_mode = 2;
        x.ground_transparency = 40;
        x.sky = false;
        x.ambient_occlusion = true;
        x.transparency = false;
        x.front_color = [1, 2, 3];
        x.hidden_geometry = [9, 8, 7];
        x.inference[2] = [1, 1, 1];
        apply_extras(&mut m, &x);
        assert_eq!(m.style.modeling_extras.hidden_geometry, [9, 8, 7]);
        assert_eq!(m.style.modeling_extras.inference.on_edge, [1, 1, 1]);
        assert_eq!(tray_extras(&m).0, x);
        assert_eq!(m.front_color, [1, 2, 3]);
        let spec = mt_spec::Spec::builtin().unwrap().default_model;
        let mut style = mt_render::Style::from_spec(&spec);
        apply_render(&m, &mut style);
        assert!(style.sky.is_none());
        assert!((style.ground_opacity - 0.6).abs() < 1e-6);
        assert!(style.ambient_occlusion.is_some());
        assert_eq!(style.transparency, mt_render::Transparency::Off);
    }

    #[test]
    fn edge_colors_by_axis_and_material() {
        let mut m = Model::default();
        let red = [0.5, 0.1, 0.1, 1.0];
        assert_eq!(edge_color(&m, DVec3::X, Some(red)), [0.0, 0.0, 0.0, 1.0]);
        m.style.edge_look.color_mode = EdgeColorMode::ByAxis;
        assert_eq!(
            edge_color(&m, DVec3::NEG_Y * 3.0, None),
            [0.0, 0.7, 0.0, 1.0]
        );
        assert_eq!(
            edge_color(&m, DVec3::new(1.0, 1.0, 0.0), None),
            [0.0, 0.0, 0.0, 1.0]
        );
        m.style.edge_look.color_mode = EdgeColorMode::ByMaterial;
        assert_eq!(edge_color(&m, DVec3::X, Some(red)), red);
    }

    #[test]
    fn watermark_rows_round_trip() {
        let mut w = Watermark {
            name: "logo.png".into(),
            image: Arc::from(&b"x"[..]),
            overlay: true,
            opacity: 0.3,
            mode: WatermarkMode::Positioned {
                anchor: [1.0, 1.0],
                scale: 2.0,
            },
            mask: false,
        };
        let row = watermark_row(&w);
        assert_eq!(
            (row.mode, row.anchor, row.scale, row.opacity),
            (2, 8, 200, 30)
        );
        let mut r2 = row.clone();
        r2.mode = 1;
        r2.mask = true;
        apply_watermark_row(&mut w, &r2);
        assert_eq!(w.mode, WatermarkMode::Tiled { scale: 2.0 });
        assert!(w.mask && watermark_row(&w).mask);
        assert_eq!(watermark_row(&w).mode, 1);
    }
}
