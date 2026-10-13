//! File > Import of AutoCAD DXF drawings (ASCII and binary, R12 and later).
//!
//! - Entities read from the ENTITIES section and from blocks: LINE, LWPOLYLINE and
//!   POLYLINE (2D with bulges, 3D polylines, polygon meshes and polyface meshes), ARC,
//!   CIRCLE, ELLIPSE, SPLINE, 3DFACE, SOLID / TRACE, HATCH, INSERT, TEXT, MTEXT,
//!   ATTRIB and DIMENSION. Arcs, circles and bulged polyline segments become arc
//!   curves (circles 24 segments, arcs in proportion, at least 2; SketchUp's own
//!   counts are unverified); ellipses and splines become curves (ellipses 24 segments
//!   per turn; splines evaluated from their control points and knots, 8 segments per
//!   knot span, else through their fit points). Object coordinate systems (extrusion
//!   directions) are honored.
//! - Edges are drawn with the kernel's [`crate::add_line`] / [`crate::add_curve`], so
//!   closed planar loops get faces as when drawing; 3DFACE, SOLID and mesh faces add
//!   their outlines the same way (a non-planar quad is split along a diagonal).
//!   Invisible edges of 3DFACEs and polyface meshes are hidden.
//! - HATCH boundaries (polyline paths and line / arc / elliptic arc / spline edges)
//!   are drawn first and become faces; faces inside an even number of boundary loops
//!   (islands' holes) are removed again. Hatch patterns are not drawn.
//! - Blocks become component definitions (named after the block, only those used),
//!   INSERTs component instances (scale, rotation, extrusion and column / row arrays;
//!   one instance per array cell). Nested blocks nest; a block that contains itself
//!   is left out where it repeats.
//! - TEXT, MTEXT and visible ATTRIBs become texts at their insertion point with no
//!   leader (MTEXT formatting codes removed, `%%d` / `%%c` / `%%p` turned into
//!   symbols); their height, font and rotation are not kept (model texts are screen
//!   sized).
//! - DIMENSIONs the model can hold become dimensions: aligned ones, rotated ones
//!   measured along their own points, radius and diameter, with a text override when
//!   the drawing has one (`<>` keeps the measured value). Others (angular, ordinate,
//!   rotated ones measured along another direction) are drawn as the lines and texts
//!   of their dimension block.
//! - Layers become groups, one per layer named after it (SketchUp 2026's importer,
//!   release notes), or tags of one drawing. Inside blocks, entities on layer 0 take
//!   no tag (they show with the instance's).
//! - Units: from the header's `$INSUNITS` or as chosen.
//! - Not read: solids and ACIS bodies, regions, images, leaders, tables, points,
//!   hatch patterns, paper space and external references. DWG is a closed format and
//!   not read.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::f64::consts::TAU;

use glam::{DMat4, DVec2, DVec3, DVec4};

use crate::annotation::{Dimension, DimensionKind};
use crate::entities::{Arc, DefinitionId, EdgeId, Entities, FaceId, Instance, LayerId};
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

/// A shape read from the drawing, in drawing units and the coordinates of the block
/// (or the drawing) holding it.
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
    /// A polyline drawn as one curve (ellipses, splines); a closed one repeats its
    /// first point.
    Curve(Vec<DVec3>),
    /// A filled area (HATCH): boundary loops in the plane with normal `normal`; by
    /// the even-odd rule, loops inside loops are holes.
    Region {
        loops: Vec<Vec<DVec3>>,
        normal: DVec3,
    },
    /// A block reference: `transform` places the block's coordinates (relative to
    /// its base point) in the parent's.
    Insert { block: String, transform: DMat4 },
    /// A text at a point.
    Text { point: DVec3, text: String },
    /// A dimension the model can hold.
    Dimension(Dimension),
    /// A dimension the model cannot hold: the block with its drawn lines and texts,
    /// drawn in place.
    Exploded(String),
}

/// A block definition.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Block {
    /// Base point (block coordinates placed at an INSERT's insertion point).
    pub base: DVec3,
    /// Shapes with the index of their layer.
    pub shapes: Vec<(usize, Shape)>,
}

/// A parsed drawing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drawing {
    /// Layer names in table order, then layers only entities name.
    pub layers: Vec<DxfLayer>,
    /// Model space shapes with the index of their layer.
    pub shapes: Vec<(usize, Shape)>,
    /// Blocks by name (external references left out).
    pub blocks: BTreeMap<String, Block>,
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

    fn has(&self, code: i32) -> bool {
        self.groups.iter().any(|g| g.code == code)
    }

    fn point(&self, code: i32) -> DVec3 {
        DVec3::new(self.f(code), self.f(code + 10), self.f(code + 20))
    }

    /// Every point of codes `code`, `code + 10`, `code + 20`, in order (a new point
    /// starts at each `code` group).
    fn points(&self, code: i32) -> Vec<DVec3> {
        let mut out: Vec<DVec3> = Vec::new();
        for g in self.groups {
            if g.code == code {
                out.push(DVec3::new(g.num, 0.0, 0.0));
            } else if g.code == code + 10 {
                if let Some(p) = out.last_mut() {
                    p.y = g.num;
                }
            } else if g.code == code + 20
                && let Some(p) = out.last_mut()
            {
                p.z = g.num;
            }
        }
        out
    }

    /// Every value of `code`, in order.
    fn all(&self, code: i32) -> Vec<f64> {
        self.groups
            .iter()
            .filter(|g| g.code == code)
            .map(|g| g.num)
            .collect()
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

    /// Object coordinates to world coordinates.
    fn ocs(&self) -> DMat4 {
        ocs_matrix(self.extrusion())
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

fn ocs_matrix(n: DVec3) -> DMat4 {
    let (ax, ay) = ocs_axes(n);
    DMat4::from_cols(ax.extend(0.0), ay.extend(0.0), n.extend(0.0), DVec4::W)
}

fn arc_segments(sweep: f64) -> usize {
    // Damaged files: no more than a turn's worth (NaN gives the minimum).
    ((24.0 * sweep.abs().min(TAU) / TAU).ceil() as usize).max(2)
}

/// `t` plus the whole turns that put it above `floor` (unchanged when it is above
/// already), as repeated `t += TAU` gives it; `None` when no number of turns does
/// (NaN, infinite or huge angles from a damaged file).
fn turns_above(mut t: f64, floor: f64) -> Option<f64> {
    for _ in 0..64 {
        if t > floor {
            return Some(t);
        }
        t += TAU;
    }
    if t <= floor {
        t += (((floor - t) / TAU).floor() + 1.0) * TAU;
    }
    (t > floor && t.is_finite() && (t - floor) <= 2.0 * TAU).then_some(t)
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
        let (center, start, sweep) = bulge_arc(p1, p2, bulge);
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

/// The arc of a bulged segment: center, start angle and signed sweep.
fn bulge_arc(p1: DVec2, p2: DVec2, bulge: f64) -> (DVec2, f64, f64) {
    let c = p2 - p1;
    let center = (p1 + p2) / 2.0 + c.perp() * ((1.0 - bulge * bulge) / (4.0 * bulge));
    let sweep = 4.0 * bulge.atan();
    let start = (p1 - center).y.atan2((p1 - center).x);
    (center, start, sweep)
}

/// Points along a bulged segment from `p1` to `p2` (both included).
fn bulge_points(p1: DVec2, p2: DVec2, bulge: f64) -> Vec<DVec2> {
    if bulge.abs() < 1e-9 || p1.distance(p2) < 1e-12 {
        return vec![p1, p2];
    }
    let (center, start, sweep) = bulge_arc(p1, p2, bulge);
    let r = p1.distance(center);
    let n = arc_segments(sweep);
    let mut out: Vec<DVec2> = (0..n)
        .map(|i| {
            let a = start + sweep * i as f64 / n as f64;
            center + DVec2::new(a.cos(), a.sin()) * r
        })
        .collect();
    out.push(p2);
    out
}

/// Points of an elliptical arc: `center`, major axis vector, minor axis vector, from
/// parameter `t0` to `t1` (radians, `t1 > t0`), 24 segments per turn.
fn ellipse_points(center: DVec3, major: DVec3, minor: DVec3, t0: f64, t1: f64) -> Vec<DVec3> {
    let n = arc_segments(t1 - t0);
    (0..=n)
        .map(|i| {
            let t = t0 + (t1 - t0) * i as f64 / n as f64;
            center + major * t.cos() + minor * t.sin()
        })
        .collect()
}

/// A point of a NURBS curve of degree `p` at parameter `t` (de Boor).
fn nurbs_point(p: usize, knots: &[f64], ctrl: &[DVec3], weights: &[f64], t: f64) -> DVec3 {
    let n = ctrl.len();
    let mut k = p;
    while k + 1 < n && knots[k + 1] <= t {
        k += 1;
    }
    let mut d: Vec<DVec4> = (0..=p)
        .map(|j| {
            let i = k - p + j;
            let w = weights.get(i).copied().filter(|w| *w > 0.0).unwrap_or(1.0);
            (ctrl[i] * w).extend(w)
        })
        .collect();
    for r in 1..=p {
        for j in (r..=p).rev() {
            let i = k - p + j;
            let den = knots[i + p + 1 - r] - knots[i];
            let a = if den.abs() < 1e-15 {
                0.0
            } else {
                (t - knots[i]) / den
            };
            d[j] = d[j - 1] * (1.0 - a) + d[j] * a;
        }
    }
    let h = d[p];
    if h.w.abs() < 1e-15 {
        h.truncate()
    } else {
        h.truncate() / h.w
    }
}

/// Points along a spline: from its control points and knots when they agree, else
/// through its fit points, else along its control polygon.
fn spline_points(
    degree: usize,
    knots: &[f64],
    ctrl: &[DVec3],
    weights: &[f64],
    fit: &[DVec3],
) -> Vec<DVec3> {
    let n = ctrl.len();
    if degree >= 1 && n > degree && knots.len() == n + degree + 1 {
        let mut out = Vec::new();
        for k in degree..n {
            let (a, b) = (knots[k], knots[k + 1]);
            if b - a <= 1e-12 {
                continue;
            }
            for s in 0..8 {
                out.push(nurbs_point(
                    degree,
                    knots,
                    ctrl,
                    weights,
                    a + (b - a) * f64::from(s) / 8.0,
                ));
            }
        }
        out.push(nurbs_point(degree, knots, ctrl, weights, knots[n]));
        if out.len() >= 2 {
            return out;
        }
    }
    if fit.len() >= 2 {
        fit.to_vec()
    } else {
        ctrl.to_vec()
    }
}

/// TEXT special codes: `%%d` degree, `%%c` diameter, `%%p` plus-minus, `%%%`
/// percent; `%%u` / `%%o` (underline / overline toggles) dropped.
fn text_codes(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("%%") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let mut chars = after.chars();
        match chars.next().map(|c| c.to_ascii_lowercase()) {
            Some('d') => out.push('°'),
            Some('c') => out.push('Ø'),
            Some('p') => out.push('±'),
            Some('%') => out.push('%'),
            Some('u' | 'o' | 'k') => {}
            Some(c) => {
                out.push_str("%%");
                out.push(c);
            }
            None => out.push_str("%%"),
        }
        rest = chars.as_str();
    }
    out.push_str(rest);
    out
}

/// MTEXT with its formatting codes removed: `\P` line breaks, `\~` spaces, stacked
/// fractions as `a/b`, `\U+XXXX` characters; font, height, color, width and other
/// codes and `{}` grouping dropped.
fn mtext_plain(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '{' | '}' => {}
            '\\' => match it.next() {
                Some('P' | 'n') => out.push('\n'),
                Some('~') => out.push(' '),
                Some(c @ ('\\' | '{' | '}')) => out.push(c),
                Some('S') => {
                    for c in it.by_ref() {
                        match c {
                            ';' => break,
                            '^' | '#' => out.push('/'),
                            c => out.push(c),
                        }
                    }
                }
                Some('U') if it.peek() == Some(&'+') => {
                    it.next();
                    let hex: String = (0..4).filter_map(|_| it.next()).collect();
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(ch);
                    }
                }
                Some('L' | 'l' | 'O' | 'o' | 'K' | 'k' | 'X' | 'N') => {}
                Some(_) => {
                    // A code with a value up to `;`.
                    for c in it.by_ref() {
                        if c == ';' {
                            break;
                        }
                    }
                }
                None => {}
            },
            c => out.push(c),
        }
    }
    text_codes(&out).trim().to_owned()
}

/// Reads the groups of a HATCH from `i` on.
struct Cursor<'a> {
    g: &'a [Group],
    i: usize,
}

impl Cursor<'_> {
    /// The next group with `code` (skipping others).
    fn take(&mut self, code: i32) -> Option<f64> {
        while self.i < self.g.len() {
            let g = &self.g[self.i];
            self.i += 1;
            if g.code == code {
                return Some(g.num);
            }
        }
        None
    }

    /// The next group's value when its code is `code`.
    fn take_if(&mut self, code: i32) -> Option<f64> {
        let g = self.g.get(self.i).filter(|g| g.code == code)?;
        self.i += 1;
        Some(g.num)
    }

    fn take2(&mut self, code: i32) -> Option<DVec2> {
        Some(DVec2::new(self.take(code)?, self.take(code + 10)?))
    }
}

/// Points of an arc (`ccw` false: AutoCAD stores clockwise hatch arcs with their
/// angles mirrored) from `start` to `end` degrees.
fn hatch_arc(
    center: DVec2,
    major: DVec2,
    ratio: f64,
    start: f64,
    end: f64,
    ccw: bool,
) -> Vec<DVec2> {
    // Elliptic edges store angles, not parameters.
    let param = |a: f64| {
        if (ratio - 1.0).abs() < 1e-12 || ratio <= 0.0 {
            a
        } else {
            let p = (a.sin() / ratio).atan2(a.cos());
            p + ((a - p) / TAU).round() * TAU
        }
    };
    let (mut s, mut e) = (param(start.to_radians()), param(end.to_radians()));
    if !ccw {
        (s, e) = (-s, -e);
    }
    let minor = major.perp() * ratio;
    let sweep = if ccw {
        turns_above(e - s, 1e-12)
    } else {
        turns_above(s - e, 1e-12).map(|t| -t)
    };
    let Some(sweep) = sweep else {
        return Vec::new();
    };
    let sweep = sweep.clamp(-TAU, TAU);
    let n = arc_segments(sweep);
    (0..=n)
        .map(|i| {
            let t = s + sweep * i as f64 / n as f64;
            center + major * t.cos() + minor * t.sin()
        })
        .collect()
}

/// The boundary loops of a HATCH (in its OCS), or `None` when it has none.
fn hatch_loops(e: &Ent) -> Option<Vec<Vec<DVec2>>> {
    let start = e.groups.iter().position(|g| g.code == 91)?;
    let paths = e.groups[start].num as usize;
    let mut c = Cursor {
        g: e.groups,
        i: start + 1,
    };
    let mut loops = Vec::new();
    for _ in 0..paths.min(10_000) {
        let flags = c.take(92)? as i64;
        let mut pts: Vec<DVec2> = Vec::new();
        let push = |pts: &mut Vec<DVec2>, more: Vec<DVec2>| {
            for p in more {
                if pts.last().is_none_or(|l| l.distance(p) > 1e-12) {
                    pts.push(p);
                }
            }
        };
        if flags & 2 != 0 {
            let bulges = c.take(72)? != 0.0;
            let closed = c.take(73)? != 0.0;
            let n = c.take(93)? as usize;
            let mut verts: Vec<(DVec2, f64)> = Vec::new();
            for _ in 0..n.min(1_000_000) {
                let p = c.take2(10)?;
                let b = if bulges {
                    c.take_if(42).unwrap_or(0.0)
                } else {
                    0.0
                };
                verts.push((p, b));
            }
            let count = if closed {
                verts.len()
            } else {
                verts.len().saturating_sub(1)
            };
            for i in 0..count {
                let (p1, b) = verts[i];
                let p2 = verts[(i + 1) % verts.len()].0;
                push(&mut pts, bulge_points(p1, p2, b));
            }
            if !closed && let Some(&(p, _)) = verts.last() {
                push(&mut pts, vec![p]);
            }
        } else {
            let n = c.take(93)? as usize;
            for _ in 0..n.min(1_000_000) {
                let kind = c.take(72)? as i64;
                match kind {
                    1 => {
                        let a = c.take2(10)?;
                        let b = c.take2(11)?;
                        push(&mut pts, vec![a, b]);
                    }
                    2 => {
                        let center = c.take2(10)?;
                        let r = c.take(40)?;
                        let (s, t) = (c.take(50)?, c.take(51)?);
                        let ccw = c.take(73)? != 0.0;
                        push(&mut pts, hatch_arc(center, DVec2::X * r, 1.0, s, t, ccw));
                    }
                    3 => {
                        let center = c.take2(10)?;
                        let major = c.take2(11)?;
                        let ratio = c.take(40)?;
                        let (s, t) = (c.take(50)?, c.take(51)?);
                        let ccw = c.take(73)? != 0.0;
                        push(&mut pts, hatch_arc(center, major, ratio, s, t, ccw));
                    }
                    4 => {
                        let degree = c.take(94)? as usize;
                        let rational = c.take(73)? != 0.0;
                        c.take(74)?;
                        let nk = c.take(95)? as usize;
                        let nc = c.take(96)? as usize;
                        let knots: Vec<f64> =
                            (0..nk.min(100_000)).map_while(|_| c.take(40)).collect();
                        let mut ctrl = Vec::new();
                        let mut weights = Vec::new();
                        for _ in 0..nc.min(100_000) {
                            ctrl.push(c.take2(10)?.extend(0.0));
                            if rational {
                                weights.push(c.take_if(42).unwrap_or(1.0));
                            }
                        }
                        let mut fit = Vec::new();
                        if let Some(nf) = c.take_if(97) {
                            for _ in 0..(nf as usize).min(100_000) {
                                fit.push(c.take2(11)?.extend(0.0));
                            }
                        }
                        let p = spline_points(degree, &knots, &ctrl, &weights, &fit);
                        push(&mut pts, p.iter().map(|p| p.truncate()).collect());
                    }
                    _ => return None,
                }
            }
        }
        // Source boundary objects.
        if let Some(n) = c.take_if(97) {
            for _ in 0..(n as usize).min(1_000_000) {
                c.take_if(330);
            }
        }
        if pts.len() >= 2 && pts[0].distance(pts[pts.len() - 1]) < 1e-9 {
            pts.pop();
        }
        if pts.len() >= 3 {
            loops.push(pts);
        }
    }
    Some(loops)
}

/// A dimension's text override: `None` for the measured value (empty, or containing
/// `<>`).
fn dimension_text(s: &str) -> Option<String> {
    let s = mtext_plain(s);
    (!s.is_empty() && !s.contains("<>")).then_some(s)
}

/// The dimension the model holds for a DIMENSION entity, if any.
fn dimension(e: &Ent) -> Option<Dimension> {
    let ocs = e.ocs();
    let label = e.has(11).then(|| ocs.transform_point3(e.point(11)));
    let mut d = match e.int(70) & 7 {
        kind @ (0 | 1) => {
            let (a, b) = (e.point(13), e.point(14));
            let dir = (b - a).try_normalize()?;
            if kind == 0 {
                let r = e.f(50).to_radians();
                let along = ocs.transform_vector3(DVec3::new(r.cos(), r.sin(), 0.0));
                if dir.cross(along).length() > 1e-6 {
                    return None;
                }
            }
            Dimension::linear(a, b, Dimension::offset_towards(a, b, e.point(10)))
        }
        kind @ (3 | 4) => {
            let (a, b) = if kind == 4 {
                (e.point(10), e.point(15))
            } else {
                (e.point(15), e.point(10))
            };
            if a.distance(b) < 1e-12 {
                return None;
            }
            let mut d = Dimension::linear(a, b, label.map_or(DVec3::ZERO, |l| l - b));
            d.kind = if kind == 4 {
                DimensionKind::Radius
            } else {
                DimensionKind::Diameter
            };
            d
        }
        _ => return None,
    };
    d.text = dimension_text(e.text(1));
    Some(d)
}

/// Layer names to indices, in the order met.
#[derive(Default)]
struct Layers {
    list: Vec<DxfLayer>,
    index: HashMap<String, usize>,
}

impl Layers {
    fn get(&mut self, name: &str) -> usize {
        let name = if name.is_empty() { "0" } else { name };
        if let Some(&i) = self.index.get(name) {
            return i;
        }
        self.list.push(DxfLayer {
            name: name.to_owned(),
            color: [255, 255, 255],
            hidden: false,
        });
        self.index.insert(name.to_owned(), self.list.len() - 1);
        self.list.len() - 1
    }
}

/// Reads a DXF file.
pub fn parse(bytes: &[u8]) -> Result<Drawing, String> {
    let groups = read_groups(bytes)?;
    let mut d = Drawing::default();
    let mut layers = Layers::default();
    // Split into entities at code 0, remembering sections.
    let mut section = String::new();
    let mut i = 0;
    let mut ents: Vec<(String, Ent)> = Vec::new();
    // Array cells (INSERT rows × columns) left for the whole file.
    let mut cells = MAX_ARRAY_CELLS;
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
        match (sec.as_str(), e.kind) {
            ("TABLES", "LAYER") => {
                let l = layers.get(e.text(2));
                let color = e.int(62);
                layers.list[l].color = aci_color(color);
                layers.list[l].hidden = color < 0 || e.int(70) & 1 != 0;
                k += 1;
            }
            ("BLOCKS", "BLOCK") => {
                let mut end = k + 1;
                while end < ents.len() && ents[end].0 == "BLOCKS" && ents[end].1.kind != "ENDBLK" {
                    end += 1;
                }
                let shapes = read_entities(&ents[k + 1..end], &mut layers, false, &mut cells);
                // External references (4) and overlays (8) are not in the file.
                if e.int(70) & 12 == 0 {
                    d.blocks.insert(
                        e.text(2).to_owned(),
                        Block {
                            base: e.point(10),
                            shapes,
                        },
                    );
                }
                k = end + 1;
            }
            ("ENTITIES", _) => {
                let mut end = k;
                while end < ents.len() && ents[end].0 == "ENTITIES" {
                    end += 1;
                }
                let shapes = read_entities(&ents[k..end], &mut layers, true, &mut cells);
                d.shapes.extend(shapes);
                k = end;
            }
            _ => k += 1,
        }
    }
    d.layers = layers.list;
    Ok(d)
}

/// Most INSERT array cells in a file: a few bytes can ask for 1000 × 1000 copies, any
/// number of times.
const MAX_ARRAY_CELLS: usize = 100_000;

/// Shapes [`to_model`] may add over all block expansions besides twice the
/// drawing's own: exploded blocks (dimension blocks) naming each other many times
/// over grow exponentially with nesting.
const EXTRA_BUILT_SHAPES: usize = 200_000;

/// The shapes of a run of entities (a block's or the ENTITIES section's); `model`
/// skips paper space entities. `cells` counts down the INSERT array cells the file
/// may still have; past it, arrays keep their first cell.
fn read_entities(
    ents: &[(String, Ent)],
    layers: &mut Layers,
    model: bool,
    cells: &mut usize,
) -> Vec<(usize, Shape)> {
    let mut out: Vec<(usize, Shape)> = Vec::new();
    let mut k = 0;
    while k < ents.len() {
        let e = &ents[k].1;
        k += 1;
        // Paper space entities are skipped.
        if model && e.int(67) == 1 {
            if e.kind == "POLYLINE" {
                while k < ents.len() && ents[k].1.kind != "SEQEND" {
                    k += 1;
                }
            }
            continue;
        }
        let l = layers.get(e.text(8));
        let mut push = |s: Shape| out.push((l, s));
        match e.kind {
            "LINE" => push(Shape::Line {
                a: e.point(10),
                b: e.point(11),
                hidden: false,
            }),
            "3DFACE" => {
                let mut corners = vec![e.point(10), e.point(11), e.point(12), e.point(13)];
                let flags = e.int(70);
                let mut hidden: Vec<bool> = (0..4).map(|b| flags & (1 << b) != 0).collect();
                if corners[3].distance(corners[2]) < 1e-12 {
                    corners.pop();
                    hidden.pop();
                }
                push(Shape::Face { corners, hidden });
            }
            "SOLID" | "TRACE" => {
                // Corners in the order 1, 2, 4, 3 (OCS).
                let ocs = e.ocs();
                let mut corners: Vec<DVec3> = [10, 11, 13, 12]
                    .map(|c| ocs.transform_point3(e.point(c)))
                    .to_vec();
                corners.dedup_by(|a, b| a.distance(*b) < 1e-12);
                if corners.len() > 3 && corners[0].distance(corners[corners.len() - 1]) < 1e-12 {
                    corners.pop();
                }
                if corners.len() >= 3 {
                    let hidden = vec![false; corners.len()];
                    push(Shape::Face { corners, hidden });
                }
            }
            "CIRCLE" | "ARC" => {
                let n = e.extrusion();
                let (ax, ay) = ocs_axes(n);
                let c = e.point(10);
                let (start, end) = if e.kind == "CIRCLE" {
                    (0.0, TAU)
                } else {
                    let s = e.f(50).to_radians();
                    let Some(t) = turns_above(e.f(51).to_radians(), s) else {
                        continue;
                    };
                    // More than a turn (a damaged file) is a circle.
                    (s, t.min(s + TAU))
                };
                let radius = e.f(40);
                if radius > 0.0 {
                    push(Shape::Arc(Arc {
                        center: ax * c.x + ay * c.y + n * c.z,
                        normal: n,
                        x_axis: ax,
                        radius,
                        start_angle: start,
                        end_angle: end,
                        segments: arc_segments(end - start),
                    }));
                }
            }
            "ELLIPSE" => {
                let n = e.extrusion();
                let major = e.point(11);
                let minor = n.cross(major) * e.f(40);
                let t0 = e.f(41);
                let Some(t1) = turns_above(e.num(42).unwrap_or(TAU), t0 + 1e-12) else {
                    continue;
                };
                let t1 = t1.min(t0 + TAU);
                if major.length() > 1e-12 && minor.length() > 1e-12 {
                    let mut pts = ellipse_points(e.point(10), major, minor, t0, t1);
                    if t1 - t0 >= TAU - 1e-9 {
                        let first = pts[0];
                        if let Some(last) = pts.last_mut() {
                            *last = first;
                        }
                    }
                    push(Shape::Curve(pts));
                }
            }
            "SPLINE" => {
                let mut pts = spline_points(
                    e.int(71).max(0) as usize,
                    &e.all(40),
                    &e.points(10),
                    &e.all(41),
                    &e.points(11),
                );
                // Closed (1) or periodic (2): join the ends.
                if e.int(70) & 3 != 0
                    && pts.len() > 2
                    && let Some(&first) = pts.first()
                {
                    let last = pts[pts.len() - 1];
                    if last.distance(first) < 1e-9 {
                        let n = pts.len();
                        pts[n - 1] = first;
                    } else {
                        pts.push(first);
                    }
                }
                if pts.len() >= 2 {
                    push(Shape::Curve(pts));
                }
            }
            "HATCH" => {
                let n = e.extrusion();
                let ocs = ocs_matrix(n);
                let elevation = e.f(30);
                if let Some(loops) = hatch_loops(e).filter(|l| !l.is_empty()) {
                    let loops = loops
                        .iter()
                        .map(|l| {
                            l.iter()
                                .map(|p| ocs.transform_point3(p.extend(elevation)))
                                .collect()
                        })
                        .collect();
                    push(Shape::Region { loops, normal: n });
                }
            }
            "INSERT" => {
                let n = e.extrusion();
                let ocs = ocs_matrix(n);
                let scale = DVec3::new(
                    e.num(41).unwrap_or(1.0),
                    e.num(42).unwrap_or(1.0),
                    e.num(43).unwrap_or(1.0),
                );
                if scale.x * scale.y * scale.z == 0.0 {
                    continue;
                }
                let place = ocs
                    * DMat4::from_translation(e.point(10))
                    * DMat4::from_rotation_z(e.f(50).to_radians());
                let (mut cols, mut rows) = (e.int(70).clamp(1, 1000), e.int(71).clamp(1, 1000));
                let n = (cols * rows) as usize;
                if n > *cells {
                    (cols, rows) = (1, 1);
                } else {
                    *cells -= n;
                }
                let (dx, dy) = (e.f(44), e.f(45));
                for r in 0..rows {
                    for c in 0..cols {
                        let cell =
                            DMat4::from_translation(DVec3::new(c as f64 * dx, r as f64 * dy, 0.0));
                        push(Shape::Insert {
                            block: e.text(2).to_owned(),
                            transform: place * cell * DMat4::from_scale(scale),
                        });
                    }
                }
            }
            "TEXT" | "ATTRIB" => {
                // Invisible attributes (1) are left out.
                if e.kind == "ATTRIB" && e.int(70) & 1 != 0 {
                    continue;
                }
                let aligned = (e.int(72) != 0 || e.int(73) != 0) && e.has(11);
                let p = e.point(if aligned { 11 } else { 10 });
                let text = text_codes(e.text(1)).trim().to_owned();
                if !text.is_empty() {
                    push(Shape::Text {
                        point: e.ocs().transform_point3(p),
                        text,
                    });
                }
            }
            "MTEXT" => {
                let mut raw: String = e
                    .groups
                    .iter()
                    .filter(|g| g.code == 3)
                    .map(|g| g.text.as_str())
                    .collect();
                raw.push_str(e.text(1));
                let text = mtext_plain(&raw);
                if !text.is_empty() {
                    push(Shape::Text {
                        point: e.point(10),
                        text,
                    });
                }
            }
            "DIMENSION" => match dimension(e) {
                Some(d) => push(Shape::Dimension(d)),
                None if !e.text(2).is_empty() => push(Shape::Exploded(e.text(2).to_owned())),
                None => {}
            },
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
                    push(s);
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
                            push(Shape::Face { corners, hidden });
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
                                push(Shape::Face {
                                    corners: vec![
                                        p(a, b),
                                        p(a + 1, b),
                                        p(a + 1, b + 1),
                                        p(a, b + 1),
                                    ],
                                    hidden: vec![false; 4],
                                });
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
                        push(Shape::Line {
                            a: pts[i],
                            b: pts[(i + 1) % pts.len()],
                            hidden: false,
                        });
                    }
                } else {
                    let pts: Vec<(DVec2, f64)> = verts
                        .iter()
                        .map(|v| (v.point(10).truncate(), v.f(42)))
                        .collect();
                    for s in polyline_2d(&pts, flags & 1 != 0, e.f(30), e.extrusion()) {
                        push(s);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Points scaled with repeats closer than the tolerance dropped (a closed list keeps
/// its first point as its last).
fn clean_points(points: &[DVec3], scale: f64) -> Vec<DVec3> {
    let closed = points.len() > 2 && points[0].distance(points[points.len() - 1]) < 1e-12;
    let mut out: Vec<DVec3> = Vec::new();
    for p in points {
        let q = *p * scale;
        if out.last().is_none_or(|l| l.distance(q) >= crate::TOL) {
            out.push(q);
        }
    }
    if closed && out.len() > 2 {
        let first = out[0];
        let n = out.len();
        if out[n - 1].distance(first) < crate::TOL {
            out[n - 1] = first;
        } else {
            out.push(first);
        }
    }
    out
}

/// Adds `shape` (a geometric one) scaled by `scale` to `ents`; returns the edges it
/// covers.
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
        Shape::Curve(points) => {
            let pts = clean_points(points, scale);
            if pts.len() < 2 {
                return Vec::new();
            }
            crate::add_curve(ents, &pts, None)
        }
        Shape::Region { loops, normal } => draw_region(ents, loops, *normal, scale),
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
        _ => Vec::new(),
    };
    for e in hide {
        if let Some(edge) = ents.edges.get_mut(e) {
            edge.attrs.hidden = true;
        }
    }
    edges
}

/// Whether `p` is inside polygon `poly` (crossing count).
fn inside(poly: &[DVec2], p: DVec2) -> bool {
    let mut c = false;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            c = !c;
        }
    }
    c
}

/// Draws a hatch's boundary loops; of the faces they make, those not inside an odd
/// number of loops (holes) are removed.
fn draw_region(
    ents: &mut Entities,
    loops: &[Vec<DVec3>],
    normal: DVec3,
    scale: f64,
) -> Vec<EdgeId> {
    let before: BTreeSet<FaceId> = ents.faces.keys().collect();
    let loops: Vec<Vec<DVec3>> = loops
        .iter()
        .map(|l| {
            let mut c = clean_points(l, scale);
            if c.len() > 1 && c[0].distance(c[c.len() - 1]) < crate::TOL {
                c.pop();
            }
            c
        })
        .filter(|l| l.len() >= 3)
        .collect();
    let mut edges = Vec::new();
    for l in &loops {
        for i in 0..l.len() {
            let (a, b) = (l[i], l[(i + 1) % l.len()]);
            if a.distance(b) >= crate::TOL {
                edges.extend(crate::add_line(ents, a, b));
            }
        }
    }
    let (ax, ay) = ocs_axes(normal);
    let flat = |p: DVec3| DVec2::new(p.dot(ax), p.dot(ay));
    let polys: Vec<Vec<DVec2>> = loops
        .iter()
        .map(|l| l.iter().map(|&p| flat(p)).collect())
        .collect();
    let new: Vec<FaceId> = ents.faces.keys().filter(|f| !before.contains(f)).collect();
    for f in new {
        if ents.faces[f].normal.cross(normal).length() > 1e-6 {
            continue;
        }
        let tris = crate::tessellate::triangulate(ents, f);
        let Some(t) = tris.iter().max_by(|a, b| {
            let area = |t: &[DVec3; 3]| (t[1] - t[0]).cross(t[2] - t[0]).length();
            area(a).total_cmp(&area(b))
        }) else {
            continue;
        };
        let c = flat((t[0] + t[1] + t[2]) / 3.0);
        if polys.iter().filter(|p| inside(p, c)).count() % 2 == 0 {
            ents.remove_face(f);
        }
    }
    edges.retain(|e| ents.edges.contains_key(*e));
    edges
}

fn instance(definition: DefinitionId, transform: DMat4, name: String) -> Instance {
    Instance {
        definition,
        transform,
        name,
        hidden: false,
        locked: false,
        layer: None,
        material: None,
        attrs: Default::default(),
    }
}

fn is_empty(ents: &Entities) -> bool {
    ents.edges.is_empty()
        && ents.instances.is_empty()
        && ents.texts.is_empty()
        && ents.dimensions.is_empty()
}

/// Builds the model: blocks as definitions on first use.
struct Builder<'a> {
    d: &'a Drawing,
    scale: f64,
    /// The tag of each DXF layer (tags mode).
    tags: Option<Vec<Option<LayerId>>>,
    model: Model,
    /// Definitions made for blocks; `None` while being built (a block containing
    /// itself) or when empty.
    defs: HashMap<String, Option<DefinitionId>>,
    /// Shapes that may still be added ([`EXTRA_BUILT_SHAPES`]).
    budget: usize,
}

impl<'a> Builder<'a> {
    fn tag(&self, layer: usize) -> Option<LayerId> {
        self.tags
            .as_ref()
            .and_then(|t| t.get(layer).copied().flatten())
    }

    /// Adds `shapes` to `ents`: hatches first (their faces are sorted out before
    /// other lines split them), then the rest in order.
    fn add(&mut self, ents: &mut Entities, shapes: &[&'a (usize, Shape)], depth: usize) {
        let region = |x: &&(usize, Shape)| matches!(x.1, Shape::Region { .. });
        let ordered: Vec<&'a (usize, Shape)> = shapes
            .iter()
            .copied()
            .filter(region)
            .chain(shapes.iter().copied().filter(|x| !region(x)))
            .collect();
        for (l, s) in ordered {
            if self.budget == 0 {
                return;
            }
            self.budget -= 1;
            let l = *l;
            let tag = self.tag(l);
            let scale = self.scale;
            match s {
                Shape::Insert { block, transform } => {
                    let Some(def) = self.definition(block, depth) else {
                        continue;
                    };
                    let base = self.d.blocks[block].base;
                    let xf = DMat4::from_scale(DVec3::splat(scale))
                        * *transform
                        * DMat4::from_translation(-base)
                        * DMat4::from_scale(DVec3::splat(1.0 / scale));
                    let mut inst = instance(def, xf, String::new());
                    inst.layer = tag;
                    ents.instances.push(inst);
                }
                Shape::Text { point, text } => {
                    let mut t =
                        crate::annotation::Text::leader(*point * scale, DVec3::ZERO, text.clone());
                    t.style = Some(crate::model_data::TextStyle {
                        leader: crate::model_data::LeaderKind::Hidden,
                        ..Default::default()
                    });
                    ents.texts.push(t);
                }
                Shape::Dimension(d) => {
                    let mut d = d.clone();
                    d.a *= scale;
                    d.b *= scale;
                    d.offset *= scale;
                    ents.dimensions.push(d);
                }
                Shape::Exploded(block) => {
                    let d = self.d;
                    if depth < 32
                        && let Some(b) = d.blocks.get(block)
                    {
                        let items: Vec<&(usize, Shape)> = b.shapes.iter().collect();
                        self.add(ents, &items, depth + 1);
                    }
                }
                _ => {
                    for e in draw(ents, s, scale) {
                        if let Some(edge) = ents.edges.get_mut(e) {
                            edge.attrs.layer = tag;
                        }
                    }
                }
            }
        }
    }

    /// Faces take the tag of their first edge (tags mode).
    fn finish(&self, ents: &mut Entities) {
        if self.tags.is_none() {
            return;
        }
        let faces: Vec<_> = ents.faces.keys().collect();
        for f in faces {
            let first = ents.faces[f].outer.coedges.first().map(|c| c.edge);
            ents.faces[f].attrs.layer = first.and_then(|e| ents.edges[e].attrs.layer);
        }
    }

    /// The definition of block `name`, made on first use.
    fn definition(&mut self, name: &str, depth: usize) -> Option<DefinitionId> {
        if let Some(&d) = self.defs.get(name) {
            return d;
        }
        let d = self.d;
        let block = d.blocks.get(name)?;
        if depth > 32 {
            return None;
        }
        self.defs.insert(name.to_owned(), None);
        let mut ents = Entities::new();
        let items: Vec<&(usize, Shape)> = block.shapes.iter().collect();
        self.add(&mut ents, &items, depth + 1);
        self.finish(&mut ents);
        if is_empty(&ents) {
            return None;
        }
        let def_name = self.model.unique_definition_name(name);
        let id = self.model.definitions.insert(Definition {
            name: def_name,
            entities: ents,
            ..Default::default()
        });
        self.defs.insert(name.to_owned(), Some(id));
        Some(id)
    }
}

/// Builds a model from a parsed drawing: per layer a group in the root, or one
/// drawing with a tag per layer.
pub fn to_model(d: &Drawing, opts: &DxfOptions) -> Model {
    let scale = opts.inches_per_unit.or(d.inches_per_unit).unwrap_or(1.0);
    let mut b = Builder {
        d,
        scale,
        tags: None,
        model: Model::default(),
        defs: HashMap::new(),
        budget: EXTRA_BUILT_SHAPES
            + 2 * (d.shapes.len() + d.blocks.values().map(|b| b.shapes.len()).sum::<usize>()),
    };
    match opts.layers {
        LayerMode::Groups => {
            for (l, layer) in d.layers.iter().enumerate() {
                let items: Vec<&(usize, Shape)> =
                    d.shapes.iter().filter(|(x, _)| *x == l).collect();
                if items.is_empty() {
                    continue;
                }
                let mut ents = Entities::new();
                b.add(&mut ents, &items, 0);
                if is_empty(&ents) {
                    continue;
                }
                let name = b.model.unique_definition_name("Group#1");
                let def = b.model.definitions.insert(Definition {
                    name,
                    entities: ents,
                    is_group: true,
                    ..Default::default()
                });
                let mut inst = instance(def, DMat4::IDENTITY, layer.name.clone());
                inst.hidden = layer.hidden;
                b.model.root.instances.push(inst);
            }
        }
        LayerMode::Tags => {
            let mut tags: Vec<Option<LayerId>> = Vec::new();
            for layer in &d.layers {
                if layer.name == "0" {
                    tags.push(None);
                    continue;
                }
                b.model.layers.push(Layer {
                    name: layer.name.clone(),
                    color: layer.color,
                    visible: !layer.hidden,
                    ..Layer::default()
                });
                tags.push(Some(b.model.layers.len() - 1));
            }
            b.tags = Some(tags);
            let mut root = Entities::new();
            let items: Vec<&(usize, Shape)> = d.shapes.iter().collect();
            b.add(&mut root, &items, 0);
            b.finish(&mut root);
            b.model.root = root;
        }
    }
    b.model
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

    fn line(
        layer: &'static str,
        a: (&'static str, &'static str),
        b: (&'static str, &'static str),
    ) -> Vec<(i32, &'static str)> {
        vec![
            (0, "LINE"),
            (8, layer),
            (10, a.0),
            (20, a.1),
            (30, "0"),
            (11, b.0),
            (21, b.1),
            (31, "0"),
        ]
    }

    /// A drawing with a BLOCKS section.
    fn with_blocks(blocks: &[(i32, &str)], entities: &[(i32, &str)]) -> String {
        let mut g = vec![(0, "SECTION"), (2, "BLOCKS")];
        g.extend_from_slice(blocks);
        g.extend([(0, "ENDSEC"), (0, "SECTION"), (2, "ENTITIES")]);
        g.extend_from_slice(entities);
        g.extend([(0, "ENDSEC"), (0, "EOF")]);
        ascii(&g)
    }

    #[test]
    fn blocks_become_components_with_arrays_and_nesting() {
        let mut blocks = vec![
            (0, "BLOCK"),
            (8, "0"),
            (2, "Sq"),
            (70, "0"),
            (10, "1"),
            (20, "1"),
            (30, "0"),
        ];
        let c = [("1", "1"), ("3", "1"), ("3", "3"), ("1", "3")];
        for i in 0..4 {
            blocks.extend(line("0", c[i], c[(i + 1) % 4]));
        }
        blocks.push((0, "ENDBLK"));
        blocks.extend([(0, "BLOCK"), (8, "0"), (2, "Pair"), (70, "0")]);
        blocks.extend([(10, "0"), (20, "0"), (30, "0")]);
        blocks.extend([(0, "INSERT"), (8, "0"), (2, "Sq"), (10, "0"), (20, "0")]);
        blocks.extend([(0, "INSERT"), (8, "0"), (2, "Sq"), (10, "10"), (20, "0")]);
        blocks.push((50, "90"));
        // A block containing itself is left out where it repeats.
        blocks.extend([(0, "INSERT"), (8, "0"), (2, "Pair"), (10, "5"), (20, "5")]);
        blocks.push((0, "ENDBLK"));
        let ents = [
            (0, "INSERT"),
            (8, "Walls"),
            (2, "Pair"),
            (10, "100"),
            (20, "0"),
            (30, "0"),
            // 2 columns, 3 rows, scaled 2.
            (0, "INSERT"),
            (8, "Walls"),
            (2, "Sq"),
            (10, "0"),
            (20, "50"),
            (30, "0"),
            (41, "2"),
            (42, "2"),
            (43, "2"),
            (70, "2"),
            (71, "3"),
            (44, "5"),
            (45, "6"),
            // Unknown blocks are skipped.
            (0, "INSERT"),
            (8, "Walls"),
            (2, "Nope"),
        ];
        let d = parse(with_blocks(&blocks, &ents).as_bytes()).unwrap();
        assert_eq!(d.blocks.len(), 2);
        let m = to_model(&d, &DxfOptions::default());
        assert_eq!(m.root.instances.len(), 1);
        let walls = &m.definitions[m.root.instances[0].definition];
        assert_eq!(walls.entities.instances.len(), 1 + 6);
        let def = |name: &str| {
            m.definitions
                .iter()
                .find(|(_, d)| d.name == name)
                .map(|(id, _)| id)
                .unwrap()
        };
        let (sq, pair) = (def("Sq"), def("Pair"));
        assert!(!m.definitions[sq].is_group);
        assert_eq!(m.definitions[sq].entities.faces.len(), 1);
        assert_eq!(m.definitions[pair].entities.instances.len(), 2);
        for d in m.definitions.values() {
            assert!(crate::validate(&d.entities, crate::Checks::ALL).is_empty());
        }
        // The base point lands on the insertion point; the rotated copy turns.
        let rotated = &m.definitions[pair].entities.instances[1];
        let p = rotated
            .transform
            .transform_point3(DVec3::new(3.0, 1.0, 0.0));
        assert!(p.abs_diff_eq(DVec3::new(10.0, 2.0, 0.0), 1e-9), "{p}");
        // Array cell (column 1, row 2): spacing unscaled, contents scaled by 2.
        let cell = walls.entities.instances.iter().any(|i| {
            let a = i.transform.transform_point3(DVec3::new(1.0, 1.0, 0.0));
            let b = i.transform.transform_point3(DVec3::new(3.0, 1.0, 0.0));
            i.definition == sq
                && a.abs_diff_eq(DVec3::new(5.0, 62.0, 0.0), 1e-9)
                && b.abs_diff_eq(DVec3::new(9.0, 62.0, 0.0), 1e-9)
        });
        assert!(cell);
    }

    #[test]
    fn texts_dimensions_hatches_splines_and_ellipses() {
        let mut ents: Vec<(i32, &str)> = vec![
            (0, "TEXT"),
            (8, "T"),
            (10, "1"),
            (20, "2"),
            (30, "0"),
            (40, "1"),
            (1, "%%c10"),
            (0, "MTEXT"),
            (8, "T"),
            (10, "5"),
            (20, "5"),
            (30, "0"),
            (1, "{\\fArial|b1;Big}\\PLine 2"),
            // Aligned dimension.
            (0, "DIMENSION"),
            (8, "D"),
            (2, "*D1"),
            (10, "10"),
            (20, "3"),
            (30, "0"),
            (70, "1"),
            (1, ""),
            (13, "0"),
            (23, "0"),
            (33, "0"),
            (14, "10"),
            (24, "0"),
            (34, "0"),
            // Angular: drawn from its block.
            (0, "DIMENSION"),
            (8, "D"),
            (2, "*D1"),
            (70, "2"),
            // A square with a square hole (polyline paths).
            (0, "HATCH"),
            (8, "H"),
            (10, "0"),
            (20, "0"),
            (30, "0"),
            (2, "SOLID"),
            (70, "1"),
            (91, "2"),
        ];
        for (lo, hi, b, t) in [("20", "30", "0", "10"), ("23", "27", "3", "7")] {
            ents.extend([(92, "2"), (72, "0"), (73, "1"), (93, "4")]);
            for (x, y) in [(lo, b), (hi, b), (hi, t), (lo, t)] {
                ents.extend([(10, x), (20, y)]);
            }
            ents.push((97, "0"));
        }
        ents.extend([(75, "0"), (76, "1"), (98, "0")]);
        // A triangle from line edges.
        ents.extend([
            (0, "HATCH"),
            (8, "H"),
            (70, "1"),
            (91, "1"),
            (92, "1"),
            (93, "3"),
        ]);
        let tri = [("40", "0"), ("44", "0"), ("40", "4")];
        for i in 0..3 {
            let (a, b) = (tri[i], tri[(i + 1) % 3]);
            ents.extend([(72, "1"), (10, a.0), (20, a.1), (11, b.0), (21, b.1)]);
        }
        ents.push((97, "0"));
        // A quadratic spline over one knot span.
        ents.extend([
            (0, "SPLINE"),
            (8, "S"),
            (70, "8"),
            (71, "2"),
            (72, "6"),
            (73, "3"),
        ]);
        for k in ["0", "0", "0", "1", "1", "1"] {
            ents.push((40, k));
        }
        for (x, y) in [("0", "-20"), ("5", "-10"), ("10", "-20")] {
            ents.extend([(10, x), (20, y), (30, "0")]);
        }
        // A full ellipse.
        ents.extend([
            (0, "ELLIPSE"),
            (8, "E"),
            (10, "60"),
            (20, "0"),
            (30, "0"),
            (11, "4"),
            (21, "0"),
            (31, "0"),
            (40, "0.5"),
            (41, "0"),
            (42, "6.283185307179586"),
        ]);
        let mut blocks = vec![(0, "BLOCK"), (8, "0"), (2, "*D1"), (70, "1")];
        blocks.extend([(10, "0"), (20, "0"), (30, "0")]);
        blocks.extend(line("0", ("0", "-50"), ("5", "-50")));
        blocks.push((0, "ENDBLK"));
        let d = parse(with_blocks(&blocks, &ents).as_bytes()).unwrap();
        let m = to_model(
            &d,
            &DxfOptions {
                inches_per_unit: Some(1.0),
                layers: LayerMode::Tags,
            },
        );
        let r = &m.root;
        assert!(crate::validate(r, crate::Checks::ALL).is_empty());
        let texts: Vec<&str> = r.texts.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, ["Ø10", "Big\nLine 2"]);
        assert_eq!(r.texts[0].point, DVec3::new(1.0, 2.0, 0.0));
        assert_eq!(r.dimensions.len(), 1);
        let dim = &r.dimensions[0];
        assert_eq!(dim.kind, DimensionKind::Linear);
        assert!(dim.offset.abs_diff_eq(DVec3::new(0.0, 3.0, 0.0), 1e-9));
        assert_eq!(dim.text, None);
        // The hatch ring, the triangle and the ellipse.
        assert_eq!(r.faces.len(), 3);
        let ring = r.faces.values().find(|f| !f.holes.is_empty()).unwrap();
        assert_eq!(ring.holes.len(), 1);
        // Edges: exploded dimension 1, hatches 8 + 3, spline 8, ellipse 24.
        assert_eq!(r.edges.len(), 1 + 8 + 3 + 8 + 24);
        assert_eq!(r.curves.len(), 2);
        // The spline's middle point: the quadratic's peak at y = -15.
        let top = r
            .vertices
            .values()
            .filter(|v| v.pos.y < -12.0)
            .map(|v| v.pos.y)
            .fold(f64::MIN, f64::max);
        assert!((top + 15.0).abs() < 1e-9, "{top}");
        // Faces take their edges' tags.
        let h = m.layers.iter().position(|l| l.name == "H");
        assert!(r.faces.values().filter(|f| f.attrs.layer == h).count() == 2);
    }

    #[test]
    fn mtext_codes_and_dimension_overrides() {
        assert_eq!(
            mtext_plain("\\A1;{\\C1;Red} \\S1^2; \\U+00B0x\\~y\\\\z %%p1"),
            "Red 1/2 °x y\\z ±1"
        );
        assert_eq!(dimension_text("<> TYP"), None);
        assert_eq!(dimension_text("Wall"), Some("Wall".into()));
        assert_eq!(text_codes("50%%%"), "50%");
    }
}
