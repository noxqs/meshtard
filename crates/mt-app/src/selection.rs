//! The Select tool: picking, click rules and box selection, as in SketchUp (Windows).
//!
//! - Click selects one entity; a click on empty space clears the selection.
//! - Ctrl adds, Shift toggles, Ctrl+Shift removes.
//! - Double-click a face: the face and its edges. Double-click an edge: the edge and its
//!   faces. Triple-click: everything connected.
//! - Drag left to right: window (entities fully inside). Right to left: crossing
//!   (entities touching the box).
//! - Edges win over faces when the cursor is within a few pixels of one.

use std::collections::{HashMap, HashSet, VecDeque};

use glam::{DMat4, DVec2, DVec3};
use mt_kernel::Entities;
use mt_kernel::bvh::RayIndex;
use mt_kernel::spatial::FxMap;
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

    /// The Lasso's keys (2026 Windows Quick Reference Card [doc], FEATURES.md 76):
    /// Shift adds and subtracts, Alt adds, Shift+Alt subtracts. Without Alt the
    /// Select tool's Ctrl keys work too (Meshtard's earlier keys).
    pub fn for_lasso(ctrl: bool, shift: bool, alt: bool) -> Self {
        match (alt, shift) {
            (true, false) => Self::Add,
            (true, true) => Self::Remove,
            (false, _) => Self::from_modifiers(ctrl, shift),
        }
    }
}

/// A set of owners, faster than a hash set for the large ones Select All makes: each
/// kind has a table by index (faces and edges by their key's slot, holding the key's
/// version, so a stale key never matches a live one). An owner whose place is taken
/// (another version of the slot) or far out goes to a hash set.
#[derive(Debug, Clone, Default)]
pub struct OwnerSet {
    /// Per kind (see `place`), by index: the version stored there (0: none).
    tables: [Vec<u32>; 6],
    other: mt_kernel::spatial::FxSet<Owner>,
}

/// Indices at or past this go to the hash set (bounds a table's size).
const TABLE_MAX: usize = 1 << 24;

impl OwnerSet {
    /// Kind, index and version of `o`.
    fn place(o: &Owner) -> (usize, usize, u32) {
        use slotmap::Key;
        let key = |k: slotmap::KeyData| {
            let ffi = k.as_ffi();
            ((ffi & 0xffff_ffff) as usize, (ffi >> 32) as u32)
        };
        let ((i, v), kind) = match *o {
            Owner::Face(f) => (key(f.data()), 0),
            Owner::Edge(e) => (key(e.data()), 1),
            Owner::Instance(i) => ((i, 1), 2),
            Owner::Section(i) => ((i, 1), 3),
            Owner::Dimension(i) => ((i, 1), 4),
            Owner::Text(i) => ((i, 1), 5),
        };
        // Versions of live keys are never 0; a null key goes to the hash set.
        (kind, if v == 0 { TABLE_MAX } else { i }, v)
    }

    pub fn contains(&self, o: &Owner) -> bool {
        let (kind, i, v) = Self::place(o);
        self.tables[kind].get(i).is_some_and(|&x| x == v)
            || (!self.other.is_empty() && self.other.contains(o))
    }

    /// Adds `o`; false when it was there.
    pub fn insert(&mut self, o: Owner) -> bool {
        let (kind, i, v) = Self::place(&o);
        if i >= TABLE_MAX {
            return self.other.insert(o);
        }
        let table = &mut self.tables[kind];
        if table.len() <= i {
            table.resize((i + 1).max(table.len() * 3 / 2), 0);
        }
        match table[i] {
            0 => {
                table[i] = v;
                true
            }
            x if x == v => false,
            _ => self.other.insert(o),
        }
    }

    /// Removes `o`; false when it was not there.
    pub fn remove(&mut self, o: &Owner) -> bool {
        let (kind, i, v) = Self::place(o);
        match self.tables[kind].get_mut(i) {
            Some(x) if *x == v => {
                *x = 0;
                true
            }
            _ => self.other.remove(o),
        }
    }

    pub fn clear(&mut self) {
        for t in &mut self.tables {
            t.clear();
        }
        self.other.clear();
    }
}

/// Selected top-level entities in selection order.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    items: Vec<Owner>,
    set: OwnerSet,
}

impl Selection {
    pub fn apply(&mut self, mode: Mode, items: impl IntoIterator<Item = Owner>) {
        if mode == Mode::Replace {
            self.clear();
        }
        let items = items.into_iter();
        if matches!(mode, Mode::Replace | Mode::Add) {
            self.items.reserve(items.size_hint().0);
        }
        for o in items {
            if matches!(mode, Mode::Replace | Mode::Add) {
                // One lookup: insert reports whether it was there.
                if self.set.insert(o) {
                    self.items.push(o);
                }
                continue;
            }
            let has = self.set.contains(&o);
            match mode {
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

    /// Keeps the items for which `keep` is true (e.g. those that still exist).
    pub fn retain(&mut self, mut keep: impl FnMut(&Owner) -> bool) {
        self.items.retain(|o| keep(o));
        self.set.clear();
        for o in &self.items {
            self.set.insert(*o);
        }
    }
}

/// Whether `o` still exists in `ents` (annotations and section planes are not checked).
pub fn exists(ents: &Entities, o: &Owner) -> bool {
    match *o {
        Owner::Edge(e) => ents.edges.contains_key(e),
        Owner::Face(f) => ents.faces.contains_key(f),
        Owner::Instance(i) => i < ents.instances.len(),
        Owner::Section(_) | Owner::Dimension(_) | Owner::Text(_) => true,
    }
}

/// A box with axes of its own: SketchUp's selection box of a component or group
/// (aligned with the component's axes) and the Scale tool's grip box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrientedBox {
    pub center: DVec3,
    /// Unit axis directions.
    pub dirs: [DVec3; 3],
    /// Half the size along each of `dirs` (0 for a flat box).
    pub half: DVec3,
}

impl OrientedBox {
    /// The box from `min` to `max` along the coordinate axes.
    pub fn from_aabb(min: DVec3, max: DVec3) -> Self {
        Self {
            center: (min + max) * 0.5,
            dirs: [DVec3::X, DVec3::Y, DVec3::Z],
            half: (max - min).abs() * 0.5,
        }
    }

    /// The axis-aligned box around `points`; `None` without points.
    pub fn around(points: impl IntoIterator<Item = DVec3>) -> Option<Self> {
        let mut it = points.into_iter();
        let first = it.next()?;
        let (min, max) = it.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p)));
        Some(Self::from_aabb(min, max))
    }

    /// The box carried by `m` (an affine transform without shear keeps it a box).
    pub fn transformed(&self, m: DMat4) -> Self {
        let mut out = Self {
            center: m.transform_point3(self.center),
            ..*self
        };
        for i in 0..3 {
            let v = m.transform_vector3(self.dirs[i]);
            let l = v.length();
            if l > 1e-12 {
                out.dirs[i] = v / l;
                out.half[i] = self.half[i] * l;
            } else {
                out.half[i] = 0.0;
            }
        }
        out
    }

    /// The point at box coordinates `g` (-1 to 1 along each axis; 0 is the center).
    pub fn at(&self, g: DVec3) -> DVec3 {
        self.center
            + self.dirs[0] * (self.half.x * g.x)
            + self.dirs[1] * (self.half.y * g.y)
            + self.dirs[2] * (self.half.z * g.z)
    }

    pub fn corners(&self) -> [DVec3; 8] {
        std::array::from_fn(|k| {
            let s = |b: usize| if k & b == 0 { -1.0 } else { 1.0 };
            self.at(DVec3::new(s(1), s(2), s(4)))
        })
    }

    /// The twelve edges (repeated points when the box is flat).
    pub fn edges(&self) -> [[DVec3; 2]; 12] {
        let c = self.corners();
        // Corner k has bit 1 for +x, 2 for +y, 4 for +z: edges join corners one bit apart.
        let mut out = [[DVec3::ZERO; 2]; 12];
        let mut n = 0;
        for k in 0..8usize {
            for b in [1usize, 2, 4] {
                if k & b == 0 {
                    out[n] = [c[k], c[k | b]];
                    n += 1;
                }
            }
        }
        out
    }
}

/// Bounds of each top-level instance of the scene in its definition's space, from the
/// drawn triangles and edges (nested geometry included, hidden geometry not).
/// `to_world(i)` is instance `i`'s definition-to-world transform.
pub fn instance_bounds(
    scene: &Scene,
    to_world: impl Fn(usize) -> Option<DMat4>,
) -> HashMap<usize, (DVec3, DVec3)> {
    let mut inverse: FxMap<usize, Option<DMat4>> = FxMap::default();
    let mut out: FxMap<usize, (DVec3, DVec3)> = FxMap::default();
    let mut add = |i: usize, p: DVec3| {
        let inv = *inverse.entry(i).or_insert_with(|| {
            to_world(i)
                .filter(|m| m.determinant().abs() > 1e-18)
                .map(|m| m.inverse())
        });
        let Some(inv) = inv else { return };
        let q = inv.transform_point3(p);
        let b = out.entry(i).or_insert((q, q));
        b.0 = b.0.min(q);
        b.1 = b.1.max(q);
    };
    for (tri, owner) in scene.triangles.iter().zip(&scene.tri_owner) {
        if let Some(Owner::Instance(i)) = owner {
            tri.iter().for_each(|p| add(*i, *p));
        }
    }
    for (seg, owner) in scene.edges.iter().zip(&scene.edge_owner) {
        if let Some(Owner::Instance(i)) = owner {
            add(*i, v3(seg.a));
            add(*i, v3(seg.b));
        }
    }
    out.into_iter().collect()
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
        (_, 0 | 1)
        | (Owner::Instance(_) | Owner::Section(_) | Owner::Dimension(_) | Owner::Text(_), _) => {
            vec![hit]
        }
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
        Owner::Instance(_) | Owner::Section(_) | Owner::Dimension(_) | Owner::Text(_) => {
            return vec![start];
        }
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
    bvh: &dyn RayIndex,
    camera: &Camera,
    cursor: DVec2,
    viewport: DVec2,
) -> Option<Owner> {
    // Dimension and text labels are drawn over everything.
    if let Some(o) = crate::annot::pick(scene, |p| camera.project(p, viewport), cursor, viewport) {
        return Some(o);
    }
    let ray = camera.ray(cursor, viewport);
    let face_hit = bvh.ray(ray.origin, ray.dir);
    let face_t = face_hit.map_or(f64::INFINITY, |h| h.t);

    let mut best: Option<(f64, Owner)> = None;
    // Only edges near the cursor on screen can be picked; their index order keeps the
    // tie-breaking of a scan of every edge.
    let near = scene.edges_near(camera, viewport, cursor, APERTURE_PX);
    let all = 0..scene.edges.len();
    let candidates: Box<dyn Iterator<Item = usize>> = match &near {
        Some(near) => Box::new(near.iter().copied()),
        None => Box::new(all),
    };
    let view_proj = camera.view_proj(viewport.x / viewport.y);
    let project = |p: DVec3| Camera::project_with(&view_proj, p, viewport);
    for i in candidates {
        let (seg, Some(owner)) = (&scene.edges[i], scene.edge_owner.get(i).copied().flatten())
        else {
            continue;
        };
        let owner = &owner;
        let (a, b) = (v3(seg.a), v3(seg.b));
        let (Some(pa), Some(pb)) = (project(a), project(b)) else {
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
}

/// A screen region a drag selects with: the Select tool's box or the Lasso's polygon.
pub trait Region {
    fn contains(&self, p: DVec2) -> bool;
    /// Whether segment `a-b` touches the region.
    fn touches_segment(&self, a: DVec2, b: DVec2) -> bool;
    /// Whether a point of the region lies inside the screen triangle (with
    /// [`Region::touches_segment`] on its sides: whether the triangle touches it).
    fn any_point_in(&self, tri: [DVec2; 3]) -> bool;

    fn touches_triangle(&self, [a, b, c]: [DVec2; 3]) -> bool {
        self.touches_segment(a, b)
            || self.touches_segment(b, c)
            || self.touches_segment(c, a)
            || self.any_point_in([a, b, c])
    }
}

/// Whether `p` lies inside (or on) the screen triangle.
fn in_triangle(p: DVec2, [a, b, c]: [DVec2; 3]) -> bool {
    let s = |u: DVec2, v: DVec2| (v - u).perp_dot(p - u);
    let (d1, d2, d3) = (s(a, b), s(b, c), s(c, a));
    (d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0) || (d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0)
}

impl Region for Rect {
    fn contains(&self, p: DVec2) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }

    /// Liang-Barsky clipping.
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

    fn any_point_in(&self, tri: [DVec2; 3]) -> bool {
        in_triangle(self.min, tri)
    }
}

/// A closed screen polygon (the Lasso tool's outline), in viewport pixels.
#[derive(Debug, Clone, Default)]
pub struct Polygon {
    pub points: Vec<DVec2>,
}

impl Polygon {
    pub fn new(points: Vec<DVec2>) -> Self {
        Self { points }
    }

    /// Twice the signed area with y pointing down (screen pixels): positive when the
    /// outline runs clockwise as seen on screen.
    pub fn clockwise_area2(&self) -> f64 {
        let n = self.points.len();
        (0..n)
            .map(|i| {
                let (a, b) = (self.points[i], self.points[(i + 1) % n]);
                a.perp_dot(b)
            })
            .sum()
    }

    /// Drawn clockwise on screen: the Lasso selects what it touches (crossing);
    /// counterclockwise selects only what lies fully inside (window).
    pub fn is_clockwise(&self) -> bool {
        self.clockwise_area2() > 0.0
    }

    fn sides(&self) -> impl Iterator<Item = (DVec2, DVec2)> + '_ {
        let n = self.points.len();
        (0..n).map(move |i| (self.points[i], self.points[(i + 1) % n]))
    }
}

/// Whether segments `a-b` and `c-d` intersect (touching counts).
fn segments_cross(a: DVec2, b: DVec2, c: DVec2, d: DVec2) -> bool {
    let o = |p: DVec2, q: DVec2, r: DVec2| (q - p).perp_dot(r - p);
    let on = |p: DVec2, q: DVec2, r: DVec2| {
        r.x >= p.x.min(q.x) && r.x <= p.x.max(q.x) && r.y >= p.y.min(q.y) && r.y <= p.y.max(q.y)
    };
    let (d1, d2, d3, d4) = (o(c, d, a), o(c, d, b), o(a, b, c), o(a, b, d));
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    (d1 == 0.0 && on(c, d, a))
        || (d2 == 0.0 && on(c, d, b))
        || (d3 == 0.0 && on(a, b, c))
        || (d4 == 0.0 && on(a, b, d))
}

impl Region for Polygon {
    /// Even-odd rule (a self-crossing lasso leaves its doubly wound parts out).
    fn contains(&self, p: DVec2) -> bool {
        if self.points.len() < 3 {
            return false;
        }
        let mut inside = false;
        for (a, b) in self.sides() {
            if (a.y > p.y) != (b.y > p.y) {
                let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
                if p.x < x {
                    inside = !inside;
                }
            }
        }
        inside
    }

    fn touches_segment(&self, a: DVec2, b: DVec2) -> bool {
        self.points.len() >= 3
            && (self.contains(a)
                || self.contains(b)
                || self.sides().any(|(c, d)| segments_cross(a, b, c, d)))
    }

    fn any_point_in(&self, tri: [DVec2; 3]) -> bool {
        self.points.first().is_some_and(|p| in_triangle(*p, tri))
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
    region_select(scene, camera, viewport, &rect, window)
}

/// Whether a lasso drawn this way selects what it touches (crossing). Preferences >
/// Drawing > Lasso Direction: Clockwise (the default, help) makes clockwise lassos
/// crossing and counterclockwise ones window; Counterclockwise swaps them.
pub fn lasso_crossing(clockwise: bool, clockwise_is_crossing: bool) -> bool {
    clockwise == clockwise_is_crossing
}

/// Entities selected by a lasso: with `clockwise_is_crossing`, clockwise (on screen)
/// selects what the outline touches, counterclockwise only what lies fully inside it;
/// the other way round without.
pub fn lasso_select(
    scene: &Scene,
    camera: &Camera,
    viewport: DVec2,
    lasso: &Polygon,
    clockwise_is_crossing: bool,
) -> Vec<Owner> {
    if lasso.points.len() < 3 {
        return Vec::new();
    }
    let crossing = lasso_crossing(lasso.is_clockwise(), clockwise_is_crossing);
    region_select(scene, camera, viewport, lasso, !crossing)
}

/// Entities in a screen region: `window` needs them fully inside, otherwise touching
/// is enough. Geometry behind the camera never counts as inside.
pub fn region_select(
    scene: &Scene,
    camera: &Camera,
    viewport: DVec2,
    rect: &impl Region,
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
    for (owner, p) in crate::annot::label_points(scene, proj, viewport) {
        let inside = p.is_some_and(|p| rect.contains(p));
        note(owner, inside, inside);
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

    /// FEATURES.md 76: the Lasso's Shift / Alt / Shift+Alt.
    #[test]
    fn lasso_modifiers() {
        assert_eq!(Mode::for_lasso(false, false, false), Mode::Replace);
        assert_eq!(Mode::for_lasso(false, true, false), Mode::Toggle);
        assert_eq!(Mode::for_lasso(false, false, true), Mode::Add);
        assert_eq!(Mode::for_lasso(false, true, true), Mode::Remove);
        // Select's Ctrl keys still work.
        assert_eq!(Mode::for_lasso(true, false, false), Mode::Add);
        assert_eq!(Mode::for_lasso(true, true, false), Mode::Remove);
    }

    /// `pick` before the edge culling index: every edge projected and tested.
    fn pick_scanning(
        scene: &Scene,
        bvh: &dyn RayIndex,
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
            let t = closest_ray_param(ray.origin, ray.dir, a, b);
            if t <= face_t * (1.0 + 1e-4) + 1e-3 {
                best = Some((d, *owner));
            }
        }
        best.map(|(_, o)| o)
            .or_else(|| face_hit.and_then(|h| scene.tri_owner[h.triangle]))
    }

    /// A stale key (an erased edge) and the live one reusing its slot are different
    /// owners, as in a hash set.
    #[test]
    fn owner_set_tells_key_versions_apart() {
        let mut ents = Entities::default();
        let old = add_line(&mut ents, DVec3::ZERO, DVec3::X)[0];
        mt_kernel::erase_edge(&mut ents, old);
        let new = add_line(&mut ents, DVec3::ZERO, DVec3::Y)[0];
        assert_ne!(old, new);
        let mut set = OwnerSet::default();
        assert!(set.insert(Owner::Edge(old)));
        assert!(!set.contains(&Owner::Edge(new)));
        assert!(set.insert(Owner::Edge(new)));
        assert!(!set.insert(Owner::Edge(new)));
        assert!(set.contains(&Owner::Edge(old)) && set.contains(&Owner::Edge(new)));
        assert!(set.remove(&Owner::Edge(old)));
        assert!(!set.contains(&Owner::Edge(old)) && set.contains(&Owner::Edge(new)));
        assert!(set.insert(Owner::Instance(usize::MAX)));
        assert!(set.contains(&Owner::Instance(usize::MAX)));
        assert!(!set.contains(&Owner::Instance(7)));
        set.clear();
        assert!(!set.contains(&Owner::Edge(new)));
    }

    /// With many edges `pick` culls them with an index and still picks what testing
    /// every edge picks, in perspective and parallel views.
    #[test]
    fn culled_pick_matches_a_full_scan() {
        let mut scene = Scene::default();
        let f = |p: DVec3| p.as_vec3().to_array();
        for k in 0..=100 {
            let t = k as f64 * 1.2;
            let z = (k as f64 * 0.37).sin() * 4.0;
            for (a, b) in [
                (DVec3::new(t, 0.0, z), DVec3::new(t, 120.0, z)),
                (DVec3::new(0.0, t, -z), DVec3::new(120.0, t, z)),
            ] {
                scene
                    .edges
                    .push(mt_render::Segment::line(f(a), f(b), [0.0; 4], [1.0, 0.0]));
                scene
                    .edge_owner
                    .push(Some(Owner::Section(scene.edge_owner.len())));
            }
            for j in 0..10 {
                let a = DVec3::new(t, j as f64 * 12.0, z);
                let b = a + DVec3::new(0.9, 1.7, 2.0);
                scene
                    .edges
                    .push(mt_render::Segment::line(f(a), f(b), [0.0; 4], [1.0, 0.0]));
                scene
                    .edge_owner
                    .push((j % 3 != 0).then_some(Owner::Instance(scene.edge_owner.len())));
            }
        }
        assert!(scene.edges.len() >= 1024);
        let c = [
            DVec3::new(0.0, 0.0, 1.0),
            DVec3::new(120.0, 0.0, 1.0),
            DVec3::new(120.0, 120.0, 1.0),
            DVec3::new(0.0, 120.0, 1.0),
        ];
        scene.triangles = vec![[c[0], c[1], c[2]], [c[0], c[2], c[3]]];
        scene.tri_owner = vec![Some(Owner::Text(0)), Some(Owner::Text(1))];
        let bvh = mt_kernel::bvh::Bvh::build(scene.triangles.clone());
        let viewport = DVec2::new(900.0, 700.0);
        for perspective in [true, false] {
            let camera = Camera {
                eye: DVec3::new(-150.0, -200.0, 180.0),
                target: DVec3::new(60.0, 60.0, 0.0),
                fov_deg: 35.0,
                perspective,
                two_point: false,
            };
            let mut hits = 0;
            for k in 0..900 {
                let cursor = DVec2::new((k % 30) as f64 * 30.0 + 1.5, (k / 30) as f64 * 23.0 + 2.5);
                let fast = pick(&scene, &bvh, &camera, cursor, viewport);
                let slow = pick_scanning(&scene, &bvh, &camera, cursor, viewport);
                assert_eq!(fast, slow, "cursor {cursor}");
                hits += usize::from(matches!(fast, Some(Owner::Section(_) | Owner::Instance(_))));
            }
            assert!(hits > 50, "edges are picked ({hits})");
        }
    }

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

    /// A scene of the entities' faces and edges, every piece owned by itself.
    fn scene_of(e: &Entities) -> Scene {
        let mut scene = Scene::default();
        let f = |p: DVec3| p.as_vec3().to_array();
        for (id, edge) in &e.edges {
            scene.edges.push(mt_render::Segment::line(
                f(e.pos(edge.start)),
                f(e.pos(edge.end)),
                [0.0, 0.0, 0.0, 1.0],
                [1.0, 0.0],
            ));
            scene.edge_owner.push(Some(Owner::Edge(id)));
        }
        for fid in e.faces.keys() {
            for tri in mt_kernel::tessellate::triangulate(e, fid) {
                scene.triangles.push(tri);
                scene.tri_owner.push(Some(Owner::Face(fid)));
            }
        }
        scene
    }

    #[test]
    fn polygon_orientation_and_containment() {
        // Screen pixels, y down: right, then down, then left is clockwise on screen.
        let cw = Polygon::new(vec![
            DVec2::new(0.0, 0.0),
            DVec2::new(10.0, 0.0),
            DVec2::new(10.0, 10.0),
            DVec2::new(0.0, 10.0),
        ]);
        assert!(cw.is_clockwise());
        let mut ccw = cw.clone();
        ccw.points.reverse();
        assert!(!ccw.is_clockwise());
        assert!(cw.contains(DVec2::new(5.0, 5.0)) && ccw.contains(DVec2::new(5.0, 5.0)));
        assert!(!cw.contains(DVec2::new(15.0, 5.0)));
        assert!(cw.touches_segment(DVec2::new(5.0, -5.0), DVec2::new(5.0, 15.0)));
        assert!(!cw.touches_segment(DVec2::new(11.0, -5.0), DVec2::new(11.0, 15.0)));
        // A triangle around the whole lasso touches it.
        let big = [
            DVec2::new(-100.0, -100.0),
            DVec2::new(100.0, -100.0),
            DVec2::new(0.0, 100.0),
        ];
        assert!(cw.touches_triangle(big));
    }

    #[test]
    fn lasso_clockwise_touches_counterclockwise_inside() {
        // Square A at x 0..1 and square B at x 5..6 on the ground.
        let mut e = Entities::new();
        square(&mut e, 0.0);
        square(&mut e, 5.0);
        let scene = scene_of(&e);
        let camera = Camera {
            eye: DVec3::new(3.0, -40.0, 60.0),
            target: DVec3::new(3.0, 0.5, 0.0),
            fov_deg: 35.0,
            perspective: true,
            two_point: false,
        };
        let viewport = DVec2::new(800.0, 600.0);
        // A lasso around all of A and the left half of B.
        let outline: Vec<DVec2> = [(-0.5, -0.5), (5.5, -0.5), (5.5, 1.5), (-0.5, 1.5)]
            .iter()
            .map(|&(x, y)| camera.project(DVec3::new(x, y, 0.0), viewport).unwrap())
            .collect();
        let mut cw = Polygon::new(outline);
        if !cw.is_clockwise() {
            cw.points.reverse();
        }
        let mut ccw = cw.clone();
        ccw.points.reverse();
        let in_a = |o: &Owner| match *o {
            Owner::Edge(id) => {
                let ed = &e.edges[id];
                e.pos(ed.start).x < 2.0 && e.pos(ed.end).x < 2.0
            }
            Owner::Face(f) => e.loop_points(&e.faces[f].outer).iter().all(|p| p.x < 2.0),
            _ => false,
        };
        let inside = lasso_select(&scene, &camera, viewport, &ccw, true);
        // A's face and four edges, and B's left edge (x = 5) which lies inside too.
        assert_eq!(inside.len(), 6, "{inside:?}");
        assert_eq!(inside.iter().filter(|o| !in_a(o)).count(), 1);
        assert!(
            !inside
                .iter()
                .any(|o| matches!(o, Owner::Face(_)) && !in_a(o))
        );
        let touched = lasso_select(&scene, &camera, viewport, &cw, true);
        // B's face, its two edges across the outline and its left edge are touched
        // too; its right edge is not.
        assert_eq!(touched.len(), 5 + 4, "{touched:?}");
        assert_eq!(touched.iter().filter(|o| !in_a(o)).count(), 4);
        // Lasso Direction > Counterclockwise swaps them.
        assert_eq!(lasso_select(&scene, &camera, viewport, &cw, false), inside);
        assert_eq!(
            lasso_select(&scene, &camera, viewport, &ccw, false),
            touched
        );
        assert!(lasso_crossing(true, true) && !lasso_crossing(false, true));
        assert!(lasso_crossing(false, false) && !lasso_crossing(true, false));
        // Too few points select nothing.
        assert!(lasso_select(&scene, &camera, viewport, &Polygon::new(vec![]), true).is_empty());
    }

    #[test]
    fn aligned_box_follows_the_instance_axes() {
        // A 10 x 2 x 4 definition box, rotated 90 degrees about blue and moved.
        let b = OrientedBox::from_aabb(DVec3::ZERO, DVec3::new(10.0, 2.0, 4.0));
        let xf = DMat4::from_translation(DVec3::new(100.0, 0.0, 0.0))
            * DMat4::from_rotation_z(std::f64::consts::FRAC_PI_2)
            * DMat4::from_scale(DVec3::new(2.0, 1.0, 1.0));
        let t = b.transformed(xf);
        assert!((t.half - DVec3::new(10.0, 1.0, 2.0)).length() < 1e-9);
        assert!(
            (t.dirs[0] - DVec3::Y).length() < 1e-9,
            "red axis now points along green"
        );
        let mut corners = t.corners().to_vec();
        corners.sort_by(|a, b| a.to_array().partial_cmp(&b.to_array()).unwrap());
        let near = |p: DVec3| corners.iter().any(|c| c.distance(p) < 1e-9);
        assert!(near(DVec3::new(100.0, 0.0, 0.0)));
        assert!(near(DVec3::new(98.0, 20.0, 4.0)));
        assert!(
            !near(DVec3::new(120.0, 2.0, 0.0)),
            "not the world-aligned box"
        );
        for [a, c] in t.edges() {
            assert!(
                (a.distance(c) - 20.0).abs() < 1e-9
                    || (a.distance(c) - 2.0).abs() < 1e-9
                    || (a.distance(c) - 4.0).abs() < 1e-9
            );
        }
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
        let bvh = mt_kernel::bvh::Bvh::build(scene.triangles.clone());
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
