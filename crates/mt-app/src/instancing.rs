//! GPU instancing of groups and components (spec/PERFORMANCE.md).
//!
//! A definition placed more than once is drawn from one mesh in its own coordinates
//! (`Mesh`), once per placement with the placement's transform, instead of having
//! its triangles and edges copied into the scene's vertex and edge buffers per
//! instance. A mesh is made per definition *and* per everything else its contents
//! are drawn with (inherited material, tag, fading while another context is open),
//! so instances drawn differently get meshes of their own.
//!
//! Instanced geometry is not copied into the scene's flat lists either:
//! `Scene::vertices` / `Scene::indices` hold the triangles not instanced, and the
//! world-space triangles and edges used for picking, snapping and section cuts are
//! numbered as if flattened (every triangle and edge, in the order the model is
//! walked: [`Scene::tri`], [`Scene::edge`], with owners, materials and containers
//! stored for each) but only those outside placements are stored; a placement's are
//! computed from its mesh and transform when asked for, exactly as flattening made
//! them. [`Scene::tri_vertices`] gives any triangle's drawn vertices the same way.
//! Picking queries the meshes in their own coordinates (`pick_index`).
//!
//! Drawn flattened instead (each instance copied into the scene's buffers):
//! definitions placed once, instances on the path to the open context (their
//! contents are edited in place), hidden instances (View > Hidden Objects), "Shadows
//! face sun" components and what they contain, and meshes with a translucent
//! triangle (translucent triangles are sorted back to front one by one across the
//! whole model). Edges stay flattened with Jitter or edges colored by axis, and for
//! meshes with few edges. While Move, Rotate or Scale runs or a placement is
//! selected, `live_view` draws its edges flattened.

use std::ops::Range;

use glam::{DMat3, DMat4, DVec3, Mat4};
use mt_kernel::DefinitionId;
use mt_kernel::spatial::FxMap;
use mt_render::{FaceVertex, InstanceMesh, Segment, Viewport};

use crate::document::{Owner, Scene};

/// Meshes with fewer edges keep their edges flattened: an instance of the edges
/// costs as much GPU memory as about this many copied edges.
pub const MIN_INSTANCED_EDGES: usize = 8;

/// One triangle of a face in the coordinates of its collection, with what its
/// drawn vertices need besides the placement: colors, texture coordinates.
#[derive(Clone, Copy)]
pub struct LocalTri {
    pub local: [DVec3; 3],
    /// The face's normal (its own coordinates).
    pub normal: DVec3,
    /// The face's front and back material (inherited included).
    pub materials: [Option<mt_kernel::MaterialId>; 2],
    /// Vertex `k` for `local[k]`; positions and normals are set by `place`.
    pub verts: [FaceVertex; 3],
}

/// The order that winds `world` counter-clockwise around `n` (a mirroring transform
/// flips the winding).
pub fn ccw(world: &[DVec3; 3], n: DVec3) -> [usize; 3] {
    let [a, b, c] = *world;
    if (b - a).cross(c - a).dot(n) < 0.0 {
        [0, 2, 1]
    } else {
        [0, 1, 2]
    }
}

/// The world triangle of local triangle `local` placed by `xf`, counter-clockwise
/// around its face's world normal `n`.
pub fn placed_tri(local: &[DVec3; 3], xf: &DMat4, n: DVec3) -> [DVec3; 3] {
    let world = local.map(|p| xf.transform_point3(p));
    ccw(&world, n).map(|k| world[k])
}

/// World normal of a face of normal `normal` placed by `normal_xf` (the placement's
/// inverse transpose).
pub fn world_normal(normal_xf: &DMat3, normal: DVec3) -> DVec3 {
    (*normal_xf * normal).normalize_or_zero()
}

fn f32x3(p: DVec3) -> [f32; 3] {
    [p.x as f32, p.y as f32, p.z as f32]
}

impl LocalTri {
    /// The world triangle of [`LocalTri::place`] alone.
    pub fn world(&self, xf: &DMat4, n: DVec3) -> [DVec3; 3] {
        placed_tri(&self.local, xf, n)
    }

    /// The drawn vertices and the world triangle (both counter-clockwise around the
    /// face's world normal `n`) of this triangle placed by `xf`.
    pub fn place(&self, xf: &DMat4, n: DVec3) -> ([FaceVertex; 3], [DVec3; 3]) {
        let world = self.local.map(|p| xf.transform_point3(p));
        let order = ccw(&world, n);
        let verts = order.map(|k| FaceVertex {
            position: f32x3(world[k]),
            normal: f32x3(n),
            ..self.verts[k]
        });
        (verts, order.map(|k| world[k]))
    }

    fn opaque(&self) -> bool {
        self.verts[0].front[3] >= 1.0 && self.verts[0].back[3] >= 1.0
    }
}

/// One edge of a mesh as collected, in its coordinates, with what its world segment
/// needs.
#[derive(Clone, Copy)]
pub struct LocalEdge {
    pub a: DVec3,
    pub b: DVec3,
    /// The world segment of the first placement: color, style, flags and profile
    /// (its ends and normals are set when placed).
    pub seg: Segment,
    /// The normals of its first `count` faces (none when hidden).
    pub normals: [DVec3; 2],
    pub count: u8,
    /// Drawn as hidden geometry (`Scene::hidden_edges`).
    pub hidden: bool,
}

/// One edge of a mesh: a [`LocalEdge`] with its ends and normals as indices into the
/// mesh's `edge_points` and `edge_normals` (shared ends and normals are placed once
/// per placement).
#[derive(Clone, Copy)]
pub struct MeshEdge {
    pub seg: Segment,
    pub ends: [u32; 2],
    pub normals: [u32; 2],
    pub count: u8,
}

/// A mesh's edge points and normals placed by one transform (single precision, as
/// drawn): what [`Mesh::placed_edge`] reads.
#[derive(Default)]
pub struct PlacedTables {
    points: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
}

impl PlacedTables {
    /// Fills the tables for `mesh` placed by `xf` (`normal_xf` its inverse transpose).
    pub fn fill(&mut self, mesh: &Mesh, xf: &DMat4, normal_xf: &DMat3) {
        self.points.clear();
        (self.points).extend(
            mesh.edge_points
                .iter()
                .map(|p| f32x3(xf.transform_point3(*p))),
        );
        self.normals.clear();
        (self.normals).extend(
            mesh.edge_normals
                .iter()
                .map(|n| f32x3(world_normal(normal_xf, *n))),
        );
    }
}

/// A definition's own faces and edges (not those of the instances it contains, which
/// are placements of their own), in its coordinates.
pub struct Mesh {
    /// Unindexed triangles, counter-clockwise around their face's normal.
    pub vertices: Vec<FaceVertex>,
    pub indices: Vec<u32>,
    /// Empty when the edges are drawn flattened.
    pub edges: Vec<Segment>,
    /// The triangles as collected, to give the flattened vertices exactly.
    pub tris: Vec<LocalTri>,
    /// Every edge of a placement (drawn or not), to give its world segments; `None`
    /// when their colors depend on the transform (Styles > Edge > Color by axis): the
    /// placements' edges are then stored in the scene.
    pub local_edges: Option<Vec<MeshEdge>>,
    /// Indices into `local_edges` of the hidden ones.
    pub hidden_edges: Vec<u32>,
    /// The distinct corners of `tris` (for bounds).
    pub tri_points: Vec<DVec3>,
    /// The distinct ends of the edges, and the distinct normals they take.
    pub edge_points: Vec<DVec3>,
    pub edge_normals: Vec<DVec3>,
}

impl Mesh {
    /// The world segment of edge `k` (of `local_edges`) placed with `tables`, exactly
    /// as the scene build makes it.
    pub fn placed_edge(&self, k: usize, tables: &PlacedTables) -> Segment {
        let e = &self.local_edges.as_ref().expect("edges given by the mesh")[k];
        let mut s = e.seg;
        s.a = tables.points[e.ends[0] as usize];
        s.b = tables.points[e.ends[1] as usize];
        if e.count >= 1 {
            s.n1 = tables.normals[e.normals[0] as usize];
        }
        if e.count >= 2 {
            s.n2 = tables.normals[e.normals[1] as usize];
        }
        s
    }

    /// The ends of edge `k` (of `local_edges`) in the mesh's coordinates.
    pub fn edge_ends(&self, k: usize) -> [DVec3; 2] {
        let e = &self.local_edges.as_ref().expect("edges given by the mesh")[k];
        e.ends.map(|i| self.edge_points[i as usize])
    }
}

/// Distinct values by bit pattern, in first-seen order, and the index of each.
#[derive(Default)]
struct Distinct {
    values: Vec<DVec3>,
    index: FxMap<[u64; 3], u32>,
}

impl Distinct {
    fn add(&mut self, p: DVec3) -> u32 {
        let next = self.values.len() as u32;
        let i = *self
            .index
            .entry(p.to_array().map(f64::to_bits))
            .or_insert(next);
        if i == next {
            self.values.push(p);
        }
        i
    }
}

/// One instance drawn from a mesh.
pub struct Placement {
    pub mesh: u32,
    /// Its coordinates to world.
    pub transform: DMat4,
    /// What selects it (`None` outside the open context).
    pub owner: Option<Owner>,
    /// Its triangles ([`Scene::tri`]) and its edges ([`Scene::edge`]).
    pub tris: Range<usize>,
    pub edges: Range<usize>,
    /// The inverse transpose of `transform`, for normals.
    pub normal_xf: DMat3,
    /// Its edges come from the mesh's `local_edges` (not stored in the scene).
    pub lazy_edges: bool,
    /// Triangles before `tris` that are not instanced (in `Scene::indices`).
    flat_before: usize,
    /// Edges of placements before this one that come from their mesh.
    lazy_edges_before: usize,
}

impl Placement {
    /// Whether its edges are drawn from the mesh.
    pub fn edges_instanced(&self, scene: &Scene) -> bool {
        !scene.instancing.meshes[self.mesh as usize].edges.is_empty()
    }
}

/// The instanced part of a scene.
#[derive(Default)]
pub struct Instancing {
    pub meshes: Vec<Mesh>,
    /// In the order the model is walked (ascending `tris` and `edges`).
    pub placements: Vec<Placement>,
    /// Triangles in placements so far.
    instanced_tris: usize,
    /// Edges of placements so far that come from their mesh.
    lazy_edges: usize,
}

/// What a mesh is made for: its definition and what its contents are drawn with.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub definition: DefinitionId,
    pub material: Option<mt_kernel::MaterialId>,
    pub layer: Option<usize>,
    pub similar: bool,
    pub faded: bool,
}

/// What became of a [`Key`].
#[derive(Clone, Copy)]
pub enum Made {
    Mesh(u32),
    /// Drawn flattened (translucent, or nothing to draw).
    Flat,
}

/// Instancing state of one scene build.
pub struct Builder {
    /// Times each definition is placed in the model (counted up to 3).
    pub reach: FxMap<DefinitionId, u32>,
    pub made: FxMap<Key, Made>,
    /// Edges can be instanced (not colored by axis, no jitter).
    pub edges: bool,
    /// Edges' world segments can be made from their mesh (their colors do not depend
    /// on the transform: not colored by axis).
    pub lazy_edges: bool,
}

impl Builder {
    pub fn new(model: &mt_kernel::Model) -> Self {
        let by_axis =
            model.style.edge_look.color_mode == mt_kernel::style_extras::EdgeColorMode::ByAxis;
        Self {
            reach: reach(model),
            made: FxMap::default(),
            edges: !by_axis && !model.style.edge_look.jitter,
            lazy_edges: !by_axis,
        }
    }
}

/// How many times each definition is placed in the model, counted up to 3 (each
/// definition's contents are walked at most twice).
fn reach(model: &mt_kernel::Model) -> FxMap<DefinitionId, u32> {
    let mut count: FxMap<DefinitionId, u32> = FxMap::default();
    let mut stack = vec![&model.root];
    while let Some(ents) = stack.pop() {
        for i in &ents.instances {
            let c = count.entry(i.definition).or_insert(0);
            if *c >= 3 {
                continue;
            }
            *c += 1;
            if *c <= 2
                && let Some(d) = model.definitions.get(i.definition)
            {
                stack.push(&d.entities);
            }
        }
    }
    count
}

/// A mesh being made from the first placement of its key.
#[derive(Default)]
pub struct Build {
    pub tris: Vec<LocalTri>,
    pub edges: Vec<Segment>,
    pub local_edges: Vec<LocalEdge>,
}

impl Build {
    /// Ends the first placement of `key` (triangles `tris`, edges `edges`, placed by
    /// `xf`): makes the mesh, or, when the placement must be drawn flattened, draws it
    /// into the scene's flat lists.
    #[allow(clippy::too_many_arguments)]
    pub fn finish(
        self,
        builder: &mut Builder,
        key: Key,
        xf: DMat4,
        owner: Option<Owner>,
        tris: Range<usize>,
        edges: Range<usize>,
        out: &mut Scene,
    ) {
        let edges_instanced = builder.edges && self.edges.len() >= MIN_INSTANCED_EDGES;
        let flat =
            self.tris.iter().any(|t| !t.opaque()) || (self.tris.is_empty() && !edges_instanced);
        if flat {
            builder.made.insert(key, Made::Flat);
            let normal_xf = DMat3::from_mat4(xf).inverse().transpose();
            for t in &self.tris {
                let (verts, _) = t.place(&xf, world_normal(&normal_xf, t.normal));
                for v in verts {
                    out.indices.push(out.vertices.len() as u32);
                    out.vertices.push(v);
                }
            }
            return;
        }
        let mut vertices = Vec::with_capacity(3 * self.tris.len());
        for t in &self.tris {
            vertices.extend(t.place(&DMat4::IDENTITY, t.normal.normalize_or_zero()).0);
        }
        // The placement's world triangles (and edges) were stored while collected:
        // the mesh gives them from now on.
        out.tri_list.truncate(out.tri_list.len() - tris.len());
        let lazy = builder.lazy_edges;
        if lazy {
            out.edge_list.truncate(out.edge_list.len() - edges.len());
        }
        let hidden_edges = (self.local_edges.iter().enumerate())
            .filter(|(_, e)| e.hidden)
            .map(|(k, _)| k as u32)
            .collect();
        let mut tri_points = Distinct::default();
        for p in self.tris.iter().flat_map(|t| t.local) {
            tri_points.add(p);
        }
        let (mut points, mut normals) = (Distinct::default(), Distinct::default());
        let local_edges: Vec<MeshEdge> = (self.local_edges.iter())
            .map(|e| MeshEdge {
                seg: e.seg,
                ends: [points.add(e.a), points.add(e.b)],
                normals: [0, 1].map(|k| {
                    if k < usize::from(e.count) {
                        normals.add(e.normals[k])
                    } else {
                        0
                    }
                }),
                count: e.count,
            })
            .collect();
        let mesh = Mesh {
            indices: (0..vertices.len() as u32).collect(),
            vertices,
            edges: if edges_instanced {
                self.edges
            } else {
                Vec::new()
            },
            tri_points: tri_points.values,
            edge_points: points.values,
            edge_normals: normals.values,
            tris: self.tris,
            local_edges: lazy.then_some(local_edges),
            hidden_edges,
        };
        let m = out.instancing.meshes.len() as u32;
        out.instancing.meshes.push(mesh);
        builder.made.insert(key, Made::Mesh(m));
        out.instancing.place(m, xf, owner, tris, edges, lazy);
    }
}

impl Instancing {
    /// Records a placement of mesh `mesh`.
    pub fn place(
        &mut self,
        mesh: u32,
        transform: DMat4,
        owner: Option<Owner>,
        tris: Range<usize>,
        edges: Range<usize>,
        lazy_edges: bool,
    ) {
        let flat_before = tris.start - self.instanced_tris;
        self.instanced_tris += tris.len();
        let lazy_edges_before = self.lazy_edges;
        if lazy_edges {
            self.lazy_edges += edges.len();
        }
        self.placements.push(Placement {
            mesh,
            transform,
            owner,
            tris,
            edges,
            normal_xf: DMat3::from_mat4(transform).inverse().transpose(),
            lazy_edges,
            flat_before,
            lazy_edges_before,
        });
    }
}

impl Scene {
    /// Number of triangles (instanced ones included).
    pub fn tri_count(&self) -> usize {
        self.tri_owner.len()
    }

    /// Number of edges (those of placements included).
    pub fn edge_count(&self) -> usize {
        self.edge_owner.len()
    }

    /// The world triangles stored (those not instanced), by [`Scene::flat_tri`] index.
    pub fn stored_tris(&self) -> &[[DVec3; 3]] {
        &self.tri_list
    }

    /// The world edges stored (those not given by a mesh).
    pub fn stored_edges(&self) -> &[Segment] {
        &self.edge_list
    }

    /// World triangle `t`, exactly as flattening made it.
    pub fn tri(&self, t: usize) -> [DVec3; 3] {
        match self.placement_of_tri(t) {
            Some(j) => {
                let p = &self.instancing.placements[j];
                let tri = &self.instancing.meshes[p.mesh as usize].tris[t - p.tris.start];
                tri.world(&p.transform, world_normal(&p.normal_xf, tri.normal))
            }
            None => self.tri_list[self.flat_tri(t).expect("not instanced")],
        }
    }

    /// Where edge `e` is: its index in the stored edges, or its placement and its
    /// index in the placement's mesh.
    fn edge_slot(&self, e: usize) -> Result<usize, (usize, usize)> {
        let p = &self.instancing.placements;
        let Some(j) = p.partition_point(|x| x.edges.start <= e).checked_sub(1) else {
            return Ok(e);
        };
        let x = &p[j];
        if x.lazy_edges && x.edges.contains(&e) {
            return Err((j, e - x.edges.start));
        }
        let lazy = x.lazy_edges_before + if x.lazy_edges { x.edges.len() } else { 0 };
        Ok(e - lazy)
    }

    /// World segment of edge `e`, exactly as flattening made it.
    pub fn edge(&self, e: usize) -> Segment {
        match self.edge_slot(e) {
            Ok(i) => self.edge_list[i],
            Err((j, k)) => self.placed_edge(j, k),
        }
    }

    /// Edge `k` of placement `j` (whose edges come from its mesh).
    fn placed_edge(&self, j: usize, k: usize) -> Segment {
        let p = &self.instancing.placements[j];
        let mesh = &self.instancing.meshes[p.mesh as usize];
        let e = &mesh.local_edges.as_ref().expect("edges given by the mesh")[k];
        let mut s = e.seg;
        let point = |i: u32| f32x3(p.transform.transform_point3(mesh.edge_points[i as usize]));
        let normal = |i: u32| f32x3(world_normal(&p.normal_xf, mesh.edge_normals[i as usize]));
        (s.a, s.b) = (point(e.ends[0]), point(e.ends[1]));
        if e.count >= 1 {
            s.n1 = normal(e.normals[0]);
        }
        if e.count >= 2 {
            s.n2 = normal(e.normals[1]);
        }
        s
    }

    /// Calls `f` with every edge in `range` (index and world segment), in order.
    pub fn for_each_edge_in(&self, range: Range<usize>, mut f: impl FnMut(usize, Segment)) {
        let p = &self.instancing.placements;
        let mut e = range.start;
        let mut j = p.partition_point(|x| x.edges.end <= e);
        let mut tables = PlacedTables::default();
        while e < range.end {
            // The next placement whose edges come from its mesh.
            while p
                .get(j)
                .is_some_and(|x| !x.lazy_edges || x.edges.is_empty())
            {
                j += 1;
            }
            let next = p.get(j);
            let stop = next.map_or(range.end, |x| x.edges.start.max(e).min(range.end));
            if e < stop {
                let Ok(first) = self.edge_slot(e) else {
                    unreachable!("a stored edge")
                };
                for (k, seg) in self.edge_list[first..first + stop - e].iter().enumerate() {
                    f(e + k, *seg);
                }
                e = stop;
            }
            let Some(x) = next else { break };
            let end = x.edges.end.min(range.end);
            if e < end {
                let mesh = &self.instancing.meshes[x.mesh as usize];
                tables.fill(mesh, &x.transform, &x.normal_xf);
                for i in e..end {
                    f(i, mesh.placed_edge(i - x.edges.start, &tables));
                }
                e = end;
            }
            j += 1;
        }
    }

    /// Calls `f` with every edge (index and world segment), in order.
    pub fn for_each_edge(&self, f: impl FnMut(usize, Segment)) {
        self.for_each_edge_in(0..self.edge_count(), f);
    }

    /// Calls `f` with every triangle (index and world triangle), in order.
    pub fn for_each_tri(&self, mut f: impl FnMut(usize, [DVec3; 3])) {
        let mut t = 0;
        let mut flat = 0;
        for p in &self.instancing.placements {
            let n = p.tris.start - t;
            for tri in &self.tri_list[flat..flat + n] {
                f(t, *tri);
                t += 1;
            }
            flat += n;
            let tris = &self.instancing.meshes[p.mesh as usize].tris;
            for tri in tris {
                f(
                    t,
                    tri.world(&p.transform, world_normal(&p.normal_xf, tri.normal)),
                );
                t += 1;
            }
        }
        for tri in &self.tri_list[flat..] {
            f(t, *tri);
            t += 1;
        }
    }

    /// Every edge in world space (made on first use and kept).
    pub fn edges(&self) -> &[Segment] {
        self.all_edges.get_or_init(|| {
            if self.instancing.lazy_edges == 0 {
                return self.edge_list.clone();
            }
            let mut out = Vec::with_capacity(self.edge_count());
            self.for_each_edge(|_, s| out.push(s));
            out
        })
    }

    /// Every triangle in world space (made on first use and kept).
    pub fn triangles(&self) -> &[[DVec3; 3]] {
        self.all_tris.get_or_init(|| {
            if self.instancing.placements.is_empty() {
                return self.tri_list.clone();
            }
            let mut out = Vec::with_capacity(self.tri_count());
            self.for_each_tri(|_, t| out.push(t));
            out
        })
    }

    /// The placement drawing triangle `t`, if instanced.
    pub fn placement_of_tri(&self, t: usize) -> Option<usize> {
        let p = &self.instancing.placements;
        let j = p.partition_point(|x| x.tris.start <= t).checked_sub(1)?;
        p[j].tris.contains(&t).then_some(j)
    }

    /// Index of triangle `t` in `indices` (`indices[3 * i..3 * i + 3]`), or `None`
    /// when it is instanced.
    pub fn flat_tri(&self, t: usize) -> Option<usize> {
        let p = &self.instancing.placements;
        let Some(j) = p.partition_point(|x| x.tris.start <= t).checked_sub(1) else {
            return Some(t);
        };
        let x = &p[j];
        (!x.tris.contains(&t)).then(|| x.flat_before + (t - x.tris.end))
    }

    /// The triangles not instanced: (index in `triangles`, index in `indices / 3`).
    pub fn flat_tris(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        let p = &self.instancing.placements;
        (0..=p.len()).flat_map(move |j| {
            // After placement j - 1, the first flat triangle is its `flat_before`.
            let (start, flat) = match j.checked_sub(1) {
                Some(k) => (p[k].tris.end, p[k].flat_before),
                None => (0, 0),
            };
            let end = p.get(j).map_or(self.tri_count(), |x| x.tris.start);
            (start..end).map(move |t| (t, flat + t - start))
        })
    }

    /// Placements whose owner `keep` accepts.
    pub fn placements_where(&self, keep: impl Fn(Option<Owner>) -> bool) -> Vec<usize> {
        (self.instancing.placements.iter().enumerate())
            .filter(|(_, p)| keep(p.owner))
            .map(|(j, _)| j)
            .collect()
    }

    /// The drawn vertices of triangle `t` (of `triangles`), as the flattened scene
    /// would hold them.
    pub fn tri_vertices(&self, t: usize) -> [FaceVertex; 3] {
        match self.placement_of_tri(t) {
            Some(j) => {
                let p = &self.instancing.placements[j];
                let tri = &self.instancing.meshes[p.mesh as usize].tris[t - p.tris.start];
                let normal_xf = DMat3::from_mat4(p.transform).inverse().transpose();
                tri.place(&p.transform, world_normal(&normal_xf, tri.normal))
                    .0
            }
            None => {
                let i = self.flat_tri(t).expect("not instanced");
                [0, 1, 2].map(|k| self.vertices[self.indices[3 * i + k] as usize])
            }
        }
    }

    /// Every triangle's drawn vertices in triangle order, as the flattened scene
    /// would hold them (`indices` are then `0..3 * triangles`).
    #[cfg(test)]
    pub fn flattened_vertices(&self) -> Vec<FaceVertex> {
        let mut out = Vec::with_capacity(3 * self.tri_count());
        let mut t = 0;
        for p in &self.instancing.placements {
            for k in t..p.tris.start {
                out.extend(self.tri_vertices(k));
            }
            let tris = &self.instancing.meshes[p.mesh as usize].tris;
            let normal_xf = DMat3::from_mat4(p.transform).inverse().transpose();
            for tri in tris {
                out.extend(
                    tri.place(&p.transform, world_normal(&normal_xf, tri.normal))
                        .0,
                );
            }
            t = p.tris.end;
        }
        for k in t..self.tri_count() {
            out.extend(self.tri_vertices(k));
        }
        out
    }

    /// The edges drawn flattened when nothing is selected or moving (those of
    /// placements whose edges are instanced left out), and those placements.
    pub fn flat_edges(&self) -> (Vec<Segment>, Vec<usize>) {
        let mut out = Vec::with_capacity(self.edge_list.len());
        let mut instanced = Vec::new();
        let mut e = 0;
        for (j, p) in self.instancing.placements.iter().enumerate() {
            if !p.edges_instanced(self) {
                continue;
            }
            self.for_each_edge_in(e..p.edges.start, |_, s| out.push(s));
            e = p.edges.end;
            instanced.push(j);
        }
        self.for_each_edge_in(e..self.edge_count(), |_, s| out.push(s));
        (out, instanced)
    }

    /// The placements as GPU instances: mesh and transform.
    pub fn gpu_instances(&self, placements: impl IntoIterator<Item = usize>) -> Vec<(u32, Mat4)> {
        placements
            .into_iter()
            .map(|j| {
                let p = &self.instancing.placements[j];
                (p.mesh, p.transform.as_mat4())
            })
            .collect()
    }
}

/// Sends the scene's faces to `viewport`: the flattened triangles, the meshes and
/// every placement. Edges are sent with the highlight (`live_view`).
pub fn upload_faces(viewport: &mut Viewport, device: &egui_wgpu::wgpu::Device, scene: &Scene) {
    viewport.set_faces(device, &scene.vertices, &scene.indices);
    let meshes: Vec<InstanceMesh> = (scene.instancing.meshes.iter())
        .map(|m| InstanceMesh {
            vertices: &m.vertices,
            indices: &m.indices,
            edges: &m.edges,
        })
        .collect();
    viewport.set_instance_meshes(device, &meshes);
    let all = scene.gpu_instances(0..scene.instancing.placements.len());
    viewport.set_face_instances(device, &all);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench::{add_box, checksum, definition, instance};
    use crate::document::Document;
    use crate::live_view::Highlight;
    use crate::selection::Selection;

    /// Boxes placed with a variety of transforms, materials and tags, a group with a
    /// translucent material, nested components and a box of the root's own.
    fn model() -> mt_kernel::Model {
        let mut model = mt_kernel::Model::default();
        for (name, color, opacity) in [("Red", [200, 0, 0], 1.0), ("Glass", [0, 0, 200], 0.5)] {
            model.materials.push(mt_kernel::Material {
                name: name.into(),
                color,
                opacity,
                texture: None,
            });
        }
        model.layers.push(mt_kernel::Layer {
            name: "Tag".into(),
            color: [0, 150, 0],
            ..Default::default()
        });
        let mut ents = mt_kernel::Entities::default();
        add_box(&mut ents, DVec3::ZERO, DVec3::splat(10.0));
        let leaf = model.definitions.insert(definition("Leaf", ents, false));
        let mut ents = mt_kernel::Entities::default();
        add_box(&mut ents, DVec3::Z * 20.0, DVec3::new(5.0, 5.0, 2.0));
        for k in 0..3 {
            let xf = DMat4::from_translation(DVec3::X * (12.0 * k as f64));
            ents.instances.push(instance(leaf, xf));
        }
        let pair = model.definitions.insert(definition("Pair", ents, false));
        add_box(
            &mut model.root,
            DVec3::new(-50.0, 0.0, 0.0),
            DVec3::splat(8.0),
        );
        let xfs = [
            DMat4::from_translation(DVec3::new(0.0, 40.0, 0.0)),
            DMat4::from_rotation_z(0.7) * DMat4::from_translation(DVec3::Y * 80.0),
            DMat4::from_scale(DVec3::new(-1.0, 1.0, 1.0)),
            DMat4::from_scale(DVec3::new(2.0, 0.5, 1.5)),
        ];
        for (k, xf) in xfs.into_iter().enumerate() {
            let mut i = instance(if k % 2 == 0 { leaf } else { pair }, xf);
            i.material = (k == 3).then_some(0);
            i.layer = (k == 1).then_some(1);
            model.root.instances.push(i);
            let mut glass = instance(leaf, DMat4::from_translation(DVec3::Z * 50.0) * xf);
            glass.material = Some(1);
            model.root.instances.push(glass);
        }
        model
    }

    /// The scene built with instancing: what the flattened one draws, triangle for
    /// triangle and edge for edge, in every style that changes what is collected.
    #[test]
    fn instanced_scene_flattens_to_the_flattened_one() {
        let mut doc = Document::untitled();
        doc.model = model();
        type Change = fn(&mut Document);
        let changes: [(&str, Change); 7] = [
            ("plain", |_| {}),
            ("color by tag", |d| d.model.color_by_tag = true),
            ("hidden geometry", |d| d.model.style.hidden_geometry = true),
            ("open a pair", |d| assert!(d.open_instance(2))),
            ("hide the rest", |d| d.model.style.hide_rest_of_model = true),
            ("close", |d| {
                d.close_instance();
                d.model.style.edge_look.jitter = true;
            }),
            ("hide one", |d| d.model.root.instances[0].hidden = true),
        ];
        for (name, change) in changes {
            change(&mut doc);
            let scene = doc.scene();
            let flat = doc.scene_with(false);
            assert!(flat.instancing.placements.is_empty());
            assert_eq!(checksum(&scene), checksum(&flat), "{name}");
            assert_eq!(scene.triangles(), flat.triangles(), "{name}");
            let expanded = scene.flattened_vertices();
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&expanded),
                bytemuck::cast_slice::<_, u8>(&flat.vertices),
                "{name}"
            );
            for t in 0..scene.triangles().len() {
                assert_eq!(
                    bytemuck::cast_slice::<_, u8>(&scene.tri_vertices(t)),
                    bytemuck::cast_slice::<_, u8>(&expanded[3 * t..3 * t + 3]),
                    "{name}"
                );
            }
            // Something is instanced, and the translucent glass boxes are not.
            assert!(!scene.instancing.placements.is_empty(), "{name}");
            let glass = scene.vertices.iter().any(|v| v.front[3] < 1.0);
            assert_eq!(glass, !doc.model.color_by_tag, "{name}");
            // The flattened edges and the placements' edges are all the edges.
            let (edges, instanced) = scene.flat_edges();
            let n: usize = (instanced.iter())
                .map(|&j| scene.instancing.placements[j].edges.len())
                .sum();
            assert_eq!(edges.len() + n, scene.edges().len(), "{name}");
            for &j in &instanced {
                let p = &scene.instancing.placements[j];
                assert_eq!(
                    scene.instancing.meshes[p.mesh as usize].edges.len(),
                    p.edges.len()
                );
            }
        }
    }

    /// A selected instance's edges are drawn flattened (in the selection color), the
    /// others' instanced.
    #[test]
    fn selected_instances_draw_their_edges_flattened() {
        let mut doc = Document::untitled();
        doc.model = crate::bench::instances(4);
        let scene = doc.scene();
        assert_eq!(scene.instancing.placements.len(), 4);
        assert_eq!(scene.vertices.len(), 0);
        let mut selection = Selection::default();
        selection.apply(crate::selection::Mode::Replace, vec![Owner::Instance(2)]);
        let highlight = |selection: &Selection| {
            Highlight {
                scene: &scene,
                selection,
                document: Some(&doc),
                stretch: &Default::default(),
                box_only: false,
                live: None,
                ctx_xf: DMat4::IDENTITY,
                inst_bounds: &Default::default(),
                ghost: None,
                cut_lines: &[],
            }
            .build()
        };
        let (edges, _, instanced) = highlight(&Selection::default());
        assert_eq!((edges.len(), instanced), (0, vec![0, 1, 2, 3]));
        let (edges, _, instanced) = highlight(&selection);
        assert_eq!(instanced, vec![0, 1, 3]);
        assert_eq!(edges.len(), 12);
    }
}
