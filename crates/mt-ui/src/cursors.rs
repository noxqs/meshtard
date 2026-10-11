//! Mouse cursors of the 3D viewport, drawn procedurally at the pointer.
//!
//! egui's system cursor set is small, so over the viewport the app hides the system
//! cursor and paints the tool's cursor with [`show`]. Outside the viewport (menus,
//! toolbars, tray) the system cursors stay.
//!
//! Every cursor is meshtard's own drawing on a 32 x 32 grid, painted [`SIZE`] points
//! wide (so it scales with `pixels_per_point`), with a white halo around all ink so it
//! reads on dark and light backgrounds. The shapes, hotspots and badges are from memory
//! of SketchUp and **unverified**: the trial records the real ones (TRIAL_CHECKLIST.md
//! 1c), and SketchUp's cursor artwork is Trimble's and must not be copied anyway.

use egui::epaint::{Mesh, PathShape, PathStroke};
use egui::{Color32, Context, CursorIcon, Id, LayerId, Order, Painter, Pos2, Shape, Stroke, vec2};

/// Size of the 32-unit cursor grid on screen, in points (unverified; Windows cursors
/// are 32 px at 100 % scaling with the drawing a little smaller).
pub const SIZE: f32 = 28.0;

/// The cursor's main drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Arrow,
    /// The Lasso tool: an arrow with a lasso loop (unverified).
    Lasso,
    Pencil,
    Rectangle,
    RotatedRectangle,
    Circle,
    Polygon,
    Arc,
    Arc2Point,
    Arc3Point,
    Pie,
    Freehand,
    Sandbox,
    Eraser,
    PaintBucket,
    Eyedropper,
    Move,
    Rotate,
    Scale,
    Flip,
    PushPull,
    FollowMe,
    Offset,
    TapeMeasure,
    Protractor,
    Dimension,
    Text,
    SectionPlane,
    Axes,
    Tag,
    OuterShell,
    Intersect,
    Union,
    Subtract,
    Trim,
    Split,
    Orbit,
    Pan,
    Zoom,
    ZoomWindow,
    FieldOfView,
    Walk,
    LookAround,
    PositionCamera,
}

/// A small mark next to the drawing for a modifier or a state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Badge {
    #[default]
    None,
    /// Select with Ctrl; Move / Rotate copy; Push/Pull new face.
    Plus,
    /// Select with Shift (toggle).
    PlusMinus,
    /// Select with Ctrl+Shift (remove).
    Minus,
    /// Solid Tools: picking the first solid.
    One,
    /// Solid Tools: picking the second solid.
    Two,
    /// An invalid target under the cursor (circle-slash).
    NotAllowed,
    /// Eraser with Shift.
    Hide,
    /// Eraser with Ctrl.
    Soften,
    /// Eraser with Ctrl+Shift.
    Unsoften,
}

/// A viewport cursor: drawing plus badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cursor {
    pub kind: Kind,
    pub badge: Badge,
}

impl Cursor {
    pub const fn new(kind: Kind) -> Self {
        Self {
            kind,
            badge: Badge::None,
        }
    }

    pub const fn with(kind: Kind, badge: Badge) -> Self {
        Self { kind, badge }
    }
}

impl Kind {
    pub const ALL: &[Kind] = &[
        Self::Arrow,
        Self::Lasso,
        Self::Pencil,
        Self::Rectangle,
        Self::RotatedRectangle,
        Self::Circle,
        Self::Polygon,
        Self::Arc,
        Self::Arc2Point,
        Self::Arc3Point,
        Self::Pie,
        Self::Freehand,
        Self::Sandbox,
        Self::Eraser,
        Self::PaintBucket,
        Self::Eyedropper,
        Self::Move,
        Self::Rotate,
        Self::Scale,
        Self::Flip,
        Self::PushPull,
        Self::FollowMe,
        Self::Offset,
        Self::TapeMeasure,
        Self::Protractor,
        Self::Dimension,
        Self::Text,
        Self::SectionPlane,
        Self::Axes,
        Self::Tag,
        Self::OuterShell,
        Self::Intersect,
        Self::Union,
        Self::Subtract,
        Self::Trim,
        Self::Split,
        Self::Orbit,
        Self::Pan,
        Self::Zoom,
        Self::ZoomWindow,
        Self::FieldOfView,
        Self::Walk,
        Self::LookAround,
        Self::PositionCamera,
    ];

    fn layout(self) -> Layout {
        use Kind::*;
        match self {
            Arrow => Layout::Arrow,
            Lasso | Dimension | Text | SectionPlane | Tag | OuterShell | Intersect | Union
            | Subtract | Trim | Split => Layout::ArrowGlyph,
            Pencil | Rectangle | RotatedRectangle | Circle | Polygon | Arc | Arc2Point
            | Arc3Point | Pie | Freehand | Sandbox | Eraser | PaintBucket | Eyedropper
            | TapeMeasure => Layout::Tip,
            Axes => Layout::Origin,
            Move | Rotate | Scale | Flip | PushPull | FollowMe | Offset | Protractor | Orbit
            | Pan | Zoom | ZoomWindow | FieldOfView | Walk | LookAround | PositionCamera => {
                Layout::Centered
            }
        }
    }

    /// The hotspot (the pixel that clicks) in grid units, 0..32 with y down
    /// (unverified).
    pub fn hotspot(self) -> (f32, f32) {
        use Kind::*;
        match self {
            Arrow | Lasso | Dimension | Text | SectionPlane | Tag | OuterShell | Intersect
            | Union | Subtract | Trim | Split => (2.0, 2.0),
            Pencil | Rectangle | RotatedRectangle | Circle | Polygon | Arc | Arc2Point
            | Arc3Point | Pie | Freehand | Sandbox | Eyedropper | TapeMeasure | PaintBucket => {
                (3.0, 29.0)
            }
            Eraser => (4.0, 28.0),
            Axes => (7.0, 25.0),
            Rotate | Protractor => (16.0, 21.0),
            PushPull => (14.0, 18.0),
            Zoom | ZoomWindow => (12.0, 12.0),
            PositionCamera => (16.0, 30.0),
            Move | Scale | Flip | FollowMe | Offset | Orbit | Pan | FieldOfView | Walk
            | LookAround => (16.0, 16.0),
        }
    }

    /// Center of the badge, in grid units.
    fn badge_center(self) -> (f32, f32) {
        match self.layout() {
            Layout::Arrow => (19.0, 24.0),
            Layout::ArrowGlyph => (25.0, 7.0),
            Layout::Tip | Layout::Centered | Layout::Origin => (26.0, 26.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// Plain arrow, hotspot at its tip.
    Arrow,
    /// Arrow at the top left with a tool glyph at the lower right.
    ArrowGlyph,
    /// A tool whose tip at the lower left is the hotspot.
    Tip,
    /// Hotspot in (or near) the middle.
    Centered,
    /// Hotspot at an origin point.
    Origin,
}

/// Hides the system cursor and paints `cursor` with its hotspot at `pointer` (points)
/// on a layer above everything. Call every frame the pointer is over the viewport (or
/// a drag that started there continues).
pub fn show(ctx: &Context, pointer: Pos2, cursor: Cursor) {
    ctx.set_cursor_icon(CursorIcon::None);
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("mt_ui_tool_cursor")));
    paint(&painter, pointer, cursor, ctx.pixels_per_point());
}

/// Paints `cursor` with its hotspot at `pointer` (points); `ppp`: physical pixels per
/// point, to place the drawing on whole pixels and keep the halo at least a pixel wide.
pub fn paint(painter: &Painter, pointer: Pos2, cursor: Cursor, ppp: f32) {
    let s = SIZE / 32.0;
    let (hx, hy) = cursor.kind.hotspot();
    let ppp = if ppp.is_finite() && ppp > 0.0 {
        ppp
    } else {
        1.0
    };
    let snap = |v: f32| (v * ppp).round() / ppp;
    let origin = Pos2::new(snap(pointer.x - hx * s), snap(pointer.y - hy * s));
    let mut ink = Ink { prims: Vec::new() };
    draw_kind(&mut ink, cursor.kind);
    draw_badge(&mut ink, cursor.badge, cursor.kind.badge_center());
    let halo = (HALO * s).max(1.0 / ppp);
    ink.render(painter, origin, s, halo);
}

// ------------------------------------------------------------------ drawing kit

type Pt = (f32, f32);

const INK: Color32 = Color32::from_rgb(20, 20, 20);
const HALO_COLOR: Color32 = Color32::WHITE;
const PAPER: Color32 = Color32::from_rgb(252, 252, 252);
const YELLOW: Color32 = Color32::from_rgb(244, 196, 48);
const PINK: Color32 = Color32::from_rgb(238, 140, 150);
const RED: Color32 = Color32::from_rgb(214, 44, 36);
const GREEN: Color32 = Color32::from_rgb(40, 160, 70);
const BLUE: Color32 = Color32::from_rgb(40, 110, 215);
const LIGHT_BLUE: Color32 = Color32::from_rgb(178, 208, 245);
const FACE: Color32 = Color32::from_rgb(236, 222, 190);
const FACE_DARK: Color32 = Color32::from_rgb(190, 170, 130);
const GREY: Color32 = Color32::from_rgb(150, 150, 150);
const ORANGE: Color32 = Color32::from_rgb(240, 140, 30);

/// Outline width, in grid units.
const W: f32 = 1.3;
/// Halo width on each side of the ink, in grid units.
const HALO: f32 = 1.2;

enum Prim {
    Stroke {
        pts: Vec<Pt>,
        closed: bool,
        w: f32,
        color: Color32,
    },
    /// Polygon star-shaped from its first point.
    Fill {
        pts: Vec<Pt>,
        color: Color32,
    },
    Disc {
        c: Pt,
        r: f32,
        color: Color32,
    },
}

/// Shapes in grid units, rendered twice: white halo first, then the ink.
struct Ink {
    prims: Vec<Prim>,
}

impl Ink {
    fn line(&mut self, pts: &[Pt], w: f32, color: Color32) {
        self.prims.push(Prim::Stroke {
            pts: pts.to_vec(),
            closed: false,
            w,
            color,
        });
    }

    fn outline(&mut self, pts: &[Pt], w: f32, color: Color32) {
        self.prims.push(Prim::Stroke {
            pts: pts.to_vec(),
            closed: true,
            w,
            color,
        });
    }

    /// Filled polygon (star-shaped from its first point) outlined in ink.
    fn shape(&mut self, pts: &[Pt], fill: Color32) {
        self.prims.push(Prim::Fill {
            pts: pts.to_vec(),
            color: fill,
        });
        self.outline(pts, W, INK);
    }

    /// Filled polygon without outline.
    fn fill(&mut self, pts: &[Pt], fill: Color32) {
        self.prims.push(Prim::Fill {
            pts: pts.to_vec(),
            color: fill,
        });
    }

    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, fill: Color32) {
        self.shape(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)], fill);
    }

    fn disc(&mut self, c: Pt, r: f32, color: Color32) {
        self.prims.push(Prim::Disc { c, r, color });
    }

    /// Filled circle outlined in ink.
    fn circle(&mut self, c: Pt, r: f32, fill: Color32) {
        self.disc(c, r, fill);
        self.outline(&arc_pts(c, r, 0.0, 360.0), W, INK);
    }

    fn arc(&mut self, c: Pt, r: f32, a0: f32, a1: f32, w: f32, color: Color32) {
        self.line(&arc_pts(c, r, a0, a1), w, color);
    }

    /// A bar from `a` to `b`, `half` wide on each side, filled and outlined.
    fn bar(&mut self, a: Pt, b: Pt, half: f32, fill: Color32) {
        let (dx, dy) = unit((b.0 - a.0, b.1 - a.1));
        let (nx, ny) = (-dy * half, dx * half);
        self.shape(
            &[
                (a.0 + nx, a.1 + ny),
                (b.0 + nx, b.1 + ny),
                (b.0 - nx, b.1 - ny),
                (a.0 - nx, a.1 - ny),
            ],
            fill,
        );
    }

    /// Triangular arrow head with its tip at `tip`, pointing along `dir`.
    fn head(&mut self, tip: Pt, dir: Pt, size: f32, fill: Color32) {
        let (dx, dy) = unit(dir);
        let back = (tip.0 - dx * size, tip.1 - dy * size);
        let half = size * 0.6;
        self.shape(
            &[
                tip,
                (back.0 - dy * half, back.1 + dx * half),
                (back.0 + dy * half, back.1 - dx * half),
            ],
            fill,
        );
    }

    /// Line from `a` to `b` with a head at `b`.
    fn arrow(&mut self, a: Pt, b: Pt, w: f32, color: Color32) {
        let d = unit((b.0 - a.0, b.1 - a.1));
        let size = 5.0;
        self.line(
            &[a, (b.0 - d.0 * size * 0.7, b.1 - d.1 * size * 0.7)],
            w,
            color,
        );
        self.head(b, d, size, color);
    }

    /// Circular arrow along an arc with a head at `a1`.
    fn arc_arrow(&mut self, c: Pt, r: f32, a0: f32, a1: f32, color: Color32) {
        let pts = arc_pts(c, r, a0, a1);
        let n = pts.len();
        // Leave room for the head.
        let keep = n.saturating_sub(2).max(2);
        self.line(&pts[..keep], 2.0, color);
        let tip = pts[n - 1];
        let a = a1.to_radians();
        let sign = if a1 > a0 { 1.0 } else { -1.0 };
        // Tangent of the arc at a1 (y down).
        let dir = (-a.sin() * sign, -a.cos() * sign);
        self.head(tip, dir, 5.0, color);
    }

    fn render(&self, painter: &Painter, origin: Pos2, s: f32, halo: f32) {
        let p = |q: Pt| origin + vec2(q.0 * s, q.1 * s);
        let pts = |v: &[Pt]| v.iter().map(|&q| p(q)).collect::<Vec<_>>();
        // Halo pass.
        for prim in &self.prims {
            match prim {
                Prim::Stroke {
                    pts: v, closed, w, ..
                } => {
                    let stroke = Stroke::new(w * s + 2.0 * halo, HALO_COLOR);
                    let shape = if *closed {
                        Shape::closed_line(pts(v), stroke)
                    } else {
                        Shape::line(pts(v), stroke)
                    };
                    painter.add(shape);
                    // Round the ends of open strokes.
                    if !*closed {
                        for q in [v.first(), v.last()].into_iter().flatten() {
                            painter.add(Shape::circle_filled(
                                p(*q),
                                (w * s) / 2.0 + halo,
                                HALO_COLOR,
                            ));
                        }
                    }
                }
                Prim::Fill { pts: v, .. } => {
                    painter.add(fan(&pts(v), HALO_COLOR));
                    painter.add(Shape::Path(PathShape {
                        points: pts(v),
                        closed: true,
                        fill: Color32::TRANSPARENT,
                        stroke: PathStroke::new(2.0 * halo, HALO_COLOR),
                    }));
                }
                Prim::Disc { c, r, .. } => {
                    painter.add(Shape::circle_filled(p(*c), r * s + halo, HALO_COLOR));
                }
            }
        }
        // Ink pass.
        for prim in &self.prims {
            match prim {
                Prim::Stroke {
                    pts: v,
                    closed,
                    w,
                    color,
                } => {
                    let stroke = Stroke::new((w * s).max(1.0), *color);
                    painter.add(if *closed {
                        Shape::closed_line(pts(v), stroke)
                    } else {
                        Shape::line(pts(v), stroke)
                    });
                }
                Prim::Fill { pts: v, color } => {
                    painter.add(fan(&pts(v), *color));
                }
                Prim::Disc { c, r, color } => {
                    painter.add(Shape::circle_filled(p(*c), r * s, *color));
                }
            }
        }
    }
}

/// Triangle fan from the first point.
fn fan(pts: &[Pos2], color: Color32) -> Shape {
    let mut mesh = Mesh::default();
    for &q in pts {
        mesh.colored_vertex(q, color);
    }
    for i in 1..pts.len().saturating_sub(1) {
        mesh.add_triangle(0, i as u32, i as u32 + 1);
    }
    Shape::mesh(mesh)
}

fn unit(d: Pt) -> Pt {
    let len = (d.0 * d.0 + d.1 * d.1).sqrt().max(1e-6);
    (d.0 / len, d.1 / len)
}

/// Points on an arc; degrees counter-clockwise on screen (y down).
fn arc_pts(c: Pt, r: f32, a0: f32, a1: f32) -> Vec<Pt> {
    let n = (((a1 - a0).abs() / 10.0).ceil() as usize).max(2);
    (0..=n)
        .map(|i| {
            let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
            (c.0 + r * a.cos(), c.1 - r * a.sin())
        })
        .collect()
}

/// A point `t` along `d` and `side` across it from `o`.
fn along(o: Pt, d: Pt, t: f32, side: f32) -> Pt {
    (o.0 + d.0 * t - d.1 * side, o.1 + d.1 * t + d.0 * side)
}

const DIAG: Pt = (
    std::f32::consts::FRAC_1_SQRT_2,
    -std::f32::consts::FRAC_1_SQRT_2,
);

// ------------------------------------------------------------------ drawings

fn draw_kind(k: &mut Ink, kind: Kind) {
    use Kind::*;
    match kind {
        Arrow => arrow_cursor(k),
        Lasso => {
            arrow_cursor(k);
            // A loop of rope at the lower right, its tail running to the arrow.
            k.outline(&arc_pts((24.0, 22.0), 5.5, 0.0, 360.0), W, INK);
            k.line(&[(20.0, 26.0), (17.0, 30.0)], W, INK);
        }
        Pencil => pencil(k),
        Rectangle => {
            pencil(k);
            k.outline(
                &[(3.0, 4.0), (13.0, 4.0), (13.0, 11.0), (3.0, 11.0)],
                W,
                INK,
            );
        }
        RotatedRectangle => {
            pencil(k);
            k.outline(&[(2.0, 8.0), (8.0, 2.0), (14.0, 8.0), (8.0, 14.0)], W, INK);
        }
        Circle => {
            pencil(k);
            k.outline(&arc_pts((8.0, 8.0), 5.5, 0.0, 360.0), W, INK);
        }
        Polygon => {
            pencil(k);
            let hex: Vec<Pt> = (0..6)
                .map(|i| {
                    let a = (30.0 + 60.0 * i as f32).to_radians();
                    (8.0 + 5.8 * a.cos(), 8.0 - 5.8 * a.sin())
                })
                .collect();
            k.outline(&hex, W, INK);
        }
        Arc => {
            pencil(k);
            k.arc((8.0, 11.0), 6.0, 0.0, 180.0, W, INK);
            k.disc((8.0, 11.0), 1.2, INK);
        }
        Arc2Point => {
            pencil(k);
            k.arc((8.0, 11.0), 6.0, 0.0, 180.0, W, INK);
            k.disc((2.0, 11.0), 1.2, INK);
            k.disc((14.0, 11.0), 1.2, INK);
        }
        Arc3Point => {
            pencil(k);
            k.arc((8.0, 11.0), 6.0, 0.0, 180.0, W, INK);
            k.disc((2.0, 11.0), 1.2, INK);
            k.disc((8.0, 5.0), 1.2, INK);
            k.disc((14.0, 11.0), 1.2, INK);
        }
        Pie => {
            pencil(k);
            let mut pts = vec![(8.0, 11.0)];
            pts.extend(arc_pts((8.0, 11.0), 6.0, 0.0, 120.0));
            k.outline(&pts, W, INK);
        }
        Freehand => {
            pencil(k);
            k.line(
                &[
                    (2.0, 10.0),
                    (4.0, 6.0),
                    (6.0, 5.0),
                    (8.0, 8.0),
                    (10.0, 10.0),
                    (12.0, 8.0),
                    (14.0, 4.0),
                ],
                W,
                INK,
            );
        }
        Sandbox => {
            pencil(k);
            for i in 0..4 {
                let t = 2.0 + 4.0 * i as f32;
                k.line(&[(2.0, t), (14.0, t)], 1.0, INK);
                k.line(&[(t, 2.0), (t, 14.0)], 1.0, INK);
            }
        }
        Eraser => eraser(k),
        PaintBucket => paint_bucket(k),
        Eyedropper => eyedropper(k),
        Move => move_cursor(k),
        Rotate => {
            protractor(k);
            k.arc_arrow((16.0, 21.0), 15.0, 150.0, 40.0, RED);
        }
        Protractor => protractor(k),
        Scale => scale(k),
        Flip => flip(k),
        PushPull => push_pull(k),
        FollowMe => follow_me(k),
        Offset => offset(k),
        TapeMeasure => tape_measure(k),
        Dimension => {
            arrow_cursor(k);
            k.line(&[(14.0, 20.0), (14.0, 29.0)], 1.0, INK);
            k.line(&[(29.0, 20.0), (29.0, 29.0)], 1.0, INK);
            k.line(&[(14.0, 24.0), (29.0, 24.0)], 1.2, INK);
            k.head((14.0, 24.0), (-1.0, 0.0), 3.5, INK);
            k.head((29.0, 24.0), (1.0, 0.0), 3.5, INK);
        }
        Text => {
            arrow_cursor(k);
            k.line(&[(10.0, 16.0), (15.0, 21.0)], 1.0, INK);
            k.rect(15.0, 18.0, 30.0, 28.0, PAPER);
            for y in [21.0, 23.5] {
                k.line(&[(17.5, y), (27.5, y)], 1.0, INK);
            }
            k.line(&[(17.5, 26.0), (23.0, 26.0)], 1.0, INK);
        }
        SectionPlane => {
            arrow_cursor(k);
            k.shape(
                &[(15.0, 19.0), (25.0, 16.0), (29.0, 25.0), (19.0, 29.0)],
                ORANGE,
            );
            k.arrow((22.0, 22.5), (27.0, 31.0), 1.2, INK);
        }
        Axes => axes(k),
        Tag => {
            arrow_cursor(k);
            k.shape(
                &[
                    (24.0, 18.0),
                    (30.0, 18.0),
                    (30.0, 29.0),
                    (19.0, 29.0),
                    (19.0, 23.0),
                ],
                YELLOW,
            );
            k.circle((26.5, 21.5), 1.2, PAPER);
        }
        OuterShell | Intersect | Union | Subtract | Trim | Split => {
            arrow_cursor(k);
            solid_glyph(k, kind);
        }
        Orbit => {
            k.disc((16.0, 16.0), 2.0, INK);
            k.arc_arrow((16.0, 16.0), 11.0, 200.0, 340.0, RED);
            k.arc_arrow((16.0, 16.0), 11.0, 20.0, 160.0, RED);
        }
        Pan => hand(k),
        Zoom => {
            magnifier(k);
            k.line(&[(8.0, 12.0), (16.0, 12.0)], 1.6, INK);
            k.line(&[(12.0, 8.0), (12.0, 16.0)], 1.6, INK);
        }
        ZoomWindow => {
            magnifier(k);
            k.outline(
                &[(8.0, 9.0), (16.0, 9.0), (16.0, 15.0), (8.0, 15.0)],
                1.0,
                INK,
            );
        }
        FieldOfView => {
            k.line(&[(28.0, 5.0), (6.0, 16.0), (28.0, 27.0)], 1.8, INK);
            k.arc((6.0, 16.0), 16.0, -28.0, 28.0, 1.5, RED);
            k.circle((16.0, 16.0), 2.0, PAPER);
        }
        Walk => {
            foot(k, (11.0, 20.0));
            foot(k, (21.0, 11.0));
        }
        LookAround => eye(k),
        PositionCamera => figure(k),
    }
}

/// The system-style arrow, tip at (2, 2).
fn arrow_cursor(k: &mut Ink) {
    k.shape(
        &[
            (2.0, 2.0),
            (2.0, 22.0),
            (6.8, 17.6),
            (10.0, 25.0),
            (13.2, 23.6),
            (10.0, 16.4),
            (16.5, 16.4),
        ],
        PAPER,
    );
}

/// Pencil from the tip (3, 29) up to the right.
fn pencil(k: &mut Ink) {
    let tip = (3.0, 29.0);
    let w = 3.0;
    let at = |t: f32, side: f32| along(tip, DIAG, t, side);
    let len = 30.0;
    k.shape(
        &[at(7.0, -w), at(len - 4.0, -w), at(len - 4.0, w), at(7.0, w)],
        YELLOW,
    );
    k.shape(
        &[at(len - 4.0, -w), at(len, -w), at(len, w), at(len - 4.0, w)],
        PINK,
    );
    k.shape(&[tip, at(7.0, -w), at(7.0, w)], FACE);
    k.fill(&[tip, at(2.8, -1.2), at(2.8, 1.2)], INK);
}

/// Eraser block from the lower left corner (4, 28).
fn eraser(k: &mut Ink) {
    let o = (4.0, 28.0);
    let at = |t: f32, side: f32| along(o, DIAG, t, side);
    k.shape(
        &[at(0.0, -5.0), at(8.0, -5.0), at(8.0, 5.0), at(0.0, 5.0)],
        PINK,
    );
    k.shape(
        &[at(8.0, -5.0), at(22.0, -5.0), at(22.0, 5.0), at(8.0, 5.0)],
        BLUE,
    );
}

fn paint_bucket(k: &mut Ink) {
    // Bucket tilted to pour to the left.
    k.shape(
        &[(10.0, 9.0), (24.0, 3.0), (29.0, 16.0), (17.0, 22.0)],
        GREY,
    );
    k.line(&[(24.0, 3.0), (30.0, 6.0), (31.0, 12.0)], 1.4, INK);
    k.shape(
        &[(10.0, 9.0), (13.0, 7.7), (14.5, 12.0), (11.0, 13.0)],
        BLUE,
    );
    // Paint stream to the hotspot.
    k.shape(
        &[
            (11.0, 13.0),
            (9.0, 16.0),
            (4.5, 26.0),
            (3.0, 29.0),
            (6.0, 25.5),
            (12.0, 15.0),
        ],
        BLUE,
    );
}

fn eyedropper(k: &mut Ink) {
    let tip = (3.0, 29.0);
    let at = |t: f32, side: f32| along(tip, DIAG, t, side);
    k.shape(&[tip, at(4.0, -1.6), at(4.0, 1.6)], INK);
    k.shape(
        &[at(4.0, -2.2), at(17.0, -2.2), at(17.0, 2.2), at(4.0, 2.2)],
        LIGHT_BLUE,
    );
    k.shape(
        &[at(17.0, -4.0), at(19.0, -4.0), at(19.0, 4.0), at(17.0, 4.0)],
        GREY,
    );
    let c = at(24.0, 0.0);
    k.circle(c, 5.0, INK);
}

fn move_cursor(k: &mut Ink) {
    let c = (16.0, 16.0);
    for d in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        let tip = (c.0 + d.0 * 14.0, c.1 + d.1 * 14.0);
        k.line(&[c, (c.0 + d.0 * 9.5, c.1 + d.1 * 9.5)], 2.2, INK);
        k.head(tip, d, 5.5, RED);
    }
    k.disc(c, 2.0, INK);
}

/// Half-disc protractor with its center at (16, 21).
fn protractor(k: &mut Ink) {
    let c = (16.0, 21.0);
    let mut pts = vec![c];
    pts.extend(arc_pts(c, 11.0, 0.0, 180.0));
    k.shape(&pts, LIGHT_BLUE);
    for i in 1..6 {
        let a = (30.0 * i as f32).to_radians();
        let (cos, sin) = (a.cos(), a.sin());
        k.line(
            &[
                (c.0 + 8.5 * cos, c.1 - 8.5 * sin),
                (c.0 + 11.0 * cos, c.1 - 11.0 * sin),
            ],
            1.0,
            INK,
        );
    }
    k.disc(c, 1.5, INK);
}

/// A shape and its mirror image across a dashed plane, with a double arrow.
fn flip(k: &mut Ink) {
    k.shape(&[(13.0, 9.0), (13.0, 23.0), (4.0, 23.0)], FACE);
    k.shape(&[(19.0, 9.0), (19.0, 23.0), (28.0, 23.0)], LIGHT_BLUE);
    for y in [2.0, 7.0, 12.0, 17.0, 22.0, 27.0] {
        k.line(&[(16.0, y), (16.0, y + 3.0)], 1.2, RED);
    }
    k.arrow((12.0, 5.0), (4.0, 5.0), 1.4, INK);
    k.arrow((20.0, 5.0), (28.0, 5.0), 1.4, INK);
}

fn scale(k: &mut Ink) {
    k.rect(9.0, 9.0, 23.0, 23.0, FACE);
    k.outline(
        &[(13.0, 13.0), (19.0, 13.0), (19.0, 19.0), (13.0, 19.0)],
        1.0,
        INK,
    );
    k.line(&[(9.0, 23.0), (23.0, 9.0)], 1.0, INK);
    k.arrow((20.0, 12.0), (30.0, 2.0), 1.8, GREEN);
    k.arrow((12.0, 20.0), (2.0, 30.0), 1.8, GREEN);
}

fn push_pull(k: &mut Ink) {
    // A box with its top face pulled up.
    k.shape(
        &[(4.0, 22.0), (18.0, 22.0), (18.0, 29.0), (4.0, 29.0)],
        FACE_DARK,
    );
    k.shape(
        &[(18.0, 22.0), (26.0, 16.0), (26.0, 23.0), (18.0, 29.0)],
        FACE_DARK,
    );
    k.shape(
        &[(4.0, 22.0), (12.0, 16.0), (26.0, 16.0), (18.0, 22.0)],
        FACE,
    );
    k.arrow((14.0, 19.0), (14.0, 1.5), 2.2, RED);
}

fn follow_me(k: &mut Ink) {
    // A profile at the start of a curved path.
    k.shape(&[(3.0, 4.0), (11.0, 4.0), (11.0, 14.0), (3.0, 14.0)], FACE);
    let pts = arc_pts((16.0, 9.0), 13.0, 200.0, 330.0);
    k.line(&pts, 1.4, INK);
    let end = pts[pts.len() - 1];
    k.head((end.0 + 2.0, end.1 - 3.5), (0.5, -1.0), 5.0, RED);
}

fn offset(k: &mut Ink) {
    k.outline(
        &[(3.0, 3.0), (29.0, 3.0), (29.0, 29.0), (3.0, 29.0)],
        1.6,
        INK,
    );
    k.outline(
        &[(9.0, 9.0), (23.0, 9.0), (23.0, 23.0), (9.0, 23.0)],
        1.6,
        RED,
    );
    k.arrow((5.5, 5.5), (10.0, 10.0), 1.4, RED);
}

fn tape_measure(k: &mut Ink) {
    k.bar((3.5, 28.5), (16.0, 16.0), 1.8, YELLOW);
    k.line(&[(2.0, 27.0), (5.0, 30.0)], 1.6, INK);
    k.circle((21.0, 11.0), 8.5, YELLOW);
    k.circle((21.0, 11.0), 3.0, GREY);
}

fn axes(k: &mut Ink) {
    let o = (7.0, 25.0);
    k.line(&[o, (29.0, 25.0)], 2.0, RED);
    k.line(&[o, (21.0, 13.0)], 2.0, GREEN);
    k.line(&[o, (7.0, 2.0)], 2.0, BLUE);
    k.disc(o, 1.8, INK);
}

/// Two overlapping squares, filled per Solid Tools operation.
fn solid_glyph(k: &mut Ink, kind: Kind) {
    use Kind::*;
    let a = [(14.0, 15.0), (24.0, 15.0), (24.0, 25.0), (14.0, 25.0)];
    let b = [(19.0, 20.0), (29.0, 20.0), (29.0, 30.0), (19.0, 30.0)];
    let overlap = [(19.0, 20.0), (24.0, 20.0), (24.0, 25.0), (19.0, 25.0)];
    // B minus A (an L, star-shaped from its inner corner).
    let b_rest = [
        (24.0, 25.0),
        (24.0, 20.0),
        (29.0, 20.0),
        (29.0, 30.0),
        (19.0, 30.0),
        (19.0, 25.0),
    ];
    // A minus B.
    let a_rest = [
        (19.0, 20.0),
        (19.0, 25.0),
        (14.0, 25.0),
        (14.0, 15.0),
        (24.0, 15.0),
        (24.0, 20.0),
    ];
    match kind {
        OuterShell => {
            let outer = [
                (24.0, 20.0),
                (24.0, 15.0),
                (14.0, 15.0),
                (14.0, 25.0),
                (19.0, 25.0),
                (19.0, 30.0),
                (29.0, 30.0),
                (29.0, 20.0),
            ];
            k.fill(&a, FACE);
            k.fill(&b, FACE);
            k.outline(&outer, W, INK);
        }
        Union => {
            k.fill(&a, FACE);
            k.fill(&b, FACE);
            k.outline(&a, W, INK);
            k.outline(&b, W, INK);
        }
        Intersect => {
            k.outline(&a, 1.0, GREY);
            k.outline(&b, 1.0, GREY);
            k.shape(&overlap, FACE);
        }
        Subtract => {
            k.outline(&a, 1.0, GREY);
            k.shape(&b_rest, FACE);
        }
        Trim => {
            k.shape(&a, FACE_DARK);
            k.shape(&b_rest, FACE);
        }
        Split => {
            k.shape(&a_rest, FACE_DARK);
            k.shape(&b_rest, FACE);
            k.shape(&overlap, LIGHT_BLUE);
        }
        _ => {}
    }
}

fn hand(k: &mut Ink) {
    // Palm, then fingers and thumb as bars over it.
    k.shape(
        &[
            (16.0, 21.0),
            (9.0, 15.0),
            (24.0, 15.0),
            (24.0, 23.0),
            (20.0, 29.0),
            (12.0, 29.0),
            (9.0, 23.0),
        ],
        PAPER,
    );
    for (x, top) in [(10.5, 7.0), (14.5, 4.0), (18.5, 4.5), (22.5, 7.5)] {
        k.bar((x, 17.0), (x, top), 1.9, PAPER);
        k.disc((x, top), 1.9, PAPER);
        k.arc((x, top), 1.9, 0.0, 180.0, W, INK);
    }
    k.bar((10.0, 23.0), (4.5, 17.5), 1.9, PAPER);
    // Cover the finger outlines inside the palm.
    k.fill(
        &[(16.0, 20.0), (10.0, 16.4), (23.0, 16.4), (23.0, 22.0)],
        PAPER,
    );
}

fn magnifier(k: &mut Ink) {
    k.bar((17.5, 17.5), (28.0, 28.0), 2.4, INK);
    k.circle((12.0, 12.0), 9.0, PAPER);
}

fn foot(k: &mut Ink, c: Pt) {
    let sole: Vec<Pt> = (0..16)
        .map(|i| {
            let a = (i as f32 * 22.5).to_radians();
            (c.0 + 3.6 * a.cos(), c.1 + 6.5 * a.sin())
        })
        .collect();
    k.shape(&sole, INK);
    for (i, dx) in [-2.6f32, -0.9, 0.8, 2.5].into_iter().enumerate() {
        k.disc(
            (c.0 + dx, c.1 - 8.5 + (i as f32 - 1.5).abs() * 0.6),
            0.95,
            INK,
        );
    }
}

fn eye(k: &mut Ink) {
    let mut lens = arc_pts((16.0, 26.0), 16.0, 30.0, 150.0);
    lens.extend(arc_pts((16.0, 6.0), 16.0, 210.0, 330.0));
    // Star-shaped from the center: start the fan there.
    let mut pts = vec![(16.0, 16.0)];
    pts.extend(lens.iter().copied());
    pts.push(lens[0]);
    k.fill(&pts, PAPER);
    k.outline(&lens, W, INK);
    k.disc((16.0, 16.0), 5.0, BLUE);
    k.disc((16.0, 16.0), 2.3, INK);
}

fn figure(k: &mut Ink) {
    k.circle((16.0, 5.0), 3.2, FACE);
    k.bar((16.0, 9.0), (16.0, 20.0), 3.0, BLUE);
    k.bar((15.0, 19.5), (12.0, 30.0), 1.4, INK);
    k.bar((17.0, 19.5), (20.0, 30.0), 1.4, INK);
    k.bar((13.5, 10.5), (9.0, 18.0), 1.2, BLUE);
    k.bar((18.5, 10.5), (23.0, 18.0), 1.2, BLUE);
}

// ------------------------------------------------------------------ badges

fn draw_badge(k: &mut Ink, badge: Badge, c: Pt) {
    let w = 2.0;
    match badge {
        Badge::None => {}
        Badge::Plus => {
            k.line(&[(c.0 - 4.0, c.1), (c.0 + 4.0, c.1)], w, INK);
            k.line(&[(c.0, c.1 - 4.0), (c.0, c.1 + 4.0)], w, INK);
        }
        Badge::Minus => k.line(&[(c.0 - 4.0, c.1), (c.0 + 4.0, c.1)], w, INK),
        Badge::PlusMinus => {
            k.line(&[(c.0 - 3.5, c.1 - 2.5), (c.0 + 3.5, c.1 - 2.5)], 1.6, INK);
            k.line(&[(c.0, c.1 - 6.0), (c.0, c.1 + 1.0)], 1.6, INK);
            k.line(&[(c.0 - 3.5, c.1 + 4.0), (c.0 + 3.5, c.1 + 4.0)], 1.6, INK);
        }
        Badge::One => {
            k.line(
                &[
                    (c.0 - 2.0, c.1 - 2.5),
                    (c.0 + 0.5, c.1 - 4.5),
                    (c.0 + 0.5, c.1 + 4.5),
                ],
                1.8,
                INK,
            );
        }
        Badge::Two => {
            let mut pts = arc_pts((c.0, c.1 - 1.8), 2.8, 160.0, -40.0);
            pts.push((c.0 - 3.0, c.1 + 4.5));
            pts.push((c.0 + 3.2, c.1 + 4.5));
            k.line(&pts, 1.8, INK);
        }
        Badge::NotAllowed => {
            k.outline(&arc_pts(c, 4.8, 0.0, 360.0), 2.0, RED);
            k.line(&[(c.0 - 3.4, c.1 - 3.4), (c.0 + 3.4, c.1 + 3.4)], 2.0, RED);
        }
        Badge::Hide => {
            // A closed eye with a slash.
            let mut lens = arc_pts((c.0, c.1 + 5.0), 6.0, 35.0, 145.0);
            lens.extend(arc_pts((c.0, c.1 - 5.0), 6.0, 215.0, 325.0));
            k.outline(&lens, 1.3, INK);
            k.disc(c, 1.3, INK);
            k.line(&[(c.0 - 4.0, c.1 + 4.0), (c.0 + 4.0, c.1 - 4.0)], 1.5, INK);
        }
        Badge::Soften => {
            k.line(&arc_pts((c.0 - 4.5, c.1 + 4.5), 8.0, 0.0, 90.0), 1.6, INK);
        }
        Badge::Unsoften => {
            k.line(
                &[
                    (c.0 - 4.5, c.1 - 3.5),
                    (c.0 + 3.5, c.1 - 3.5),
                    (c.0 + 3.5, c.1 + 4.5),
                ],
                1.6,
                INK,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BADGES: &[Badge] = &[
        Badge::None,
        Badge::Plus,
        Badge::PlusMinus,
        Badge::Minus,
        Badge::One,
        Badge::Two,
        Badge::NotAllowed,
        Badge::Hide,
        Badge::Soften,
        Badge::Unsoften,
    ];

    #[test]
    fn every_kind_and_badge_paints() {
        let ctx = Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let ctx = ui.ctx();
            let painter = ctx.layer_painter(LayerId::background());
            for &kind in Kind::ALL {
                for &badge in BADGES {
                    for ppp in [1.0, 1.5, 2.0, 0.0] {
                        paint(
                            &painter,
                            Pos2::new(100.0, 100.0),
                            Cursor::with(kind, badge),
                            ppp,
                        );
                    }
                }
            }
            show(ctx, Pos2::new(5.0, 5.0), Cursor::new(Kind::Arrow));
        });
        out.textures_delta.clear();
        assert_eq!(out.platform_output.cursor_icon, CursorIcon::None);
        let meshes = ctx.tessellate(std::mem::take(&mut out.shapes), 2.0);
        assert!(!meshes.is_empty());
    }

    #[test]
    fn hotspots_and_badges_lie_on_the_grid() {
        for &kind in Kind::ALL {
            let (x, y) = kind.hotspot();
            assert!(
                (0.0..=32.0).contains(&x) && (0.0..=32.0).contains(&y),
                "{kind:?}"
            );
            let (bx, by) = kind.badge_center();
            assert!(
                (0.0..=32.0).contains(&bx) && (0.0..=32.0).contains(&by),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn every_drawing_has_ink_inside_the_grid() {
        for &kind in Kind::ALL {
            let mut ink = Ink { prims: Vec::new() };
            draw_kind(&mut ink, kind);
            assert!(!ink.prims.is_empty(), "{kind:?}");
            for prim in &ink.prims {
                let pts: Vec<Pt> = match prim {
                    Prim::Stroke { pts, .. } | Prim::Fill { pts, .. } => pts.clone(),
                    Prim::Disc { c, .. } => vec![*c],
                };
                for (x, y) in pts {
                    assert!(
                        (-1.0..=33.0).contains(&x) && (-1.0..=33.0).contains(&y),
                        "{kind:?} ({x}, {y})"
                    );
                }
            }
        }
    }
}
