//! File > Export > Section Slice: the active section cut's lines as a 2D DXF drawing.
//!
//! The cut segments ([`mt_render::section_lines`] over the drawn model, the same lines
//! the viewport draws thick) are projected into the section plane, welded and joined
//! where collinear, and written as AutoCAD R12 ASCII `LINE` entities on layer `0`
//! (z = 0). Plane axes: for a horizontal plane the drawing's x and y are the model's
//! red and green axes; otherwise y is the model's blue axis projected into the plane and
//! x is to the right when looking at the cut from the removed side. Coordinates are
//! measured from the model origin projected into the plane.
//!
//! Options, after SketchUp's Section Slice export options (from memory, unverified):
//! "Full scale (1:1)" or a drawing scale ("In drawing" / "In model" lengths), and the
//! units of the drawing's coordinates. DXF R12 has no units header (`$INSUNITS` came
//! with R2000), so the unit and scale are written as a `999` comment at the top. DWG
//! (a closed format) and SketchUp's AutoCAD version choice are not offered.

use eframe::egui;
use glam::{DVec2, DVec3};
use mt_kernel::mesh_export::Units;

use crate::App;

/// Section Slice export options.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SectionOptions {
    pub full_scale: bool,
    /// Drawing scale: `in_drawing` (paper length) equals `in_model` (model length),
    /// both in `units`. Used when not full scale.
    pub in_drawing: f64,
    pub in_model: f64,
    /// Unit of the DXF coordinates; `None` for the model's units.
    pub units: Option<Units>,
}

impl Default for SectionOptions {
    fn default() -> Self {
        Self {
            full_scale: true,
            in_drawing: 1.0,
            in_model: 100.0,
            units: None,
        }
    }
}

impl SectionOptions {
    /// Drawing length per model length (1 at full scale).
    pub(crate) fn factor(&self) -> f64 {
        if self.full_scale || self.in_model <= 0.0 || self.in_drawing <= 0.0 {
            1.0
        } else {
            self.in_drawing / self.in_model
        }
    }
}

/// The plane's 2D axes `(x, y)` (unit, in the plane) for a cut with `normal`.
pub(crate) fn plane_axes(normal: DVec3) -> (DVec3, DVec3) {
    let n = normal.normalize_or(DVec3::Z);
    if n.z.abs() > 0.999 {
        // Horizontal plane: a plan, x and y along the red and green axes whichever
        // way the plane faces.
        return (DVec3::X, DVec3::Y);
    }
    let up = (DVec3::Z - n * n.z).normalize();
    (up.cross(n).normalize(), up)
}

/// Projects 3D cut segments into the plane's 2D coordinates (model inches).
pub(crate) fn project(segments: &[[DVec3; 2]], normal: DVec3) -> Vec<[DVec2; 2]> {
    let (x, y) = plane_axes(normal);
    let f = |p: DVec3| DVec2::new(p.dot(x), p.dot(y));
    segments.iter().map(|[a, b]| [f(*a), f(*b)]).collect()
}

/// Welds ends closer than `tol`, drops zero-length and duplicate segments and joins
/// chains of collinear segments into single lines.
pub(crate) fn merge(segments: &[[DVec2; 2]], tol: f64) -> Vec<[DVec2; 2]> {
    use std::collections::HashMap;
    let key = |p: DVec2| ((p.x / tol).round() as i64, (p.y / tol).round() as i64);
    let mut points: Vec<DVec2> = Vec::new();
    let mut index: HashMap<(i64, i64), usize> = HashMap::new();
    let mut id = |p: DVec2| {
        *index.entry(key(p)).or_insert_with(|| {
            points.push(p);
            points.len() - 1
        })
    };
    let mut edges: Vec<Option<(usize, usize)>> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for [a, b] in segments {
        let (i, j) = (id(*a), id(*b));
        if i == j || !seen.insert((i.min(j), i.max(j))) {
            continue;
        }
        edges.push(Some((i, j)));
    }
    let mut at: Vec<Vec<usize>> = vec![Vec::new(); points.len()];
    for (e, ij) in edges.iter().enumerate() {
        let (i, j) = ij.expect("all set");
        at[i].push(e);
        at[j].push(e);
    }
    // Remove each vertex joining exactly two collinear segments.
    for v in 0..points.len() {
        let live: Vec<usize> = at[v]
            .iter()
            .copied()
            .filter(|&e| edges[e].is_some())
            .collect();
        let [e1, e2] = live[..] else { continue };
        let other = |e: usize| {
            let (i, j) = edges[e].expect("live");
            if i == v { j } else { i }
        };
        let (a, b) = (other(e1), other(e2));
        if a == b {
            continue;
        }
        let (pa, pv, pb) = (points[a], points[v], points[b]);
        let d = pb - pa;
        let len = d.length();
        if len <= tol || (pv - pa).perp_dot(d).abs() / len > tol {
            continue;
        }
        // Collinear and v between a and b.
        if (pv - pa).dot(d) <= 0.0 || (pv - pb).dot(-d) <= 0.0 {
            continue;
        }
        edges[e1] = Some((a, b));
        edges[e2] = None;
        at[b].retain(|&e| e != e2);
        at[b].push(e1);
    }
    edges
        .into_iter()
        .flatten()
        .map(|(i, j)| [points[i], points[j]])
        .collect()
}

/// An R12 ASCII DXF with one `LINE` per segment. `lines` are already in drawing units.
pub(crate) fn to_dxf(lines: &[[DVec2; 2]], comment: &str) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let mut pair = |code: i32, value: &str| {
        let _ = writeln!(s, "{code:>3}\n{value}");
    };
    for line in comment.lines() {
        pair(999, line);
    }
    let (mut min, mut max) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
    for p in lines.iter().flatten() {
        min = min.min(*p);
        max = max.max(*p);
    }
    if !min.is_finite() {
        (min, max) = (DVec2::ZERO, DVec2::ZERO);
    }
    let num = |x: f64| format!("{x:.6}");
    pair(0, "SECTION");
    pair(2, "HEADER");
    pair(9, "$ACADVER");
    pair(1, "AC1009");
    for (name, p) in [("$EXTMIN", min), ("$EXTMAX", max)] {
        pair(9, name);
        pair(10, &num(p.x));
        pair(20, &num(p.y));
        pair(30, &num(0.0));
    }
    pair(0, "ENDSEC");
    pair(0, "SECTION");
    pair(2, "ENTITIES");
    for [a, b] in lines {
        pair(0, "LINE");
        pair(8, "0");
        pair(10, &num(a.x));
        pair(20, &num(a.y));
        pair(30, &num(0.0));
        pair(11, &num(b.x));
        pair(21, &num(b.y));
        pair(31, &num(0.0));
    }
    pair(0, "ENDSEC");
    pair(0, "EOF");
    s
}

/// The whole export: cut segments (world inches) of a plane with `normal` to DXF text.
pub(crate) fn section_dxf(
    segments: &[[DVec3; 2]],
    normal: DVec3,
    opts: &SectionOptions,
    model_units: Units,
) -> String {
    let units = opts.units.unwrap_or(model_units);
    let k = units.per_inch() * opts.factor();
    let lines: Vec<[DVec2; 2]> = merge(&project(segments, normal), 1e-6)
        .into_iter()
        .map(|l| l.map(|p| p * k))
        .collect();
    let scale = if opts.full_scale {
        "1:1 (true size)".to_owned()
    } else {
        format!("{} drawing = {} model", opts.in_drawing, opts.in_model)
    };
    to_dxf(
        &lines,
        &format!(
            "meshtard section slice\nunits: {}\nscale: {scale}",
            units.name().to_lowercase()
        ),
    )
}

impl App {
    pub(crate) fn open_export_section(&mut self) {
        if self.scene.section.is_none() {
            self.message = Some(
                "Export Section Slice needs an active section plane with View > Section Cut on."
                    .into(),
            );
            return;
        }
        self.present.section = Some(self.present.last_section.clone());
    }

    pub(crate) fn export_section_ui(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.present.section.take() else {
            return;
        };
        let model_units = self
            .document
            .as_ref()
            .map_or(Units::Inches, |doc| crate::export::kernel_units(doc.units));
        let r = mt_ui::Dialog::new("export_section", "Export Section Slice")
            .width(320.0)
            .show(ctx, |ui| {
                crate::export::dialog_visuals(ui);
                ui.strong("Drawing Scale & Size");
                ui.add_space(4.0);
                ui.checkbox(&mut d.full_scale, "Full Scale (1:1)");
                ui.add_space(4.0);
                egui::Grid::new("mt_section_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("In Drawing:");
                        ui.add_enabled(
                            !d.full_scale,
                            egui::DragValue::new(&mut d.in_drawing)
                                .range(0.001..=1e6)
                                .speed(0.1),
                        );
                        ui.end_row();
                        ui.label("In Model:");
                        ui.add_enabled(
                            !d.full_scale,
                            egui::DragValue::new(&mut d.in_model)
                                .range(0.001..=1e6)
                                .speed(1.0),
                        );
                        ui.end_row();
                        ui.label("Units:");
                        let label = |u: Option<Units>| match u {
                            None => format!("Model Units ({})", model_units.name()),
                            Some(u) => u.name().to_owned(),
                        };
                        egui::ComboBox::from_id_salt("mt_section_units")
                            .selected_text(label(d.units))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut d.units, None, label(None));
                                for u in Units::ALL {
                                    ui.selectable_value(&mut d.units, Some(u), label(Some(u)));
                                }
                            });
                        ui.end_row();
                    });
                ui.add_space(8.0);
                ui.weak("AutoCAD R12 DXF, LINE entities in the section plane.");
                crate::presentation::ok_cancel(ui, "Export...")
            });
        let done = crate::presentation::answer(r);
        match done {
            None => self.present.section = Some(d),
            Some(false) => self.present.last_section = d,
            Some(true) => {
                self.present.last_section = d.clone();
                let Some((p, n)) = self.scene.section else {
                    return;
                };
                let segments = mt_render::section_lines(self.scene.triangles(), p, n);
                let text = section_dxf(&segments, n, &d, model_units);
                let name = self.export_file_name("dxf");
                let Some(path) =
                    crate::filedialog::save(&[("AutoCAD DXF File (*.dxf)", &["dxf"])], &name)
                else {
                    return;
                };
                self.message = Some(match std::fs::write(&path, text) {
                    Ok(()) => format!("Exported {}.", path.display()),
                    Err(e) => format!("Could not export {}: {e}", path.display()),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_axes_keep_red_and_green() {
        let (x, y) = plane_axes(DVec3::Z);
        assert!((x - DVec3::X).length() < 1e-12 && (y - DVec3::Y).length() < 1e-12);
        let (x, y) = plane_axes(-DVec3::Z);
        assert!((x - DVec3::X).length() < 1e-12 && (y - DVec3::Y).length() < 1e-12);
        // Vertical plane: y is up, x is right when looking against the normal.
        let (x, y) = plane_axes(DVec3::Y);
        assert!((y - DVec3::Z).length() < 1e-12);
        assert!((x + DVec3::X).length() < 1e-12, "{x}");
    }

    #[test]
    fn merge_joins_collinear_and_drops_duplicates() {
        let p = |x, y| DVec2::new(x, y);
        let segs = [
            [p(0., 0.), p(1., 0.)],
            [p(1., 0.), p(2., 0.)],
            [p(2., 0.), p(1., 0.)], // duplicate reversed
            [p(2., 0.), p(2., 1.)],
            [p(3., 3.), p(3., 3.)], // zero length
        ];
        let out = merge(&segs, 1e-9);
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(out.iter().any(|l| {
            let (a, b) = (l[0].min(l[1]), l[0].max(l[1]));
            a == p(0., 0.) && b == p(2., 0.)
        }));
    }

    #[test]
    fn dxf_of_a_cut_box_is_true_size() {
        // A 10 x 20 inch rectangle cut out of a box, in the plane z = 5.
        let q = |x, y| DVec3::new(x, y, 5.0);
        let segs = [
            [q(0., 0.), q(10., 0.)],
            [q(10., 0.), q(10., 20.)],
            [q(10., 20.), q(0., 20.)],
            [q(0., 20.), q(0., 0.)],
        ];
        let dxf = section_dxf(
            &segs,
            DVec3::Z,
            &SectionOptions {
                units: Some(Units::Millimeters),
                ..Default::default()
            },
            Units::Inches,
        );
        assert!(dxf.contains("AC1009"));
        assert!(dxf.contains("units: millimeters"));
        assert_eq!(dxf.matches("\nLINE\n").count(), 4);
        assert!(dxf.contains("254.000000"), "10 in = 254 mm");
        assert!(dxf.contains("508.000000"), "20 in = 508 mm");
        assert!(dxf.trim_end().ends_with("EOF"));
        // It reads back through File > Import's DXF reader as a closed 10 x 20 loop.
        let d = mt_kernel::dxf::parse(dxf.as_bytes()).unwrap();
        assert_eq!(d.shapes.len(), 4);
        let m = mt_kernel::dxf::to_model(
            &d,
            &mt_kernel::dxf::DxfOptions {
                inches_per_unit: Some(1.0 / 25.4),
                ..Default::default()
            },
        );
        let ents = &m.definitions[m.root.instances[0].definition].entities;
        assert_eq!(ents.faces.len(), 1);
        let ymax = ents
            .vertices
            .values()
            .map(|v| v.pos.y)
            .fold(f64::MIN, f64::max);
        let ymin = ents
            .vertices
            .values()
            .map(|v| v.pos.y)
            .fold(f64::MAX, f64::min);
        assert!(
            ((ymax - ymin).abs() - 20.0).abs() < 1e-6 || ((ymax - ymin).abs() - 10.0).abs() < 1e-6
        );
        // Group codes and values alternate on their own lines.
        let lines: Vec<&str> = dxf.lines().collect();
        assert_eq!(lines.len() % 2, 0);
        for pair in lines.chunks(2) {
            assert!(pair[0].trim().parse::<i32>().is_ok(), "{pair:?}");
        }
        // At 1:50 the coordinates are a fiftieth.
        let scaled = section_dxf(
            &segs,
            DVec3::Z,
            &SectionOptions {
                full_scale: false,
                in_drawing: 1.0,
                in_model: 50.0,
                units: Some(Units::Inches),
            },
            Units::Inches,
        );
        assert!(scaled.contains("0.400000"), "20 in at 1:50");
        assert!(scaled.contains("1 drawing = 50 model"));
    }
}
