//! Builds a kernel [`Model`] from a parsed `.skp` file.
//!
//! Topology is taken as the file has it (SketchUp already enforced its rules), so imported
//! models pass the topology checks but are not re-snapped to our tolerance.

use std::collections::HashMap;

use glam::{DMat4, DVec3, DVec4};
use mt_skp as skp;

use crate::entities::{
    CoEdge, DefinitionId, EdgeAttrs, EdgeId, Entities, FaceAttrs, Instance, LayerId, Loop,
    MaterialId, VertexId,
};
use crate::model::{Definition, Layer, Material, Model};

/// SketchUp's 13-double transform (row-major 3x3, translation) as a matrix.
pub fn transform_from_skp(m: &[f64]) -> DMat4 {
    if m.len() < 12 {
        return DMat4::IDENTITY;
    }
    DMat4::from_cols(
        DVec4::new(m[0], m[3], m[6], 0.0),
        DVec4::new(m[1], m[4], m[7], 0.0),
        DVec4::new(m[2], m[5], m[8], 0.0),
        DVec4::new(m[9], m[10], m[11], 1.0),
    )
}

struct Maps<'a> {
    materials: HashMap<i64, MaterialId>,
    layers: HashMap<&'a str, LayerId>,
    definitions: HashMap<i64, DefinitionId>,
}

pub fn from_skp(src: &skp::Model) -> Model {
    let mut model = Model::default();
    if let Some(style) = src.styles.iter().find(|s| s.active).or(src.styles.first()) {
        if let Some(c) = style.front_color {
            model.front_color = c;
        }
        if let Some(c) = style.back_color {
            model.back_color = c;
        }
        // style.xml item 2001 looks like the render mode (2 = shaded in every corpus
        // style) and 2005 like "show textures": both are set in "[Simple Style]", whose
        // description says "Shaded with textures face style". Unverified guesses; other
        // modes are left at the default.
        let item = |id: i32| style.items.get(&id).map(|i| i.value.trim());
        if item(2001) == Some("2") {
            model.style.face_style = if item(2005) == Some("1") {
                crate::model::FaceStyle::ShadedWithTextures
            } else {
                crate::model::FaceStyle::Shaded
            };
        }
    }

    let mut maps = Maps {
        materials: HashMap::new(),
        layers: HashMap::new(),
        definitions: HashMap::new(),
    };
    let mut by_name: HashMap<&str, MaterialId> = HashMap::new();
    for m in &src.materials {
        by_name.insert(&m.name, model.materials.len());
        model.materials.push(Material {
            name: m.name.clone(),
            color: [m.color[0], m.color[1], m.color[2]],
            opacity: m.transparency,
            texture: m.texture.as_ref().and_then(|t| {
                Some(crate::texture::Texture {
                    filename: t.filename.clone(),
                    data: t.data.as_deref()?.into(),
                    width: t.width,
                    height: t.height,
                    colorize: m.colorized,
                })
            }),
        });
    }
    for (id, m) in src.materials_by_id() {
        if let Some(&i) = by_name.get(m.name.as_str()) {
            maps.materials.insert(id, i);
        }
    }
    maps.layers.insert("Layer0", 0);
    maps.layers.insert("", 0);
    for l in &src.layers {
        if l.name == "Layer0" {
            model.layers[0].color = l.color;
            model.layers[0].visible = !l.hidden;
            continue;
        }
        maps.layers.insert(&l.name, model.layers.len());
        model.layers.push(Layer {
            name: l.name.clone(),
            color: l.color,
            visible: !l.hidden,
        });
    }

    for (&id, d) in &src.definitions {
        let key = model.definitions.insert(Definition {
            description: String::new(),
            name: d.name.clone(),
            guid: d.guid.clone(),
            entities: Entities::new(),
            is_group: d.name.starts_with("Group#"),
        });
        maps.definitions.insert(id, key);
    }
    // Definitions placed by a group instance are groups.
    let group_refs = src
        .definitions
        .values()
        .chain(std::iter::once(&src.root))
        .flat_map(|d| &d.instances)
        .filter(|i| i.is_group)
        .filter_map(|i| i.ref_idx);
    for r in group_refs {
        if let Some(k) = maps.definitions.get(&r) {
            model.definitions[*k].is_group = true;
        }
    }
    for (&id, d) in &src.definitions {
        let ents = entities(d, &maps);
        model.definitions[maps.definitions[&id]].entities = ents;
    }
    model.root = entities(&src.root, &maps);
    model.root.guides = guides(&src.root);
    model.root.dimensions = crate::annotation::dimensions_from_skp(&src.dimensions);
    model.root.section_planes = src
        .root
        .section_planes
        .iter()
        .filter_map(|s| {
            // a x + b y + c z + d = 0. Active state is not read from the file yet.
            let n = DVec3::new(s.plane[0], s.plane[1], s.plane[2]);
            let len = n.length();
            (len > 1e-12).then(|| crate::entities::SectionPlane {
                point: -n * s.plane[3] / (len * len),
                normal: n / len,
                name: s.name.clone(),
                hidden: s.hidden,
            })
        })
        .collect();
    for p in &src.pages {
        let (Some(eye), Some(target)) = (p.eye, p.target) else {
            continue;
        };
        model.pages.push(crate::model::Page {
            name: p.name.clone(),
            eye: DVec3::from_array(eye),
            target: DVec3::from_array(target),
            fov_deg: p.fov,
            perspective: !p.parallel,
            hidden_layers: p
                .hidden_layers
                .iter()
                .filter_map(|n| maps.layers.get(n.as_str()).copied())
                .collect(),
        });
    }
    model
}

fn entities(d: &skp::Definition, maps: &Maps) -> Entities {
    let mut ents = Entities::new();
    let mut vertices: HashMap<i64, VertexId> = HashMap::new();
    for (&id, v) in &d.vertices {
        vertices.insert(id, ents.add_vertex(DVec3::new(v.x, v.y, v.z)));
    }
    let mut edges: HashMap<i64, EdgeId> = HashMap::new();
    // One kernel curve per file curve (arc parameters are not read yet: polyline curves).
    let mut curves: HashMap<i64, crate::entities::CurveId> = HashMap::new();
    for (&id, e) in &d.edges {
        let (Some(&a), Some(&b)) = (vertices.get(&e.v1_id), vertices.get(&e.v2_id)) else {
            continue;
        };
        if a == b || ents.edge_between(a, b).is_some() {
            continue;
        }
        let attrs = EdgeAttrs {
            soft: e.soft,
            smooth: e.smooth,
            hidden: e.hidden,
            layer: None,
            curve: e.curve_id.map(|c| {
                *curves
                    .entry(c)
                    .or_insert_with(|| ents.curves.insert(crate::entities::Curve { arc: None }))
            }),
        };
        edges.insert(id, ents.add_raw_edge(a, b, attrs));
    }
    let layer = |name: &str| maps.layers.get(name).copied().filter(|&l| l != 0);
    let material = |id: Option<i64>| id.and_then(|i| maps.materials.get(&i).copied());
    for f in d.faces.values() {
        let loops: Option<Vec<Loop>> = f
            .loops
            .iter()
            .map(|l| {
                let coedges = l
                    .iter()
                    .map(|c| {
                        Some(CoEdge {
                            edge: *edges.get(&c.edge_id)?,
                            reversed: c.orientation < 0,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?;
                (coedges.len() >= 3).then_some(Loop { coedges })
            })
            .collect();
        let Some(mut loops) = loops.filter(|l| !l.is_empty()) else {
            continue;
        };
        let normal = f
            .normal
            .map(DVec3::from)
            .filter(|n| n.length() > 1e-9)
            .map_or_else(|| face_normal(&ents, &loops[0]), DVec3::normalize);
        let outer = loops.remove(0);
        let attrs = FaceAttrs {
            material: material(f.material_id),
            back_material: material(f.back_material_id),
            hidden: f.hidden,
            layer: layer(&f.layer),
            uv: crate::texture::FaceUv::boxed(f.uv_transform, f.uv_transform_back),
        };
        ents.add_raw_face(outer, loops, normal, attrs);
    }
    for i in &d.instances {
        let Some(&def) = i.ref_idx.and_then(|r| maps.definitions.get(&r)) else {
            continue;
        };
        ents.instances.push(Instance {
            definition: def,
            transform: transform_from_skp(&i.matrix),
            name: i.name.clone(),
            hidden: i.hidden,
            layer: layer(&i.layer),
            material: material(i.material_id),
        });
    }
    ents
}

fn face_normal(ents: &Entities, l: &Loop) -> DVec3 {
    crate::geom::Plane::from_polygon(&ents.loop_points(l)).map_or(DVec3::Z, |p| p.normal)
}

/// Construction lines and points of a definition as guides.
fn guides(d: &skp::Definition) -> Vec<crate::entities::Guide> {
    let lines = d.construction_lines.iter().filter_map(|c| {
        let dir = DVec3::from(c.direction).normalize_or_zero();
        (dir != DVec3::ZERO).then_some(crate::entities::Guide {
            point: c.point.into(),
            dir: Some(dir),
        })
    });
    let points = d
        .construction_points
        .iter()
        .map(|c| crate::entities::Guide {
            point: c.position.into(),
            dir: None,
        });
    lines.chain(points).collect()
}
