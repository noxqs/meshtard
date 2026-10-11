//! Icons for the toolbars beyond Getting Started and the Large Tool Set (Standard,
//! Views, Styles, Section, Solid Tools, Tags, Shadows, Sandbox). Same rules as the
//! parent module: meshtard's own drawings on the 24 x 24 grid, never copies of
//! SketchUp's artwork.

use egui::epaint::{Mesh, PathShape, PathStroke};
use egui::{Color32, Shape, Stroke};

use super::{Pen, Pt, W};

/// Command ids drawn by this module.
pub const KNOWN: &[&str] = &[
    "file.new",
    "file.open",
    "file.save",
    "file.print",
    "edit.cut",
    "edit.copy",
    "edit.paste",
    "edit.delete",
    "edit.undo",
    "edit.redo",
    "window.model_info",
    "camera.view_iso",
    "camera.view_top",
    "camera.view_bottom",
    "camera.view_front",
    "camera.view_back",
    "camera.view_left",
    "camera.view_right",
    "view.xray",
    "view.back_edges",
    "view.wireframe",
    "view.hidden_line",
    "view.shaded",
    "view.shaded_textures",
    "view.monochrome",
    "view.section_planes",
    "view.section_cuts",
    "view.section_fill",
    "tool.outer_shell",
    "tool.solid_intersect",
    "tool.solid_union",
    "tool.solid_subtract",
    "tool.solid_trim",
    "tool.solid_split",
    "window.tags",
    "window.shadows",
    "view.shadows",
    "tool.sandbox_from_contours",
    "tool.sandbox_from_scratch",
];

/// Paints `id` if this module has it; false otherwise (nothing painted).
pub(super) fn paint(p: &Pen, id: &str) -> bool {
    match id {
        "file.new" => file_new(p),
        "file.open" => file_open(p),
        "file.save" => file_save(p),
        "file.print" => file_print(p),
        "edit.cut" => cut(p),
        "edit.copy" => copy(p),
        "edit.paste" => paste(p),
        "edit.delete" => delete(p),
        "edit.undo" => turn_arrow(p, false),
        "edit.redo" => turn_arrow(p, true),
        "window.model_info" => model_info(p),
        "camera.view_iso" => view_iso(p),
        "camera.view_top" => view_box(p, Side::Top),
        "camera.view_bottom" => view_box(p, Side::Bottom),
        "camera.view_front" => view_box(p, Side::Front),
        "camera.view_back" => view_box(p, Side::Back),
        "camera.view_left" => view_box(p, Side::Left),
        "camera.view_right" => view_box(p, Side::Right),
        "view.xray" => xray(p),
        "view.back_edges" => back_edges(p),
        "view.wireframe" => wireframe(p),
        "view.hidden_line" => hidden_line(p),
        "view.shaded" => shaded(p),
        "view.shaded_textures" => shaded_textures(p),
        "view.monochrome" => monochrome(p),
        "view.section_planes" => section_planes(p),
        "view.section_cuts" => section_cut(p, false),
        "view.section_fill" => section_cut(p, true),
        "tool.outer_shell" => outer_shell(p),
        "tool.solid_intersect" => solid_intersect(p),
        "tool.solid_union" => solid_union(p),
        "tool.solid_subtract" => solid_subtract(p),
        "tool.solid_trim" => solid_trim(p),
        "tool.solid_split" => solid_split(p),
        "window.tags" => tags_panel(p),
        "window.shadows" => shadow_settings(p),
        "view.shadows" => display_shadows(p),
        "tool.sandbox_from_contours" => from_contours(p),
        "tool.sandbox_from_scratch" => from_scratch(p),
        _ => return false,
    }
    true
}

// ---------------------------------------------------------------- helpers

/// `a` blended towards `b` by `t` (0..1).
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        l(a.r(), b.r()),
        l(a.g(), b.g()),
        l(a.b(), b.b()),
        l(a.a(), b.a()),
    )
}

fn alpha(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

impl Pen<'_> {
    /// Convex polygon filled without outline.
    fn flat(&self, pts: &[Pt], fill: Color32) {
        self.painter
            .add(Shape::convex_polygon(self.pts(pts), fill, Stroke::NONE));
    }

    /// Region star-shaped from `center` bounded by `ring` (any order: sorted by angle
    /// around `center`), filled and outlined with `outline` (width, color).
    fn star(&self, center: Pt, ring: &[Pt], fill: Color32, outline: Option<(f32, Color32)>) {
        let mut ring = ring.to_vec();
        ring.sort_by(|a, b| {
            let ang = |q: &Pt| (q.1 - center.1).atan2(q.0 - center.0);
            ang(a).total_cmp(&ang(b))
        });
        let pts = self.pts(&ring);
        let mut mesh = Mesh::default();
        mesh.colored_vertex(self.p(center), fill);
        for &q in &pts {
            mesh.colored_vertex(q, fill);
        }
        let n = pts.len() as u32;
        for i in 0..n {
            mesh.add_triangle(0, i + 1, (i + 1) % n + 1);
        }
        self.painter.add(Shape::mesh(mesh));
        if let Some((w, c)) = outline {
            self.painter.add(Shape::Path(PathShape {
                points: pts,
                closed: true,
                fill: Color32::TRANSPARENT,
                stroke: PathStroke::from(self.stroke(w, c)),
            }));
        }
    }

    /// Several separate straight segments.
    fn segments(&self, segs: &[(Pt, Pt)], width: f32, color: Color32) {
        for &(a, b) in segs {
            self.line(&[a, b], width, color);
        }
    }

    /// Sheet of paper with a folded top-right corner.
    fn sheet(&self, x0: f32, y0: f32, x1: f32, y1: f32) {
        let f = 4.0;
        self.poly(
            &[(x0, y0), (x1 - f, y0), (x1, y0 + f), (x1, y1), (x0, y1)],
            self.c.paper,
        );
        self.line(
            &[(x1 - f, y0), (x1 - f, y0 + f), (x1, y0 + f)],
            1.0,
            self.c.ink,
        );
    }

    /// Text lines on a sheet.
    fn text_lines(&self, x0: f32, x1: f32, ys: &[f32]) {
        for &y in ys {
            self.line(&[(x0, y), (x1, y)], 1.1, self.c.accent);
        }
    }
}

/// An oblique box: front square and the back square shifted up and right.
/// Corners are top-left, top-right, bottom-right, bottom-left.
struct Box3 {
    f: [Pt; 4],
    b: [Pt; 4],
}

impl Box3 {
    fn new(x0: f32, y0: f32, x1: f32, y1: f32, depth: f32) -> Self {
        let f = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
        let b = f.map(|(x, y)| (x + depth, y - depth));
        Self { f, b }
    }

    /// The box the view and style icons use.
    fn standard() -> Self {
        Self::new(3.0, 9.0, 15.0, 21.0, 6.0)
    }

    fn face(&self, side: Side) -> [Pt; 4] {
        let (f, b) = (self.f, self.b);
        match side {
            Side::Front => f,
            Side::Back => b,
            Side::Top => [f[0], b[0], b[1], f[1]],
            Side::Bottom => [f[3], b[3], b[2], f[2]],
            Side::Right => [f[1], b[1], b[2], f[2]],
            Side::Left => [f[0], b[0], b[3], f[3]],
        }
    }

    /// The 9 edges seen from the front, top and right.
    fn visible_edges(&self) -> Vec<(Pt, Pt)> {
        let (f, b) = (self.f, self.b);
        vec![
            (f[0], f[1]),
            (f[1], f[2]),
            (f[2], f[3]),
            (f[3], f[0]),
            (f[0], b[0]),
            (f[1], b[1]),
            (f[2], b[2]),
            (b[0], b[1]),
            (b[1], b[2]),
        ]
    }

    /// The 3 edges meeting at the hidden back-bottom-left corner.
    fn hidden_edges(&self) -> Vec<(Pt, Pt)> {
        let (f, b) = (self.f, self.b);
        vec![(b[3], b[0]), (b[3], b[2]), (b[3], f[3])]
    }

    /// Front, top and right filled with the given colors, outlined in ink.
    fn paint(&self, p: &Pen, front: Color32, top: Color32, right: Color32) {
        p.poly(&self.face(Side::Top), top);
        p.poly(&self.face(Side::Right), right);
        p.poly(&self.face(Side::Front), front);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Front,
    Back,
    Top,
    Bottom,
    Left,
    Right,
}

/// Light, middle and dark shades of the warm face color for top, front and right.
fn shades(p: &Pen) -> (Color32, Color32, Color32) {
    (mix(p.c.face, p.c.paper, 0.45), p.c.face, p.c.face_dark)
}

// ---------------------------------------------------------------- Standard

fn file_new(p: &Pen) {
    p.sheet(5.0, 2.0, 20.0, 22.0);
    // A spark: something new.
    let c = (6.0, 6.0);
    p.circle(c, 4.2, p.c.green, Some((1.0, p.c.ink)));
    p.line(&[(c.0 - 2.3, c.1), (c.0 + 2.3, c.1)], 1.6, p.c.paper);
    p.line(&[(c.0, c.1 - 2.3), (c.0, c.1 + 2.3)], 1.6, p.c.paper);
}

fn file_open(p: &Pen) {
    // Folder: back with its tab, then the front flap tipped open.
    p.fan(
        &[
            (2.0, 19.5),
            (2.0, 5.0),
            (9.0, 5.0),
            (11.0, 7.5),
            (19.0, 7.5),
            (19.0, 19.5),
        ],
        p.c.face_dark,
    );
    p.poly(
        &[(5.5, 10.5), (22.5, 10.5), (19.0, 19.5), (2.0, 19.5)],
        p.c.face,
    );
}

fn file_save(p: &Pen) {
    // Floppy disk.
    p.poly(
        &[
            (3.0, 3.0),
            (18.0, 3.0),
            (21.0, 6.0),
            (21.0, 21.0),
            (3.0, 21.0),
        ],
        p.c.accent,
    );
    p.rect(7.0, 3.0, 16.0, 9.0, p.c.paper);
    p.rect(12.5, 4.3, 14.5, 7.7, p.c.ink);
    p.rect(6.0, 13.0, 18.0, 21.0, p.c.paper);
    p.text_lines(8.0, 16.0, &[15.5, 18.2]);
}

fn file_print(p: &Pen) {
    p.rect(6.0, 2.5, 18.0, 10.0, p.c.paper);
    p.rect(2.0, 9.0, 22.0, 18.0, p.c.face_dark);
    p.dot((18.8, 12.0), 1.3, p.c.green);
    p.rect(6.0, 14.0, 18.0, 22.0, p.c.paper);
    p.text_lines(8.0, 16.0, &[17.0, 19.5]);
}

fn cut(p: &Pen) {
    // Scissors, points up.
    p.line(&[(9.5, 14.5), (16.5, 2.5)], 2.0, p.c.ink);
    p.line(&[(14.5, 14.5), (7.5, 2.5)], 2.0, p.c.ink);
    p.circle((7.5, 18.0), 3.3, p.c.paper, Some((1.8, p.c.red)));
    p.circle((16.5, 18.0), 3.3, p.c.paper, Some((1.8, p.c.red)));
    p.dot((12.0, 10.2), 1.2, p.c.paper);
}

fn copy(p: &Pen) {
    p.sheet(3.0, 2.0, 15.0, 16.0);
    p.text_lines(5.0, 10.5, &[7.0, 9.5]);
    p.sheet(9.0, 8.0, 21.0, 22.0);
    p.text_lines(11.0, 19.0, &[14.0, 16.5, 19.0]);
}

fn paste(p: &Pen) {
    // Clipboard with a sheet on it.
    p.rect(3.0, 4.0, 17.0, 22.0, p.c.face_dark);
    p.rect(7.0, 2.0, 13.0, 6.0, p.c.paper);
    p.sheet(10.0, 9.0, 21.5, 22.5);
    p.text_lines(12.0, 19.5, &[15.0, 17.5, 20.0]);
}

fn delete(p: &Pen) {
    // Bold red cross.
    for (a, b) in [((5.0, 5.0), (19.0, 19.0)), ((19.0, 5.0), (5.0, 19.0))] {
        p.line(&[a, b], 4.4, p.c.ink);
        p.line(&[a, b], 2.8, p.c.red);
    }
}

/// Undo: arrow turning back to the left; redo: mirrored.
fn turn_arrow(p: &Pen, mirror: bool) {
    let m = |(x, y): Pt| if mirror { (24.0 - x, y) } else { (x, y) };
    let c = (13.0, 13.0);
    let r = 7.0;
    let pts: Vec<Pt> = Pen::arc_pts(c, r, -60.0, 165.0)
        .into_iter()
        .map(m)
        .collect();
    p.line(&pts, 2.4, p.c.accent);
    let a = 165f32.to_radians();
    let end = (c.0 + r * a.cos(), c.1 - r * a.sin());
    let dir = (-a.sin(), -a.cos());
    let tip = (end.0 + dir.0 * 3.5, end.1 + dir.1 * 3.5);
    let (tip, dir) = (m(tip), if mirror { (-dir.0, dir.1) } else { dir });
    p.head(tip, dir, 6.0, p.c.accent);
}

fn model_info(p: &Pen) {
    p.cube((10.0, 13.5), 8.5);
    let c = (17.0, 7.0);
    p.circle(c, 5.5, p.c.accent, Some((1.0, p.c.ink)));
    p.dot((c.0, c.1 - 2.6), 1.0, p.c.paper);
    p.line(&[(c.0, c.1 - 0.8), (c.0, c.1 + 3.2)], 1.8, p.c.paper);
}

// ---------------------------------------------------------------- Views

fn view_iso(p: &Pen) {
    p.cube((12.0, 12.5), 10.0);
    // Door on the left wall: an iso view of a small house.
    p.poly(
        &[(5.0, 15.5), (7.6, 17.0), (7.6, 21.3), (5.0, 19.8)],
        p.c.face_dark,
    );
}

/// The standard box seen from the front-top-right with face `side` in blue. Faces
/// that are out of sight show through the others, whose hidden edges are dashed.
fn view_box(p: &Pen, side: Side) {
    let bx = Box3::standard();
    let hidden = matches!(side, Side::Back | Side::Left | Side::Bottom);
    let (top, front, right) = shades(p);
    if hidden {
        p.flat(&bx.face(side), p.c.accent);
        for (a, b) in bx.hidden_edges() {
            p.dashed(a, b, 1.2, 1.0, p.c.ink);
        }
        let glass = alpha(p.c.paper, 70);
        bx.paint(p, glass, glass, glass);
    } else {
        let pick = |s: Side, c: Color32| if s == side { p.c.accent } else { c };
        bx.paint(
            p,
            pick(Side::Front, front),
            pick(Side::Top, top),
            pick(Side::Right, right),
        );
    }
    // A door marks the front.
    let door = if side == Side::Front {
        p.c.paper
    } else {
        p.c.face_dark
    };
    p.rect(7.5, 15.0, 10.5, 21.0, door);
}

// ---------------------------------------------------------------- Styles

fn xray(p: &Pen) {
    let bx = Box3::standard();
    let glass = alpha(p.c.accent, 70);
    for s in [
        Side::Back,
        Side::Left,
        Side::Bottom,
        Side::Top,
        Side::Right,
        Side::Front,
    ] {
        p.flat(&bx.face(s), glass);
    }
    p.segments(&bx.hidden_edges(), 1.0, p.c.ink);
    p.segments(&bx.visible_edges(), W, p.c.ink);
}

fn back_edges(p: &Pen) {
    let bx = Box3::standard();
    let (top, front, right) = shades(p);
    bx.paint(p, front, top, right);
    for (a, b) in bx.hidden_edges() {
        p.dashed(a, b, 1.3, 1.1, p.c.ink);
    }
}

fn wireframe(p: &Pen) {
    let bx = Box3::standard();
    p.segments(&bx.visible_edges(), W, p.c.ink);
    p.segments(&bx.hidden_edges(), W, p.c.ink);
}

fn hidden_line(p: &Pen) {
    let bx = Box3::standard();
    bx.paint(p, p.c.paper, p.c.paper, p.c.paper);
}

fn shaded(p: &Pen) {
    let bx = Box3::standard();
    let (top, front, right) = shades(p);
    bx.paint(p, front, top, right);
}

fn shaded_textures(p: &Pen) {
    // Brick box.
    let bx = Box3::standard();
    let brick = p.c.red;
    bx.paint(
        p,
        brick,
        mix(brick, p.c.paper, 0.35),
        mix(brick, p.c.ink, 0.3),
    );
    let mortar = mix(p.c.paper, brick, 0.15);
    for (i, y) in [12.0, 15.0, 18.0].into_iter().enumerate() {
        p.line(&[(3.7, y), (14.3, y)], 1.0, mortar);
        let x0 = if i % 2 == 0 { 6.0 } else { 9.0 };
        let mut x = x0;
        while x < 14.5 {
            p.line(&[(x, y - 3.0 + 0.5), (x, y - 0.5)], 1.0, mortar);
            x += 6.0;
        }
    }
    for y in [12.0, 15.0, 18.0] {
        p.line(&[(15.5, y - 0.5), (20.5, y - 5.5)], 1.0, mortar);
    }
    p.segments(&bx.visible_edges(), W, p.c.ink);
}

fn monochrome(p: &Pen) {
    let bx = Box3::standard();
    bx.paint(
        p,
        mix(p.c.paper, p.c.ink, 0.2),
        p.c.paper,
        mix(p.c.paper, p.c.ink, 0.45),
    );
}

// ---------------------------------------------------------------- Section

fn section_planes(p: &Pen) {
    let bx = Box3::standard();
    let (top, front, right) = shades(p);
    bx.paint(p, front, top, right);
    // A section plane standing across the box, parallel to its side.
    let plane = [(6.5, 22.5), (6.5, 9.5), (14.3, 1.7), (14.3, 14.7)];
    p.flat(&plane, alpha(p.c.accent, 90));
    p.outline(&plane, 1.6, p.c.accent);
}

/// The left half of the box with its cut face; `fill`: the cut face filled dark.
fn section_cut(p: &Pen, fill: bool) {
    let removed = Box3::new(9.0, 9.0, 15.0, 21.0, 6.0);
    for (a, b) in removed.visible_edges() {
        p.dashed(a, b, 1.2, 1.0, p.c.accent);
    }
    let half = Box3::new(3.0, 9.0, 9.0, 21.0, 6.0);
    let (top, front, _) = shades(p);
    p.poly(&half.face(Side::Top), top);
    p.poly(&half.face(Side::Front), front);
    let cut = half.face(Side::Right);
    let face = if fill {
        mix(p.c.ink, p.c.paper, 0.3)
    } else {
        p.c.paper
    };
    p.flat(&cut, face);
    p.outline(&cut, 2.6, p.c.ink);
}

// ---------------------------------------------------------------- Solid Tools

/// The two solids of the Solid Tools icons: a square and a disc overlapping its
/// top-right corner.
const SQ: (f32, f32, f32, f32) = (3.0, 9.0, 15.0, 21.0);
const DISC: (Pt, f32) = ((14.0, 10.0), 7.0);

fn in_sq((x, y): Pt) -> bool {
    x > SQ.0 && x < SQ.2 && y > SQ.1 && y < SQ.3
}

fn in_disc((x, y): Pt) -> bool {
    let ((cx, cy), r) = DISC;
    (x - cx).powi(2) + (y - cy).powi(2) < r * r
}

fn sq_pts() -> Vec<Pt> {
    let (x0, y0, x1, y1) = SQ;
    let mut out = Vec::new();
    let n = 48;
    for i in 0..n {
        let t = i as f32 / n as f32;
        out.push((x0 + (x1 - x0) * t, y0));
        out.push((x1, y0 + (y1 - y0) * t));
        out.push((x1 - (x1 - x0) * t, y1));
        out.push((x0, y1 - (y1 - y0) * t));
    }
    out
}

fn disc_pts() -> Vec<Pt> {
    let ((cx, cy), r) = DISC;
    (0..120)
        .map(|i| {
            let a = (i as f32 * 3.0).to_radians();
            (cx + r * a.cos(), cy - r * a.sin())
        })
        .collect()
}

/// Boundary points of a region: square points kept by `sq`, disc points kept by
/// `disc`.
fn region(sq: impl Fn(Pt) -> bool, disc: impl Fn(Pt) -> bool) -> Vec<Pt> {
    let mut v: Vec<Pt> = sq_pts().into_iter().filter(|&q| sq(q)).collect();
    v.extend(disc_pts().into_iter().filter(|&q| disc(q)));
    v
}

/// Center points the regions are star-shaped from.
const MID: Pt = (11.0, 13.0);
const SQ_ONLY: Pt = (4.5, 19.5);
const DISC_ONLY: Pt = (18.0, 5.5);

fn union_ring() -> Vec<Pt> {
    region(|q| !in_disc(q), |q| !in_sq(q))
}

fn intersect_ring() -> Vec<Pt> {
    region(in_disc, in_sq)
}

fn sq_minus_disc() -> Vec<Pt> {
    region(|q| !in_disc(q), in_sq)
}

fn disc_minus_sq() -> Vec<Pt> {
    region(in_disc, |q| !in_sq(q))
}

fn dashed_shapes(p: &Pen) {
    let (x0, y0, x1, y1) = SQ;
    for (a, b) in [
        ((x0, y0), (x1, y0)),
        ((x1, y0), (x1, y1)),
        ((x1, y1), (x0, y1)),
        ((x0, y1), (x0, y0)),
    ] {
        p.dashed(a, b, 1.3, 1.0, p.c.ink);
    }
    let ((cx, cy), r) = DISC;
    let pts = p.pts(&Pen::arc_pts((cx, cy), r, 0.0, 360.0));
    p.painter.extend(Shape::dashed_line(
        &pts,
        p.stroke(1.0, p.c.ink),
        1.3 * p.s,
        1.3 * p.s,
    ));
}

fn outer_shell(p: &Pen) {
    p.star(MID, &union_ring(), p.c.face, None);
    // The inner faces the shell drops, dashed.
    let ((cx, cy), r) = DISC;
    let inner: Vec<Pt> = Pen::arc_pts((cx, cy), r, 180.0, 270.0)
        .into_iter()
        .filter(|&q| in_sq(q))
        .collect();
    let inner = p.pts(&inner);
    p.painter.extend(Shape::dashed_line(
        &inner,
        p.stroke(1.0, p.c.ink),
        1.2 * p.s,
        1.2 * p.s,
    ));
    p.dashed((SQ.0 + 4.0, SQ.1), (SQ.2, SQ.1), 1.2, 1.0, p.c.ink);
    p.dashed((SQ.2, SQ.1), (SQ.2, 17.0), 1.2, 1.0, p.c.ink);
    p.star(
        MID,
        &union_ring(),
        Color32::TRANSPARENT,
        Some((2.2, p.c.accent)),
    );
}

fn solid_intersect(p: &Pen) {
    dashed_shapes(p);
    p.star(MID, &intersect_ring(), p.c.face, Some((W, p.c.ink)));
}

fn solid_union(p: &Pen) {
    p.star(MID, &union_ring(), p.c.face, Some((W, p.c.ink)));
}

fn solid_subtract(p: &Pen) {
    dashed_shapes(p);
    p.star(SQ_ONLY, &sq_minus_disc(), p.c.face, Some((W, p.c.ink)));
}

fn solid_trim(p: &Pen) {
    p.star(DISC_ONLY, &disc_minus_sq(), p.c.face, Some((W, p.c.ink)));
    let (x0, y0, x1, y1) = SQ;
    p.rect(x0, y0, x1, y1, mix(p.c.accent, p.c.paper, 0.45));
}

fn solid_split(p: &Pen) {
    p.star(SQ_ONLY, &sq_minus_disc(), p.c.face, Some((W, p.c.ink)));
    p.star(
        DISC_ONLY,
        &disc_minus_sq(),
        p.c.face_dark,
        Some((W, p.c.ink)),
    );
    p.star(MID, &intersect_ring(), p.c.accent, Some((W, p.c.ink)));
}

// ---------------------------------------------------------------- Tags, Shadows

fn tags_panel(p: &Pen) {
    // Three stacked sheets: tags as layers.
    for (i, fill) in [(2.0, p.c.face_dark), (1.0, p.c.accent), (0.0, p.c.face)] {
        let y = 4.0 + i * 4.5;
        p.poly(
            &[(2.0, y + 5.0), (12.0, y), (22.0, y + 5.0), (12.0, y + 10.0)],
            fill,
        );
    }
}

/// Sun disc with rays.
fn sun(p: &Pen, c: Pt, r: f32) {
    for i in 0..8 {
        let a = (i as f32 * 45.0).to_radians();
        let (x, y) = (a.cos(), -a.sin());
        p.line(
            &[
                (c.0 + (r + 1.2) * x, c.1 + (r + 1.2) * y),
                (c.0 + (r + 3.2) * x, c.1 + (r + 3.2) * y),
            ],
            1.4,
            p.c.red,
        );
    }
    p.circle(c, r, mix(p.c.face, p.c.red, 0.3), Some((1.2, p.c.ink)));
}

fn shadow_settings(p: &Pen) {
    sun(p, (8.0, 8.0), 3.8);
    p.rect(10.0, 13.0, 22.5, 22.5, p.c.paper);
    for (y, knob) in [(16.0, 18.5), (19.5, 13.5)] {
        p.line(&[(12.0, y), (20.5, y)], 1.2, p.c.ink);
        p.rect(knob - 1.0, y - 1.8, knob + 1.0, y + 1.8, p.c.accent);
    }
}

fn display_shadows(p: &Pen) {
    sun(p, (5.5, 5.5), 3.0);
    // Shadow of the box falling down to the right.
    let shadow = mix(p.c.ink, p.c.paper, 0.4);
    p.flat(
        &[(6.0, 21.0), (17.0, 21.0), (23.5, 23.5), (12.5, 23.5)],
        shadow,
    );
    p.flat(
        &[(17.0, 21.0), (22.0, 16.0), (23.5, 19.0), (23.5, 23.5)],
        shadow,
    );
    let bx = Box3::new(6.0, 11.0, 17.0, 21.0, 5.0);
    let (top, front, right) = shades(p);
    bx.paint(p, front, top, right);
}

// ---------------------------------------------------------------- Sandbox

fn from_contours(p: &Pen) {
    // Contour lines stacked into a hill, joined by mesh edges.
    let rings: Vec<Vec<Pt>> = (0..3)
        .map(|k| {
            let k = k as f32;
            let (cx, cy) = (12.0, 18.5 - 5.5 * k);
            let (rx, ry) = (10.0 - 3.0 * k, 3.5 - 0.7 * k);
            (0..=36)
                .map(|i| {
                    let a = (i as f32 * 10.0).to_radians();
                    (cx + rx * a.cos(), cy - ry * a.sin())
                })
                .collect()
        })
        .collect();
    p.segments(
        &[
            ((2.0, 18.5), (5.0, 13.0)),
            ((5.0, 13.0), (8.0, 7.5)),
            ((22.0, 18.5), (19.0, 13.0)),
            ((19.0, 13.0), (16.0, 7.5)),
            ((12.0, 22.0), (12.0, 16.2)),
            ((12.0, 16.2), (12.0, 10.4)),
        ],
        0.9,
        p.c.ink,
    );
    for ring in &rings {
        p.line(ring, 1.6, p.c.green);
    }
}

fn from_scratch(p: &Pen) {
    // A flat grid seen at an angle.
    let (o, u, v) = ((2.0, 13.0), (10.0, -6.0), (10.0, 6.0));
    let at = |s: f32, t: f32| (o.0 + u.0 * s + v.0 * t, o.1 + u.1 * s + v.1 * t);
    p.poly(
        &[at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0), at(0.0, 1.0)],
        mix(p.c.green, p.c.paper, 0.7),
    );
    for i in 1..3 {
        let k = i as f32 / 3.0;
        p.line(&[at(k, 0.0), at(k, 1.0)], 1.0, p.c.green);
        p.line(&[at(0.0, k), at(1.0, k)], 1.0, p.c.green);
    }
}
