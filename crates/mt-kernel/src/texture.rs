//! Material textures and per-face texture positioning.

use std::sync::Arc;

use glam::{DVec2, DVec3};
use mt_skp::uv::UvMapping;

use crate::entities::Face;
use crate::model::Material;

/// A material's texture image.
#[derive(Debug, Clone, PartialEq)]
pub struct Texture {
    /// File name as stored (SketchUp keeps the source path).
    pub filename: String,
    /// Image file bytes as stored (PNG, JPEG or BMP), shared by model copies.
    pub data: Arc<[u8]>,
    /// Size of one tile in inches (0 when the file has none; treated as 1).
    pub width: f64,
    pub height: f64,
    /// SketchUp colorizes the image towards the material color. The material color
    /// is the image's average color unless the user recolored it; files often set
    /// this flag either way.
    pub colorize: bool,
}

/// Texture positioning of a face's sides (SketchUp's pinned/positioned textures): the
/// 3x3 row-major texture-to-plane matrix as stored in the file, `None` for the default
/// projection.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FaceUv {
    pub front: Option<[f64; 9]>,
    pub back: Option<[f64; 9]>,
}

impl FaceUv {
    /// Boxed when at least one side is positioned, else `None` (the cheap default).
    pub fn boxed(front: Option<[f64; 9]>, back: Option<[f64; 9]>) -> Option<Box<Self>> {
        (front.is_some() || back.is_some()).then(|| Box::new(Self { front, back }))
    }
}

/// The texture mapping of one side of `face` painted with `material`, or `None` when
/// the material has no texture. Evaluate it at the face's vertices in the face's own
/// (definition) coordinates; [`uv_at`] does that.
pub fn face_mapping(face: &Face, back: bool, material: &Material) -> Option<UvMapping> {
    let t = material.texture.as_ref()?;
    let uv = face.attrs.uv.as_deref();
    let transform = uv.and_then(|u| {
        if back {
            u.back.as_ref()
        } else {
            u.front.as_ref()
        }
    });
    Some(UvMapping::new(
        face.normal.to_array(),
        transform,
        [t.width, t.height],
    ))
}

/// UV (in texture repeats) of point `p` under `mapping`.
pub fn uv_at(mapping: &UvMapping, p: DVec3) -> DVec2 {
    DVec2::from_array(mapping.uv(p.to_array()))
}

/// One side's stored `uv_transform` after moving its face by `xf`: the texture stays on
/// the same points of the face. `old_normal`/`new_normal`: the face's unit normal
/// before and after; `on_face`: a point of the face before the move.
pub fn transform_uv(
    m: &[f64; 9],
    old_normal: DVec3,
    new_normal: DVec3,
    on_face: DVec3,
    xf: glam::DMat4,
) -> [f64; 9] {
    let basis = |n: DVec3| {
        let (x, y) = mt_skp::uv::face_uv_basis(n.to_array());
        (DVec3::from_array(x), DVec3::from_array(y))
    };
    let (xr, yr) = basis(old_normal);
    let (nxr, nyr) = basis(new_normal);
    // Texture point (u, v) to plane coordinates: [px py q] = [u v 1] * m.
    let plane = |u: f64, v: f64| {
        let q = u * m[2] + v * m[5] + m[8];
        let q = if q.abs() < 1e-12 { 1.0 } else { q };
        DVec2::new(u * m[0] + v * m[3] + m[6], u * m[1] + v * m[4] + m[7]) / q
    };
    let lift = |p: DVec2| on_face + xr * (p.x - on_face.dot(xr)) + yr * (p.y - on_face.dot(yr));
    let moved = |u, v| {
        let p = xf.transform_point3(lift(plane(u, v)));
        DVec2::new(p.dot(nxr), p.dot(nyr))
    };
    let (o, a, b) = (moved(0.0, 0.0), moved(1.0, 0.0), moved(0.0, 1.0));
    let (a, b) = (a - o, b - o);
    [a.x, a.y, 0.0, b.x, b.y, 0.0, o.x, o.y, 1.0]
}

/// The texture-to-plane matrix that puts one tile of a `tile`-sized texture on the
/// rectangle `origin + s * u + t * v` (`s`, `t` in 0..1) of a face with unit normal
/// `normal` (an image entity's face, a texture fit to a face).
pub fn fit_uv(normal: DVec3, origin: DVec3, u: DVec3, v: DVec3, tile: [f64; 2]) -> [f64; 9] {
    let (xr, yr) = mt_skp::uv::face_uv_basis(normal.to_array());
    let (xr, yr) = (DVec3::from_array(xr), DVec3::from_array(yr));
    let size = |s: f64| if s > 1e-9 { s } else { 1.0 };
    let (tw, th) = (size(tile[0]), size(tile[1]));
    [
        u.dot(xr) / tw,
        u.dot(yr) / tw,
        0.0,
        v.dot(xr) / th,
        v.dot(yr) / th,
        0.0,
        origin.dot(xr),
        origin.dot(yr),
        1.0,
    ]
}

/// Texture-to-plane matrix of the default projection (the texture as if never
/// positioned): identity, see [`UvMapping`].
pub const DEFAULT_UV: [f64; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

/// The four pins of Texture Position, in SketchUp's colors: Red (move) on the tile
/// corner `(k, l)`, Green (scale/rotate) on `(k + 1, l)`, Blue (scale/shear) on
/// `(k, l + 1)` and Yellow (distort) on `(k + 1, l + 1)`, in texture repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pin {
    Red,
    Green,
    Blue,
    Yellow,
}

impl Pin {
    pub const ALL: [Pin; 4] = [Pin::Red, Pin::Green, Pin::Blue, Pin::Yellow];

    fn index(self) -> usize {
        self as usize
    }

    /// Offset of the pin from the red pin, in texture repeats.
    pub fn offset(self) -> DVec2 {
        match self {
            Pin::Red => DVec2::ZERO,
            Pin::Green => DVec2::X,
            Pin::Blue => DVec2::Y,
            Pin::Yellow => DVec2::ONE,
        }
    }
}

/// Plane point of the texture point `t` (in repeats) under a texture-to-plane matrix
/// `m` for a texture of tile size `tile` (row vector convention, as stored in files:
/// `[px py q] = [u v 1] * m` with `u`, `v` in repeats times the tile size).
pub fn plane_point(m: &[f64; 9], tile: [f64; 2], t: DVec2) -> DVec2 {
    let (u, v) = (t.x * tile_size(tile[0]), t.y * tile_size(tile[1]));
    let q = u * m[2] + v * m[5] + m[8];
    let q = if q.abs() < 1e-12 { 1.0 } else { q };
    DVec2::new(u * m[0] + v * m[3] + m[6], u * m[1] + v * m[4] + m[7]) / q
}

/// Tile sizes of 0 count as 1 (as in [`UvMapping`]).
fn tile_size(s: f64) -> f64 {
    if s > 1e-9 { s } else { 1.0 }
}

/// Plane positions of the four pins (in [`Pin::ALL`] order) with the red pin on the
/// texture point `corner` (repeats).
pub fn pins_from_uv(m: &[f64; 9], tile: [f64; 2], corner: DVec2) -> [DVec2; 4] {
    Pin::ALL.map(|p| plane_point(m, tile, corner + p.offset()))
}

/// The texture-to-plane matrix that puts the pins' texture points (red on `corner`)
/// on the plane points `pins` (in [`Pin::ALL`] order): an affine map when the pins
/// form a parallelogram, a projective one otherwise. `None` when the pins do not
/// form a convex quadrilateral.
pub fn uv_from_pins(pins: &[DVec2; 4], tile: [f64; 2], corner: DVec2) -> Option<[f64; 9]> {
    // Square-to-quad (Heckbert) with the square's corners (0,0), (1,0), (1,1), (0,1).
    let [p0, p1, p3, p2] = *pins;
    if !convex([p0, p1, p2, p3]) {
        return None;
    }
    let s = p0 - p1 + p2 - p3;
    let scale = (p1 - p0).length_squared();
    let (g, h) = if s.length_squared() <= 1e-24 * scale {
        (0.0, 0.0)
    } else {
        let d1 = p1 - p2;
        let d2 = p3 - p2;
        let den = d1.perp_dot(d2);
        if den.abs() < 1e-18 {
            return None;
        }
        (s.perp_dot(d2) / den, d1.perp_dot(s) / den)
    };
    let a = p1.x - p0.x + g * p1.x;
    let b = p3.x - p0.x + h * p3.x;
    let d = p1.y - p0.y + g * p1.y;
    let e = p3.y - p0.y + h * p3.y;
    // Row-vector form of the square map: [x y w] = [s t 1] * hs.
    let hs = [a, d, g, b, e, h, p0.x, p0.y, 1.0];
    // Texture units to square coordinates: s = u / tw - k, t = v / th - l.
    let pre = [
        1.0 / tile_size(tile[0]),
        0.0,
        0.0,
        0.0,
        1.0 / tile_size(tile[1]),
        0.0,
        -corner.x,
        -corner.y,
        1.0,
    ];
    let m = mul3(&pre, &hs);
    m.iter().all(|x| x.is_finite()).then_some(m)
}

/// `a * b` for row-major 3x3 matrices.
fn mul3(a: &[f64; 9], b: &[f64; 9]) -> [f64; 9] {
    let mut out = [0.0; 9];
    for r in 0..3 {
        for c in 0..3 {
            out[r * 3 + c] = (0..3).map(|k| a[r * 3 + k] * b[k * 3 + c]).sum();
        }
    }
    out
}

/// Whether the quadrilateral `q` (corners in order) is convex and not degenerate.
fn convex(q: [DVec2; 4]) -> bool {
    let turns = [0, 1, 2, 3].map(|i| {
        let (a, b, c) = (q[i], q[(i + 1) % 4], q[(i + 2) % 4]);
        (b - a).perp_dot(c - b)
    });
    let scale = (q[1] - q[0])
        .length_squared()
        .max((q[3] - q[0]).length_squared());
    let eps = 1e-12 * scale;
    scale > 0.0 && (turns.iter().all(|&t| t > eps) || turns.iter().all(|&t| t < -eps))
}

/// The pins after dragging `pin` to `to` (Texture Position). With fixed pins, as in
/// SketchUp: Red moves the texture; Green scales and rotates it about the red pin;
/// Blue scales and shears it (red and green stay, yellow moves with blue); Yellow
/// distorts it (perspective; the others stay). With free pins (`free`) each pin
/// moves alone.
pub fn drag_pin(pins: &[DVec2; 4], pin: Pin, to: DVec2, free: bool) -> [DVec2; 4] {
    let mut out = *pins;
    let i = pin.index();
    let delta = to - pins[i];
    if free {
        out[i] = to;
        return out;
    }
    match pin {
        Pin::Red => {
            for p in &mut out {
                *p += delta;
            }
        }
        Pin::Green => {
            let red = pins[0];
            let (from, onto) = (pins[1] - red, to - red);
            if from.length_squared() < 1e-24 || onto.length_squared() < 1e-24 {
                return out;
            }
            // Complex multiplication by onto / from: a rotation and uniform scale.
            let k = DVec2::new(from.dot(onto), from.perp_dot(onto)) / from.length_squared();
            for p in &mut out[1..] {
                *p = red + k.rotate(*p - red);
            }
        }
        Pin::Blue => {
            out[2] = to;
            out[3] += delta;
        }
        Pin::Yellow => out[3] = to,
    }
    out
}

/// A Texture Position action on the pins (the right-click menu's Flip and Rotate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinAction {
    /// Mirror left to right.
    FlipLeftRight,
    /// Mirror up and down.
    FlipUpDown,
    /// Turn the texture counterclockwise by this many quarter turns.
    Rotate(u8),
}

/// The pins after `action`: the texture flips or turns within the pins'
/// quadrilateral (the pins stay where they are drawn; the texture corner each one
/// holds changes).
pub fn pin_action(pins: &[DVec2; 4], action: PinAction) -> [DVec2; 4] {
    let [r, g, b, y] = *pins;
    match action {
        PinAction::FlipLeftRight => [g, r, y, b],
        PinAction::FlipUpDown => [b, y, r, g],
        PinAction::Rotate(n) => {
            // Corners counterclockwise: red, green, yellow, blue. A quarter turn puts
            // each corner of the image where the next one was.
            let mut ring = [r, g, y, b];
            for _ in 0..n % 4 {
                ring = [ring[1], ring[2], ring[3], ring[0]];
            }
            [ring[0], ring[1], ring[3], ring[2]]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DMat4;

    fn close(a: DVec2, b: DVec2) -> bool {
        a.abs_diff_eq(b, 1e-9)
    }

    #[test]
    fn pins_round_trip_through_the_matrix() {
        let tile = [40.0, 20.0];
        let corner = DVec2::new(2.0, -1.0);
        let affine = [0.5, 0.25, 0.0, -0.1, 0.8, 0.0, 3.0, -7.0, 1.0];
        let pins = pins_from_uv(&affine, tile, corner);
        let m = uv_from_pins(&pins, tile, corner).expect("convex");
        for (a, b) in m.iter().zip(affine) {
            assert!((a - b).abs() < 1e-9, "{m:?}");
        }
        // A distorted (projective) set of pins comes back exactly.
        let mut distorted = pins;
        distorted[3] += DVec2::new(4.0, 3.0);
        let m = uv_from_pins(&distorted, tile, corner).expect("convex");
        let back = pins_from_uv(&m, tile, corner);
        for (a, b) in back.iter().zip(distorted) {
            assert!(close(*a, b), "{back:?} vs {distorted:?}");
        }
        // UvMapping (what the renderer uses) puts the pins' texture points there.
        let map = UvMapping::new([0.0, 0.0, 1.0], Some(&m), tile);
        for (pin, at) in Pin::ALL.iter().zip(distorted) {
            let uv = uv_at(&map, DVec3::new(at.x, at.y, 0.0));
            assert!(close(uv, corner + pin.offset()), "{pin:?}: {uv}");
        }
        // The default projection is the identity.
        let pins = pins_from_uv(&DEFAULT_UV, tile, DVec2::ZERO);
        assert!(close(pins[3], DVec2::new(40.0, 20.0)));
        let m = uv_from_pins(&pins, tile, DVec2::ZERO).unwrap();
        for (a, b) in m.iter().zip(DEFAULT_UV) {
            assert!((a - b).abs() < 1e-12);
        }
    }

    #[test]
    fn degenerate_pins_are_refused() {
        let tile = [1.0, 1.0];
        let line = [DVec2::ZERO, DVec2::X, DVec2::X * 2.0, DVec2::X * 3.0];
        assert!(uv_from_pins(&line, tile, DVec2::ZERO).is_none());
        // Yellow dragged across: a bow tie.
        let bow = [DVec2::ZERO, DVec2::X, DVec2::Y, DVec2::new(-1.0, 0.5)];
        assert!(uv_from_pins(&bow, tile, DVec2::ZERO).is_none());
    }

    #[test]
    fn fixed_and_free_pin_drags() {
        let pins = [DVec2::ZERO, DVec2::X, DVec2::Y, DVec2::ONE];
        let moved = drag_pin(&pins, Pin::Red, DVec2::new(2.0, 3.0), false);
        assert!(close(moved[3], DVec2::new(3.0, 4.0)));
        // Green to (0, 2): turned a quarter and doubled about red.
        let g = drag_pin(&pins, Pin::Green, DVec2::new(0.0, 2.0), false);
        assert!(close(g[0], DVec2::ZERO));
        assert!(close(g[1], DVec2::new(0.0, 2.0)));
        assert!(close(g[2], DVec2::new(-2.0, 0.0)));
        assert!(close(g[3], DVec2::new(-2.0, 2.0)));
        // Blue sheared: still a parallelogram.
        let b = drag_pin(&pins, Pin::Blue, DVec2::new(0.5, 1.0), false);
        assert!(close(b[3], b[1] + b[2] - b[0]));
        // Yellow alone; free pins too.
        let y = drag_pin(&pins, Pin::Yellow, DVec2::new(2.0, 2.0), false);
        assert_eq!(&y[..3], &pins[..3]);
        let f = drag_pin(&pins, Pin::Green, DVec2::new(3.0, 0.0), true);
        assert_eq!(f[2], pins[2]);
        assert!(close(f[1], DVec2::new(3.0, 0.0)));
    }

    #[test]
    fn flips_and_turns() {
        let tile = [1.0, 1.0];
        let pins = [DVec2::ZERO, DVec2::X, DVec2::Y, DVec2::ONE];
        let at = |pins: &[DVec2; 4], p: DVec2| {
            let m = uv_from_pins(pins, tile, DVec2::ZERO).unwrap();
            uv_at(
                &UvMapping::new([0.0, 0.0, 1.0], Some(&m), tile),
                p.extend(0.0),
            )
        };
        let lr = pin_action(&pins, PinAction::FlipLeftRight);
        assert!(close(at(&lr, DVec2::new(0.25, 0.5)), DVec2::new(0.75, 0.5)));
        let ud = pin_action(&pins, PinAction::FlipUpDown);
        assert!(close(at(&ud, DVec2::new(0.25, 0.1)), DVec2::new(0.25, 0.9)));
        // A quarter turn counterclockwise: the image's right-hand point (1, 0.5) is
        // now at the top (0.5, 1).
        let r1 = pin_action(&pins, PinAction::Rotate(1));
        assert!(close(at(&r1, DVec2::new(0.5, 1.0)), DVec2::new(1.0, 0.5)));
        // Four quarter turns, two half turns and two flips change nothing.
        let mut p = pins;
        for _ in 0..4 {
            p = pin_action(&p, PinAction::Rotate(1));
        }
        assert_eq!(p, pins);
        let r2 = pin_action(&pins, PinAction::Rotate(2));
        assert_eq!(pin_action(&r2, PinAction::Rotate(2)), pins);
        assert_eq!(pin_action(&r1, PinAction::Rotate(3)), pins);
        assert_eq!(pin_action(&lr, PinAction::FlipLeftRight), pins);
        assert_eq!(pin_action(&ud, PinAction::FlipUpDown), pins);
    }

    #[test]
    fn fit_and_move_keep_texture_on_the_rectangle() {
        let tile = [40.0, 20.0];
        let n = DVec3::Z;
        let m = fit_uv(n, DVec3::ZERO, DVec3::X * 40.0, DVec3::Y * 20.0, tile);
        let map = UvMapping::new(n.to_array(), Some(&m), tile);
        assert!(uv_at(&map, DVec3::new(40.0, 20.0, 0.0)).abs_diff_eq(DVec2::ONE, 1e-12));
        // Stand the rectangle up on a wall and move it.
        let xf = DMat4::from_translation(DVec3::new(5.0, 7.0, 3.0))
            * DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2);
        let n2 = xf.transform_vector3(n).normalize();
        let m2 = transform_uv(&m, n, n2, DVec3::ZERO, xf);
        let map2 = UvMapping::new(n2.to_array(), Some(&m2), tile);
        for (p, uv) in [
            (DVec3::ZERO, DVec2::ZERO),
            (DVec3::new(40.0, 0.0, 0.0), DVec2::X),
            (DVec3::new(40.0, 20.0, 0.0), DVec2::ONE),
        ] {
            let got = uv_at(&map2, xf.transform_point3(p));
            assert!(got.abs_diff_eq(uv, 1e-9), "{p} -> {got}");
        }
    }
}
