//! Picking instanced geometry from its meshes (spec/PERFORMANCE.md): two-level
//! indexes that never put a placement's triangles or edges in world space to build.
//!
//! The top level indexes the placements' world boxes (their mesh's box, transformed,
//! grown a little for rounding); the second level is one index per mesh, in its own
//! coordinates, queried with the ray (or the view pyramid) taken into the placement's
//! coordinates. The candidates it finds are then tested exactly as before, on their
//! world triangle or segment computed as the scene build would flatten it
//! (`Scene::tri`, `Scene::edge`), so the answers are those of the flattened scene.
//! Geometry outside placements keeps a plain index over its stored world lists.

use glam::{DMat3, DMat4, DVec2, DVec3};
use mt_infer::cull::{BoxIndex, Pyramid, ray_meets_box};
use mt_kernel::bvh::{Bvh, Hit, RayIndex, ray_triangle};
use mt_render::Camera;

use crate::document::Scene;
use crate::instancing::{placed_tri, world_normal};

/// Relative growth of boxes and margins, far above rounding (about 1e-16 per
/// operation) and far below anything a pixel test tells apart.
const GROW: f64 = 1e-7;
/// Error of a coordinate after a few operations, relative to its size.
const ROUND: f64 = 1e-12;
/// Meshes with fewer edges are tested whole when their placement's box is near.
const MESH_EDGE_INDEX_MIN: usize = 32;

/// The box around `points`, or `None` when there are none.
fn bounds(points: impl IntoIterator<Item = DVec3>) -> Option<(DVec3, DVec3)> {
    points.into_iter().fold(None, |b, p| match b {
        None => Some((p, p)),
        Some((lo, hi)) => Some((lo.min(p), hi.max(p))),
    })
}

/// The world box of local box `(lo, hi)` placed by `xf`, grown so that every point
/// computed by transforming a point of the local box lies inside, and its growth.
fn world_box(xf: &DMat4, (lo, hi): (DVec3, DVec3)) -> (DVec3, DVec3, f64) {
    let corners = (0..8).map(|k| {
        let pick = |bit: usize, a: f64, b: f64| if k & bit == 0 { a } else { b };
        xf.transform_point3(DVec3::new(
            pick(1, lo.x, hi.x),
            pick(2, lo.y, hi.y),
            pick(4, lo.z, hi.z),
        ))
    });
    let (min, max) = bounds(corners).expect("eight corners");
    let size = (max - min).max_element();
    let pad = GROW * size + ROUND * min.abs().max(max.abs()).max_element();
    (min - pad, max + pad, pad)
}

/// The largest row sum of `m`'s absolute values: how much it can stretch a vector
/// (in the max norm).
fn stretch(m: &DMat3) -> f64 {
    let t = m.transpose();
    [t.x_axis, t.y_axis, t.z_axis]
        .iter()
        .map(|r| r.abs().element_sum())
        .fold(0.0, f64::max)
}

/// The placement's inverse, when it has one.
fn inverse(xf: &DMat4) -> Option<DMat4> {
    let inv = xf.inverse();
    (xf.determinant() != 0.0 && inv.is_finite()).then_some(inv)
}

/// A placement as the pick index sees it.
struct Place {
    mesh: u32,
    transform: DMat4,
    normal_xf: DMat3,
    inverse: Option<DMat4>,
    /// Its triangles: `start..start + len` (as numbered by the scene).
    start: usize,
    len: usize,
    /// Stored triangles before it.
    flat_before: usize,
    /// Its world box.
    min: DVec3,
    max: DVec3,
    /// Growth of the mesh's boxes in its coordinates.
    local_pad: f64,
}

/// Nearest-hit ray queries over a scene's triangles, instanced ones from their
/// meshes (see the module documentation). Hits are numbered as the scene numbers
/// triangles; among hits at the same distance the lowest-numbered wins, as with
/// [`Bvh`] over the flattened triangles.
pub struct PickIndex {
    /// Over the stored triangles.
    flat: Box<dyn RayIndex>,
    /// The triangle number of each stored triangle.
    flat_ids: Vec<u32>,
    /// Per mesh: its triangles in its coordinates, and their faces' normals.
    meshes: Vec<(Bvh, Vec<DVec3>)>,
    /// The placements with triangles, in triangle order.
    places: Vec<Place>,
    /// Over `places`' world boxes.
    top: BoxIndex,
}

impl PickIndex {
    /// The pick index of `scene`; `flat` builds the index over the triangles not
    /// instanced (on the GPU for large models). A scene without placements gets that
    /// index alone.
    pub fn build(
        scene: &Scene,
        flat: impl FnOnce(Vec<[DVec3; 3]>) -> Box<dyn RayIndex>,
    ) -> Box<dyn RayIndex> {
        let inst = &scene.instancing;
        if inst.placements.is_empty() {
            return flat(scene.stored_tris().to_vec());
        }
        let flat_ids = scene
            .flat_tris()
            .map(|(t, _)| u32::try_from(t).expect("fewer than 2^32 triangles"))
            .collect();
        let meshes: Vec<(Bvh, Vec<DVec3>)> = (inst.meshes.iter())
            .map(|m| {
                (
                    Bvh::build(m.tris.iter().map(|t| t.local).collect()),
                    m.tris.iter().map(|t| t.normal).collect(),
                )
            })
            .collect();
        let mut flat_before = 0;
        let mut next = 0;
        let mut places = Vec::new();
        for p in &inst.placements {
            flat_before += p.tris.start - next;
            next = p.tris.end;
            let Some(local) = meshes[p.mesh as usize].0.bounds() else {
                continue;
            };
            let (min, max, pad) = world_box(&p.transform, local);
            let inverse = inverse(&p.transform);
            let local_size = local.0.abs().max(local.1.abs()).max_element();
            let world_size = min.abs().max(max.abs()).max_element();
            let local_pad = inverse.map_or(0.0, |inv| {
                GROW * local_size + (ROUND * world_size + pad) * stretch(&DMat3::from_mat4(inv))
            });
            places.push(Place {
                mesh: p.mesh,
                transform: p.transform,
                normal_xf: p.normal_xf,
                inverse,
                start: p.tris.start,
                len: p.tris.len(),
                flat_before,
                min,
                max,
                local_pad,
            });
        }
        let top = BoxIndex::boxes(places.iter().map(|p| (p.min, p.max)));
        Box::new(Self {
            flat: flat(scene.stored_tris().to_vec()),
            flat_ids,
            meshes,
            places,
            top,
        })
    }

    /// World triangle `k` of place `p`, as the scene build makes it.
    fn placed(&self, p: &Place, k: usize) -> [DVec3; 3] {
        let (bvh, normals) = &self.meshes[p.mesh as usize];
        placed_tri(
            &bvh.triangle(k),
            &p.transform,
            world_normal(&p.normal_xf, normals[k]),
        )
    }
}

/// How far candidates are looked for past the best hit so far (rounding between the
/// world and the meshes' coordinates).
fn limit(best: Option<Hit>) -> f64 {
    best.map_or(f64::INFINITY, |h| h.t + h.t.abs() * GROW + ROUND)
}

impl RayIndex for PickIndex {
    fn ray(&self, origin: DVec3, dir: DVec3) -> Option<Hit> {
        let mut best = self.flat.ray(origin, dir).map(|h| Hit {
            t: h.t,
            triangle: self.flat_ids[h.triangle] as usize,
        });
        let inv_dir = DVec3::ONE / dir;
        let test = |p: &Place, k: usize, best: &mut Option<Hit>| {
            let i = p.start + k;
            if let Some(t) = ray_triangle(origin, dir, self.placed(p, k))
                && Hit::nearer(t, i, *best)
            {
                *best = Some(Hit { t, triangle: i });
            }
        };
        self.top.ray_visit(origin, dir, limit(best), |j| {
            let p = &self.places[j];
            if !ray_meets_box(origin, inv_dir, limit(best), p.min, p.max) {
                return limit(best);
            }
            match p.inverse {
                Some(inv) => {
                    let (o, d) = (inv.transform_point3(origin), inv.transform_vector3(dir));
                    let bvh = &self.meshes[p.mesh as usize].0;
                    bvh.ray_visit(o, d, p.local_pad, limit(best), |k| {
                        test(p, k, &mut best);
                        limit(best)
                    });
                }
                // A flattening transform: every triangle (all degenerate).
                None => (0..p.len).for_each(|k| test(p, k, &mut best)),
            }
            limit(best)
        });
        best
    }

    fn triangle(&self, i: usize) -> [DVec3; 3] {
        let j = self.places.partition_point(|p| p.start <= i);
        match j.checked_sub(1).map(|j| &self.places[j]) {
            Some(p) if i < p.start + p.len => self.placed(p, i - p.start),
            Some(p) => self.flat.triangle(p.flat_before + i - (p.start + p.len)),
            None => self.flat.triangle(i),
        }
    }
}

/// Culling index over a scene's edges for picking ([`Scene::edges_near`]): the stored
/// edges in one index, the placements whose edges come from their mesh by their
/// world box, then by an index over their mesh's edges in its coordinates.
pub struct EdgeIndex {
    /// Edges indexed (the scene's count when built).
    len: usize,
    /// Over the stored edges, with the edge number of each.
    flat: BoxIndex,
    flat_ids: Vec<u32>,
    /// Over the world boxes of `lazy`'s placements.
    top: BoxIndex,
    /// Placements whose edges come from their mesh (and have some), their inverse
    /// and the growth of their box (in world units).
    lazy: Vec<(usize, Option<DMat4>, f64)>,
    /// Per mesh with many edges: an index over them in its coordinates.
    meshes: Vec<Option<BoxIndex>>,
}

impl EdgeIndex {
    pub fn build(scene: &Scene) -> Self {
        let v3 = |p: [f32; 3]| glam::Vec3::from(p).as_dvec3();
        let stored = scene.stored_edges();
        let flat = BoxIndex::segments(stored.iter().map(|s| [v3(s.a), v3(s.b)]));
        let inst = &scene.instancing;
        let mut flat_ids = Vec::with_capacity(stored.len());
        let mut e = 0;
        let mut lazy = Vec::new();
        let mut boxes = Vec::new();
        for (j, p) in inst.placements.iter().enumerate() {
            if !p.lazy_edges {
                continue;
            }
            flat_ids.extend((e..p.edges.start).map(|i| i as u32));
            e = p.edges.end;
            let mesh = &inst.meshes[p.mesh as usize];
            let Some(local) = bounds(mesh.edge_points.iter().copied()) else {
                continue;
            };
            let (min, max, pad) = world_box(&p.transform, local);
            // World segments' ends are rounded to single precision.
            let pad32 = 1e-6 * min.abs().max(max.abs()).max_element();
            boxes.push((min - pad32, max + pad32));
            lazy.push((j, inverse(&p.transform), pad + pad32));
        }
        flat_ids.extend((e..scene.edge_count()).map(|i| i as u32));
        debug_assert_eq!(flat_ids.len(), stored.len());
        let meshes = (inst.meshes.iter())
            .map(|m| {
                let n = m.local_edges.as_ref()?.len();
                (n >= MESH_EDGE_INDEX_MIN)
                    .then(|| BoxIndex::segments((0..n).map(|k| m.edge_ends(k))))
            })
            .collect();
        Self {
            len: scene.edge_count(),
            flat,
            flat_ids,
            top: BoxIndex::boxes(boxes.into_iter()),
            lazy,
            meshes,
        }
    }

    /// Number of edges indexed.
    pub fn len(&self) -> usize {
        self.len
    }

    /// See [`Scene::edges_near`].
    pub fn near(
        &self,
        scene: &Scene,
        camera: &Camera,
        viewport: DVec2,
        cursor: DVec2,
        radius: f64,
    ) -> Option<Vec<usize>> {
        let pyramid = Pyramid::new(camera, viewport, cursor, radius)?;
        let mut out: Vec<usize> = (self.flat.near_in(&pyramid, 0.0).into_iter())
            .map(|i| self.flat_ids[i] as usize)
            .collect();
        let round = ROUND * pyramid.scale();
        for k in self.top.near_in(&pyramid, 0.0) {
            let (j, inverse, pad) = self.lazy[k];
            let p = &scene.instancing.placements[j];
            match (&self.meshes[p.mesh as usize], inverse) {
                (Some(index), Some(inv)) => {
                    let local = pyramid.local(&p.transform, &inv);
                    let margin = 2.0 * pad + round;
                    out.extend(
                        (index.near_in(&local, margin).into_iter()).map(|i| p.edges.start + i),
                    );
                }
                _ => out.extend(p.edges.clone()),
            }
        }
        out.sort_unstable();
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench::{add_box, definition, grid, instance};
    use crate::document::Document;

    /// Boxes and a waved grid (many edges: its mesh gets an edge index) placed moved,
    /// turned, mirrored, unevenly scaled and tilted, nested two levels deep, with
    /// geometry of the root's own and enough edges for the culling indexes.
    fn model() -> mt_kernel::Model {
        let mut model = mt_kernel::Model::default();
        let mut ents = mt_kernel::Entities::default();
        add_box(&mut ents, DVec3::ZERO, DVec3::new(10.0, 6.0, 4.0));
        let leaf = model.definitions.insert(definition("Leaf", ents, false));
        let mesh = grid(6).root;
        let sheet = model.definitions.insert(definition("Sheet", mesh, false));
        let mut ents = mt_kernel::Entities::default();
        add_box(&mut ents, DVec3::Z * 20.0, DVec3::new(5.0, 5.0, 2.0));
        for k in 0..3 {
            let xf = DMat4::from_translation(DVec3::X * (12.0 * k as f64))
                * DMat4::from_rotation_z(0.3 * k as f64);
            ents.instances.push(instance(leaf, xf));
        }
        ents.instances.push(instance(
            sheet,
            DMat4::from_scale(DVec3::new(0.2, 0.3, 1.0)) * DMat4::from_rotation_x(0.4),
        ));
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
            DMat4::from_translation(DVec3::new(30.0, -40.0, 5.0))
                * DMat4::from_scale(DVec3::new(2.0, 0.5, 1.5)),
            DMat4::from_translation(DVec3::new(-30.0, -60.0, 0.0))
                * DMat4::from_axis_angle(DVec3::new(1.0, 1.0, 0.3).normalize(), 0.9)
                * DMat4::from_scale(DVec3::new(1.0, -2.0, 0.7)),
        ];
        for (k, xf) in xfs.into_iter().enumerate() {
            let def = [leaf, pair, sheet][k % 3];
            model.root.instances.push(instance(def, xf));
            let up = DMat4::from_translation(DVec3::new(0.0, 0.0, 60.0));
            model.root.instances.push(instance(pair, up * xf));
        }
        // Enough boxes for the culling indexes (over 1,024 edges).
        for k in 0..80 {
            let xf = DMat4::from_translation(DVec3::new(
                (k % 10) as f64 * 15.0 - 60.0,
                (k / 10) as f64 * 15.0 - 140.0,
                -20.0,
            ));
            model.root.instances.push(instance(leaf, xf));
        }
        model
    }

    fn bytes(s: &[mt_render::Segment]) -> &[u8] {
        bytemuck::cast_slice(s)
    }

    /// Picking (triangles from the meshes, edges culled per mesh), inference, the
    /// instances' bounds and every world triangle and edge are those of the
    /// flattened scene, in several views, with instanced edges given by the meshes or
    /// stored (edges colored by axis), in an open component and with hidden geometry.
    #[test]
    fn two_level_picking_matches_the_flattened_scene() {
        let mut doc = Document::untitled();
        doc.model = model();
        type Change = fn(&mut Document);
        let changes: [(&str, Change); 4] = [
            ("plain", |_| {}),
            ("edges by axis", |d| {
                d.model.style.edge_look.color_mode = mt_kernel::style_extras::EdgeColorMode::ByAxis;
            }),
            ("open a pair", |d| {
                d.model.style.edge_look.color_mode = Default::default();
                assert!(d.open_instance(3));
            }),
            ("hidden geometry", |d| {
                d.close_instance();
                d.model.style.hidden_geometry = true;
                let def = d.model.root.instances[1].definition;
                let ents = &mut d.model.definitions[def].entities;
                let e = ents.edges.keys().next().unwrap();
                ents.edges[e].attrs.hidden = true;
            }),
        ];
        let viewport = DVec2::new(640.0, 480.0);
        for (name, change) in changes {
            change(&mut doc);
            let scene = doc.scene();
            let flat = doc.scene_with(false);
            assert!(!scene.instancing.placements.is_empty(), "{name}");
            assert!(scene.stored_tris().len() < flat.tri_count() / 2, "{name}");
            if name != "edges by axis" {
                assert!(scene.stored_edges().len() < flat.edge_count() / 2, "{name}");
            }
            assert!(scene.edge_count() >= 1024, "{name}");
            // Every triangle and edge, one by one and all at once.
            assert_eq!(scene.triangles(), flat.triangles(), "{name}");
            assert_eq!(bytes(scene.edges()), bytes(flat.edges()), "{name}");
            for t in 0..scene.tri_count() {
                assert_eq!(scene.tri(t), flat.triangles()[t], "{name}");
            }
            for e in 0..scene.edge_count() {
                assert_eq!(bytes(&[scene.edge(e)]), bytes(&[flat.edges()[e]]), "{name}");
            }
            assert_eq!(scene.hidden_edges, flat.hidden_edges, "{name}");
            assert_eq!(scene.edge_container, flat.edge_container, "{name}");
            assert_eq!(scene.tri_container, flat.tri_container, "{name}");
            let checksum = crate::bench::checksum;
            assert_eq!(checksum(&scene), checksum(&flat), "{name}");
            let two = PickIndex::build(&scene, |t| Box::new(Bvh::build(t)));
            let one = Bvh::build(flat.triangles().to_vec());
            for t in (0..scene.tri_count()).step_by(3) {
                assert_eq!(two.triangle(t), one.triangle(t), "{name}");
            }
            // The instances' bounds.
            let ents = doc.active();
            let to_world = |i: usize| {
                (ents.instances.get(i)).map(|inst| doc.drawn_transform(inst.transform, inst))
            };
            assert_eq!(
                crate::selection::instance_bounds(&scene, to_world),
                crate::selection::instance_bounds(&flat, to_world),
                "{name}"
            );
            let ctx_xf = doc.context_transform();
            let geometry = crate::snap::geometry(&scene, Some(&doc), ctx_xf);
            let flat_geometry = crate::snap::geometry(&flat, Some(&doc), ctx_xf);
            assert_eq!(geometry.edges, flat_geometry.edges, "{name}");
            assert_eq!(geometry.vertices, flat_geometry.vertices, "{name}");
            let mut infer = mt_infer::Engine::default();
            let mut flat_infer = mt_infer::Engine::default();
            let mut hits = 0;
            for (eye, perspective) in [
                (DVec3::new(-150.0, -260.0, 190.0), true),
                (DVec3::new(120.0, 200.0, 140.0), true),
                (DVec3::new(10.0, -30.0, 300.0), false),
            ] {
                let camera = Camera {
                    eye,
                    target: DVec3::new(0.0, -20.0, 10.0),
                    fov_deg: 40.0,
                    perspective,
                    two_point: false,
                };
                let view = mt_infer::View {
                    camera: &camera,
                    viewport,
                };
                for k in 0..40 * 30 {
                    let cursor =
                        DVec2::new((k % 40) as f64 * 16.0 + 3.5, (k / 40) as f64 * 16.0 + 2.5);
                    let ray = camera.ray(cursor, viewport);
                    let hit = two.ray(ray.origin, ray.dir);
                    assert_eq!(hit, one.ray(ray.origin, ray.dir), "{name} {cursor}");
                    hits += usize::from(hit.is_some());
                    let pick = crate::selection::pick;
                    let picked = pick(&scene, &*two, &camera, cursor, viewport);
                    let want = pick(&flat, &one, &camera, cursor, viewport);
                    assert_eq!(picked, want, "{name} {cursor}");
                    if k % 7 == 0 {
                        let a = infer.infer(&geometry, &*two, view, cursor, None);
                        let b = flat_infer.infer(&flat_geometry, &one, view, cursor, None);
                        assert_eq!(format!("{a:?}"), format!("{b:?}"), "{name} {cursor}");
                    }
                }
            }
            assert!(hits > 500, "{name}: {hits} hits");
            // Rays from everywhere, in every direction.
            let mut seed = 7u64;
            let mut rnd = || {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (seed >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
            };
            let mut hits = 0;
            for _ in 0..3000 {
                let o = DVec3::new(rnd(), rnd(), rnd()) * 150.0;
                let to = DVec3::new(rnd() * 60.0, rnd() * 100.0 - 40.0, rnd() * 40.0 + 20.0);
                let d = (to - o).normalize();
                let hit = two.ray(o, d);
                assert_eq!(hit, one.ray(o, d), "{name} {o} {d}");
                hits += usize::from(hit.is_some());
            }
            assert!(hits > 300, "{name}: {hits} random hits");
        }
    }
}
