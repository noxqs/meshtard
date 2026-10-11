//! Toolbar icons, drawn procedurally with the egui painter.
//!
//! These are meshtard's own drawings. SketchUp's icons are Trimble's copyrighted artwork
//! and must not be copied; only the size and placement match. Every icon is designed on
//! a 24 x 24 grid and scales to the rect it is painted in (16 to 32 points look fine).
//!
//! Style: dark ink outlines, a warm face color for geometry, blue for selection and
//! camera, red for "action" arrows, green for scaling. Flat, no gradients.

use egui::epaint::{Mesh, PathShape, PathStroke};
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, vec2};
use mt_spec::IconColors;

mod extra;

/// Colors used by the icons.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub ink: Color32,
    pub accent: Color32,
    pub face: Color32,
    pub face_dark: Color32,
    pub red: Color32,
    pub green: Color32,
    pub paper: Color32,
}

impl Palette {
    pub fn from_spec(c: &IconColors) -> Self {
        let rgb = |c: [u8; 3]| Color32::from_rgb(c[0], c[1], c[2]);
        Self {
            ink: rgb(c.ink),
            accent: rgb(c.accent),
            face: rgb(c.face),
            face_dark: rgb(c.face_dark),
            red: rgb(c.red),
            green: rgb(c.green),
            paper: rgb(c.paper),
        }
    }

    /// The same palette washed out to light grey, for disabled buttons.
    pub fn disabled(&self) -> Self {
        let grey = |c: Color32| {
            let l = (0.3 * f32::from(c.r()) + 0.59 * f32::from(c.g()) + 0.11 * f32::from(c.b()))
                / 255.0;
            let v = (150.0 + l * 100.0).min(250.0) as u8;
            Color32::from_gray(v)
        };
        Self {
            ink: Color32::from_gray(165),
            accent: grey(self.accent),
            face: grey(self.face),
            face_dark: grey(self.face_dark),
            red: grey(self.red),
            green: grey(self.green),
            paper: self.paper,
        }
    }
}

/// Command ids that have their own icon. Every other id gets the fallback icon.
pub const KNOWN: &[&str] = &[
    "tool.select",
    "tool.lasso",
    "edit.make_component",
    "tool.eraser",
    "tool.tag",
    "tool.paint_bucket",
    "tool.line",
    "tool.freehand",
    "tool.rectangle",
    "tool.rotated_rectangle",
    "tool.circle",
    "tool.polygon",
    "tool.arc",
    "tool.arc_2pt",
    "tool.arc_3pt",
    "tool.pie",
    "tool.move",
    "tool.push_pull",
    "tool.rotate",
    "tool.follow_me",
    "tool.scale",
    "tool.flip",
    "tool.offset",
    "tool.tape_measure",
    "tool.dimensions",
    "tool.protractor",
    "tool.text",
    "tool.axes",
    "tool.text_3d",
    "tool.section_plane",
    "camera.orbit",
    "camera.pan",
    "camera.zoom",
    "camera.zoom_window",
    "camera.zoom_extents",
    "camera.previous",
    "camera.position",
    "camera.look_around",
    "camera.walk",
    "window.3d_warehouse",
    "extensions.warehouse",
    "file.send_to_layout",
    "extensions.manager",
];

/// Whether `id` has its own icon (otherwise [`paint`] draws the fallback).
pub fn has_icon(id: &str) -> bool {
    KNOWN.contains(&id) || extra::KNOWN.contains(&id)
}

/// Every command id with its own icon ([`KNOWN`] and the later toolbars' icons).
pub fn known() -> impl Iterator<Item = &'static str> {
    KNOWN.iter().chain(extra::KNOWN).copied()
}

/// Paints the icon for command `id` into `rect` (square). Returns false if the
/// fallback icon was drawn because `id` has no icon of its own.
pub fn paint(painter: &Painter, rect: Rect, id: &str, palette: &Palette) -> bool {
    let side = rect.width().min(rect.height());
    let rect = Rect::from_center_size(rect.center(), vec2(side, side));
    let p = Pen {
        painter,
        origin: rect.min,
        s: side / 24.0,
        c: *palette,
    };
    let known = has_icon(id);
    match id {
        "tool.select" => select(&p),
        "tool.lasso" => lasso(&p),
        "edit.make_component" => make_component(&p),
        "tool.eraser" => eraser(&p),
        "tool.tag" => tag(&p),
        "tool.paint_bucket" => paint_bucket(&p),
        "tool.line" => line(&p),
        "tool.freehand" => freehand(&p),
        "tool.rectangle" => rectangle(&p),
        "tool.rotated_rectangle" => rotated_rectangle(&p),
        "tool.circle" => circle(&p),
        "tool.polygon" => polygon(&p),
        "tool.arc" => arc(&p),
        "tool.arc_2pt" => arc_2pt(&p),
        "tool.arc_3pt" => arc_3pt(&p),
        "tool.pie" => pie(&p),
        "tool.move" => move_tool(&p),
        "tool.push_pull" => push_pull(&p),
        "tool.rotate" => rotate(&p),
        "tool.follow_me" => follow_me(&p),
        "tool.scale" => scale(&p),
        "tool.flip" => flip(&p),
        "tool.offset" => offset(&p),
        "tool.tape_measure" => tape_measure(&p),
        "tool.dimensions" => dimensions(&p),
        "tool.protractor" => protractor(&p),
        "tool.text" => text(&p),
        "tool.axes" => axes(&p),
        "tool.text_3d" => text_3d(&p),
        "tool.section_plane" => section_plane(&p),
        "camera.orbit" => orbit(&p),
        "camera.pan" => pan(&p),
        "camera.zoom" => zoom(&p),
        "camera.zoom_window" => zoom_window(&p),
        "camera.zoom_extents" => zoom_extents(&p),
        "camera.previous" => previous(&p),
        "camera.position" => position_camera(&p),
        "camera.look_around" => look_around(&p),
        "camera.walk" => walk(&p),
        "window.3d_warehouse" => warehouse_3d(&p),
        "extensions.warehouse" => extension_warehouse(&p),
        "file.send_to_layout" => send_to_layout(&p),
        "extensions.manager" => extension_manager(&p),
        other => {
            if !extra::paint(&p, other) {
                fallback(&p);
            }
        }
    }
    known
}

/// Drawing helper in 24 x 24 icon units.
struct Pen<'a> {
    painter: &'a Painter,
    origin: Pos2,
    s: f32,
    c: Palette,
}

type Pt = (f32, f32);

/// Standard outline width, in icon units.
const W: f32 = 1.4;

impl Pen<'_> {
    fn p(&self, (x, y): Pt) -> Pos2 {
        self.origin + vec2(x * self.s, y * self.s)
    }

    fn pts(&self, pts: &[Pt]) -> Vec<Pos2> {
        pts.iter().map(|&q| self.p(q)).collect()
    }

    fn stroke(&self, width: f32, color: Color32) -> Stroke {
        Stroke::new((width * self.s).max(1.0), color)
    }

    /// Open polyline.
    fn line(&self, pts: &[Pt], width: f32, color: Color32) {
        self.painter
            .add(Shape::line(self.pts(pts), self.stroke(width, color)));
    }

    /// Closed outline.
    fn outline(&self, pts: &[Pt], width: f32, color: Color32) {
        self.painter
            .add(Shape::closed_line(self.pts(pts), self.stroke(width, color)));
    }

    /// Convex polygon, filled and outlined in ink.
    fn poly(&self, pts: &[Pt], fill: Color32) {
        self.painter.add(Shape::convex_polygon(
            self.pts(pts),
            fill,
            self.stroke(W, self.c.ink),
        ));
    }

    /// Any simple polygon that is star-shaped from its first point, filled and outlined.
    fn fan(&self, pts: &[Pt], fill: Color32) {
        let pts = self.pts(pts);
        let mut mesh = Mesh::default();
        for &q in &pts {
            mesh.colored_vertex(q, fill);
        }
        for i in 1..pts.len().saturating_sub(1) {
            mesh.add_triangle(0, i as u32, i as u32 + 1);
        }
        self.painter.add(Shape::mesh(mesh));
        self.painter.add(Shape::Path(PathShape {
            points: pts,
            closed: true,
            fill: Color32::TRANSPARENT,
            stroke: PathStroke::from(self.stroke(W, self.c.ink)),
        }));
    }

    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32, fill: Color32) {
        self.poly(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)], fill);
    }

    fn circle(&self, c: Pt, r: f32, fill: Color32, stroke: Option<(f32, Color32)>) {
        let stroke = stroke.map_or(Stroke::NONE, |(w, col)| self.stroke(w, col));
        self.painter
            .add(Shape::circle_filled(self.p(c), r * self.s, fill));
        if stroke != Stroke::NONE {
            self.painter
                .add(Shape::circle_stroke(self.p(c), r * self.s, stroke));
        }
    }

    fn dot(&self, c: Pt, r: f32, color: Color32) {
        self.painter.add(Shape::circle_filled(
            self.p(c),
            (r * self.s).max(1.0),
            color,
        ));
    }

    /// Points on an arc; angles in degrees, counter-clockwise on screen (y up).
    fn arc_pts(c: Pt, r: f32, a0: f32, a1: f32) -> Vec<Pt> {
        let n = (((a1 - a0).abs() / 10.0).ceil() as usize).max(2);
        (0..=n)
            .map(|i| {
                let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
                (c.0 + r * a.cos(), c.1 - r * a.sin())
            })
            .collect()
    }

    fn arc(&self, c: Pt, r: f32, a0: f32, a1: f32, width: f32, color: Color32) {
        self.line(&Self::arc_pts(c, r, a0, a1), width, color);
    }

    /// Solid triangular arrow head with its tip at `tip`, pointing along `dir`.
    fn head(&self, tip: Pt, dir: Pt, size: f32, color: Color32) {
        let len = (dir.0 * dir.0 + dir.1 * dir.1).sqrt().max(1e-6);
        let (dx, dy) = (dir.0 / len, dir.1 / len);
        let back = (tip.0 - dx * size, tip.1 - dy * size);
        let half = size * 0.6;
        let a = (back.0 - dy * half, back.1 + dx * half);
        let b = (back.0 + dy * half, back.1 - dx * half);
        self.painter.add(Shape::convex_polygon(
            self.pts(&[tip, a, b]),
            color,
            Stroke::NONE,
        ));
    }

    /// Straight arrow from `a` to `b` with a head at `b`.
    fn arrow(&self, a: Pt, b: Pt, width: f32, color: Color32) {
        let dir = (b.0 - a.0, b.1 - a.1);
        let len = (dir.0 * dir.0 + dir.1 * dir.1).sqrt().max(1e-6);
        let size = 4.0;
        let stop = (
            b.0 - dir.0 / len * size * 0.8,
            b.1 - dir.1 / len * size * 0.8,
        );
        self.line(&[a, stop], width, color);
        self.head(b, dir, size, color);
    }

    /// Isometric cube centered at `c` with "radius" `r`.
    fn cube(&self, c: Pt, r: f32) {
        let k = 0.866 * r;
        let top = (c.0, c.1 - r);
        let lt = (c.0 - k, c.1 - r * 0.5);
        let rt = (c.0 + k, c.1 - r * 0.5);
        let lb = (c.0 - k, c.1 + r * 0.5);
        let rb = (c.0 + k, c.1 + r * 0.5);
        let bottom = (c.0, c.1 + r);
        self.poly(&[top, rt, c, lt], self.c.face);
        self.poly(&[lt, c, bottom, lb], self.c.face);
        self.poly(&[c, rt, rb, bottom], self.c.face_dark);
    }

    fn dashed(&self, a: Pt, b: Pt, dash: f32, width: f32, color: Color32) {
        let pts = [self.p(a), self.p(b)];
        self.painter.extend(Shape::dashed_line(
            &pts,
            self.stroke(width, color),
            dash * self.s,
            dash * self.s,
        ));
    }

    /// Pencil lying from `tip` towards the upper right, `len` long.
    fn pencil(&self, tip: Pt, len: f32) {
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let d = (h, -h);
        let n = (h, h);
        let w = 2.2;
        let at = |t: f32, side: f32| (tip.0 + d.0 * t + n.0 * side, tip.1 + d.1 * t + n.1 * side);
        // Body.
        self.poly(
            &[at(5.0, -w), at(len - 2.0, -w), at(len - 2.0, w), at(5.0, w)],
            self.c.face,
        );
        // Eraser end.
        self.poly(
            &[at(len - 2.0, -w), at(len, -w), at(len, w), at(len - 2.0, w)],
            self.c.red,
        );
        // Sharpened wood and graphite.
        self.poly(&[tip, at(5.0, -w), at(5.0, w)], self.c.paper);
        self.painter.add(Shape::convex_polygon(
            self.pts(&[tip, at(2.0, -0.9), at(2.0, 0.9)]),
            self.c.ink,
            Stroke::NONE,
        ));
    }

    /// Magnifying glass: lens centered at `c`, handle to the lower right.
    fn magnifier(&self, c: Pt, r: f32) {
        let h0 = (c.0 + r * 0.72, c.1 + r * 0.72);
        let h1 = (h0.0 + 6.0, h0.1 + 6.0);
        self.line(&[h0, h1], 3.2, self.c.ink);
        self.circle(c, r, self.c.paper, Some((1.8, self.c.ink)));
        self.arc(c, r - 2.0, 100.0, 160.0, 1.2, self.c.accent);
    }
}

fn select(p: &Pen) {
    p.fan(
        &[
            (6.0, 2.5),
            (6.0, 19.0),
            (10.0, 15.3),
            (13.0, 21.5),
            (15.6, 20.3),
            (12.6, 14.2),
            (18.0, 14.2),
        ],
        p.c.accent,
    );
}

/// The Lasso: a small arrow and a loose loop (own drawing, unverified).
fn lasso(p: &Pen) {
    let pts: Vec<Pt> = (0..=24)
        .map(|i| {
            let a = (i as f32 / 24.0) * std::f32::consts::TAU;
            (13.0 + 8.5 * a.cos(), 9.0 + 5.5 * a.sin())
        })
        .collect();
    p.line(&pts, 1.6, p.c.ink);
    p.line(&[(8.0, 13.5), (6.0, 18.0), (8.5, 22.0)], 1.6, p.c.ink);
    p.fan(
        &[
            (14.0, 11.0),
            (14.0, 20.5),
            (16.2, 18.4),
            (17.8, 21.8),
            (19.2, 21.1),
            (17.6, 17.8),
            (20.6, 17.8),
        ],
        p.c.accent,
    );
}

fn make_component(p: &Pen) {
    p.cube((12.0, 13.0), 8.5);
    // Component marker: dashed bounding corners in accent blue.
    for (a, b, c) in [
        ((2.0, 6.0), (2.0, 2.0), (6.0, 2.0)),
        ((18.0, 2.0), (22.0, 2.0), (22.0, 6.0)),
        ((2.0, 18.0), (2.0, 22.0), (6.0, 22.0)),
        ((18.0, 22.0), (22.0, 22.0), (22.0, 18.0)),
    ] {
        p.line(&[a, b, c], 1.4, p.c.accent);
    }
}

fn eraser(p: &Pen) {
    // A block tilted 35 degrees: paper sleeve and red rubber.
    p.poly(
        &[(3.0, 15.0), (10.0, 8.0), (14.0, 12.0), (7.0, 19.0)],
        p.c.red,
    );
    p.poly(
        &[(10.0, 8.0), (16.0, 2.0), (20.0, 6.0), (14.0, 12.0)],
        p.c.paper,
    );
    p.line(&[(12.0, 6.0), (16.0, 10.0)], 1.0, p.c.accent);
    p.line(&[(2.0, 21.5), (21.0, 21.5)], 1.2, p.c.ink);
}

fn tag(p: &Pen) {
    p.poly(
        &[
            (3.0, 12.0),
            (9.0, 5.0),
            (21.0, 5.0),
            (21.0, 19.0),
            (9.0, 19.0),
        ],
        p.c.face,
    );
    p.circle((9.0, 12.0), 1.8, p.c.paper, Some((1.2, p.c.ink)));
    p.line(&[(13.0, 10.0), (18.5, 10.0)], 1.4, p.c.ink);
    p.line(&[(13.0, 14.0), (18.5, 14.0)], 1.4, p.c.ink);
}

fn paint_bucket(p: &Pen) {
    // Tilted bucket with paint pouring from its lip.
    p.poly(
        &[(4.0, 9.0), (13.0, 3.5), (19.0, 12.0), (10.0, 17.5)],
        p.c.paper,
    );
    p.poly(
        &[(4.0, 9.0), (13.0, 3.5), (14.5, 5.8), (5.5, 11.3)],
        p.c.accent,
    );
    p.arc((8.5, 6.5), 4.5, 60.0, 200.0, 1.3, p.c.ink);
    p.fan(
        &[
            (17.0, 8.5),
            (21.0, 13.0),
            (21.0, 18.0),
            (19.5, 19.5),
            (18.0, 18.0),
        ],
        p.c.accent,
    );
}

fn line(p: &Pen) {
    p.line(&[(2.5, 21.5), (9.0, 21.5)], 1.6, p.c.ink);
    p.pencil((3.0, 21.0), 21.0);
}

fn freehand(p: &Pen) {
    let wave: Vec<Pt> = (0..=20)
        .map(|i| {
            let x = 2.0 + i as f32 * 0.6;
            (x, 18.0 + 2.5 * (i as f32 * 0.6).sin())
        })
        .collect();
    p.line(&wave, 1.5, p.c.accent);
    p.pencil((14.0, 18.0), 12.0);
}

fn rectangle(p: &Pen) {
    p.rect(3.0, 6.0, 21.0, 18.0, p.c.face);
    for c in [(3.0, 6.0), (21.0, 6.0), (21.0, 18.0), (3.0, 18.0)] {
        p.dot(c, 1.6, p.c.ink);
    }
}

fn rotated_rectangle(p: &Pen) {
    p.poly(
        &[(3.0, 15.0), (14.0, 4.0), (21.0, 11.0), (10.0, 22.0)],
        p.c.face,
    );
    p.line(&[(3.0, 15.0), (12.0, 15.0)], 1.0, p.c.accent);
    p.arc((3.0, 15.0), 7.0, 0.0, 45.0, 1.2, p.c.accent);
}

fn circle(p: &Pen) {
    p.circle((12.0, 12.0), 9.0, p.c.face, Some((W, p.c.ink)));
    p.line(&[(12.0, 12.0), (21.0, 12.0)], 1.0, p.c.ink);
    p.dot((12.0, 12.0), 1.4, p.c.ink);
}

fn polygon(p: &Pen) {
    let hex: Vec<Pt> = (0..6)
        .map(|i| {
            let a = (30.0 + 60.0 * i as f32).to_radians();
            (12.0 + 9.5 * a.cos(), 12.0 - 9.5 * a.sin())
        })
        .collect();
    p.poly(&hex, p.c.face);
    p.dot((12.0, 12.0), 1.4, p.c.ink);
}

fn arc(p: &Pen) {
    // Center-point arc: center, two radii and the arc.
    let c = (6.0, 18.0);
    p.line(&[(20.0, 18.0), c, (6.0, 4.0)], 1.0, p.c.accent);
    p.arc(c, 14.0, 0.0, 90.0, 1.8, p.c.ink);
    p.dot(c, 1.6, p.c.ink);
}

fn arc_2pt(p: &Pen) {
    // Chord with its bulge, endpoints marked.
    p.line(&[(3.0, 17.0), (21.0, 17.0)], 1.0, p.c.accent);
    p.line(&[(12.0, 17.0), (12.0, 8.0)], 1.0, p.c.accent);
    p.arc((12.0, 21.85), 13.85, 139.5, 40.5, 1.8, p.c.ink);
    p.dot((3.0, 17.0), 1.6, p.c.ink);
    p.dot((21.0, 17.0), 1.6, p.c.ink);
}

fn arc_3pt(p: &Pen) {
    p.arc((12.0, 15.0), 9.0, 200.0, -20.0, 1.8, p.c.ink);
    for a in [200.0f32, 90.0, -20.0] {
        let r = a.to_radians();
        p.dot(
            (12.0 + 9.0 * r.cos(), 15.0 - 9.0 * r.sin()),
            1.7,
            p.c.accent,
        );
    }
}

fn pie(p: &Pen) {
    let c = (6.0, 18.0);
    let mut pts = vec![c];
    pts.extend(Pen::arc_pts(c, 15.0, 0.0, 75.0));
    p.fan(&pts, p.c.face);
    p.dot(c, 1.5, p.c.ink);
}

fn move_tool(p: &Pen) {
    let c = (12.0, 12.0);
    for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        p.arrow(c, (12.0 + dx * 10.5, 12.0 + dy * 10.5), 2.0, p.c.red);
    }
    p.circle(c, 2.0, p.c.paper, Some((1.2, p.c.ink)));
}

fn push_pull(p: &Pen) {
    // A slab with an arrow lifting its top face.
    p.poly(
        &[(2.0, 17.0), (12.0, 12.0), (22.0, 17.0), (12.0, 22.0)],
        p.c.face,
    );
    p.arrow((12.0, 17.0), (12.0, 2.0), 2.2, p.c.red);
}

fn rotate(p: &Pen) {
    // Protractor disc with a turning arrow around it.
    p.circle((12.0, 13.0), 5.0, p.c.face, Some((1.2, p.c.ink)));
    p.line(&[(12.0, 13.0), (17.0, 13.0)], 1.0, p.c.ink);
    p.arc((12.0, 13.0), 9.0, 200.0, 20.0, 2.0, p.c.red);
    p.head((20.6, 9.8), (0.6, 1.0), 4.0, p.c.red);
}

fn follow_me(p: &Pen) {
    // Profile face swept along a path.
    p.poly(
        &[(3.0, 4.0), (9.0, 4.0), (9.0, 12.0), (3.0, 12.0)],
        p.c.face,
    );
    p.line(&[(6.0, 12.0), (6.0, 19.0), (19.0, 19.0)], 1.4, p.c.ink);
    p.arrow((9.0, 15.0), (21.0, 15.0), 2.0, p.c.red);
    p.dashed((6.0, 19.0), (19.0, 19.0), 1.5, 1.0, p.c.accent);
}

fn scale(p: &Pen) {
    p.rect(2.0, 10.0, 14.0, 22.0, p.c.face);
    for (a, b) in [
        ((2.0, 2.0), (22.0, 2.0)),
        ((22.0, 2.0), (22.0, 22.0)),
        ((22.0, 22.0), (14.0, 22.0)),
        ((2.0, 10.0), (2.0, 2.0)),
    ] {
        p.dashed(a, b, 1.6, 1.2, p.c.green);
    }
    p.arrow((10.0, 14.0), (21.0, 3.0), 2.0, p.c.green);
}

fn flip(p: &Pen) {
    // A shape and its mirror image across a dashed plane.
    p.poly(&[(10.0, 6.0), (10.0, 19.0), (2.0, 19.0)], p.c.face);
    p.poly(&[(14.0, 6.0), (14.0, 19.0), (22.0, 19.0)], p.c.face_dark);
    p.dashed((12.0, 1.0), (12.0, 23.0), 2.0, 1.4, p.c.red);
    p.arrow((9.0, 3.0), (3.0, 3.0), 1.4, p.c.accent);
    p.arrow((15.0, 3.0), (21.0, 3.0), 1.4, p.c.accent);
}

fn offset(p: &Pen) {
    p.rect(2.0, 3.0, 22.0, 21.0, p.c.face);
    p.rect(7.0, 8.0, 17.0, 16.0, p.c.paper);
    p.arrow((7.0, 8.0), (3.5, 4.5), 1.4, p.c.red);
    p.arrow((17.0, 16.0), (20.5, 19.5), 1.4, p.c.red);
}

fn tape_measure(p: &Pen) {
    // Tape strip with ticks coming out of a round case.
    p.rect(9.0, 15.0, 23.0, 20.0, p.c.face);
    for i in 0..5 {
        let x = 11.0 + i as f32 * 2.8;
        p.line(&[(x, 15.0), (x, 17.0)], 1.0, p.c.ink);
    }
    p.circle((8.0, 11.0), 7.0, p.c.accent, Some((W, p.c.ink)));
    p.circle((8.0, 11.0), 2.2, p.c.paper, Some((1.0, p.c.ink)));
}

fn dimensions(p: &Pen) {
    p.line(&[(3.0, 4.0), (3.0, 20.0)], 1.2, p.c.ink);
    p.line(&[(21.0, 4.0), (21.0, 20.0)], 1.2, p.c.ink);
    p.arrow((12.0, 12.0), (3.5, 12.0), 1.4, p.c.ink);
    p.arrow((12.0, 12.0), (20.5, 12.0), 1.4, p.c.ink);
    p.rect(8.0, 5.0, 16.0, 9.0, p.c.paper);
}

fn protractor(p: &Pen) {
    let c = (12.0, 18.0);
    let mut pts = vec![(1.5, 18.0)];
    pts.extend(Pen::arc_pts(c, 10.5, 180.0, 0.0));
    p.fan(&pts, p.c.face);
    p.circle(c, 3.0, p.c.paper, Some((1.0, p.c.ink)));
    for i in 1..6 {
        let a = (i as f32 * 30.0).to_radians();
        let (x, y) = (a.cos(), -a.sin());
        p.line(
            &[
                (c.0 + 8.0 * x, c.1 + 8.0 * y),
                (c.0 + 10.5 * x, c.1 + 10.5 * y),
            ],
            1.0,
            p.c.ink,
        );
    }
    p.line(&[c, (20.0, 9.0)], 1.4, p.c.red);
}

fn text(p: &Pen) {
    // Callout with a leader line.
    p.line(&[(4.0, 21.0), (9.0, 13.0)], 1.2, p.c.ink);
    p.dot((4.0, 21.0), 1.5, p.c.ink);
    p.rect(6.0, 3.0, 22.0, 13.0, p.c.paper);
    for (y, x1) in [(6.0, 19.0), (8.5, 19.0), (11.0, 15.0)] {
        p.line(&[(9.0, y), (x1, y)], 1.2, p.c.accent);
    }
}

fn axes(p: &Pen) {
    let o = (8.0, 15.0);
    p.arrow(o, (22.0, 19.0), 1.8, p.c.red);
    p.arrow(o, (17.0, 7.0), 1.8, p.c.green);
    p.arrow(o, (8.0, 1.5), 1.8, p.c.accent);
    p.dashed(o, (2.0, 13.3), 1.0, 1.0, p.c.red);
    p.dashed(o, (3.0, 19.5), 1.0, 1.0, p.c.green);
}

fn text_3d(p: &Pen) {
    // Extruded block letter T.
    let d = (2.5, -2.5);
    let front = [
        (3.0, 7.0),
        (17.0, 7.0),
        (17.0, 11.0),
        (12.0, 11.0),
        (12.0, 21.0),
        (8.0, 21.0),
        (8.0, 11.0),
        (3.0, 11.0),
    ];
    let back: Vec<Pt> = front.iter().map(|&(x, y)| (x + d.0, y + d.1)).collect();
    p.poly(&[front[0], back[0], back[1], front[1]], p.c.face_dark);
    p.poly(&[front[1], back[1], back[2], front[2]], p.c.face_dark);
    p.poly(&[front[3], back[3], back[4], front[4]], p.c.face_dark);
    p.poly(&front[..4], p.c.face);
    p.poly(&[front[3], front[4], front[5], front[6]], p.c.face);
    p.poly(&[front[0], front[3], front[6], front[7]], p.c.face);
    p.outline(&front, W, p.c.ink);
}

fn section_plane(p: &Pen) {
    p.poly(
        &[(3.0, 8.0), (14.0, 3.0), (21.0, 16.0), (10.0, 21.0)],
        p.c.paper,
    );
    p.line(&[(3.0, 8.0), (14.0, 3.0)], 2.0, p.c.accent);
    p.arrow((12.0, 12.0), (5.0, 15.5), 1.6, p.c.red);
    p.arrow((12.0, 12.0), (19.0, 8.5), 1.6, p.c.red);
}

fn orbit(p: &Pen) {
    p.circle((12.0, 12.0), 4.5, p.c.face, Some((W, p.c.ink)));
    p.arc((12.0, 12.0), 9.5, 160.0, 20.0, 2.0, p.c.red);
    p.head((21.4, 9.5), (0.4, 1.0), 4.0, p.c.red);
    p.arc((12.0, 12.0), 9.5, 340.0, 200.0, 2.0, p.c.red);
    p.head((2.6, 14.5), (-0.4, -1.0), 4.0, p.c.red);
}

fn pan(p: &Pen) {
    // Open hand.
    let skin = p.c.paper;
    for (x, top) in [(6.0, 7.0), (9.5, 4.0), (13.0, 3.5), (16.5, 5.5)] {
        p.poly(
            &[
                (x, top + 1.5),
                (x + 3.0, top + 1.5),
                (x + 3.0, 14.0),
                (x, 14.0),
            ],
            skin,
        );
        p.circle((x + 1.5, top + 1.5), 1.5, skin, Some((W, p.c.ink)));
    }
    p.poly(
        &[
            (6.0, 12.0),
            (19.5, 12.0),
            (18.0, 21.0),
            (9.0, 21.0),
            (6.0, 16.0),
        ],
        skin,
    );
    p.poly(&[(2.5, 12.5), (5.0, 11.0), (8.0, 15.5), (6.5, 18.0)], skin);
    p.line(&[(6.2, 13.0), (19.3, 13.0)], 2.0, skin);
}

fn zoom(p: &Pen) {
    p.magnifier((10.0, 10.0), 7.0);
    p.line(&[(7.0, 10.0), (13.0, 10.0)], 1.6, p.c.ink);
    p.line(&[(10.0, 7.0), (10.0, 13.0)], 1.6, p.c.ink);
}

fn zoom_window(p: &Pen) {
    for (a, b) in [
        ((1.5, 1.5), (16.0, 1.5)),
        ((16.0, 1.5), (16.0, 6.0)),
        ((1.5, 1.5), (1.5, 15.0)),
        ((1.5, 15.0), (5.0, 15.0)),
    ] {
        p.dashed(a, b, 1.5, 1.2, p.c.accent);
    }
    p.magnifier((11.0, 11.0), 6.0);
}

fn zoom_extents(p: &Pen) {
    for (a, b, c) in [
        ((1.5, 6.0), (1.5, 1.5), (6.0, 1.5)),
        ((18.0, 1.5), (22.5, 1.5), (22.5, 6.0)),
        ((1.5, 18.0), (1.5, 22.5), (6.0, 22.5)),
    ] {
        p.line(&[a, b, c], 1.6, p.c.red);
    }
    p.magnifier((10.5, 10.5), 6.0);
}

fn previous(p: &Pen) {
    p.magnifier((13.0, 11.0), 6.5);
    p.arc((13.0, 11.0), 3.5, 160.0, 20.0, 1.4, p.c.red);
    p.head((9.0, 12.5), (-0.3, 1.0), 3.0, p.c.red);
}

fn position_camera(p: &Pen) {
    // Camera on a tripod.
    p.line(&[(12.0, 13.0), (5.0, 22.0)], 1.4, p.c.ink);
    p.line(&[(12.0, 13.0), (19.0, 22.0)], 1.4, p.c.ink);
    p.line(&[(12.0, 13.0), (12.0, 22.0)], 1.4, p.c.ink);
    p.rect(4.0, 4.0, 17.0, 13.0, p.c.accent);
    p.poly(
        &[(17.0, 7.0), (22.0, 4.5), (22.0, 12.5), (17.0, 10.0)],
        p.c.accent,
    );
    p.circle((10.5, 8.5), 2.5, p.c.paper, Some((1.0, p.c.ink)));
}

fn look_around(p: &Pen) {
    let upper = Pen::arc_pts((12.0, 22.0), 13.0, 140.0, 40.0);
    let lower = Pen::arc_pts((12.0, 2.0), 13.0, 320.0, 220.0);
    let mut eye = upper;
    eye.extend(lower);
    p.fan(&eye, p.c.paper);
    p.circle((12.0, 12.0), 4.0, p.c.accent, Some((1.0, p.c.ink)));
    p.dot((12.0, 12.0), 1.6, p.c.ink);
}

fn walk(p: &Pen) {
    // Two footprints.
    for &(x, y) in &[(7.0, 13.0), (16.0, 8.0)] {
        let sole: Vec<Pt> = (0..16)
            .map(|i| {
                let a = (i as f32 * 22.5).to_radians();
                (x + 2.6 * a.cos(), y + 5.0 * a.sin())
            })
            .collect();
        p.poly(&sole, p.c.face);
        p.circle((x, y + 7.8), 2.0, p.c.face, Some((W, p.c.ink)));
    }
}

fn warehouse_3d(p: &Pen) {
    // Storehouse with a down arrow.
    p.poly(&[(2.0, 10.0), (12.0, 3.0), (22.0, 10.0)], p.c.red);
    p.rect(4.0, 10.0, 20.0, 21.0, p.c.paper);
    p.arrow((12.0, 9.0), (12.0, 19.5), 2.2, p.c.accent);
}

fn extension_warehouse(p: &Pen) {
    // Shop front with a striped awning.
    p.rect(4.0, 10.0, 20.0, 21.0, p.c.paper);
    p.rect(12.0, 14.0, 17.0, 21.0, p.c.face);
    for i in 0..4 {
        let x = 2.0 + i as f32 * 5.0;
        let fill = if i % 2 == 0 { p.c.red } else { p.c.paper };
        p.poly(
            &[(x, 5.0), (x + 5.0, 5.0), (x + 5.0, 10.0), (x, 10.0)],
            fill,
        );
    }
    p.rect(6.0, 13.0, 10.0, 17.0, p.c.accent);
}

fn send_to_layout(p: &Pen) {
    // Sheet of paper with a folded corner and an arrow onto it.
    p.poly(
        &[
            (8.0, 2.0),
            (17.0, 2.0),
            (22.0, 7.0),
            (22.0, 22.0),
            (8.0, 22.0),
        ],
        p.c.paper,
    );
    p.line(&[(17.0, 2.0), (17.0, 7.0), (22.0, 7.0)], 1.0, p.c.ink);
    p.rect(11.0, 11.0, 19.0, 18.0, p.c.face);
    p.arrow((1.5, 14.0), (13.0, 14.0), 2.2, p.c.red);
}

fn extension_manager(p: &Pen) {
    // Gear.
    let teeth = 8;
    let mut pts = Vec::new();
    for i in 0..teeth * 4 {
        let a = (i as f32 * 360.0 / (teeth * 4) as f32 + 5.6).to_radians();
        let r = if (i / 2) % 2 == 0 { 10.0 } else { 7.5 };
        pts.push((12.0 + r * a.cos(), 12.0 - r * a.sin()));
    }
    let mut fan = vec![(12.0, 12.0)];
    fan.extend(pts.iter().copied());
    fan.push(pts[0]);
    // Fan from the center, then redraw the outline without the center spoke.
    let painted: Vec<Pos2> = p.pts(&fan);
    let mut mesh = Mesh::default();
    for &q in &painted {
        mesh.colored_vertex(q, p.c.accent);
    }
    for i in 1..painted.len() - 1 {
        mesh.add_triangle(0, i as u32, i as u32 + 1);
    }
    p.painter.add(Shape::mesh(mesh));
    p.outline(&pts, W, p.c.ink);
    p.circle((12.0, 12.0), 3.0, p.c.paper, Some((W, p.c.ink)));
}

fn fallback(p: &Pen) {
    p.rect(3.0, 3.0, 21.0, 21.0, p.c.paper);
    p.line(
        &[
            (8.0, 9.0),
            (8.5, 6.5),
            (12.0, 5.5),
            (15.5, 6.5),
            (16.0, 9.0),
            (12.0, 12.0),
            (12.0, 15.0),
        ],
        1.8,
        p.c.ink,
    );
    p.dot((12.0, 18.5), 1.3, p.c.ink);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_toolbar_command_has_an_icon() {
        let spec = mt_spec::Spec::builtin().unwrap();
        for toolbar in &spec.toolbars.toolbar {
            for button in toolbar.buttons() {
                for id in std::iter::once(&button.id).chain(&button.flyout) {
                    assert!(has_icon(id), "no icon for `{id}`");
                }
            }
        }
    }

    #[test]
    fn icons_paint_without_panicking() {
        let ctx = egui::Context::default();
        let painter = Painter::new(
            ctx,
            egui::LayerId::background(),
            Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0)),
        );
        let spec = mt_spec::Spec::builtin().unwrap();
        let palette = Palette::from_spec(&spec.ui_theme.icons);
        let rect = Rect::from_min_size(egui::pos2(10.0, 10.0), vec2(24.0, 24.0));
        for id in known() {
            assert!(paint(&painter, rect, id, &palette));
            assert!(paint(&painter, rect, id, &palette.disabled()));
        }
        assert!(!paint(&painter, rect, "no.such", &palette.disabled()));
    }
}
