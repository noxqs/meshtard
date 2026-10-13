//! GPU instancing of groups and components (spec/PERFORMANCE.md).
//!
//! A definition placed more than once is drawn from one mesh in its own coordinates
//! (`Mesh`), once per placement with the placement's transform, instead of having
//! its triangles and edges copied into the scene's vertex and edge buffers per
//! instance. A mesh is made per definition *and* per everything else its contents
//! are drawn with (inherited material, tag, fading while another context is open),
//! so instances drawn differently get meshes of their own.
//!
//! What instancing leaves out of the scene's flat lists is only what the GPU draws:
//! `Scene::vertices` / `Scene::indices` hold the triangles not instanced, while the
//! world-space lists used for picking, snapping and section cuts
//! (`Scene::triangles`, `tri_owner`, `Scene::edges`, ...) still hold every triangle
//! and edge, in the order the model is walked. [`Scene::tri_vertices`] gives any
//! triangle's drawn vertices, exactly as they would have been flattened.
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

/// World normal of a face of normal `normal` placed by `normal_xf` (the placement's
/// inverse transpose).
pub fn world_normal(normal_xf: &DMat3, normal: DVec3) -> DVec3 {
    (*normal_xf * normal).normalize_or_zero()
}

fn f32x3(p: DVec3) -> [f32; 3] {
    [p.x as f32, p.y as f32, p.z as f32]
}

impl LocalTri {
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
}

/// One instance drawn from a mesh.
pub struct Placement {
    pub mesh: u32,
    /// Its coordinates to world.
    pub transform: DMat4,
    /// What selects it (`None` outside the open context).
    pub owner: Option<Owner>,
    /// Its triangles in `Scene::triangles` and its edges in `Scene::edges`.
    pub tris: Range<usize>,
    pub edges: Range<usize>,
    /// Triangles before `tris` that are not instanced (in `Scene::indices`).
    flat_before: usize,
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
}

impl Builder {
    pub fn new(model: &mt_kernel::Model) -> Self {
        Self {
            reach: reach(model),
            made: FxMap::default(),
            edges: model.style.edge_look.color_mode
                != mt_kernel::style_extras::EdgeColorMode::ByAxis
                && !model.style.edge_look.jitter,
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
        let mesh = Mesh {
            indices: (0..vertices.len() as u32).collect(),
            vertices,
            edges: if edges_instanced {
                self.edges
            } else {
                Vec::new()
            },
            tris: self.tris,
        };
        let m = out.instancing.meshes.len() as u32;
        out.instancing.meshes.push(mesh);
        builder.made.insert(key, Made::Mesh(m));
        out.instancing.place(m, xf, owner, tris, edges);
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
    ) {
        let flat_before = tris.start - self.instanced_tris;
        self.instanced_tris += tris.len();
        self.placements.push(Placement {
            mesh,
            transform,
            owner,
            tris,
            edges,
            flat_before,
        });
    }
}

impl Scene {
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
            let end = p.get(j).map_or(self.triangles.len(), |x| x.tris.start);
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
        let mut out = Vec::with_capacity(3 * self.triangles.len());
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
        for k in t..self.triangles.len() {
            out.extend(self.tri_vertices(k));
        }
        out
    }

    /// The edges drawn flattened when nothing is selected or moving (those of
    /// placements whose edges are instanced left out), and those placements.
    pub fn flat_edges(&self) -> (Vec<Segment>, Vec<usize>) {
        let mut out = Vec::with_capacity(self.edges.len());
        let mut instanced = Vec::new();
        let mut e = 0;
        for (j, p) in self.instancing.placements.iter().enumerate() {
            if !p.edges_instanced(self) {
                continue;
            }
            out.extend_from_slice(&self.edges[e..p.edges.start]);
            e = p.edges.end;
            instanced.push(j);
        }
        out.extend_from_slice(&self.edges[e..]);
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
            assert_eq!(scene.triangles, flat.triangles, "{name}");
            let expanded = scene.flattened_vertices();
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&expanded),
                bytemuck::cast_slice::<_, u8>(&flat.vertices),
                "{name}"
            );
            for t in 0..scene.triangles.len() {
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
            assert_eq!(edges.len() + n, scene.edges.len(), "{name}");
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
