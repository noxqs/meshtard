//! File > Generate Report: the model's groups and components as a CSV or HTML table.
//!
//! Every group and component instance is visited, nested ones too (Entire Model, or
//! the selected instances and what they contain). Instances alike in definition, name,
//! tag, size and volume are one row with their count. Columns: name (the instance
//! name), definition, type, tag, count, bounding box size (x, y, z of the axis-aligned
//! box in model space) and volume (only for solids, `mt_kernel::solid`, scaled by the
//! instance's transform), in the model's units. SketchUp's Generate Report has report
//! templates over arbitrary attributes and an XLSX option (from memory, unverified);
//! this is a fixed report.

use std::collections::BTreeMap;

use eframe::egui;
use glam::{DMat3, DMat4, DVec3};
use mt_kernel::mesh_export::Units;
use mt_kernel::{Entities, Model};

use crate::App;
use crate::document::Owner;

/// One report row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Row {
    pub name: String,
    pub definition: String,
    pub group: bool,
    pub tag: String,
    pub count: usize,
    /// Bounding box size, inches.
    pub size: DVec3,
    /// Cubic inches; `None` when not a solid.
    pub volume: Option<f64>,
}

/// World-space bounding box of `ents` under `xf`, nested instances included.
fn bounds(model: &Model, ents: &Entities, xf: DMat4, depth: usize) -> Option<(DVec3, DVec3)> {
    let mut b: Option<(DVec3, DVec3)> = None;
    let mut add = |p: DVec3| {
        b = Some(match b {
            Some((lo, hi)) => (lo.min(p), hi.max(p)),
            None => (p, p),
        });
    };
    for v in ents.vertices.values() {
        add(xf.transform_point3(v.pos));
    }
    if depth < 32 {
        for inst in &ents.instances {
            if let Some(d) = model.definitions.get(inst.definition)
                && let Some((lo, hi)) = bounds(model, &d.entities, xf * inst.transform, depth + 1)
            {
                add(lo);
                add(hi);
            }
        }
    }
    b
}

/// Alike instances: definition, name, tag, rounded size and volume, group.
type RowKey = (String, String, String, [i64; 3], Option<i64>, bool);

/// Report rows for `instances` of `ents` (all when `None`) and everything nested in
/// them, placed by `xf`.
pub(crate) fn collect(
    model: &Model,
    ents: &Entities,
    xf: DMat4,
    instances: Option<&[usize]>,
) -> Vec<Row> {
    // Key: definition, name, tag and rounded size and volume; value: the row.
    let mut rows: BTreeMap<RowKey, Row> = BTreeMap::new();
    fn walk(
        model: &Model,
        ents: &Entities,
        xf: DMat4,
        only: Option<&[usize]>,
        depth: usize,
        rows: &mut BTreeMap<RowKey, Row>,
    ) {
        if depth >= 32 {
            return;
        }
        for (i, inst) in ents.instances.iter().enumerate() {
            if only.is_some_and(|o| !o.contains(&i)) {
                continue;
            }
            let Some(def) = model.definitions.get(inst.definition) else {
                continue;
            };
            let world = xf * inst.transform;
            let (lo, hi) = bounds(model, &def.entities, world, 0).unwrap_or_default();
            let size = hi - lo;
            let volume = mt_kernel::solid::is_solid(&def.entities).then(|| {
                mt_kernel::solid::volume(&def.entities)
                    * DMat3::from_mat4(world).determinant().abs()
            });
            let tag = inst
                .layer
                .and_then(|l| model.layers.get(l))
                .filter(|_| inst.layer != Some(0))
                .map_or_else(|| "Untagged".to_owned(), |l| l.name.clone());
            // Alike to 1/1000 inch (and cubic inch).
            let r = |x: f64| (x * 1000.0).round() as i64;
            let key = (
                def.name.clone(),
                inst.name.clone(),
                tag.clone(),
                size.to_array().map(r),
                volume.map(r),
                def.is_group,
            );
            rows.entry(key)
                .or_insert_with(|| Row {
                    name: inst.name.clone(),
                    definition: def.name.clone(),
                    group: def.is_group,
                    tag,
                    count: 0,
                    size,
                    volume,
                })
                .count += 1;
            walk(model, &def.entities, world, None, depth + 1, rows);
        }
    }
    walk(model, ents, xf, instances, 0, &mut rows);
    rows.into_values().collect()
}

const HEADER: [&str; 9] = [
    "Name",
    "Definition",
    "Type",
    "Tag",
    "Count",
    "Size X",
    "Size Y",
    "Size Z",
    "Volume",
];

/// Row cells as text, lengths in `units`.
fn cells(r: &Row, units: Units) -> [String; 9] {
    let k = units.per_inch();
    let n = |x: f64| {
        let s = format!("{:.3}", x);
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" {
            "0".to_owned()
        } else {
            s.to_owned()
        }
    };
    [
        r.name.clone(),
        r.definition.clone(),
        if r.group { "Group" } else { "Component" }.to_owned(),
        r.tag.clone(),
        r.count.to_string(),
        n(r.size.x * k),
        n(r.size.y * k),
        n(r.size.z * k),
        r.volume.map_or_else(String::new, |v| n(v * k * k * k)),
    ]
}

fn unit_suffix(units: Units) -> &'static str {
    match units {
        Units::Inches => "in",
        Units::Feet => "ft",
        Units::Millimeters => "mm",
        Units::Centimeters => "cm",
        Units::Meters => "m",
    }
}

fn header(units: Units) -> Vec<String> {
    let u = unit_suffix(units);
    HEADER
        .iter()
        .map(|h| match *h {
            "Volume" => format!("Volume ({u}³)"),
            h if h.starts_with("Size") => format!("{h} ({u})"),
            h => h.to_owned(),
        })
        .collect()
}

/// RFC 4180 CSV with a header row.
pub(crate) fn to_csv(rows: &[Row], units: Units) -> String {
    let field = |s: &str| {
        if s.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_owned()
        }
    };
    let mut out = header(units)
        .iter()
        .map(|h| field(h))
        .collect::<Vec<_>>()
        .join(",");
    out.push_str("\r\n");
    for r in rows {
        out.push_str(
            &cells(r, units)
                .iter()
                .map(|c| field(c))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push_str("\r\n");
    }
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A standalone HTML page with the table.
pub(crate) fn to_html(rows: &[Row], units: Units, title: &str) -> String {
    let mut out = format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>{t} - Report</title>\n\
         <style>body{{font-family:sans-serif;margin:24px}}table{{border-collapse:collapse}}\
         th,td{{border:1px solid #bbb;padding:4px 8px}}th{{background:#eee;text-align:left}}\
         td.n{{text-align:right}}</style></head><body>\n<h1>{t}</h1>\n\
         <p>Groups and components: {n} rows, {c} instances. Generated by meshtard {v}.</p>\n\
         <table>\n<tr>",
        t = html_escape(title),
        n = rows.len(),
        c = rows.iter().map(|r| r.count).sum::<usize>(),
        v = env!("CARGO_PKG_VERSION"),
    );
    for h in header(units) {
        out.push_str(&format!("<th>{}</th>", html_escape(&h)));
    }
    out.push_str("</tr>\n");
    for r in rows {
        out.push_str("<tr>");
        for (i, c) in cells(r, units).iter().enumerate() {
            let class = if i >= 4 { " class=\"n\"" } else { "" };
            out.push_str(&format!("<td{class}>{}</td>", html_escape(c)));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table>\n</body></html>\n");
    out
}

/// Generate Report dialog: whether only the selection is reported.
#[derive(Debug, Clone, Default)]
pub(crate) struct ReportDialog {
    selection_only: bool,
}

impl App {
    pub(crate) fn open_report(&mut self) {
        self.present.report = Some(ReportDialog {
            selection_only: self
                .selection
                .items()
                .iter()
                .any(|o| matches!(o, Owner::Instance(_))),
        });
    }

    pub(crate) fn report_ui(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.present.report.take() else {
            return;
        };
        let mut done = None;
        egui::Modal::new(egui::Id::new("mt_generate_report")).show(ctx, |ui| {
            ui.set_width(320.0);
            crate::export::dialog_visuals(ui);
            ui.strong("Generate Report");
            ui.add_space(6.0);
            ui.label("Groups and components with tag, count, size and solid volume.");
            ui.radio_value(&mut d.selection_only, false, "Entire model");
            ui.radio_value(
                &mut d.selection_only,
                true,
                "Selected groups and components",
            );
            ui.weak("Format by file type: HTML (.html) or CSV (.csv).");
            done = crate::presentation::ok_cancel(ui, "Generate...");
        });
        match done {
            None => self.present.report = Some(d),
            Some(false) => {}
            Some(true) => {
                let Some(doc) = &self.document else { return };
                let selected: Vec<usize> = self
                    .selection
                    .items()
                    .iter()
                    .filter_map(|o| match o {
                        Owner::Instance(i) => Some(*i),
                        _ => None,
                    })
                    .collect();
                let rows = if d.selection_only {
                    collect(&doc.model, doc.active(), self.ctx_xf, Some(&selected))
                } else {
                    collect(&doc.model, &doc.model.root, DMat4::IDENTITY, None)
                };
                let units = crate::export::kernel_units(doc.units);
                let title = self.model_stem();
                let Some(path) = crate::filedialog::save(
                    &[("HTML File", &["html", "htm"]), ("CSV File", &["csv"])],
                    &format!("{title} report.html"),
                ) else {
                    return;
                };
                let csv = path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("csv"));
                let text = if csv {
                    to_csv(&rows, units)
                } else {
                    to_html(&rows, units, &title)
                };
                self.message = Some(match std::fs::write(&path, text) {
                    Ok(()) => format!(
                        "Report of {} rows written to {}.",
                        rows.len(),
                        path.display()
                    ),
                    Err(e) => format!("Could not write {}: {e}", path.display()),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_kernel::{Definition, Instance};

    /// A closed 1 x 2 x 3 inch box.
    fn cuboid() -> Entities {
        let mut e = Entities::default();
        let p = |x: f64, y: f64, z: f64| DVec3::new(x, y, z) * DVec3::new(1.0, 2.0, 3.0);
        let c = [
            p(0., 0., 0.),
            p(1., 0., 0.),
            p(1., 1., 0.),
            p(0., 1., 0.),
            p(0., 0., 1.),
            p(1., 0., 1.),
            p(1., 1., 1.),
            p(0., 1., 1.),
        ];
        for (a, b) in [
            (0, 1),
            (1, 2),
            (2, 3),
            (3, 0),
            (4, 5),
            (5, 6),
            (6, 7),
            (7, 4),
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 7),
        ] {
            mt_kernel::add_line(&mut e, c[a], c[b]);
        }
        e
    }

    #[test]
    fn rows_count_alike_instances_with_volume() {
        let mut m = Model::default();
        let body = cuboid();
        assert!(mt_kernel::solid::is_solid(&body), "test box is closed");
        let def = m.definitions.insert(Definition {
            name: "Box".into(),
            entities: body,
            ..Default::default()
        });
        let inst = |xf: DMat4, name: &str| Instance {
            definition: def,
            transform: xf,
            name: name.into(),
            hidden: false,
            locked: false,
            layer: None,
            material: None,
        };
        m.root.instances.push(inst(DMat4::IDENTITY, ""));
        m.root
            .instances
            .push(inst(DMat4::from_translation(DVec3::X * 10.0), ""));
        m.root
            .instances
            .push(inst(DMat4::from_scale(DVec3::splat(2.0)), "Big"));
        let rows = collect(&m, &m.root, DMat4::IDENTITY, None);
        assert_eq!(rows.len(), 2, "{rows:?}");
        let small = rows.iter().find(|r| r.name.is_empty()).unwrap();
        assert_eq!(small.count, 2);
        assert!((small.volume.unwrap() - 6.0).abs() < 1e-9);
        assert!((small.size - DVec3::new(1.0, 2.0, 3.0)).length() < 1e-9);
        let big = rows.iter().find(|r| r.name == "Big").unwrap();
        assert!((big.volume.unwrap() - 48.0).abs() < 1e-9);

        let csv = to_csv(&rows, Units::Inches);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            "Name,Definition,Type,Tag,Count,Size X (in),Size Y (in),Size Z (in),Volume (in³)"
        );
        assert!(
            lines.contains(&",Box,Component,Untagged,2,1,2,3,6"),
            "{csv}"
        );
        assert!(
            lines.contains(&"Big,Box,Component,Untagged,1,2,4,6,48"),
            "{csv}"
        );
        // Millimeters: 1 in = 25.4 mm, 6 in³ = 98322.384 mm³.
        let mm = to_csv(&rows, Units::Millimeters);
        assert!(mm.contains(",25.4,50.8,76.2,98322.384"), "{mm}");

        let html = to_html(&rows, Units::Inches, "a<b");
        assert!(html.contains("<h1>a&lt;b</h1>"));
        assert_eq!(html.matches("<tr>").count(), 3);
    }

    #[test]
    fn csv_quotes_commas_and_quotes() {
        let r = Row {
            name: "a,\"b\"".into(),
            definition: "D".into(),
            group: true,
            tag: "T".into(),
            count: 1,
            size: DVec3::ZERO,
            volume: None,
        };
        let csv = to_csv(&[r], Units::Inches);
        assert!(
            csv.lines()
                .nth(1)
                .unwrap()
                .starts_with("\"a,\"\"b\"\"\",D,Group,T,1,0,0,0,")
        );
    }
}
