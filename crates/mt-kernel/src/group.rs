//! Groups and components: Make Group / Make Component, Explode, Make Unique.
//!
//! Behaviors are drafts from memory of SketchUp (`spec/parity/STATUS.md`):
//! - The new definition's origin is the minimum corner of the selection's bounding box,
//!   axes parallel to the parent's (unverified for groups; components let the user
//!   choose axes in a dialog, not implemented).
//! - Edges of the selection that also bound unselected faces stay outside too (copied
//!   into the group), so the outside faces survive (unverified).
//! - Explode merges the contents with the surrounding geometry at shared vertices and
//!   overlapping edges (vertex welding); edges crossing mid-span are not split.

use std::collections::HashMap;

use glam::{DMat3, DMat4, DVec3};
use slotmap::SlotMap;

use crate::entities::{
    CoEdge, Curve, CurveId, DefinitionId, EdgeId, Entities, FaceId, Instance, Loop, VertexId,
};
use crate::model::{Definition, Model};

/// Copies the given edges and faces (plus the edges of the faces) of `src` into `dst`
/// transformed by `xf`, without merging. Returns the new vertices.
pub fn copy_raw(
    dst: &mut Entities,
    src: &Entities,
    edges: &[EdgeId],
    faces: &[FaceId],
    xf: DMat4,
) -> Vec<VertexId> {
    let mirror = DMat3::from_mat4(xf).determinant() < 0.0;
    let normal_xf = DMat3::from_mat4(xf).inverse().transpose();
    let mut vmap: HashMap<VertexId, VertexId> = HashMap::new();
    let mut emap: HashMap<EdgeId, EdgeId> = HashMap::new();
    let mut cmap: HashMap<CurveId, CurveId> = HashMap::new();
    let all_edges: Vec<EdgeId> = edges
        .iter()
        .copied()
        .chain(
            faces
                .iter()
                .filter_map(|f| src.faces.get(*f))
                .flat_map(|f| {
                    f.loops()
                        .flat_map(|l| l.coedges.iter().map(|c| c.edge))
                        .collect::<Vec<_>>()
                }),
        )
        .collect();
    for e in all_edges {
        if emap.contains_key(&e) {
            continue;
        }
        let Some(edge) = src.edges.get(e) else {
            continue;
        };
        let mut v = |id: VertexId, dst: &mut Entities| {
            *vmap
                .entry(id)
                .or_insert_with(|| dst.add_vertex(xf.transform_point3(src.pos(id))))
        };
        let (a, b) = (v(edge.start, dst), v(edge.end, dst));
        let mut attrs = edge.attrs.clone();
        attrs.curve = attrs.curve.and_then(|c| {
            let curve = src.curves.get(c)?;
            Some(*cmap.entry(c).or_insert_with(|| {
                dst.curves.insert(Curve {
                    arc: curve.arc.map(|a| transform_arc(a, xf)),
                })
            }))
        });
        emap.insert(e, dst.add_raw_edge(a, b, attrs));
    }
    for f in faces {
        let Some(face) = src.faces.get(*f) else {
            continue;
        };
        let map_loop = |l: &Loop| {
            let mut coedges: Vec<CoEdge> = l
                .coedges
                .iter()
                .map(|c| CoEdge {
                    edge: emap[&c.edge],
                    reversed: c.reversed,
                })
                .collect();
            if mirror {
                coedges.reverse();
                for c in &mut coedges {
                    c.reversed = !c.reversed;
                }
            }
            Loop { coedges }
        };
        // With a mirror the reversed loops still run counter-clockwise around the
        // transformed normal.
        let normal = (normal_xf * face.normal).normalize_or_zero();
        let outer = map_loop(&face.outer);
        let holes = face.holes.iter().map(map_loop).collect();
        dst.add_raw_face(outer, holes, normal, face.attrs.clone());
    }
    vmap.into_values().collect()
}

fn transform_arc(a: crate::entities::Arc, xf: DMat4) -> crate::entities::Arc {
    let x = xf.transform_vector3(a.x_axis);
    let y = xf.transform_vector3(a.y_axis());
    let normal = x.cross(y).normalize_or_zero();
    crate::entities::Arc {
        center: xf.transform_point3(a.center),
        normal,
        x_axis: x.normalize_or_zero(),
        radius: a.radius * x.length(),
        ..a
    }
}

/// What to group: edges, faces and instance indices of one `Entities`.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    pub edges: Vec<EdgeId>,
    pub faces: Vec<FaceId>,
    pub instances: Vec<usize>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty() && self.faces.is_empty() && self.instances.is_empty()
    }
}

fn bounds(
    ents: &Entities,
    sel: &Selection,
    defs: &SlotMap<DefinitionId, Definition>,
) -> Option<(DVec3, DVec3)> {
    let mut min = DVec3::splat(f64::INFINITY);
    let mut max = DVec3::splat(f64::NEG_INFINITY);
    let mut add = |p: DVec3| {
        min = min.min(p);
        max = max.max(p);
    };
    for e in &sel.edges {
        if let Some(edge) = ents.edges.get(*e) {
            add(ents.pos(edge.start));
            add(ents.pos(edge.end));
        }
    }
    for f in &sel.faces {
        if let Some(face) = ents.faces.get(*f) {
            face.loops()
                .flat_map(|l| ents.loop_points(l))
                .for_each(&mut add);
        }
    }
    for i in &sel.instances {
        if let Some(inst) = ents.instances.get(*i)
            && let Some(d) = defs.get(inst.definition)
        {
            for v in d.entities.vertices.values() {
                add(inst.transform.transform_point3(v.pos));
            }
            // Nested instances only by origin (enough for an origin corner).
            for n in &d.entities.instances {
                add((inst.transform * n.transform).transform_point3(DVec3::ZERO));
            }
        }
    }
    min.is_finite().then_some((min, max))
}

/// Make Group (`is_group`) or Make Component: moves the selection of `ents` into a new
/// definition and puts an instance of it in its place. `ents` must not be stored inside
/// `defs` while this runs (the app takes the edited context out). Returns the new
/// instance's index in `ents.instances`.
pub fn make_group(
    defs: &mut SlotMap<DefinitionId, Definition>,
    ents: &mut Entities,
    sel: &Selection,
    name: String,
    is_group: bool,
) -> Option<usize> {
    if sel.is_empty() {
        return None;
    }
    let (min, _) = bounds(ents, sel, defs)?;
    let to_local = DMat4::from_translation(-min);
    let mut inner = Entities::new();
    copy_raw(&mut inner, ents, &sel.edges, &sel.faces, to_local);
    let mut instances: Vec<usize> = sel
        .instances
        .iter()
        .copied()
        .filter(|&i| i < ents.instances.len())
        .collect();
    instances.sort_unstable();
    instances.dedup();
    for &i in &instances {
        let mut inst = ents.instances[i].clone();
        inst.transform = to_local * inst.transform;
        inner.instances.push(inst);
    }

    // Remove the selection from the parent: the faces, then the selected edges and the
    // faces' edges that no remaining face uses.
    let mut edges: Vec<EdgeId> = sel.edges.clone();
    for f in &sel.faces {
        if let Some(face) = ents.remove_face(*f) {
            edges.extend(face.loops().flat_map(|l| l.coedges.iter().map(|c| c.edge)));
        }
    }
    edges.sort();
    edges.dedup();
    let mut touched = Vec::new();
    for e in edges {
        if ents.edges.get(e).is_some_and(|x| x.faces.is_empty()) {
            let edge = ents.edges[e].clone();
            ents.remove_edge(e);
            touched.extend([edge.start, edge.end]);
        }
    }
    for v in touched {
        ents.remove_vertex_if_unused(v);
    }
    for &i in instances.iter().rev() {
        ents.instances.remove(i);
    }

    let definition = defs.insert(Definition {
        description: String::new(),
        name: name.clone(),
        guid: new_guid(defs.len()),
        entities: inner,
        is_group,
    });
    ents.instances.push(Instance {
        definition,
        transform: DMat4::from_translation(min),
        name: String::new(),
        hidden: false,
        layer: None,
        material: None,
    });
    Some(ents.instances.len() - 1)
}

fn new_guid(seed: usize) -> String {
    // Not a real GUID generator; unique enough within a session.
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{:032x}", t ^ (seed as u128) << 64)
}

/// Explode: replaces instance `index` of `ents` by its contents in place. Faces without
/// a material take the instance's material.
pub fn explode(
    defs: &SlotMap<DefinitionId, Definition>,
    ents: &mut Entities,
    index: usize,
) -> bool {
    if index >= ents.instances.len() {
        return false;
    }
    let inst = ents.instances.remove(index);
    let Some(def) = defs.get(inst.definition) else {
        return false;
    };
    let src = &def.entities;
    let edges: Vec<EdgeId> = src.edges.keys().collect();
    let faces: Vec<FaceId> = src.faces.keys().collect();
    let before: Vec<FaceId> = ents.faces.keys().collect();
    let verts = copy_raw(ents, src, &edges, &faces, inst.transform);
    let new_faces: Vec<FaceId> = ents.faces.keys().filter(|f| !before.contains(f)).collect();
    for f in new_faces {
        let a = &mut ents.faces[f].attrs;
        if a.material.is_none() {
            a.material = inst.material;
        }
        if a.back_material.is_none() {
            a.back_material = inst.material;
        }
        if a.layer.is_none() {
            a.layer = inst.layer;
        }
    }
    for n in &src.instances {
        let mut c = n.clone();
        c.transform = inst.transform * c.transform;
        if c.material.is_none() {
            c.material = inst.material;
        }
        ents.instances.push(c);
    }
    // Weld onto the surrounding geometry.
    crate::transform::transform_vertices(ents, &verts, DMat4::IDENTITY);
    true
}

/// Make Unique: gives instance `index` of `ents` its own copy of its definition.
pub fn make_unique(
    model_defs: &mut SlotMap<DefinitionId, Definition>,
    ents: &mut Entities,
    index: usize,
    name: String,
) -> bool {
    let Some(inst) = ents.instances.get(index) else {
        return false;
    };
    let Some(def) = model_defs.get(inst.definition) else {
        return false;
    };
    let mut copy = def.clone();
    copy.name = name;
    copy.guid = new_guid(model_defs.len());
    let id = model_defs.insert(copy);
    ents.instances[index].definition = id;
    true
}

/// How many instances of `def` the model has (root and all definitions).
pub fn instance_count(model: &Model, def: DefinitionId) -> usize {
    let count = |e: &Entities| e.instances.iter().filter(|i| i.definition == def).count();
    count(&model.root)
        + model
            .definitions
            .values()
            .map(|d| count(&d.entities))
            .sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Checks, add_line, validate};

    fn square(ents: &mut Entities, o: DVec3) {
        let p = [
            o,
            o + DVec3::X * 10.0,
            o + DVec3::new(10.0, 10.0, 0.0),
            o + DVec3::Y * 10.0,
        ];
        for i in 0..4 {
            add_line(ents, p[i], p[(i + 1) % 4]);
        }
    }

    #[test]
    fn group_then_explode_round_trips() {
        let mut model = Model::default();
        square(&mut model.root, DVec3::new(5.0, 5.0, 0.0));
        let sel = Selection {
            edges: model.root.edges.keys().collect(),
            faces: model.root.faces.keys().collect(),
            instances: vec![],
        };
        let i = make_group(
            &mut model.definitions,
            &mut model.root,
            &sel,
            "Group#1".into(),
            true,
        )
        .unwrap();
        assert!(
            model.root.edges.is_empty()
                && model.root.faces.is_empty()
                && model.root.vertices.is_empty()
        );
        let inst = &model.root.instances[i];
        assert_eq!(
            inst.transform.transform_point3(DVec3::ZERO),
            DVec3::new(5.0, 5.0, 0.0)
        );
        let def = &model.definitions[inst.definition];
        assert!(def.is_group);
        assert_eq!((def.entities.edges.len(), def.entities.faces.len()), (4, 1));
        assert_eq!(validate(&def.entities, Checks::ALL), Vec::<String>::new());

        assert!(explode(&model.definitions, &mut model.root, i));
        assert!(model.root.instances.is_empty());
        assert_eq!((model.root.edges.len(), model.root.faces.len()), (4, 1));
        assert_eq!(validate(&model.root, Checks::ALL), Vec::<String>::new());
        let min_x = model
            .root
            .vertices
            .values()
            .map(|v| v.pos.x)
            .fold(f64::MAX, f64::min);
        assert!((min_x - 5.0).abs() < 1e-9);
    }

    #[test]
    fn grouping_one_face_keeps_neighbors() {
        let mut ents = Entities::new();
        square(&mut ents, DVec3::ZERO);
        square(&mut ents, DVec3::X * 10.0);
        assert_eq!(ents.faces.len(), 2);
        let f = ents.faces.keys().next().unwrap();
        let mut defs = SlotMap::with_key();
        let sel = Selection {
            faces: vec![f],
            ..Default::default()
        };
        make_group(&mut defs, &mut ents, &sel, "Group#1".into(), true).unwrap();
        assert_eq!(ents.faces.len(), 1);
        // The other square keeps its four edges; the selected square's three own edges go.
        assert_eq!(ents.edges.len(), 4);
        assert_eq!(validate(&ents, Checks::ALL), Vec::<String>::new());
    }

    #[test]
    fn explode_welds_to_surroundings_and_unique_names() {
        let mut model = Model::default();
        square(&mut model.root, DVec3::ZERO);
        let sel = Selection {
            faces: model.root.faces.keys().collect(),
            edges: model.root.edges.keys().collect(),
            instances: vec![],
        };
        let i = make_group(
            &mut model.definitions,
            &mut model.root,
            &sel,
            "Box".into(),
            false,
        )
        .unwrap();
        // Draw a neighbor square sharing an edge with the component.
        square(&mut model.root, DVec3::X * 10.0);
        explode(&model.definitions, &mut model.root, i);
        assert_eq!(model.root.faces.len(), 2);
        assert_eq!(model.root.edges.len(), 7, "shared edge welded");
        assert_eq!(validate(&model.root, Checks::ALL), Vec::<String>::new());
        assert_eq!(model.unique_definition_name("Box"), "Box#2");
        assert_eq!(model.unique_definition_name("Box#2"), "Box#2");
    }
}
