//! Entity storage and low-level topology edits. Higher-level SketchUp behavior
//! (sticky geometry, auto-face, healing) lives in `ops`.

use glam::{DMat4, DVec2, DVec3};
use slotmap::{SlotMap, new_key_type};

use crate::TOL;
use crate::geom::Plane;

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

#[derive(Debug, Clone)]
pub struct Vertex {
    pub pos: DVec3,
    pub edges: Vec<EdgeId>,
}

#[derive(Debug, Clone, Default)]
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
}

#[derive(Debug, Clone)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
}

#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
pub struct Instance {
    pub definition: DefinitionId,
    /// Definition space to parent space.
    pub transform: DMat4,
    pub name: String,
    pub hidden: bool,
    pub layer: Option<LayerId>,
    /// Paints faces of the definition that have no material of their own.
    pub material: Option<MaterialId>,
}

/// A collection of geometry: the model root or a component definition's contents.
#[derive(Debug, Clone, Default)]
pub struct Entities {
    pub vertices: SlotMap<VertexId, Vertex>,
    pub edges: SlotMap<EdgeId, Edge>,
    pub faces: SlotMap<FaceId, Face>,
    pub instances: Vec<Instance>,
    pub curves: SlotMap<CurveId, Curve>,
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

    pub fn find_vertex(&self, p: DVec3) -> Option<VertexId> {
        self.vertices
            .iter()
            .find(|(_, v)| v.pos.distance(p) <= TOL)
            .map(|(id, _)| id)
    }

    pub fn edge_between(&self, a: VertexId, b: VertexId) -> Option<EdgeId> {
        self.vertices[a]
            .edges
            .iter()
            .copied()
            .find(|&e| self.edges[e].other(a) == b)
    }

    pub fn add_vertex(&mut self, pos: DVec3) -> VertexId {
        self.vertices.insert(Vertex {
            pos,
            edges: Vec::new(),
        })
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
        id
    }

    pub fn remove_face(&mut self, f: FaceId) -> Option<Face> {
        let face = self.faces.remove(f)?;
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
        let edge = self.edges.remove(e).expect("edge exists");
        for v in [edge.start, edge.end] {
            self.vertices[v].edges.retain(|&x| x != e);
        }
        removed
    }

    pub fn remove_vertex_if_unused(&mut self, v: VertexId) {
        if self.vertices.get(v).is_some_and(|x| x.edges.is_empty()) {
            self.vertices.remove(v);
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
        self.edges.remove(e);
        for v in [edge.start, edge.end] {
            self.vertices[v].edges.retain(|&x| x != e);
        }
        (a, b)
    }
}
