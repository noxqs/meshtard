//! What the inference engine snaps to, built from the drawn scene (FEATURES.md 49).
//!
//! Every visible edge and face is snapped to: the open context's own geometry, the
//! geometry inside its groups and components, and the faded rest of the model. Points
//! inside a group or component carry it ([`mt_infer::Container`]): the overlay draws
//! them magenta with an "in Group" / "in Component" tooltip, as SketchUp's help
//! describes. The active section plane's cut lines give "On Section", the open
//! context's instances "Component Origin" and its arcs "Midpoint of Arc".

use glam::{DMat4, DVec3};
use mt_infer::Geometry;

use crate::document::{Document, Owner, Scene};

/// The snap geometry of `scene` (world space); `ctx_xf` is the open context's
/// transform (its arcs give the "center" inference). The app uses [`geometry_kept`];
/// tests and the benchmark compare it with this.
#[cfg(test)]
pub fn geometry(scene: &Scene, doc: Option<&Document>, ctx_xf: DMat4) -> Geometry {
    let v3 = |p: [f32; 3]| DVec3::new(p[0].into(), p[1].into(), p[2].into());
    // Dimension and text lines and hidden geometry are not snapped to.
    let mut kept: Vec<usize> = Vec::new();
    let mut ends = Vec::new();
    scene.for_each_edge(|i, s| {
        if !matches!(
            scene.edge_owner[i],
            Some(Owner::Dimension(_) | Owner::Text(_))
        ) && !scene.hidden_edges.contains(&i)
        {
            kept.push(i);
            ends.push([v3(s.a), v3(s.b)]);
        }
    });
    let mut geometry = Geometry::from_edges(ends);
    geometry.edge_container = kept
        .iter()
        .map(|&i| scene.edge_container.get(i).copied().flatten())
        .collect();
    geometry.tri_container = (0..scene.tri_count())
        .map(|i| scene.tri_container.get(i).copied().flatten())
        .collect();
    let Some(doc) = doc else {
        return geometry;
    };
    if doc.model.style.guides {
        for g in &doc.model.root.guides {
            match g.dir {
                Some(d) => geometry.guide_lines.push((g.point, d)),
                None => geometry.guide_points.push(g.point),
            }
        }
    }
    // Arcs and circles of the open context, for the "center" inference. Indices are
    // into the kept edges.
    let ents = doc.active();
    let mut arcs: std::collections::BTreeMap<_, (mt_kernel::Arc, Vec<usize>)> =
        std::collections::BTreeMap::new();
    for (k, &i) in kept.iter().enumerate() {
        let Some(Some(Owner::Edge(e))) = scene.edge_owner.get(i) else {
            continue;
        };
        let Some(c) = ents.edges.get(*e).and_then(|e| e.attrs.curve) else {
            continue;
        };
        if let Some(arc) = ents.curves.get(c).and_then(|c| c.arc) {
            arcs.entry(c).or_insert((arc, Vec::new())).1.push(k);
        }
    }
    add_arcs(&mut geometry, arcs, ctx_xf);
    geometry.component_origins = origins(ents, ctx_xf);
    geometry
}

/// Edges per block of [`Kept`].
const BLOCK: usize = 1 << 14;

/// A vertex's key for de-duplication (as `mt_infer::Geometry::from_edges`).
fn key(p: DVec3) -> [i64; 3] {
    ((p * 1e4).round().as_i64vec3()).to_array()
}

/// What one block of the scene's edges gives the snap geometry.
struct Block {
    /// The scene's edges it is of.
    range: std::ops::Range<usize>,
    /// The edges snapped to (by scene number) and their ends.
    kept: Vec<u32>,
    ends: Vec<[DVec3; 2]>,
    /// Their distinct ends, in first-seen order, and their keys sorted.
    points: Vec<([i64; 3], DVec3)>,
    keys: Vec<[i64; 3]>,
    /// Kept edges (position in `kept`) of the open context on a curve.
    curved: Vec<(u32, mt_kernel::EdgeId)>,
}

impl Block {
    fn new(scene: &Scene, doc: Option<&Document>, range: std::ops::Range<usize>) -> Self {
        let v3 = |p: [f32; 3]| DVec3::new(p[0].into(), p[1].into(), p[2].into());
        let ents = doc.map(Document::active);
        let mut b = Self {
            range: range.clone(),
            kept: Vec::new(),
            ends: Vec::new(),
            points: Vec::new(),
            keys: Vec::new(),
            curved: Vec::new(),
        };
        let mut seen = mt_kernel::spatial::FxSet::default();
        scene.for_each_edge_in(range, |i, s| {
            let owner = scene.edge_owner[i];
            if matches!(owner, Some(Owner::Dimension(_) | Owner::Text(_)))
                || scene.hidden_edges.contains(&i)
            {
                return;
            }
            if let (Some(Owner::Edge(e)), Some(ents)) = (owner, ents)
                && ents.edges.get(e).is_some_and(|e| e.attrs.curve.is_some())
            {
                b.curved.push((b.kept.len() as u32, e));
            }
            b.kept.push(i as u32);
            let ends = [v3(s.a), v3(s.b)];
            for p in ends {
                if seen.insert(key(p)) {
                    b.points.push((key(p), p));
                }
            }
            b.ends.push(ends);
        });
        b.keys = b.points.iter().map(|(k, _)| *k).collect();
        b.keys.sort_unstable();
        b
    }
}

/// The keys in one of `a` and `b` (sorted lists) but not both.
fn either(a: &[[i64; 3]], b: &[[i64; 3]], out: &mut Vec<[i64; 3]>) {
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        match (a.get(i), b.get(j)) {
            (Some(x), Some(y)) if x == y => {
                i += 1;
                j += 1;
            }
            (Some(x), Some(y)) if x < y => {
                out.push(*x);
                i += 1;
            }
            (Some(x), None) => {
                out.push(*x);
                i += 1;
            }
            (_, Some(y)) => {
                out.push(*y);
                j += 1;
            }
            (None, None) => break,
        }
    }
}

/// What [`geometry_kept`] keeps for the next build: the snap geometry of blocks of
/// the scene's edges, and the scene they are of.
#[derive(Default)]
pub struct Kept {
    build: u64,
    blocks: Vec<std::sync::Arc<Block>>,
    /// Kept edges and distinct vertices up to the end of each block.
    edge_ends: Vec<usize>,
    vertex_ends: Vec<usize>,
    /// Each vertex's key: the block it is first seen in, and the pass that saw it
    /// there (an entry holds while that block was last looked at in that pass).
    firsts: mt_kernel::spatial::FxMap<[i64; 3], (u32, u32)>,
    /// Per block, the last pass that looked at it.
    looked: Vec<u32>,
    pass: u32,
}

/// [`geometry`], keeping per block of [`BLOCK`] edges what the next build can reuse:
/// the blocks the scene's changes (`Scene::changes`) left as they were are not looked
/// at again, and `old` (the geometry made last time) is rewritten where they changed
/// when they still hold as many edges. The vertices are de-duplicated as before, in
/// the same first-seen order: those first seen before the first changed block are
/// kept, and each vertex's first block is remembered so the later blocks only look
/// theirs up.
pub fn geometry_kept(
    scene: &Scene,
    doc: Option<&Document>,
    ctx_xf: DMat4,
    kept: &mut Kept,
    mut old: Geometry,
) -> Geometry {
    let same = kept.build != 0 && scene.changes.from == Some(kept.build);
    // Its culling index (when built): kept where its items stay the same.
    let old_index = old.take_index();
    let blocks_of = |len: usize| len.div_ceil(mt_infer::INDEX_BLOCK);
    let index_range = |k: usize, len: usize| {
        k * mt_infer::INDEX_BLOCK..((k + 1) * mt_infer::INDEX_BLOCK).min(len)
    };
    let old_blocks = std::mem::take(&mut kept.blocks);
    let n = scene.edge_count();
    let range = |k: usize| k * BLOCK..((k + 1) * BLOCK).min(n);
    let mut blocks: Vec<Option<std::sync::Arc<Block>>> = (0..n.div_ceil(BLOCK))
        .map(|k| {
            let r = range(k);
            (old_blocks.get(k))
                .filter(|_| same && scene.changes.edges.covers(r.clone()))
                .filter(|b| b.range == r)
                .cloned()
        })
        .collect();
    let todo: Vec<usize> = (0..blocks.len()).filter(|&k| blocks[k].is_none()).collect();
    let made = crate::pick_index::parallel(todo.clone(), |k| {
        std::sync::Arc::new(Block::new(scene, doc, range(k)))
    });
    for (&k, b) in todo.iter().zip(made) {
        blocks[k] = Some(b);
    }
    let blocks: Vec<std::sync::Arc<Block>> = blocks.into_iter().map(|b| b.expect("made")).collect();
    // The first block that changed (or that the last build did not have).
    let first = match (same, todo.first()) {
        (false, _) => 0,
        (true, Some(&k)) => k.min(old_blocks.len()),
        (true, None) => blocks.len().min(old_blocks.len()),
    };
    let mut edge_ends = Vec::with_capacity(blocks.len());
    let mut total = 0;
    for b in &blocks {
        total += b.kept.len();
        edge_ends.push(total);
    }
    let container = |i: &u32| scene.edge_container.get(*i as usize).copied().flatten();
    let mut geometry = Geometry::default();
    // Per block of the culling index: whether its edges are the same as before.
    let mut edges_same = vec![false; blocks_of(total)];
    if same && edge_ends == kept.edge_ends && old.edges.len() == total {
        // As many edges per block as last time: the changed blocks are rewritten.
        geometry.edges = std::mem::take(&mut old.edges);
        geometry.edge_container = std::mem::take(&mut old.edge_container);
        geometry.edge_container.resize(total, None);
        edges_same.fill(true);
        for &k in &todo {
            let r = edge_ends[k] - blocks[k].kept.len()..edge_ends[k];
            geometry.edges[r.clone()].copy_from_slice(&blocks[k].ends);
            for (c, i) in geometry.edge_container[r.clone()]
                .iter_mut()
                .zip(&blocks[k].kept)
            {
                *c = container(i);
            }
            if !r.is_empty() {
                let b = mt_infer::INDEX_BLOCK;
                edges_same[r.start / b..=(r.end - 1) / b].fill(false);
            }
        }
    } else {
        geometry.edges.reserve(total);
        geometry.edge_container.reserve(total);
        for b in &blocks {
            geometry.edges.extend_from_slice(&b.ends);
            geometry.edge_container.extend(b.kept.iter().map(container));
        }
        for (k, s) in edges_same.iter_mut().enumerate() {
            let r = index_range(k, total);
            *s = old.edges.get(r.clone()) == Some(&geometry.edges[r]);
        }
    }
    // Vertices. Their first-seen order changes only from the first changed block on,
    // and there only in changed blocks and in those holding a vertex a changed block
    // gained or lost: the others give the vertices they gave last time.
    let mut old_vertices = std::mem::take(&mut old.vertices);
    let old_ends = std::mem::take(&mut kept.vertex_ends);
    let keep_vertices = first > 0
        && old_ends.len() >= old_blocks.len()
        && old_ends.last().is_some_and(|&n| n == old_vertices.len())
        && kept.firsts.len() <= 4 * old_vertices.len() + 1024;
    let first = if keep_vertices { first } else { 0 };
    if first == 0 {
        kept.firsts.clear();
        kept.looked.clear();
    }
    // Keys that changed blocks (or blocks gone) gained or lost.
    let mut moved: Vec<[i64; 3]> = Vec::new();
    if first > 0 {
        for &k in &todo {
            let none: &[[i64; 3]] = &[];
            let was = old_blocks.get(k).map_or(none, |b| &b.keys[..]);
            either(was, &blocks[k].keys, &mut moved);
        }
        for b in old_blocks.iter().skip(blocks.len()) {
            moved.extend_from_slice(&b.keys);
        }
        moved.sort_unstable();
        moved.dedup();
    }
    let changed: mt_kernel::spatial::FxSet<usize> = todo.iter().copied().collect();
    kept.pass += 1;
    let pass = kept.pass;
    kept.looked.resize(blocks.len(), 0);
    let mut vertices: Vec<DVec3> = Vec::with_capacity(old_vertices.len());
    vertices.extend_from_slice(&old_vertices[..if first > 0 { old_ends[first - 1] } else { 0 }]);
    let mut ends = old_ends[..first].to_vec();
    for (j, b) in blocks.iter().enumerate().skip(first) {
        let affected = first == 0
            || changed.contains(&j)
            || j >= old_ends.len()
            || moved.iter().any(|k| b.keys.binary_search(k).is_ok());
        if !affected {
            let start = if j == 0 { 0 } else { old_ends[j - 1] };
            vertices.extend_from_slice(&old_vertices[start..old_ends[j]]);
            ends.push(vertices.len());
            continue;
        }
        kept.looked[j] = pass;
        for &(key, p) in &b.points {
            let looked = &kept.looked;
            match kept.firsts.get_mut(&key) {
                // First seen in an earlier block that holds it.
                Some((at, seen)) if (*at as usize) < j && looked[*at as usize] == *seen => {}
                Some(entry) => {
                    *entry = (j as u32, pass);
                    vertices.push(p);
                }
                None => {
                    kept.firsts.insert(key, (j as u32, pass));
                    vertices.push(p);
                }
            }
        }
        ends.push(vertices.len());
    }
    kept.vertex_ends = ends;
    let vertices_same: Vec<bool> = (0..blocks_of(vertices.len()))
        .map(|k| {
            let r = index_range(k, vertices.len());
            old_vertices.get(r.clone()) == Some(&vertices[r])
        })
        .collect();
    old_vertices.clear();
    geometry.vertices = vertices;
    if let Some(index) = old_index {
        let b = mt_infer::INDEX_BLOCK;
        geometry.keep_index(
            index,
            |r| edges_same[r.start / b],
            |r| vertices_same[r.start / b],
        );
    }
    geometry.tri_container = (0..scene.tri_count())
        .map(|i| scene.tri_container.get(i).copied().flatten())
        .collect();
    let mut curved = Vec::new();
    let mut at = 0;
    for b in &blocks {
        curved.extend((b.curved.iter()).map(|&(k, e)| (at + k as usize, e)));
        at += b.kept.len();
    }
    kept.build = scene.build;
    kept.blocks = blocks;
    kept.edge_ends = edge_ends;
    finish(&mut geometry, doc, ctx_xf, curved)
}

/// The rest of the snap geometry (guides, arcs, component origins); `curved` are the
/// kept edges (by position) of the open context that lie on a curve.
fn finish(
    geometry: &mut Geometry,
    doc: Option<&Document>,
    ctx_xf: DMat4,
    curved: Vec<(usize, mt_kernel::EdgeId)>,
) -> Geometry {
    let mut geometry = std::mem::take(geometry);
    let Some(doc) = doc else {
        return geometry;
    };
    if doc.model.style.guides {
        for g in &doc.model.root.guides {
            match g.dir {
                Some(d) => geometry.guide_lines.push((g.point, d)),
                None => geometry.guide_points.push(g.point),
            }
        }
    }
    // Arcs and circles of the open context, for the "center" inference. Indices are
    // into the kept edges.
    let ents = doc.active();
    let mut arcs: std::collections::BTreeMap<_, (mt_kernel::Arc, Vec<usize>)> =
        std::collections::BTreeMap::new();
    for (k, e) in curved {
        let Some(c) = ents.edges.get(e).and_then(|e| e.attrs.curve) else {
            continue;
        };
        if let Some(arc) = ents.curves.get(c).and_then(|c| c.arc) {
            arcs.entry(c).or_insert((arc, Vec::new())).1.push(k);
        }
    }
    add_arcs(&mut geometry, arcs, ctx_xf);
    geometry.component_origins = origins(ents, ctx_xf);
    geometry
}

fn add_arcs(
    geometry: &mut Geometry,
    arcs: std::collections::BTreeMap<mt_kernel::CurveId, (mt_kernel::Arc, Vec<usize>)>,
    ctx_xf: DMat4,
) {
    for (arc, edges) in arcs.into_values() {
        let curve = mt_infer::ArcCurve {
            center: ctx_xf.transform_point3(arc.center),
            normal: ctx_xf.transform_vector3(arc.normal).normalize_or_zero(),
            radius: ctx_xf.transform_vector3(arc.x_axis * arc.radius).length(),
            midpoint: (!arc.is_circle()).then(|| {
                let m = (arc.start_angle + arc.end_angle) / 2.0;
                let local =
                    arc.center + (arc.x_axis * m.cos() + arc.y_axis() * m.sin()) * arc.radius;
                ctx_xf.transform_point3(local)
            }),
        };
        geometry.add_arc(curve, edges);
    }
}

/// Origins of the open context's visible groups and components ("Component Origin").
fn origins(ents: &mt_kernel::Entities, ctx_xf: DMat4) -> Vec<DVec3> {
    ents.instances
        .iter()
        .filter(|i| !i.hidden)
        .map(|i| (ctx_xf * i.transform).transform_point3(DVec3::ZERO))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_infer::{Container, Engine, Kind, View};
    use mt_kernel::bvh::Bvh;
    use mt_render::Camera;

    /// A 100 x 100 square face drawn in the open context, and one inside a group 200
    /// to the right, and one inside a component 200 behind.
    fn model() -> Document {
        let mut doc = Document::untitled();
        let sq = |ents: &mut mt_kernel::Entities, o: DVec3| {
            let p = [
                o,
                o + DVec3::X * 100.0,
                o + DVec3::new(100.0, 100.0, 0.0),
                o + DVec3::Y * 100.0,
            ];
            for i in 0..4 {
                mt_kernel::add_line(ents, p[i], p[(i + 1) % 4]);
            }
        };
        doc.edit_model(|m, root| {
            sq(root, DVec3::ZERO);
            for (o, group) in [(DVec3::X * 200.0, true), (DVec3::Y * 200.0, false)] {
                let mut ents = mt_kernel::Entities::default();
                sq(&mut ents, DVec3::ZERO);
                let d = m.definitions.insert(mt_kernel::Definition {
                    name: if group { "Group#1" } else { "Box" }.into(),
                    is_group: group,
                    entities: ents,
                    ..Default::default()
                });
                root.instances.push(mt_kernel::Instance {
                    definition: d,
                    transform: DMat4::from_translation(o),
                    name: String::new(),
                    hidden: false,
                    locked: false,
                    layer: None,
                    material: None,
                    attrs: Default::default(),
                });
            }
            true
        });
        doc
    }

    fn infer(doc: &Document, p: DVec3) -> mt_infer::Inference {
        let scene = doc.scene();
        let g = geometry(&scene, Some(doc), doc.context_transform());
        let bvh = Bvh::build(scene.triangles().to_vec());
        let camera = Camera {
            eye: DVec3::new(150.0, -400.0, 500.0),
            target: DVec3::new(150.0, 150.0, 0.0),
            fov_deg: 35.0,
            perspective: true,
            two_point: false,
        };
        let viewport = glam::DVec2::new(800.0, 600.0);
        let cursor = camera.project(p, viewport).unwrap();
        Engine::default().infer(
            &g,
            &bvh,
            View {
                camera: &camera,
                viewport,
            },
            cursor,
            None,
        )
    }

    #[test]
    fn context_and_nested_geometry_is_inferred() {
        let doc = model();
        let i = infer(&doc, DVec3::new(100.0, 100.0, 0.0));
        assert_eq!((i.kind, i.container), (Kind::Endpoint, None));
        let i = infer(&doc, DVec3::new(250.0, 0.0, 0.0));
        assert_eq!(
            (i.kind, i.container),
            (Kind::Midpoint, Some(Container::Group))
        );
        assert_eq!(i.label(), "Midpoint in Group");
        let i = infer(&doc, DVec3::new(60.0, 260.0, 0.0));
        assert_eq!(i.kind, Kind::OnFace);
        assert_eq!(i.label(), "On Face in Component");
    }

    #[test]
    fn inside_an_open_group_its_own_geometry_is_plain() {
        let mut doc = model();
        let group = doc
            .active()
            .instances
            .iter()
            .position(|i| doc.model.definitions[i.definition].is_group)
            .unwrap();
        assert!(doc.open_instance(group));
        // The open group's own points: plain.
        let i = infer(&doc, DVec3::new(300.0, 100.0, 0.0));
        assert_eq!((i.kind, i.container), (Kind::Endpoint, None));
        let i = infer(&doc, DVec3::new(260.0, 60.0, 0.0));
        assert_eq!((i.kind, i.container), (Kind::OnFace, None));
        // The component outside it is still inferred, in magenta.
        let i = infer(&doc, DVec3::new(100.0, 250.0, 0.0));
        assert_eq!(
            (i.kind, i.container),
            (Kind::Midpoint, Some(Container::Component))
        );
        // The rest of the model's loose geometry: plain.
        let i = infer(&doc, DVec3::new(0.0, 0.0, 0.0));
        assert_eq!((i.kind, i.container), (Kind::Endpoint, None));
    }

    /// An arc of `n` segments around `c` in the open context.
    fn arc(doc: &mut Document, c: DVec3, n: usize) {
        let arc = mt_kernel::Arc {
            center: c,
            normal: DVec3::Z,
            x_axis: DVec3::X,
            radius: 30.0,
            start_angle: 0.0,
            end_angle: 2.0,
            segments: n,
        };
        let points: Vec<DVec3> = (0..=n)
            .map(|k| {
                let a = 2.0 * k as f64 / n as f64;
                c + DVec3::new(a.cos(), a.sin(), 0.0) * 30.0
            })
            .collect();
        doc.edit(|e| !mt_kernel::ops::add_curve(e, &points, Some(arc)).is_empty());
    }

    /// The snap geometry kept per block between builds is the one made in full, after
    /// edits big and small, arcs, hidden edges and a group opened.
    #[test]
    fn kept_snap_geometry_equals_a_full_build() {
        // A grid of many blocks of edges at the root, a group and a component.
        let mut doc = model();
        let instances = std::mem::take(&mut doc.model.root.instances);
        doc.model.root = crate::bench::grid(130).root;
        doc.model.root.instances = instances;
        let mut kept = Kept::default();
        let mut previous = Geometry::default();
        let mut scene = doc.scene_from(None);
        type Action = fn(&mut Document);
        let actions: [(&str, Action); 8] = [
            ("nothing", |_| {}),
            ("arc", |d| arc(d, DVec3::new(500.0, 500.0, 50.0), 12)),
            ("move a face", |d| {
                let f = d.active().faces.keys().nth(20_000).unwrap();
                d.edit(|e| {
                    let xf = DMat4::from_translation(DVec3::Z * 3.0);
                    crate::modify::apply(e, &[Owner::Face(f)], xf, false);
                    true
                });
            }),
            ("hide edges", |d| {
                d.model.style.hidden_geometry = true;
                let edges: Vec<Owner> = (d.active().edges.keys())
                    .step_by(1000)
                    .map(Owner::Edge)
                    .collect();
                d.hide(&edges);
            }),
            ("erase many", |d| {
                let edges: Vec<Owner> = (d.active().edges.keys())
                    .skip(5000)
                    .step_by(3)
                    .take(3000)
                    .map(Owner::Edge)
                    .collect();
                d.erase(&edges, false, false);
            }),
            ("another arc", |d| arc(d, DVec3::new(100.0, 900.0, 20.0), 7)),
            ("move a tile", |d| {
                d.edit(|e| {
                    let xf = DMat4::from_translation(DVec3::Y);
                    crate::modify::apply(e, &[Owner::Instance(13)], xf, false);
                    true
                });
            }),
            ("open the group", |d| {
                let i = (d.active().instances.iter())
                    .position(|i| d.model.definitions[i.definition].is_group)
                    .unwrap();
                assert!(d.open_instance(i));
            }),
        ];
        for (name, action) in actions {
            action(&mut doc);
            scene = doc.scene_from(Some(scene));
            let xf = doc.context_transform();
            let full = geometry(&scene, Some(&doc), xf);
            let last = std::mem::take(&mut previous);
            let part = geometry_kept(&scene, Some(&doc), xf, &mut kept, last);
            assert_same(&full, &part, name);
            previous = part;
            assert!(!full.vertices.is_empty(), "{name}");
            if name.contains("arc") {
                assert!(!full.arcs.is_empty(), "{name}");
            }
        }
    }

    /// Random edits all over a grid of several blocks (moves, erasing, new lines
    /// joining old vertices, undo and redo): the kept snap geometry, vertices in
    /// first-seen order included, is the one made in full after each.
    #[test]
    fn kept_snap_geometry_follows_random_edits() {
        let mut doc = Document::untitled();
        doc.model = crate::bench::grid(190);
        let mut seed = 99u64;
        let mut rnd = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let mut kept = Kept::default();
        let mut previous = Geometry::default();
        let mut scene = doc.scene_from(None);
        for step in 0..60 {
            let faces: Vec<FaceIdOf> = doc.active().faces.keys().collect();
            let edges: Vec<EdgeIdOf> = doc.active().edges.keys().collect();
            match rnd(5) {
                0 | 1 => {
                    let at = rnd(faces.len());
                    let some: Vec<Owner> = (faces[at..].iter())
                        .take(1 + rnd(40))
                        .map(|&f| Owner::Face(f))
                        .collect();
                    let by = DVec3::new(rnd(3) as f64, rnd(3) as f64, 1.0 + rnd(5) as f64);
                    doc.edit(|e| {
                        crate::modify::apply(e, &some, DMat4::from_translation(by), false);
                        true
                    });
                }
                2 => {
                    let at = rnd(edges.len());
                    let some: Vec<Owner> = (edges[at..].iter())
                        .step_by(1 + rnd(5))
                        .take(1 + rnd(30))
                        .map(|&e| Owner::Edge(e))
                        .collect();
                    doc.erase(&some, false, false);
                }
                3 => {
                    // Lines between grid points far apart (old vertices, new edges).
                    let p = |k: usize| {
                        DVec3::new((k % 190) as f64 * 10.0, (k / 190) as f64 * 10.0, 50.0)
                    };
                    let (a, b) = (rnd(190 * 190), rnd(190 * 190));
                    doc.edit(|e| !mt_kernel::add_line(e, p(a), p(b)).is_empty());
                }
                _ => {
                    if !doc.undo() {
                        doc.redo();
                    }
                }
            }
            scene = doc.scene_from(Some(scene));
            let full = geometry(&scene, Some(&doc), DMat4::IDENTITY);
            let last = std::mem::take(&mut previous);
            let part = geometry_kept(&scene, Some(&doc), DMat4::IDENTITY, &mut kept, last);
            assert_same(&full, &part, &format!("step {step}"));
            // The culling indexes (the kept one keeps blocks of the last) find the
            // same snaps.
            let bvh = Bvh::build(scene.triangles().to_vec());
            let camera = Camera {
                eye: DVec3::new(-300.0, -400.0, 900.0),
                target: DVec3::new(900.0, 900.0, 0.0),
                fov_deg: 35.0,
                perspective: true,
                two_point: false,
            };
            let viewport = glam::DVec2::new(800.0, 600.0);
            let view = View {
                camera: &camera,
                viewport,
            };
            for k in 0..40 {
                let cursor =
                    glam::DVec2::new((k % 8) as f64 * 100.0 + 7.0, (k / 8) as f64 * 120.0 + 9.0);
                let a = Engine::default().infer(&full, &bvh, view, cursor, None);
                let b = Engine::default().infer(&part, &bvh, view, cursor, None);
                assert_eq!(format!("{a:?}"), format!("{b:?}"), "step {step} {k}");
            }
            previous = part;
        }
    }

    /// Every field of the snap geometry is the same.
    fn assert_same(a: &Geometry, b: &Geometry, what: &str) {
        assert_eq!(a.vertices, b.vertices, "{what}: vertices");
        assert_eq!(a.edges, b.edges, "{what}: edges");
        assert_eq!(a.guide_lines, b.guide_lines, "{what}");
        assert_eq!(a.guide_points, b.guide_points, "{what}");
        assert_eq!(a.arcs, b.arcs, "{what}: arcs");
        assert_eq!(a.edge_arc, b.edge_arc, "{what}");
        assert_eq!(a.edge_container, b.edge_container, "{what}");
        assert_eq!(a.tri_container, b.tri_container, "{what}");
        assert_eq!(a.section_lines, b.section_lines, "{what}");
        assert_eq!(a.component_origins, b.component_origins, "{what}");
    }

    type FaceIdOf = mt_kernel::FaceId;
    type EdgeIdOf = mt_kernel::EdgeId;
}
