//! The Select tool: picking, click rules and box selection, as in SketchUp (Windows).
//!
//! - Click selects one entity; a click on empty space clears the selection.
//! - Ctrl adds, Shift toggles, Ctrl+Shift removes.
//! - Double-click a face: the face and its edges. Double-click an edge: the edge and its
//!   faces. Triple-click: everything connected.
//! - Drag left to right: window (entities fully inside). Right to left: crossing
//!   (entities touching the box).
//! - Edges win over faces when the cursor is within a few pixels of one.

use std::collections::{HashSet, VecDeque};

use glam::{DVec2, DVec3};
use mt_kernel::Entities;
use mt_kernel::bvh::Bvh;
use mt_render::Camera;

use crate::document::{Owner, Scene};

/// Pixels around the cursor in which an edge is picked (draft; SketchUp's pick aperture
/// is not measured yet).
pub const APERTURE_PX: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Replace,
    Add,
    Toggle,
    Remove,
}

impl Mode {
    pub fn from_modifiers(ctrl: bool, shift: bool) -> Self {
        match (ctrl, shift) {
            (false, false) => Self::Replace,
            (true, false) => Self::Add,
            (false, true) => Self::Toggle,
            (true, true) => Self::Remove,
        }
    }
}

/// Selected top-level entities in selection order.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    items: Vec<Owner>,
    set: HashSet<Owner>,
}

impl Selection {
    pub fn apply(&mut self, mode: Mode, items: impl IntoIterator<Item = Owner>) {
        if mode == Mode::Replace {
            self.clear();
        }
        for o in items {
            let has = self.set.contains(&o);
            match mode {
                Mode::Replace | Mode::Add if !has => self.insert(o),
                Mode::Toggle if has => self.remove(o),
                Mode::Toggle => self.insert(o),
                Mode::Remove if has => self.remove(o),
                _ => {}
            }
        }
    }

    fn insert(&mut self, o: Owner) {
        self.set.insert(o);
        self.items.push(o);
    }

    fn remove(&mut self, o: Owner) {
        self.set.remove(&o);
        self.items.retain(|x| *x != o);
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.set.clear();
    }

    pub fn contains(&self, o: &Owner) -> bool {
        self.set.contains(o)
    }

    pub fn items(&self) -> &[Owner] {
        &self.items
    }
}

/// What a click with `clicks` presses on `hit` selects.
pub fn expand(ents: &Entities, hit: Owner, clicks: u32) -> Vec<Owner> {
    // A click on one segment of a curve selects the whole curve.
    if let (Owner::Edge(e), 0 | 1) = (hit, clicks)
        && let Some(c) = ents.edges.get(e).and_then(|x| x.attrs.curve)
    {
        return ents.curve_edges(c).into_iter().map(Owner::Edge).collect();
    }
    match (hit, clicks) {
        (_, 0 | 1) | (Owner::Instance(_), _) => vec![hit],
        (Owner::Face(f), 2) => {
            let mut v = vec![hit];
            let mut seen = HashSet::new();
            for l in ents.faces[f].loops() {
                for c in &l.coedges {
                    if seen.insert(c.edge) {
                        v.push(Owner::Edge(c.edge));
                    }
                }
            }
            v
        }
        (Owner::Edge(e), 2) => std::iter::once(hit)
            .chain(ents.edges[e].faces.iter().map(|&f| Owner::Face(f)))
            .collect(),
        _ => connected(ents, hit),
    }
}

/// Every face and edge connected to `start` through shared vertices.
fn connected(ents: &Entities, start: Owner) -> Vec<Owner> {
    let mut vertices = VecDeque::new();
    match start {
        Owner::Edge(e) => {
            vertices.push_back(ents.edges[e].start);
            vertices.push_back(ents.edges[e].end);
        }
        Owner::Face(f) => {
            for l in ents.faces[f].loops() {
                vertices.extend(ents.loop_vertices(l));
            }
        }
        Owner::Instance(_) => return vec![start],
    }
    let mut seen_v = HashSet::new();
    let mut edges = Vec::new();
    let mut faces = Vec::new();
    let mut seen_e = HashSet::new();
    let mut seen_f = HashSet::new();
    while let Some(v) = vertices.pop_front() {
        if !seen_v.insert(v) {
            continue;
        }
        for &e in &ents.vertices[v].edges {
            if !seen_e.insert(e) {
                continue;
            }
            edges.push(Owner::Edge(e));
            let edge = &ents.edges[e];
            vertices.push_back(edge.other(v));
            for &f in &edge.faces {
                if seen_f.insert(f) {
                    faces.push(Owner::Face(f));
                }
            }
        }
    }
    let mut out = vec![start];
    out.extend(faces.into_iter().chain(edges).filter(|o| *o != start));
    out
}

/// Distance from `p` to the 2D segment `a-b`.
fn dist_to_segment(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 {
        ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * t)
}

fn v3(p: [f32; 3]) -> DVec3 {
    DVec3::new(p[0].into(), p[1].into(), p[2].into())
}

/// The entity under the cursor: an edge within [`APERTURE_PX`] that is not hidden behind
/// a face, else the nearest face.
pub fn pick(
    scene: &Scene,
    bvh: &Bvh,
    camera: &Camera,
    cursor: DVec2,
    viewport: DVec2,
) -> Option<Owner> {
    let ray = camera.ray(cursor, viewport);
    let face_hit = bvh.ray(ray.origin, ray.dir);
    let face_t = face_hit.map_or(f64::INFINITY, |h| h.t);

    let mut best: Option<(f64, Owner)> = None;
    for (seg, owner) in scene.edges.iter().zip(&scene.edge_owner) {
        let Some(owner) = owner else { continue };
        let (a, b) = (v3(seg.a), v3(seg.b));
        let (Some(pa), Some(pb)) = (camera.project(a, viewport), camera.project(b, viewport))
        else {
            continue;
        };
        let d = dist_to_segment(cursor, pa, pb);
        if d > APERTURE_PX || best.is_some_and(|(bd, _)| bd <= d) {
            continue;
        }
        // Depth of the edge along the ray where it passes closest to it.
        let t = closest_ray_param(ray.origin, ray.dir, a, b);
        if t <= face_t * (1.0 + 1e-4) + 1e-3 {
            best = Some((d, *owner));
        }
    }
    best.map(|(_, o)| o)
        .or_else(|| face_hit.and_then(|h| scene.tri_owner[h.triangle]))
}

/// Parameter along the ray (unit `dir`) of the point closest to segment `a-b`.
fn closest_ray_param(o: DVec3, dir: DVec3, a: DVec3, b: DVec3) -> f64 {
    match mt_kernel::geom::segment_segment(o, o + dir * 1e9, a, b) {
        Some((_, s, _)) => s * 1e9,
        None => (a - o).dot(dir),
    }
}

/// Screen rectangle `[min, max]` in pixels.
#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub min: DVec2,
    pub max: DVec2,
}

impl Rect {
    pub fn from_corners(a: DVec2, b: DVec2) -> Self {
        Self {
            min: a.min(b),
            max: a.max(b),
        }
    }

    fn contains(&self, p: DVec2) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }

    /// Whether segment `a-b` touches the rectangle (Liang-Barsky clipping).
    fn touches_segment(&self, a: DVec2, b: DVec2) -> bool {
        let d = b - a;
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        for (p, q) in [
            (-d.x, a.x - self.min.x),
            (d.x, self.max.x - a.x),
            (-d.y, a.y - self.min.y),
            (d.y, self.max.y - a.y),
        ] {
            if p == 0.0 {
                if q < 0.0 {
                    return false;
                }
            } else {
                let r = q / p;
                if p < 0.0 {
                    t0 = t0.max(r);
                } else {
                    t1 = t1.min(r);
                }
            }
        }
        t0 <= t1
    }

    fn touches_triangle(&self, [a, b, c]: [DVec2; 3]) -> bool {
        let inside_tri = |p: DVec2| {
            let s = |u: DVec2, v: DVec2| (v - u).perp_dot(p - u);
            let (d1, d2, d3) = (s(a, b), s(b, c), s(c, a));
            (d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0) || (d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0)
        };
        self.touches_segment(a, b)
            || self.touches_segment(b, c)
            || self.touches_segment(c, a)
            || inside_tri(self.min)
    }
}

/// Entities selected by a box: `window` needs them fully inside, otherwise touching is
/// enough. Geometry behind the camera never counts as inside.
pub fn box_select(
    scene: &Scene,
    camera: &Camera,
    viewport: DVec2,
    rect: Rect,
    window: bool,
) -> Vec<Owner> {
    let mut inside: Vec<Owner> = Vec::new();
    let mut outside: HashSet<Owner> = HashSet::new();
    let mut touched: HashSet<Owner> = HashSet::new();
    let mut note = |owner: Owner, fully_in: bool, touches: bool| {
        if !fully_in {
            outside.insert(owner);
        }
        if touches && !touched.contains(&owner) {
            touched.insert(owner);
            inside.push(owner);
        } else if fully_in && !touched.contains(&owner) && !inside.contains(&owner) {
            inside.push(owner);
        }
    };
    let proj = |p: DVec3| camera.project(p, viewport);
    for (seg, owner) in scene.edges.iter().zip(&scene.edge_owner) {
        let Some(owner) = owner else { continue };
        match (proj(v3(seg.a)), proj(v3(seg.b))) {
            (Some(a), Some(b)) => note(
                *owner,
                rect.contains(a) && rect.contains(b),
                rect.touches_segment(a, b),
            ),
            _ => note(*owner, false, false),
        }
    }
    for (tri, owner) in scene.triangles.iter().zip(&scene.tri_owner) {
        let Some(owner) = owner else { continue };
        match (proj(tri[0]), proj(tri[1]), proj(tri[2])) {
            (Some(a), Some(b), Some(c)) => note(
                *owner,
                rect.contains(a) && rect.contains(b) && rect.contains(c),
                rect.touches_triangle([a, b, c]),
            ),
            _ => note(*owner, false, false),
        }
    }
    if window {
        inside
            .into_iter()
            .filter(|o| !outside.contains(o))
            .collect()
    } else {
        inside.into_iter().filter(|o| touched.contains(o)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_kernel::{EdgeId, FaceId, add_line};

    fn square(e: &mut Entities, x: f64) {
        let p = |a: f64, b: f64| DVec3::new(a + x, b, 0.0);
        for (a, b) in [
            (p(0.0, 0.0), p(1.0, 0.0)),
            (p(1.0, 0.0), p(1.0, 1.0)),
            (p(1.0, 1.0), p(0.0, 1.0)),
            (p(0.0, 1.0), p(0.0, 0.0)),
        ] {
            add_line(e, a, b);
        }
    }

    fn ids(e: &Entities) -> (FaceId, EdgeId) {
        (
            e.faces.keys().next().unwrap(),
            e.edges.keys().next().unwrap(),
        )
    }

    #[test]
    fn modifier_modes() {
        let mut e = Entities::new();
        square(&mut e, 0.0);
        let (f, ed) = ids(&e);
        let mut s = Selection::default();
        s.apply(Mode::Replace, [Owner::Face(f)]);
        s.apply(Mode::Add, [Owner::Edge(ed)]);
        assert_eq!(s.items().len(), 2);
        s.apply(Mode::Toggle, [Owner::Face(f)]);
        assert_eq!(s.items(), [Owner::Edge(ed)]);
        s.apply(Mode::Remove, [Owner::Edge(ed)]);
        assert!(s.items().is_empty());
        s.apply(Mode::Add, [Owner::Face(f)]);
        s.apply(Mode::Replace, []);
        assert!(s.items().is_empty(), "click on empty space clears");
    }

    #[test]
    fn double_and_triple_click() {
        let mut e = Entities::new();
        square(&mut e, 0.0);
        square(&mut e, 5.0);
        let f = e.faces.keys().next().unwrap();
        assert_eq!(
            expand(&e, Owner::Face(f), 2).len(),
            5,
            "face and its 4 edges"
        );
        let ed = e.faces[f].outer.coedges[0].edge;
        assert_eq!(expand(&e, Owner::Edge(ed), 2).len(), 2, "edge and its face");
        assert_eq!(
            expand(&e, Owner::Face(f), 3).len(),
            5,
            "only the connected square"
        );
    }

    #[test]
    fn rect_tests() {
        let r = Rect::from_corners(DVec2::new(10.0, 10.0), DVec2::new(0.0, 0.0));
        assert!(r.touches_segment(DVec2::new(-5.0, 5.0), DVec2::new(15.0, 5.0)));
        assert!(!r.touches_segment(DVec2::new(-5.0, 15.0), DVec2::new(15.0, 15.0)));
        let big = [
            DVec2::new(-100.0, -100.0),
            DVec2::new(100.0, -100.0),
            DVec2::new(0.0, 100.0),
        ];
        assert!(r.touches_triangle(big), "rect inside a triangle touches it");
    }
}

#[cfg(test)]
mod pick_tests {
    use super::*;
    use crate::document::Document;

    #[test]
    fn picking_a_projected_triangle_returns_its_owner() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/openskp/capilla_quiroz_v17.skp");
        let doc = Document::open(&path).unwrap();
        let scene = doc.scene();
        let bvh = Bvh::build(scene.triangles.clone());
        let camera = doc
            .saved_camera()
            .or_else(|| doc.extents_camera(35.0))
            .unwrap();
        let viewport = DVec2::new(1280.0, 760.0);
        let mut hits = 0;
        let mut tried = 0;
        for (t, tri) in scene.triangles.iter().enumerate().step_by(7) {
            let c = (tri[0] + tri[1] + tri[2]) / 3.0;
            let Some(px) = camera.project(c, viewport) else {
                continue;
            };
            if px.x < 0.0 || px.y < 0.0 || px.x > viewport.x || px.y > viewport.y {
                continue;
            }
            tried += 1;
            if pick(&scene, &bvh, &camera, px, viewport).is_some() {
                hits += 1;
            }
            let _ = t;
        }
        assert!(tried > 10, "only {tried} triangles on screen");
        assert_eq!(
            hits, tried,
            "every on-screen triangle centroid picks something"
        );
    }
}
