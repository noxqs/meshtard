//! Entity storage and low-level topology edits. Higher-level SketchUp behavior
//! (sticky geometry, auto-face, healing) lives in `ops`.

use glam::{DMat4, DVec2, DVec3};
use slotmap::new_key_type;

use crate::TOL;
use crate::geom::Plane;
use crate::slot_store::{SlotStore, Snapshot};

new_key_type! {
    pub struct VertexId;
    pub struct EdgeId;
    pub struct FaceId;
    pub struct DefinitionId;
    pub struct CurveId;
}

/// Index into `Model::materials`.
pub type MaterialId = usize;
/// Index into `Model::layers`.
pub type LayerId = usize;

#[derive(Debug, Clone, PartialEq)]
pub struct Vertex {
    pub pos: DVec3,
    pub edges: Vec<EdgeId>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EdgeAttrs {
    pub soft: bool,
    pub smooth: bool,
    pub hidden: bool,
    pub layer: Option<LayerId>,
    /// The curve (arc, circle, polyline) this edge is a segment of.
    pub curve: Option<CurveId>,
}

/// An arc's geometry: points are `center + radius * (cos(a) * x_axis + sin(a) * y_axis)`
/// for angles from `start_angle` to `end_angle` (radians).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arc {
    pub center: DVec3,
    pub normal: DVec3,
    pub x_axis: DVec3,
    pub radius: f64,
    pub start_angle: f64,
    pub end_angle: f64,
    pub segments: usize,
}

impl Arc {
    pub fn y_axis(&self) -> DVec3 {
        self.normal.cross(self.x_axis)
    }

    pub fn is_circle(&self) -> bool {
        (self.end_angle - self.start_angle).abs() >= std::f64::consts::TAU - 1e-9
    }

    /// The `segments + 1` points along the arc (a circle repeats its first point).
    pub fn points(&self) -> Vec<DVec3> {
        let y = self.y_axis();
        (0..=self.segments)
            .map(|i| {
                let a = self.start_angle
                    + (self.end_angle - self.start_angle) * i as f64 / self.segments as f64;
                self.center + (self.x_axis * a.cos() + y * a.sin()) * self.radius
            })
            .collect()
    }
}

/// Edges that select and erase as one: SketchUp's curves and arc curves. Member edges
/// point to their curve through `EdgeAttrs::curve`.
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    /// `None` for a free-form curve (Freehand).
    pub arc: Option<Arc>,
    /// Context menu > Convert to Polygon: the curve shows and reports as a polygon
    /// (Entity Info, inference on its segments); the arc data stays for its center.
    pub polygon: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub start: VertexId,
    pub end: VertexId,
    /// Every face with this edge in one of its loops (any number: non-manifold).
    pub faces: Vec<FaceId>,
    pub attrs: EdgeAttrs,
}

impl Edge {
    pub fn other(&self, v: VertexId) -> VertexId {
        if self.start == v {
            self.end
        } else {
            self.start
        }
    }
}

/// An edge used by a loop, in the direction the loop runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CoEdge {
    pub edge: EdgeId,
    /// Runs end to start.
    pub reversed: bool,
}

/// Closed chain of coedges. The outer loop of a face runs counter-clockwise around
/// the face normal, holes clockwise.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Loop {
    pub coedges: Vec<CoEdge>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FaceAttrs {
    pub material: Option<MaterialId>,
    pub back_material: Option<MaterialId>,
    pub hidden: bool,
    pub layer: Option<LayerId>,
    /// Positioned texture mappings; `None` (the common case) when both sides use the
    /// default projection.
    pub uv: Option<Box<crate::texture::FaceUv>>,
    /// Entity Info > Cast Shadows / Receive Shadows (both on by default). Stored only:
    /// the renderer does not use them yet, and the .skp writer does not save them.
    pub shadows: ShadowFlags,
}

/// SketchUp's per-entity "Cast Shadows" and "Receive Shadows" toggles (faces, groups
/// and components).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowFlags {
    pub cast: bool,
    pub receive: bool,
}

impl Default for ShadowFlags {
    fn default() -> Self {
        Self {
            cast: true,
            receive: true,
        }
    }
}

/// Per-instance data shown in Entity Info that most code never touches: shadow
/// toggles, the instance half of the Advanced Attributes and attribute dictionaries.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InstanceAttrs {
    pub shadows: ShadowFlags,
    /// Advanced Attributes "Status" and "Owner" (stored on the instance; which of the
    /// six attributes SketchUp keeps on the instance rather than the definition is
    /// from memory, unverified).
    pub status: String,
    pub owner: String,
    /// Placed glued to a face (a component whose definition glues); context menu
    /// Unglue clears it. Not saved in .skp.
    pub glued: bool,
    /// SketchUp's attribute dictionaries on the instance (name -> key -> typed value):
    /// read from and written to .skp, set by extensions (`set_attribute`). Copies of
    /// the instance get them too.
    pub dictionaries: mt_skp::AttrDicts,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Face {
    pub outer: Loop,
    pub holes: Vec<Loop>,
    /// Unit front normal.
    pub normal: DVec3,
    pub attrs: FaceAttrs,
}

impl Face {
    pub fn loops(&self) -> impl Iterator<Item = &Loop> {
        std::iter::once(&self.outer).chain(&self.holes)
    }

    pub fn loops_mut(&mut self) -> impl Iterator<Item = &mut Loop> {
        std::iter::once(&mut self.outer).chain(&mut self.holes)
    }
}

/// A placed component or group.
#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    pub definition: DefinitionId,
    /// Definition space to parent space.
    pub transform: DMat4,
    pub name: String,
    pub hidden: bool,
    /// Edit > Lock: can be selected but not moved, erased, exploded or opened.
    pub locked: bool,
    pub layer: Option<LayerId>,
    /// Paints faces of the definition that have no material of their own.
    pub material: Option<MaterialId>,
    /// Shadow toggles and Advanced Attributes (Entity Info).
    pub attrs: InstanceAttrs,
}

/// A collection of geometry: the model root or a component definition's contents.
#[derive(Debug, Default)]
pub struct Entities {
    /// Vertices, edges and faces are slot maps that know what changed since their last
    /// snapshot (`slot_store`; see `Entities::freeze`).
    pub vertices: SlotStore<VertexId, Vertex>,
    pub edges: SlotStore<EdgeId, Edge>,
    pub faces: SlotStore<FaceId, Face>,
    pub instances: Vec<Instance>,
    pub curves: SlotStore<CurveId, Curve>,
    /// Construction guides (Tape Measure, Protractor). Not geometry: they never split
    /// edges or bound faces.
    pub guides: Vec<Guide>,
    /// Section planes placed in this context (Section Plane tool).
    pub section_planes: Vec<SectionPlane>,
    /// The section plane cutting this context, an index into `section_planes`.
    pub active_section: Option<usize>,
    /// Dimensions (Dimension tool).
    pub dimensions: Vec<crate::annotation::Dimension>,
    /// Leader and screen texts (Text tool).
    pub texts: Vec<crate::annotation::Text>,
    /// Component Snaps of a group's or component's definition (`crate::snaps`; not
    /// saved in .skp).
    pub snaps: Vec<crate::snaps::Snap>,
    /// Lookup acceleration, not model data: not cloned, see `spatial`.
    pub(crate) index: crate::spatial::SpatialIndex,
}

/// Above this many vertices, edges and faces together, cloning and comparing an
/// `Entities` splits the work over threads (each map on its own).
const PARALLEL_MIN: usize = 50_000;

impl Entities {
    /// Vertices, edges, faces and instances together: a measure of the work of copying
    /// or comparing the collection.
    pub fn size(&self) -> usize {
        self.vertices.len() + self.edges.len() + self.faces.len() + self.instances.len()
    }

    fn is_large(&self) -> bool {
        self.vertices.len() + self.edges.len() + self.faces.len() >= PARALLEL_MIN
    }
}

/// The state of an [`Entities`] at [`Entities::freeze`], for undo: its vertices, edges
/// and faces as [`Snapshot`]s (sharing the chunks an edit did not touch with the
/// snapshots before) and a copy of the rest.
pub struct EntitiesSnapshot {
    vertices: Snapshot<Vertex>,
    edges: Snapshot<Edge>,
    faces: Snapshot<Face>,
    /// Everything else (its slot maps empty).
    rest: Entities,
}

impl EntitiesSnapshot {
    /// [`Entities::size`] of the state held.
    pub fn size(&self) -> usize {
        self.vertices.len() + self.edges.len() + self.faces.len() + self.rest.instances.len()
    }

    /// The state held, as a new collection (a full copy).
    pub fn to_entities(&self) -> Entities {
        let mut e = Entities::default();
        e.thaw(self);
        e
    }

    /// The vertices', edges' and faces' snapshots.
    pub fn maps(&self) -> (&Snapshot<Vertex>, &Snapshot<Edge>, &Snapshot<Face>) {
        (&self.vertices, &self.edges, &self.faces)
    }
}

impl Entities {
    /// The current state for an undo step. Only the chunks of vertices, edges and faces
    /// changed since the last `freeze` or `thaw` are copied
    /// (`slot_store::SlotStore::freeze`), and the rest (instances, curves, guides,
    /// annotations). When the state is still `prev`'s, that is returned instead.
    pub fn freeze(
        &mut self,
        prev: Option<&std::sync::Arc<EntitiesSnapshot>>,
    ) -> std::sync::Arc<EntitiesSnapshot> {
        self.freeze_with(prev, false)
    }

    /// Makes this collection hold `s`'s state, rewriting only the vertex, edge and face
    /// chunks that differ (`slot_store::SlotStore::thaw`). The spatial index is
    /// dropped when anything changed. Returns whether anything did.
    pub fn thaw(&mut self, s: &EntitiesSnapshot) -> bool {
        let maps = self.thaw_maps(s);
        let rest = !same_rest(&s.rest, self);
        if rest {
            self.set_rest(s.rest.rest_copy());
        }
        self.changed_by_thaw(maps || rest)
    }

    /// [`Entities::freeze`] then [`Entities::thaw`] (undo and redo): returns the state
    /// replaced. The instances and annotations, when they differ, are moved into the
    /// returned snapshot rather than copied.
    pub fn swap_to(
        &mut self,
        s: &EntitiesSnapshot,
        prev: Option<&std::sync::Arc<EntitiesSnapshot>>,
    ) -> std::sync::Arc<EntitiesSnapshot> {
        let rest = !same_rest(&s.rest, self);
        // When the rest is to be replaced, moving it out is cheaper than checking it
        // against `prev`'s.
        let before = self.freeze_with(prev.filter(|_| !rest), rest);
        let maps = self.thaw_maps(s);
        if rest {
            self.set_rest(s.rest.rest_copy());
        }
        self.changed_by_thaw(maps || rest);
        before
    }

    /// See [`Entities::freeze`]; `take_rest` moves the rest into a new snapshot (and
    /// leaves it empty here) instead of copying it.
    fn freeze_with(
        &mut self,
        prev: Option<&std::sync::Arc<EntitiesSnapshot>>,
        take_rest: bool,
    ) -> std::sync::Arc<EntitiesSnapshot> {
        let (vertices, edges, faces) = if self.is_large() {
            let (v, e, f) = (&mut self.vertices, &mut self.edges, &mut self.faces);
            std::thread::scope(|s| {
                let e = s.spawn(|| e.freeze());
                let f = s.spawn(|| f.freeze());
                let v = v.freeze();
                (v, e.join().unwrap(), f.join().unwrap())
            })
        } else {
            (
                self.vertices.freeze(),
                self.edges.freeze(),
                self.faces.freeze(),
            )
        };
        if let Some(p) = prev
            && p.vertices.ptr_eq(&vertices)
            && p.edges.ptr_eq(&edges)
            && p.faces.ptr_eq(&faces)
            && same_rest(&p.rest, self)
        {
            return p.clone();
        }
        let rest = if take_rest {
            self.take_rest()
        } else {
            self.rest_copy()
        };
        std::sync::Arc::new(EntitiesSnapshot {
            vertices,
            edges,
            faces,
            rest,
        })
    }

    /// The vertex, edge and face part of [`Entities::thaw`].
    fn thaw_maps(&mut self, s: &EntitiesSnapshot) -> bool {
        if self.is_large() || s.size() >= PARALLEL_MIN {
            let (v, e, f) = (&mut self.vertices, &mut self.edges, &mut self.faces);
            std::thread::scope(|sc| {
                let e = sc.spawn(|| e.thaw(&s.edges));
                let f = sc.spawn(|| f.thaw(&s.faces));
                let v = v.thaw(&s.vertices);
                v | e.join().unwrap() | f.join().unwrap()
            })
        } else {
            self.vertices.thaw(&s.vertices) | self.edges.thaw(&s.edges) | self.faces.thaw(&s.faces)
        }
    }

    /// Drops the spatial index when a thaw changed anything; returns `changed`.
    fn changed_by_thaw(&mut self, changed: bool) -> bool {
        if changed {
            // Freeing an index takes a while (a million edges: ~0.2 s, ten thousand
            // instances: ~4 ms); nothing waits for it.
            let old = std::mem::take(&mut self.index);
            if old.grid.is_some() {
                std::thread::spawn(move || drop(old));
            }
        }
        changed
    }

    /// Replaces everything but the vertices, edges, faces and the spatial index with
    /// `r`'s.
    fn set_rest(&mut self, r: Entities) {
        let Self {
            instances,
            curves,
            guides,
            section_planes,
            active_section,
            dimensions,
            texts,
            snaps,
            ..
        } = r;
        self.instances = instances;
        self.curves = curves;
        self.guides = guides;
        self.section_planes = section_planes;
        self.active_section = active_section;
        self.dimensions = dimensions;
        self.texts = texts;
        self.snaps = snaps;
    }

    /// Moves out everything but the vertices, edges, faces and the spatial index.
    fn take_rest(&mut self) -> Entities {
        Entities {
            instances: std::mem::take(&mut self.instances),
            curves: std::mem::take(&mut self.curves),
            guides: std::mem::take(&mut self.guides),
            section_planes: std::mem::take(&mut self.section_planes),
            active_section: self.active_section.take(),
            dimensions: std::mem::take(&mut self.dimensions),
            texts: std::mem::take(&mut self.texts),
            snaps: std::mem::take(&mut self.snaps),
            ..Default::default()
        }
    }

    /// A copy of everything but the vertices, edges, faces and the spatial index.
    fn rest_copy(&self) -> Entities {
        Entities {
            instances: self.instances.clone(),
            curves: self.curves.clone(),
            guides: self.guides.clone(),
            section_planes: self.section_planes.clone(),
            active_section: self.active_section,
            dimensions: self.dimensions.clone(),
            texts: self.texts.clone(),
            snaps: self.snaps.clone(),
            ..Default::default()
        }
    }
}

/// Whether the two hold the same instances, curves, guides and annotations (curves'
/// free slots included: they decide the keys of curves added later).
fn same_rest(a: &Entities, b: &Entities) -> bool {
    let Entities {
        vertices: _,
        edges: _,
        faces: _,
        instances,
        curves,
        guides,
        section_planes,
        active_section,
        dimensions,
        texts,
        snaps,
        index: _,
    } = a;
    *instances == b.instances
        && curves.identical(&b.curves)
        && *guides == b.guides
        && *section_planes == b.section_planes
        && *active_section == b.active_section
        && *dimensions == b.dimensions
        && *texts == b.texts
        && *snaps == b.snaps
}

/// A copy without the spatial index (see `spatial`). Large collections (an undo step of
/// a big model) copy their vertices, edges and faces on separate threads.
impl Clone for Entities {
    fn clone(&self) -> Self {
        let Self {
            vertices,
            edges,
            faces,
            instances,
            curves,
            guides,
            section_planes,
            active_section,
            dimensions,
            texts,
            snaps,
            index,
        } = self;
        let (vertices, edges, faces) = if self.is_large() {
            std::thread::scope(|s| {
                let e = s.spawn(|| edges.clone());
                let f = s.spawn(|| faces.clone());
                let v = vertices.clone();
                (v, e.join().unwrap(), f.join().unwrap())
            })
        } else {
            (vertices.clone(), edges.clone(), faces.clone())
        };
        Self {
            vertices,
            edges,
            faces,
            instances: instances.clone(),
            curves: curves.clone(),
            guides: guides.clone(),
            section_planes: section_planes.clone(),
            active_section: *active_section,
            dimensions: dimensions.clone(),
            texts: texts.clone(),
            snaps: snaps.clone(),
            index: index.clone(),
        }
    }
}

/// Same entities under the same keys (the spatial index is not compared, and neither
/// are the maps' free slots, which only decide the keys of entities added later).
/// Floating-point fields compare as numbers: NaN never equals itself.
impl PartialEq for Entities {
    fn eq(&self, other: &Self) -> bool {
        let Self {
            vertices,
            edges,
            faces,
            instances,
            curves,
            guides,
            section_planes,
            active_section,
            dimensions,
            texts,
            snaps,
            index: _,
        } = self;
        let small = *instances == other.instances
            && curves.same_entries(&other.curves)
            && *guides == other.guides
            && *section_planes == other.section_planes
            && *active_section == other.active_section
            && *dimensions == other.dimensions
            && *texts == other.texts
            && *snaps == other.snaps
            && vertices.len() == other.vertices.len()
            && edges.len() == other.edges.len()
            && faces.len() == other.faces.len();
        if !small {
            return false;
        }
        if self.is_large() {
            std::thread::scope(|s| {
                let e = s.spawn(|| edges.same_entries(&other.edges));
                let f = s.spawn(|| faces.same_entries(&other.faces));
                let v = vertices.same_entries(&other.vertices);
                v & e.join().unwrap() & f.join().unwrap()
            })
        } else {
            vertices.same_entries(&other.vertices)
                && edges.same_entries(&other.edges)
                && faces.same_entries(&other.faces)
        }
    }
}

/// A section plane: cuts away the geometry on the side `normal` points to (which side
/// SketchUp removes is unverified; here the arrows point at the removed side).
#[derive(Debug, Clone, PartialEq)]
pub struct SectionPlane {
    pub point: DVec3,
    /// Unit normal.
    pub normal: DVec3,
    pub name: String,
    /// Entity Info > Symbol: the short label drawn in the section plane's arrows.
    pub symbol: String,
    pub hidden: bool,
}

/// A construction line (infinite, through `point` along `dir`) or construction point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guide {
    pub point: DVec3,
    /// Unit direction for a guide line, `None` for a guide point.
    pub dir: Option<DVec3>,
}

impl Entities {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pos(&self, v: VertexId) -> DVec3 {
        self.vertices[v].pos
    }

    /// Start and end vertex of a coedge in loop direction.
    pub fn coedge_ends(&self, c: CoEdge) -> (VertexId, VertexId) {
        let e = &self.edges[c.edge];
        if c.reversed {
            (e.end, e.start)
        } else {
            (e.start, e.end)
        }
    }

    /// Vertices of a loop in order (each coedge's start).
    pub fn loop_vertices(&self, l: &Loop) -> Vec<VertexId> {
        l.coedges.iter().map(|c| self.coedge_ends(*c).0).collect()
    }

    pub fn loop_points(&self, l: &Loop) -> Vec<DVec3> {
        self.loop_vertices(l)
            .into_iter()
            .map(|v| self.pos(v))
            .collect()
    }

    /// Plane of a face, oriented along its normal.
    pub fn face_plane(&self, f: FaceId) -> Plane {
        let face = &self.faces[f];
        let p = self.pos(self.coedge_ends(face.outer.coedges[0]).0);
        Plane::new(face.normal, p)
    }

    /// Loops of a face projected into `plane` coordinates.
    pub fn face_polygons(&self, f: FaceId, plane: &Plane) -> (Vec<DVec2>, Vec<Vec<DVec2>>) {
        let face = &self.faces[f];
        let proj = |l: &Loop| {
            self.loop_points(l)
                .into_iter()
                .map(|p| plane.project(p))
                .collect()
        };
        (proj(&face.outer), face.holes.iter().map(proj).collect())
    }

    /// Whether a point in the face's plane lies strictly inside the face region.
    pub fn face_contains(&self, f: FaceId, p: DVec3) -> bool {
        let plane = self.face_plane(f);
        if !plane.contains(p) {
            return false;
        }
        let (outer, holes) = self.face_polygons(f, &plane);
        let q = plane.project(p);
        crate::geom::point_in_polygon(q, &outer)
            && !holes.iter().any(|h| crate::geom::point_in_polygon(q, h))
    }

    /// Edges of a curve.
    pub fn curve_edges(&self, c: CurveId) -> Vec<EdgeId> {
        self.edges
            .iter()
            .filter(|(_, e)| e.attrs.curve == Some(c))
            .map(|(id, _)| id)
            .collect()
    }

    /// The first vertex (in key order) within TOL of `p`.
    pub fn find_vertex(&self, p: DVec3) -> Option<VertexId> {
        if self.grid().is_none() {
            return self
                .vertices
                .iter()
                .find(|(_, v)| v.pos.distance(p) <= TOL)
                .map(|(id, _)| id);
        }
        self.vertices_near(p, TOL)
            .into_iter()
            .find(|&v| self.vertices[v].pos.distance(p) <= TOL)
    }

    pub fn edge_between(&self, a: VertexId, b: VertexId) -> Option<EdgeId> {
        self.vertices[a]
            .edges
            .iter()
            .copied()
            .find(|&e| self.edges[e].other(a) == b)
    }

    pub fn add_vertex(&mut self, pos: DVec3) -> VertexId {
        let v = self.vertices.insert(Vertex {
            pos,
            edges: Vec::new(),
        });
        if let Some(g) = &mut self.index.grid {
            g.insert_vertex(v, pos);
        }
        v
    }

    /// Adds an edge with no checks for duplicates or intersections.
    pub fn add_raw_edge(&mut self, start: VertexId, end: VertexId, attrs: EdgeAttrs) -> EdgeId {
        let id = self.edges.insert(Edge {
            start,
            end,
            faces: Vec::new(),
            attrs,
        });
        self.vertices[start].edges.push(id);
        self.vertices[end].edges.push(id);
        if let Some(g) = &mut self.index.grid {
            g.insert_edge(id, self.vertices[start].pos, self.vertices[end].pos);
        }
        id
    }

    /// Adds a face from loops, registering it with its edges. No validity checks.
    pub fn add_raw_face(
        &mut self,
        outer: Loop,
        holes: Vec<Loop>,
        normal: DVec3,
        attrs: FaceAttrs,
    ) -> FaceId {
        let id = self.faces.insert(Face {
            outer,
            holes,
            normal,
            attrs,
        });
        let edges: Vec<EdgeId> = self.faces[id]
            .loops()
            .flat_map(|l| l.coedges.iter().map(|c| c.edge))
            .collect();
        for e in edges {
            let faces = &mut self.edges[e].faces;
            if !faces.contains(&id) {
                faces.push(id);
            }
        }
        if self.index.grid.is_some() {
            let b = self.face_bounds(id);
            if let Some(g) = &mut self.index.grid {
                g.insert_face(id, b);
            }
        }
        id
    }

    pub fn remove_face(&mut self, f: FaceId) -> Option<Face> {
        let face = self.faces.remove(f)?;
        if let Some(g) = &mut self.index.grid {
            g.remove_face(f);
        }
        for l in face.loops() {
            for c in &l.coedges {
                if let Some(e) = self.edges.get_mut(c.edge) {
                    e.faces.retain(|&x| x != f);
                }
            }
        }
        Some(face)
    }

    /// Removes an edge and every face using it. Vertices are kept.
    pub fn remove_edge(&mut self, e: EdgeId) -> Vec<Face> {
        let faces = self.edges[e].faces.clone();
        let removed = faces
            .into_iter()
            .filter_map(|f| self.remove_face(f))
            .collect();
        self.unlink_edge(e);
        removed
    }

    /// Removes an edge from the slot map and its vertices' lists, leaving faces alone
    /// (callers make sure no face uses it any more).
    pub fn unlink_edge(&mut self, e: EdgeId) {
        let Some(edge) = self.edges.remove(e) else {
            return;
        };
        for v in [edge.start, edge.end] {
            if let Some(x) = self.vertices.get_mut(v) {
                x.edges.retain(|&y| y != e);
            }
        }
        if let Some(g) = &mut self.index.grid {
            g.remove_edge(e);
        }
    }

    pub fn remove_vertex_if_unused(&mut self, v: VertexId) {
        if self.vertices.get(v).is_some_and(|x| x.edges.is_empty()) {
            self.vertices.remove(v);
            if let Some(g) = &mut self.index.grid {
                g.remove_vertex(v);
            }
        }
    }

    /// Splits edge `e` at a new or existing vertex `w` lying on it. Faces using `e`
    /// get both halves in their loops. Returns `(start-w, w-end)`.
    pub fn split_edge(&mut self, e: EdgeId, w: VertexId) -> (EdgeId, EdgeId) {
        let edge = self.edges[e].clone();
        let a = self.add_raw_edge(edge.start, w, edge.attrs.clone());
        let b = self.add_raw_edge(w, edge.end, edge.attrs.clone());
        for &f in &edge.faces {
            for l in self.faces[f].loops_mut() {
                let mut out = Vec::with_capacity(l.coedges.len() + 1);
                for c in &l.coedges {
                    if c.edge != e {
                        out.push(*c);
                    } else if c.reversed {
                        out.push(CoEdge {
                            edge: b,
                            reversed: true,
                        });
                        out.push(CoEdge {
                            edge: a,
                            reversed: true,
                        });
                    } else {
                        out.push(CoEdge {
                            edge: a,
                            reversed: false,
                        });
                        out.push(CoEdge {
                            edge: b,
                            reversed: false,
                        });
                    }
                }
                l.coedges = out;
            }
            self.edges[a].faces.push(f);
            self.edges[b].faces.push(f);
        }
        self.edges[e].faces.clear();
        self.unlink_edge(e);
        // `w` may be off the edge by up to the tolerance: the faces' boxes can change.
        for &f in &edge.faces {
            self.refresh_face_bounds(f);
        }
        (a, b)
    }
}
