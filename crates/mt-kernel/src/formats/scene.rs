//! A mesh with the node hierarchy it came from (FBX models), turned into nested
//! groups.
//!
//! - Each node that has polygons, or has a descendant with polygons, becomes a group
//!   (named `Group#n`, the node's name on the instance) holding its own faces in its
//!   own coordinates and its children's groups placed by their transforms relative to
//!   it. Nodes without geometry below them are left out; polygons of no node stay
//!   loose.
//! - A file with at most one such node comes in flat, as before (one group made by
//!   the importer).
//! - Group transforms are the nodes' translations and rotations only; their scale,
//!   shear and mirroring stay in the faces, so welding and merging work at the size
//!   the faces have in the model (as in a flat import) and mirrored nodes need no
//!   mirrored groups.
//! - Units and axes are converted as for flat meshes ([`MeshOptions`]): the transforms
//!   are conjugated by the same conversion, so the geometry lands where the flat
//!   import puts it.

use glam::{DMat4, DVec3, DVec4};

use crate::entities::{DefinitionId, Entities, Instance, MaterialId};
use crate::mesh_import::{self, Mesh, MeshOptions, Polygon};
use crate::model::{Definition, Model};

/// A mesh whose polygons belong to nodes of a hierarchy.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    /// Every polygon, positions in file coordinates (world space).
    pub mesh: Mesh,
    pub nodes: Vec<SceneNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneNode {
    pub name: String,
    /// Index into `nodes`.
    pub parent: Option<usize>,
    /// Node space to file world space.
    pub world: DMat4,
    /// Indices into the mesh's polygons (world positions).
    pub polygons: Vec<usize>,
}

impl Scene {
    /// Nodes holding geometry or with descendants holding it.
    fn kept(&self) -> Vec<bool> {
        let mut kept: Vec<bool> = self.nodes.iter().map(|n| !n.polygons.is_empty()).collect();
        for i in 0..self.nodes.len() {
            if !kept[i] {
                continue;
            }
            // Mark the ancestors (a cycle stops at the node count).
            let mut p = self.nodes[i].parent;
            let mut steps = 0;
            while let Some(j) = p.filter(|&j| j < self.nodes.len()) {
                if steps > self.nodes.len() {
                    break;
                }
                kept[j] = true;
                p = self.nodes[j].parent;
                steps += 1;
            }
        }
        kept
    }
}

/// The file-to-model conversion of `opts` as a matrix.
fn conversion(opts: &MeshOptions) -> DMat4 {
    let axes = if opts.y_up {
        DMat4::from_cols(DVec4::X, DVec4::Z, -DVec4::Y, DVec4::W)
    } else {
        DMat4::IDENTITY
    };
    DMat4::from_scale(DVec3::splat(opts.inches_per_unit)) * axes
}

/// The translation and rotation of `xf` (its scale, shear and mirroring left out).
fn rigid(xf: DMat4) -> DMat4 {
    let x = xf.x_axis.truncate().try_normalize();
    let y = xf.y_axis.truncate();
    let rot = x.and_then(|x| {
        let y = (y - x * y.dot(x)).try_normalize()?;
        Some((x, y, x.cross(y)))
    });
    let (x, y, z) = rot.unwrap_or((DVec3::X, DVec3::Y, DVec3::Z));
    DMat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), xf.w_axis)
}

/// The polygons `ids` of `mesh` with positions mapped by `xf`.
fn sub_mesh(mesh: &Mesh, ids: &[usize], xf: DMat4) -> Mesh {
    let mut out = Mesh {
        materials: mesh.materials.clone(),
        ..Default::default()
    };
    let mut map: std::collections::BTreeMap<usize, usize> = Default::default();
    for &i in ids {
        let Some(p) = mesh.polygons.get(i) else {
            continue;
        };
        if p.vertices.iter().any(|&v| v >= mesh.positions.len()) {
            continue;
        }
        let vertices: Vec<usize> = p
            .vertices
            .iter()
            .map(|&v| {
                *map.entry(v).or_insert_with(|| {
                    out.positions.push(xf.transform_point3(mesh.positions[v]));
                    out.positions.len() - 1
                })
            })
            .collect();
        out.polygons.push(Polygon {
            vertices,
            material: p.material,
            uvs: p.uvs.clone(),
        });
    }
    out
}

struct Build<'a> {
    scene: &'a Scene,
    opts: &'a MeshOptions,
    materials: &'a [MaterialId],
    kept: Vec<bool>,
    children: Vec<Vec<usize>>,
    c: DMat4,
    c_inv: DMat4,
}

impl Build<'_> {
    /// The group of node `i` and its kept descendants.
    fn group(&self, model: &mut Model, i: usize, depth: usize) -> DefinitionId {
        let node = &self.scene.nodes[i];
        // Scale and mirroring stay in the geometry (welding and merging work at the
        // size the faces have in the model, as in a flat import); groups only move
        // and turn.
        let inv = rigid(node.world).inverse();
        let mesh = sub_mesh(&self.scene.mesh, &node.polygons, inv);
        let mut ents = mesh_import::to_entities(&mesh, self.opts, self.materials);
        if depth < 64 {
            for &k in &self.children[i] {
                if !self.kept[k] {
                    continue;
                }
                let def = self.group(model, k, depth + 1);
                let rel = inv * rigid(self.scene.nodes[k].world);
                ents.instances.push(instance(
                    def,
                    self.c * rel * self.c_inv,
                    &self.scene.nodes[k].name,
                ));
            }
        }
        let name = model.unique_definition_name("Group#1");
        model.definitions.insert(Definition {
            name,
            entities: ents,
            is_group: true,
            ..Default::default()
        })
    }
}

fn instance(definition: DefinitionId, transform: DMat4, name: &str) -> Instance {
    Instance {
        definition,
        transform,
        name: name.to_owned(),
        hidden: false,
        locked: false,
        layer: None,
        material: None,
        attrs: Default::default(),
    }
}

/// Builds the scene's faces and groups: the returned collection holds the loose
/// polygons and the top nodes' groups (whose definitions are added to `model`).
/// `materials` maps the mesh's materials to model materials.
pub fn to_entities(
    model: &mut Model,
    scene: &Scene,
    opts: &MeshOptions,
    materials: &[MaterialId],
) -> Entities {
    let kept = scene.kept();
    if kept.iter().filter(|&&k| k).count() <= 1 {
        return mesh_import::to_entities(&scene.mesh, opts, materials);
    }
    let n = scene.nodes.len();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut roots = Vec::new();
    for (i, node) in scene.nodes.iter().enumerate() {
        match node.parent.filter(|&p| p < n && p != i) {
            Some(p) => children[p].push(i),
            None => roots.push(i),
        }
    }
    let c = conversion(opts);
    let b = Build {
        scene,
        opts,
        materials,
        kept,
        children,
        c,
        c_inv: c.inverse(),
    };
    let owned: std::collections::BTreeSet<usize> = scene
        .nodes
        .iter()
        .flat_map(|n| n.polygons.iter().copied())
        .collect();
    let loose: Vec<usize> = (0..scene.mesh.polygons.len())
        .filter(|i| !owned.contains(i))
        .collect();
    let mut ents = mesh_import::to_entities(
        &sub_mesh(&scene.mesh, &loose, DMat4::IDENTITY),
        opts,
        materials,
    );
    for r in roots {
        if !b.kept[r] {
            continue;
        }
        let def = b.group(model, r, 0);
        let node = &scene.nodes[r];
        ents.instances
            .push(instance(def, c * rigid(node.world) * b.c_inv, &node.name));
    }
    ents
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec2;

    /// A unit square polygon at `z`.
    fn square(mesh: &mut Mesh, z: f64) -> usize {
        let first = mesh.positions.len();
        for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            mesh.positions.push(DVec3::new(x, y, z));
        }
        mesh.polygons.push(Polygon {
            vertices: (first..first + 4).collect(),
            material: None,
            uvs: Vec::<DVec2>::new(),
        });
        mesh.polygons.len() - 1
    }

    #[test]
    fn nodes_become_nested_groups_in_place() {
        let mut mesh = Mesh::default();
        let a = square(&mut mesh, 5.0);
        let b = square(&mut mesh, 7.0);
        let up = |z| DMat4::from_translation(DVec3::new(0.0, 0.0, z));
        let scene = Scene {
            mesh,
            nodes: vec![
                SceneNode {
                    name: "Parent".into(),
                    parent: None,
                    world: up(5.0),
                    polygons: vec![a],
                },
                SceneNode {
                    name: "Child".into(),
                    parent: Some(0),
                    world: up(7.0),
                    polygons: vec![b],
                },
                SceneNode {
                    name: "Empty".into(),
                    parent: Some(0),
                    world: up(1.0),
                    polygons: vec![],
                },
            ],
        };
        let opts = MeshOptions {
            inches_per_unit: 2.0,
            y_up: true,
            merge_coplanar: true,
        };
        let mut model = Model::default();
        let ents = to_entities(&mut model, &scene, &opts, &[]);
        assert!(ents.faces.is_empty());
        assert_eq!(ents.instances.len(), 1);
        assert_eq!(ents.instances[0].name, "Parent");
        let parent = &model.definitions[ents.instances[0].definition];
        assert!(parent.is_group);
        assert_eq!(parent.entities.faces.len(), 1);
        assert_eq!(parent.entities.instances.len(), 1);
        assert_eq!(model.definitions.len(), 2);
        // The child's square ends up where the flat import puts it: file z 7 is model
        // -y 14 (Y up, 2 inches per unit).
        let child = &parent.entities.instances[0];
        let xf = ents.instances[0].transform * child.transform;
        let ce = &model.definitions[child.definition].entities;
        for v in ce.vertices.values() {
            let p = xf.transform_point3(v.pos);
            assert!((p.y + 14.0).abs() < 1e-9, "{p}");
        }
        for e in [&parent.entities, ce] {
            assert!(crate::validate(e, crate::Checks::ALL).is_empty());
        }
        // One node: flat.
        let mut one = scene.clone();
        one.nodes.truncate(1);
        let mut m2 = Model::default();
        let flat = to_entities(&mut m2, &one, &opts, &[]);
        assert!(flat.instances.is_empty());
        assert_eq!(flat.faces.len(), 2);
    }
}
