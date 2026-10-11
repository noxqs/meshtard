//! File > Import of AutoCAD DXF drawings (ASCII and binary, R12 and later).
//!
//! - Entities read from the ENTITIES section: LINE, LWPOLYLINE and POLYLINE (2D with
//!   bulges, 3D polylines, polygon meshes and polyface meshes), ARC, CIRCLE and
//!   3DFACE. Arcs, circles and bulged polyline segments become arc curves (circles 24
//!   segments, arcs in proportion, at least 2; SketchUp's own counts are unverified).
//!   Object coordinate systems (extrusion directions) are honored.
//! - Edges are drawn with the kernel's [`crate::add_line`] / [`crate::add_curve`], so
//!   closed planar loops get faces as when drawing; 3DFACE and mesh faces add their
//!   outlines the same way (a non-planar quad is split along a diagonal). Invisible
//!   edges of 3DFACEs and polyface meshes are hidden.
//! - Layers become groups, one per layer named after it (SketchUp 2026's importer,
//!   release notes), or tags of one drawing.
//! - Units: from the header's `$INSUNITS` or as chosen.
//! - Not read: blocks and INSERTs, text, dimensions, hatches, splines, ellipses,
//!   solids and ACIS bodies, paper space. DWG is a closed format and not read.

use std::collections::HashMap;
use std::f64::consts::TAU;

use glam::{DMat4, DVec2, DVec3};

use crate::entities::{Arc, EdgeId, Entities, Instance, LayerId};
use crate::model::{Definition, Layer, Model};

/// One group code and its value as read (`text`; `num` when it is a number).
#[derive(Debug, Clone, PartialEq)]
struct Group {
    code: i32,
    text: String,
    num: f64,
}

/// How DXF layers come into the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayerMode {
    /// One group per layer (named after the layer).
    #[default]
    Groups,
    /// One drawing; each layer becomes a tag.
    Tags,
}

/// DXF import choices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DxfOptions {
    /// Inches per drawing unit; `None` reads `$INSUNITS` (inches when unset).
    pub inches_per_unit: Option<f64>,
    pub layers: LayerMode,
}

impl Default for DxfOptions {
    fn default() -> Self {
        Self {
            inches_per_unit: None,
            layers: LayerMode::Groups,
        }
    }
}

/// A shape read from the drawing, in drawing units and world coordinates.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// A straight segment; `hidden` for invisible 3DFACE / mesh edges.
    Line { a: DVec3, b: DVec3, hidden: bool },
    /// An arc or circle.
    Arc(Arc),
    /// A face outline (3 or 4 corners, or a polygon); `hidden[i]` for the edge from
    /// corner `i` to the next.
    Face {
        corners: Vec<DVec3>,
        hidden: Vec<bool>,
    },
}

/// A parsed drawing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drawing {
    /// Layer names in table order, then layers only entities name.
    pub layers: Vec<DxfLayer>,
    /// Shapes with the index of their layer.
    pub shapes: Vec<(usize, Shape)>,
    /// `$INSUNITS` as inches per unit, when the header has a known unit.
    pub inches_per_unit: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DxfLayer {
    pub name: String,
    /// sRGB from the layer's AutoCAD color index.
    pub color: [u8; 3],
    /// Off or frozen.
    pub hidden: bool,
}

/// Inches per unit of an `$INSUNITS` value.
fn insunits(v: i64) -> Option<f64> {
    Some(match v {
        1 => 1.0,
        2 => 12.0,
        3 => 63_360.0,
        4 => 1.0 / 25.4,
        5 => 1.0 / 2.54,
        6 => 1.0 / 0.0254,
        7 => 1000.0 / 0.0254,
        8 => 1e-6,
        9 => 1e-3,
        10 => 36.0,
        14 => 0.1 / 0.0254,
        _ => return None,
    })
}

/// Rough sRGB of an AutoCAD color index (the 9 standard colors; others gray).
fn aci_color(i: i64) -> [u8; 3] {
    match i.abs() {
        1 => [255, 0, 0],
        2 => [255, 255, 0],
        3 => [0, 255, 0],
        4 => [0, 255, 255],
        5 => [0, 0, 255],
        6 => [255, 0, 255],
        8 => [128, 128, 128],
        9 => [192, 192, 192],
        _ => [255, 255, 255],
    }
}

const BINARY_SENTINEL: &[u8] = b"AutoCAD Binary DXF\r\n\x1a\0";

fn read_groups(bytes: &[u8]) -> Result<Vec<Group>, String> {
    if let Some(rest) = bytes.strip_prefix(BINARY_SENTINEL) {
        return read_binary(rest);
    }
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.lines();
    let mut out = Vec::new();
    while let Some(code) = lines.next() {
        let code = code.trim();
        if code.is_empty() {
            continue;
        }
        let code: i32 = code
            .parse()
            .map_err(|_| format!("bad group code {code:?}"))?;
        let value = lines.next().ok_or("truncated DXF file")?.trim().to_owned();
        let num = value.parse().unwrap_or(0.0);
        out.push(Group {
            code,
            text: value,
            num,
        });
    }
    if !out.iter().any(|g| g.code == 0 && g.text == "SECTION") {
        return Err("not a DXF file".into());
    }
    Ok(out)
}

/// Binary DXF: R12 files have one-byte group codes (255 escapes a two-byte one),
/// later versions two-byte codes; the first group (0 / SECTION) tells which.
fn read_binary(b: &[u8]) -> Result<Vec<Group>, String> {
    let wide = b.get(1) == Some(&0);
    let mut i = 0;
    let mut out = Vec::new();
    let take = |i: &mut usize, n: usize| -> Result<&[u8], String> {
        let s = b.get(*i..*i + n).ok_or("truncated binary DXF")?;
        *i += n;
        Ok(s)
    };
    while i < b.len() {
        let code = if wide {
            i32::from(i16::from_le_bytes(take(&mut i, 2)?.try_into().expect("2")))
        } else {
            let c = take(&mut i, 1)?[0];
            if c == 255 {
                i32::from(i16::from_le_bytes(take(&mut i, 2)?.try_into().expect("2")))
            } else {
                i32::from(c)
            }
        };
        let string = |i: &mut usize| -> Result<String, String> {
            let end = b[*i..]
                .iter()
                .position(|&c| c == 0)
                .ok_or("truncated binary DXF")?;
            let s = String::from_utf8_lossy(&b[*i..*i + end]).into_owned();
            *i += end + 1;
            Ok(s)
        };
        let f64v = |i: &mut usize| -> Result<f64, String> {
            Ok(f64::from_le_bytes(take(i, 8)?.try_into().expect("8")))
        };
        let i16v = |i: &mut usize| -> Result<f64, String> {
            Ok(f64::from(i16::from_le_bytes(
                take(i, 2)?.try_into().expect("2"),
            )))
        };
        let i32v = |i: &mut usize| -> Result<f64, String> {
            Ok(f64::from(i32::from_le_bytes(
                take(i, 4)?.try_into().expect("4"),
            )))
        };
        let (text, num) = match code {
            10..=59 | 110..=149 | 210..=239 | 460..=469 | 1010..=1059 => {
                let v = f64v(&mut i)?;
                (v.to_string(), v)
            }
            60..=79 | 170..=179 | 270..=289 | 370..=389 | 400..=409 | 1060..=1070 => {
                let v = i16v(&mut i)?;
                (v.to_string(), v)
            }
            90..=99 | 420..=429 | 440..=459 | 1071 => {
                let v = i32v(&mut i)?;
                (v.to_string(), v)
            }
            160..=169 => {
                let v = i64::from_le_bytes(take(&mut i, 8)?.try_into().expect("8")) as f64;
                (v.to_string(), v)
            }
            290..=299 => {
                let v = f64::from(take(&mut i, 1)?[0]);
                (v.to_string(), v)
            }
            310..=319 | 1004 => {
                let n = take(&mut i, 1)?[0] as usize;
                take(&mut i, n)?;
                (String::new(), 0.0)
            }
            _ => (string(&mut i)?, 0.0),
        };
        let eof = code == 0 && text == "EOF";
        out.push(Group { code, text, num });
        if eof {
            break;
        }
    }
    Ok(out)
}

/// One entity: its type and groups.
struct Ent<'a> {
    kind: &'a str,
    groups: &'a [Group],
}

impl Ent<'_> {
    fn num(&self, code: i32) -> Option<f64> {
        self.groups.iter().find(|g| g.code == code).map(|g| g.num)
    }

    fn f(&self, code: i32) -> f64 {
        self.num(code).unwrap_or(0.0)
    }

    fn int(&self, code: i32) -> i64 {
        self.num(code).unwrap_or(0.0) as i64
    }

    fn text(&self, code: i32) -> &str {
        self.groups
            .iter()
            .find(|g| g.code == code)
            .map_or("", |g| g.text.as_str())
    }

    fn point(&self, code: i32) -> DVec3 {
        DVec3::new(self.f(code), self.f(code + 10), self.f(code + 20))
    }

    /// The extrusion direction (OCS normal).
    fn extrusion(&self) -> DVec3 {
        let n = DVec3::new(
            self.num(210).unwrap_or(0.0),
            self.num(220).unwrap_or(0.0),
            self.num(230).unwrap_or(1.0),
        );
        n.try_normalize().unwrap_or(DVec3::Z)
    }
}

/// AutoCAD's arbitrary axis algorithm: the OCS x and y axes of normal `n`.
fn ocs_axes(n: DVec3) -> (DVec3, DVec3) {
    let ax = if n.x.abs() < 1.0 / 64.0 && n.y.abs() < 1.0 / 64.0 {
        DVec3::Y.cross(n)
    } else {
        DVec3::Z.cross(n)
    }
    .normalize();
    (ax, n.cross(ax))
}

fn arc_segments(sweep: f64) -> usize {
    ((24.0 * sweep.abs() / TAU).ceil() as usize).max(2)
}

/// The shapes of a 2D polyline (OCS points with bulges) in world space.
fn polyline_2d(pts: &[(DVec2, f64)], closed: bool, elevation: f64, n: DVec3) -> Vec<Shape> {
    let (ax, ay) = ocs_axes(n);
    let world = |p: DVec2| ax * p.x + ay * p.y + n * elevation;
    let mut out = Vec::new();
    let count = if closed {
        pts.len()
    } else {
        pts.len().saturating_sub(1)
    };
    for i in 0..count {
        let (p1, bulge) = pts[i];
        let p2 = pts[(i + 1) % pts.len()].0;
        if (p2 - p1).length() < 1e-12 {
            continue;
        }
        if bulge.abs() < 1e-9 {
            out.push(Shape::Line {
                a: world(p1),
                b: world(p2),
                hidden: false,
            });
            continue;
        }
        let c = p2 - p1;
        let center = (p1 + p2) / 2.0 + c.perp() * ((1.0 - bulge * bulge) / (4.0 * bulge));
        let sweep = 4.0 * bulge.atan();
        let start = (p1 - center).y.atan2((p1 - center).x);
        out.push(Shape::Arc(Arc {
            center: world(center),
            normal: n,
            x_axis: ax,
            radius: (p1 - center).length(),
            start_angle: start.min(start + sweep),
            end_angle: start.max(start + sweep),
            segments: arc_segments(sweep),
        }));
    }
    out
}

/// Reads a DXF file.
pub fn parse(bytes: &[u8]) -> Result<Drawing, String> {
    let groups = read_groups(bytes)?;
    let mut d = Drawing::default();
    let mut layer_index: HashMap<String, usize> = HashMap::new();
    let mut layer = |d: &mut Drawing, name: &str| -> usize {
        let name = if name.is_empty() { "0" } else { name };
        *layer_index.entry(name.to_owned()).or_insert_with(|| {
            d.layers.push(DxfLayer {
                name: name.to_owned(),
                color: [255, 255, 255],
                hidden: false,
            });
            d.layers.len() - 1
        })
    };
    // Split into entities at code 0, remembering sections.
    let mut section = String::new();
    let mut i = 0;
    let mut ents: Vec<(String, Ent)> = Vec::new();
    while i < groups.len() {
        let g = &groups[i];
        if g.code != 0 {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut end = start;
        while end < groups.len() && groups[end].code != 0 {
            end += 1;
        }
        let body = &groups[start..end];
        match g.text.as_str() {
            "SECTION" => {
                section = body
                    .iter()
                    .find(|x| x.code == 2)
                    .map_or_else(String::new, |x| x.text.clone());
                if section == "HEADER"
                    && let Some(k) = body
                        .iter()
                        .position(|x| x.code == 9 && x.text == "$INSUNITS")
                    && let Some(v) = body.get(k + 1)
                {
                    d.inches_per_unit = insunits(v.num as i64);
                }
            }
            "ENDSEC" => section.clear(),
            "EOF" => break,
            kind => ents.push((section.clone(), Ent { kind, groups: body })),
        }
        i = end;
    }
    let mut k = 0;
    while k < ents.len() {
        let (sec, e) = &ents[k];
        k += 1;
        if sec == "TABLES" && e.kind == "LAYER" {
            let l = layer(&mut d, e.text(2));
            let color = e.int(62);
            d.layers[l].color = aci_color(color);
            d.layers[l].hidden = color < 0 || e.int(70) & 1 != 0;
            continue;
        }
        if sec != "ENTITIES" {
            continue;
        }
        // Paper space entities are skipped.
        if e.int(67) == 1 {
            if e.kind == "POLYLINE" {
                while k < ents.len() && ents[k].1.kind != "SEQEND" {
                    k += 1;
                }
            }
            continue;
        }
        let l = layer(&mut d, e.text(8));
        match e.kind {
            "LINE" => d.shapes.push((
                l,
                Shape::Line {
                    a: e.point(10),
                    b: e.point(11),
                    hidden: false,
                },
            )),
            "3DFACE" => {
                let mut corners = vec![e.point(10), e.point(11), e.point(12), e.point(13)];
                let flags = e.int(70);
                let mut hidden: Vec<bool> = (0..4).map(|b| flags & (1 << b) != 0).collect();
                if corners[3].distance(corners[2]) < 1e-12 {
                    corners.pop();
                    hidden.pop();
                }
                d.shapes.push((l, Shape::Face { corners, hidden }));
            }
            "CIRCLE" | "ARC" => {
                let n = e.extrusion();
                let (ax, ay) = ocs_axes(n);
                let c = e.point(10);
                let (start, end) = if e.kind == "CIRCLE" {
                    (0.0, TAU)
                } else {
                    let s = e.f(50).to_radians();
                    let mut t = e.f(51).to_radians();
                    while t <= s {
                        t += TAU;
                    }
                    (s, t)
                };
                let radius = e.f(40);
                if radius > 0.0 {
                    d.shapes.push((
                        l,
                        Shape::Arc(Arc {
                            center: ax * c.x + ay * c.y + n * c.z,
                            normal: n,
                            x_axis: ax,
                            radius,
                            start_angle: start,
                            end_angle: end,
                            segments: arc_segments(end - start),
                        }),
                    ));
                }
            }
            "LWPOLYLINE" => {
                let mut pts: Vec<(DVec2, f64)> = Vec::new();
                for g in e.groups {
                    match g.code {
                        10 => pts.push((DVec2::new(g.num, 0.0), 0.0)),
                        20 => {
                            if let Some(p) = pts.last_mut() {
                                p.0.y = g.num;
                            }
                        }
                        42 => {
                            if let Some(p) = pts.last_mut() {
                                p.1 = g.num;
                            }
                        }
                        _ => {}
                    }
                }
                let closed = e.int(70) & 1 != 0;
                for s in polyline_2d(&pts, closed, e.f(38), e.extrusion()) {
                    d.shapes.push((l, s));
                }
            }
            "POLYLINE" => {
                let flags = e.int(70);
                let mut verts: Vec<&Ent> = Vec::new();
                while k < ents.len() && ents[k].1.kind == "VERTEX" {
                    verts.push(&ents[k].1);
                    k += 1;
                }
                if k < ents.len() && ents[k].1.kind == "SEQEND" {
                    k += 1;
                }
                if flags & 64 != 0 {
                    // Polyface mesh: position vertices, then face records.
                    let pos: Vec<DVec3> = verts
                        .iter()
                        .filter(|v| v.int(70) & 64 != 0)
                        .map(|v| v.point(10))
                        .collect();
                    for v in verts.iter().filter(|v| v.int(70) & 192 == 128) {
                        let idx: Vec<i64> =
                            (71..=74).map(|c| v.int(c)).filter(|&i| i != 0).collect();
                        let corners: Option<Vec<DVec3>> = idx
                            .iter()
                            .map(|&i| pos.get(i.unsigned_abs() as usize - 1).copied())
                            .collect();
                        if let Some(corners) = corners.filter(|c| c.len() >= 3) {
                            let hidden = idx.iter().map(|&i| i < 0).collect();
                            d.shapes.push((l, Shape::Face { corners, hidden }));
                        }
                    }
                } else if flags & 16 != 0 {
                    // Polygon mesh: M x N vertices.
                    let (m, n) = (e.int(71).max(0) as usize, e.int(72).max(0) as usize);
                    let pos: Vec<DVec3> = verts.iter().map(|v| v.point(10)).collect();
                    if m * n <= pos.len() && m > 0 && n > 0 {
                        let (mc, nc) = (flags & 1 != 0, flags & 32 != 0);
                        let mi = if mc { m } else { m - 1 };
                        let ni = if nc { n } else { n - 1 };
                        for a in 0..mi {
                            for b in 0..ni {
                                let p = |a: usize, b: usize| pos[(a % m) * n + (b % n)];
                                d.shapes.push((
                                    l,
                                    Shape::Face {
                                        corners: vec![
                                            p(a, b),
                                            p(a + 1, b),
                                            p(a + 1, b + 1),
                                            p(a, b + 1),
                                        ],
                                        hidden: vec![false; 4],
                                    },
                                ));
                            }
                        }
                    }
                } else if flags & 8 != 0 {
                    // 3D polyline (world coordinates).
                    let pts: Vec<DVec3> = verts.iter().map(|v| v.point(10)).collect();
                    let closed = flags & 1 != 0;
                    let count = if closed {
                        pts.len()
                    } else {
                        pts.len().saturating_sub(1)
                    };
                    for i in 0..count {
                        d.shapes.push((
                            l,
                            Shape::Line {
                                a: pts[i],
                                b: pts[(i + 1) % pts.len()],
                                hidden: false,
                            },
                        ));
                    }
                } else {
                    let pts: Vec<(DVec2, f64)> = verts
                        .iter()
                        .map(|v| (v.point(10).truncate(), v.f(42)))
                        .collect();
                    for s in polyline_2d(&pts, flags & 1 != 0, e.f(30), e.extrusion()) {
                        d.shapes.push((l, s));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(d)
}

/// Adds `shape` scaled by `scale` to `ents`; returns the edges it covers.
fn draw(ents: &mut Entities, shape: &Shape, scale: f64) -> Vec<EdgeId> {
    let mut hide = Vec::new();
    let edges = match shape {
        Shape::Line { a, b, hidden } => {
            if a.distance(*b) * scale < crate::TOL {
                return Vec::new();
            }
            let e = crate::add_line(ents, *a * scale, *b * scale);
            if *hidden {
                hide.extend(e.iter().copied());
            }
            e
        }
        Shape::Arc(arc) => {
            let arc = Arc {
                center: arc.center * scale,
                radius: arc.radius * scale,
                ..*arc
            };
            if arc.radius < crate::TOL {
                return Vec::new();
            }
            crate::add_curve(ents, &arc.points(), Some(arc))
        }
        Shape::Face { corners, hidden } => {
            let pts: Vec<DVec3> = corners.iter().map(|p| *p * scale).collect();
            let mut out = Vec::new();
            let mut line = |ents: &mut Entities, a: DVec3, b: DVec3, h: bool| {
                if a.distance(b) >= crate::TOL {
                    let e = crate::add_line(ents, a, b);
                    if h {
                        hide.extend(e.iter().copied());
                    }
                    out.extend(e);
                }
            };
            let planar = crate::geom::Plane::from_polygon(&pts)
                .is_some_and(|pl| pts.iter().all(|&p| pl.contains(p)));
            for i in 0..pts.len() {
                line(
                    ents,
                    pts[i],
                    pts[(i + 1) % pts.len()],
                    hidden.get(i) == Some(&true),
                );
            }
            if !planar && pts.len() == 4 {
                // Two triangles; the diagonal is hidden like SketchUp's soft folds.
                line(ents, pts[0], pts[2], true);
            }
            out
        }
    };
    for e in hide {
        if let Some(edge) = ents.edges.get_mut(e) {
            edge.attrs.hidden = true;
        }
    }
    edges
}

/// Builds a model from a parsed drawing: per layer a group in the root, or one
/// drawing with a tag per layer.
pub fn to_model(d: &Drawing, opts: &DxfOptions) -> Model {
    let scale = opts.inches_per_unit.or(d.inches_per_unit).unwrap_or(1.0);
    let mut model = Model::default();
    match opts.layers {
        LayerMode::Groups => {
            for (l, layer) in d.layers.iter().enumerate() {
                let mut ents = Entities::new();
                for (_, s) in d.shapes.iter().filter(|(x, _)| *x == l) {
                    draw(&mut ents, s, scale);
                }
                if ents.edges.is_empty() {
                    continue;
                }
                let name = model.unique_definition_name("Group#1");
                let def = model.definitions.insert(Definition {
                    name,
                    entities: ents,
                    is_group: true,
                    ..Default::default()
                });
                model.root.instances.push(Instance {
                    definition: def,
                    transform: DMat4::IDENTITY,
                    name: layer.name.clone(),
                    hidden: layer.hidden,
                    locked: false,
                    layer: None,
                    material: None,
                    attrs: Default::default(),
                });
            }
        }
        LayerMode::Tags => {
            let mut tags: Vec<Option<LayerId>> = Vec::new();
            for layer in &d.layers {
                if layer.name == "0" {
                    tags.push(None);
                    continue;
                }
                model.layers.push(Layer {
                    name: layer.name.clone(),
                    color: layer.color,
                    visible: !layer.hidden,
                    ..Layer::default()
                });
                tags.push(Some(model.layers.len() - 1));
            }
            let ents = &mut model.root;
            for (l, s) in &d.shapes {
                let tag = tags.get(*l).copied().flatten();
                for e in draw(ents, s, scale) {
                    if let Some(edge) = ents.edges.get_mut(e) {
                        edge.attrs.layer = tag;
                    }
                }
            }
            // Faces take the tag of their first edge.
            let faces: Vec<_> = ents.faces.keys().collect();
            for f in faces {
                let first = ents.faces[f].outer.coedges.first().map(|c| c.edge);
                ents.faces[f].attrs.layer = first.and_then(|e| ents.edges[e].attrs.layer);
            }
        }
    }
    model
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An ASCII DXF from (code, value) pairs.
    fn ascii(groups: &[(i32, &str)]) -> String {
        groups
            .iter()
            .map(|(c, v)| format!("{c:>3}\n{v}\n"))
            .collect()
    }

    fn drawing(header: &[(i32, &str)], entities: &[(i32, &str)]) -> String {
        let mut g = vec![(0, "SECTION"), (2, "HEADER")];
        g.extend_from_slice(header);
        g.extend([
            (0, "ENDSEC"),
            (0, "SECTION"),
            (2, "TABLES"),
            (0, "TABLE"),
            (2, "LAYER"),
        ]);
        g.extend([(0, "LAYER"), (2, "Walls"), (70, "0"), (62, "1")]);
        g.extend([(0, "LAYER"), (2, "Off"), (70, "0"), (62, "-3")]);
        g.extend([
            (0, "ENDTAB"),
            (0, "ENDSEC"),
            (0, "SECTION"),
            (2, "ENTITIES"),
        ]);
        g.extend_from_slice(entities);
        g.extend([(0, "ENDSEC"), (0, "EOF")]);
        ascii(&g)
    }

    fn square_lines(layer: &'static str, size: &'static str) -> Vec<(i32, &'static str)> {
        let c = [("0", "0"), (size, "0"), (size, size), ("0", size)];
        let mut g = Vec::new();
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            g.extend([
                (0, "LINE"),
                (8, layer),
                (10, a.0),
                (20, a.1),
                (30, "0"),
                (11, b.0),
                (21, b.1),
                (31, "0"),
            ]);
        }
        g
    }

    #[test]
    fn lines_make_a_face_in_a_layer_group() {
        let mut ents = square_lines("Walls", "10");
        ents.extend([
            (0, "CIRCLE"),
            (8, "Round"),
            (10, "30"),
            (20, "0"),
            (30, "0"),
            (40, "5"),
        ]);
        let text = drawing(&[(9, "$INSUNITS"), (70, "4")], &ents);
        let d = parse(text.as_bytes()).unwrap();
        // Millimeters.
        assert!((d.inches_per_unit.unwrap() - 1.0 / 25.4).abs() < 1e-12);
        assert_eq!(d.shapes.len(), 5);
        assert_eq!(
            d.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            ["Walls", "Off", "Round"]
        );
        assert!(d.layers[1].hidden);
        let m = to_model(&d, &DxfOptions::default());
        // Groups for Walls and Round (Off is empty).
        assert_eq!(m.root.instances.len(), 2);
        let walls = &m.root.instances[0];
        assert_eq!(walls.name, "Walls");
        let w = &m.definitions[walls.definition];
        assert!(w.is_group);
        assert_eq!(w.entities.faces.len(), 1);
        assert_eq!(w.entities.edges.len(), 4);
        let max = w
            .entities
            .vertices
            .values()
            .map(|v| v.pos.x)
            .fold(0.0, f64::max);
        assert!((max - 10.0 / 25.4).abs() < 1e-9);
        let round = &m.definitions[m.root.instances[1].definition];
        assert_eq!(round.entities.faces.len(), 1);
        assert_eq!(round.entities.edges.len(), 24);
        assert_eq!(round.entities.curves.len(), 1);
        assert!(crate::validate(&round.entities, crate::Checks::ALL).is_empty());
    }

    #[test]
    fn tags_mode_and_units_override() {
        let text = drawing(&[], &square_lines("Walls", "2"));
        let d = parse(text.as_bytes()).unwrap();
        assert_eq!(d.inches_per_unit, None);
        let m = to_model(
            &d,
            &DxfOptions {
                inches_per_unit: Some(12.0),
                layers: LayerMode::Tags,
            },
        );
        assert!(m.root.instances.is_empty());
        assert_eq!(m.root.faces.len(), 1);
        let tag = m.layers.iter().position(|l| l.name == "Walls").unwrap();
        assert_eq!(m.layers[tag].color, [255, 0, 0]);
        assert!(m.root.edges.values().all(|e| e.attrs.layer == Some(tag)));
        assert_eq!(m.root.faces.values().next().unwrap().attrs.layer, Some(tag));
        let max = m
            .root
            .vertices
            .values()
            .map(|v| v.pos.y)
            .fold(0.0, f64::max);
        assert!((max - 24.0).abs() < 1e-9);
    }

    #[test]
    fn polylines_bulges_arcs_and_3dfaces() {
        let ents = [
            // A closed 10 x 10 LWPOLYLINE whose top side bulges out (a half circle).
            (0, "LWPOLYLINE"),
            (8, "P"),
            (90, "4"),
            (70, "1"),
            (10, "0"),
            (20, "0"),
            (10, "10"),
            (20, "0"),
            (10, "10"),
            (20, "10"),
            (42, "1"),
            (10, "0"),
            (20, "10"),
            // A quarter arc.
            (0, "ARC"),
            (8, "A"),
            (10, "0"),
            (20, "0"),
            (30, "0"),
            (40, "3"),
            (50, "0"),
            (51, "90"),
            // A standing triangle 3DFACE with one invisible edge.
            (0, "3DFACE"),
            (8, "F"),
            (10, "0"),
            (20, "50"),
            (30, "0"),
            (11, "10"),
            (21, "50"),
            (31, "0"),
            (12, "0"),
            (22, "50"),
            (32, "10"),
            (13, "0"),
            (23, "50"),
            (33, "10"),
            (70, "2"),
            // A 3D polyline.
            (0, "POLYLINE"),
            (8, "Q"),
            (66, "1"),
            (70, "8"),
            (0, "VERTEX"),
            (8, "Q"),
            (10, "0"),
            (20, "0"),
            (30, "0"),
            (70, "32"),
            (0, "VERTEX"),
            (8, "Q"),
            (10, "0"),
            (20, "0"),
            (30, "7"),
            (70, "32"),
            (0, "SEQEND"),
        ];
        let d = parse(drawing(&[], &ents).as_bytes()).unwrap();
        let m = to_model(&d, &DxfOptions::default());
        let group = |name: &str| {
            let i = m.root.instances.iter().find(|i| i.name == name).unwrap();
            &m.definitions[i.definition].entities
        };
        let p = group("P");
        assert_eq!(p.faces.len(), 1);
        // 3 straight sides plus a 12-segment half circle.
        assert_eq!(p.edges.len(), 3 + 12);
        let top = p.vertices.values().map(|v| v.pos.y).fold(0.0, f64::max);
        assert!((top - 15.0).abs() < 1e-9, "{top}");
        let a = group("A");
        assert_eq!(a.edges.len(), 6);
        assert!(a.faces.is_empty());
        let f = group("F");
        assert_eq!(f.faces.len(), 1);
        assert_eq!(f.edges.values().filter(|e| e.attrs.hidden).count(), 1);
        let q = group("Q");
        assert_eq!(q.edges.len(), 1);
    }

    #[test]
    fn polyface_mesh() {
        let mut ents = vec![
            (0, "POLYLINE"),
            (8, "M"),
            (66, "1"),
            (70, "64"),
            (71, "4"),
            (72, "1"),
        ];
        for (x, y) in [("0", "0"), ("4", "0"), ("4", "4"), ("0", "4")] {
            ents.extend([
                (0, "VERTEX"),
                (8, "M"),
                (10, x),
                (20, y),
                (30, "0"),
                (70, "192"),
            ]);
        }
        ents.extend([
            (0, "VERTEX"),
            (8, "M"),
            (10, "0"),
            (20, "0"),
            (30, "0"),
            (70, "128"),
        ]);
        ents.extend([(71, "1"), (72, "2"), (73, "3"), (74, "4"), (0, "SEQEND")]);
        let d = parse(drawing(&[], &ents).as_bytes()).unwrap();
        assert_eq!(d.shapes.len(), 1);
        let m = to_model(&d, &DxfOptions::default());
        let e = &m.definitions[m.root.instances[0].definition].entities;
        assert_eq!(e.faces.len(), 1);
        assert_eq!(e.edges.len(), 4);
    }

    /// Writes groups as R13+ binary DXF.
    fn binary(groups: &[(i32, &str)]) -> Vec<u8> {
        let mut out = BINARY_SENTINEL.to_vec();
        for &(code, v) in groups {
            out.extend_from_slice(&(code as i16).to_le_bytes());
            match code {
                10..=59 => out.extend_from_slice(&v.parse::<f64>().unwrap().to_le_bytes()),
                60..=79 => out.extend_from_slice(&v.parse::<i16>().unwrap().to_le_bytes()),
                90..=99 => out.extend_from_slice(&v.parse::<i32>().unwrap().to_le_bytes()),
                _ => {
                    out.extend_from_slice(v.as_bytes());
                    out.push(0);
                }
            }
        }
        out
    }

    #[test]
    fn binary_dxf() {
        let mut g = vec![
            (0, "SECTION"),
            (2, "HEADER"),
            (9, "$INSUNITS"),
            (70, "1"),
            (0, "ENDSEC"),
        ];
        g.extend([(0, "SECTION"), (2, "ENTITIES")]);
        g.extend(square_lines("B", "3"));
        g.extend([(0, "ENDSEC"), (0, "EOF")]);
        let d = parse(&binary(&g)).unwrap();
        assert_eq!(d.inches_per_unit, Some(1.0));
        assert_eq!(d.shapes.len(), 4);
        let m = to_model(&d, &DxfOptions::default());
        assert_eq!(
            m.definitions[m.root.instances[0].definition]
                .entities
                .faces
                .len(),
            1
        );
        assert!(parse(b"hello").is_err());
    }
}
