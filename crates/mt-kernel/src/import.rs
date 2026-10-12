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

impl Maps<'_> {
    /// The tag of a file layer name (`None` for Layer0 and unknown names).
    fn layer(&self, name: &str) -> Option<LayerId> {
        self.layers.get(name).copied().filter(|&l| l != 0)
    }
}

pub fn from_skp(src: &skp::Model) -> Model {
    let mut model = Model::default();
    // The working copy holds the settings the model displays (edits not yet
    // updated into the style included); files without one use the current style.
    let style = src
        .styles
        .iter()
        .find(|s| s.working_copy)
        .or(src.styles.iter().find(|s| s.active))
        .or(src.styles.first());
    if let Some(style) = style {
        if let Some(c) = style.front_color {
            model.front_color = c;
        }
        if let Some(c) = style.back_color {
            model.back_color = c;
        }
        read_style(style, &mut model.style);
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
            ..Layer::default()
        });
    }

    for (&id, d) in &src.definitions {
        let key = model.definitions.insert(Definition {
            description: String::new(),
            glue: crate::model::GlueTo::None,
            name: d.name.clone(),
            guid: d.guid.clone(),
            entities: Entities::new(),
            is_group: d.name.starts_with("Group#"),
            is_image: d.is_image,
            attributes: Default::default(),
            ..Default::default()
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
    // Model-level dimensions are world space (VFF: including those inside components);
    // legacy files list the root's there too.
    if !src.dimensions.is_empty() {
        model.root.dimensions = dimensions(&src.dimensions, &maps);
    }
    // A section plane is stored as its plane only; SketchUp draws it around the
    // model, so its point is the model's bounding box center projected onto it.
    let all = crate::group::Selection {
        edges: model.root.edges.keys().collect(),
        faces: model.root.faces.keys().collect(),
        instances: (0..model.root.instances.len()).collect(),
    };
    let center = crate::group::bounds(&model.root, &all, &model.definitions)
        .map_or(DVec3::ZERO, |(lo, hi)| (lo + hi) / 2.0);
    model.root.section_planes = section_planes(&src.root.section_planes, center);
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
            ..Default::default()
        });
    }
    model
}

/// Style settings from a style's items (`style.xml` ids, the same in legacy
/// files; see [`crate::export::style_items`] for the ids and what is unverified).
/// Items a style lacks keep their current value.
fn read_style(style: &skp::Style, out: &mut crate::model::StyleSettings) {
    read_style_items(&|id| style.items.get(&id).map(|i| i.value.as_str()), out);
}

/// [`read_style`] over any source of item values (text as in `style.xml`; e.g. the
/// Styles panel's `.style` files). Items `item` lacks keep their current value.
pub fn read_style_items<'a>(
    item: &dyn Fn(i32) -> Option<&'a str>,
    out: &mut crate::model::StyleSettings,
) {
    use crate::model::FaceStyle;
    let item = |id: i32| item(id).map(str::trim);
    let int = |id: i32| item(id).and_then(|v| v.parse::<i64>().ok());
    let bool_ = |id: i32| int(id).map(|v| v != 0);
    let width = |id: i32| int(id).filter(|&v| v > 0).map(|v| v as f32);
    let set = |v: &mut bool, x: Option<bool>| {
        if let Some(x) = x {
            *v = x;
        }
    };
    let set_w = |v: &mut f32, x: Option<f32>| {
        if let Some(x) = x {
            *v = x;
        }
    };
    set(&mut out.edges, int(1000).map(|v| v != 0));
    set(&mut out.extension, bool_(1004));
    set_w(&mut out.extension_length, width(1005));
    set(&mut out.profiles, bool_(1006));
    set_w(&mut out.profile_width, width(1007));
    set(&mut out.depth_cue, bool_(1008));
    set_w(&mut out.depth_cue_width, width(1009));
    set(&mut out.back_edges, bool_(1015));
    set(&mut out.xray, bool_(2004));
    set(&mut out.axes, bool_(7008));
    set(&mut out.hidden_geometry, bool_(7017).or(bool_(7010)));
    set(&mut out.guides, bool_(7012).map(|hide| !hide));
    if let Some(bits) = int(7013) {
        out.section_planes = bits & 1 != 0;
        out.section_cuts = bits & 2 != 0;
    }
    set_w(&mut out.section_cut_width, width(7014));
    set(&mut out.section_fill, bool_(7015));
    if let Some(c) = int(7016) {
        let [r, g, b, _] = (c as i32).to_le_bytes();
        out.section_fill_color = [r, g, b];
    }
    // Styles panel extras (see `crate::export::style_items`).
    use crate::style_extras::{EdgeColorMode, TransparencyQuality, item_color};
    let color = |id: i32| int(id).map(item_color);
    let set_c = |v: &mut [u8; 3], x: Option<[u8; 3]>| {
        if let Some(x) = x {
            *v = x;
        }
    };
    let float = |id: i32| item(id).and_then(|v| v.parse::<f32>().ok());
    let e = &mut out.edge_look;
    if let Some(m) = int(1002).and_then(EdgeColorMode::from_item) {
        e.color_mode = m;
    }
    set(&mut e.endpoints, bool_(1010));
    set_w(&mut e.endpoint_length, width(1011));
    set(&mut e.jitter, bool_(1012));
    set_c(&mut e.color, color(1014));
    if let Some(on) = bool_(2005) {
        out.face_look.transparency_off = !on;
    }
    match int(2006) {
        Some(0) => out.face_look.quality = TransparencyQuality::Faster,
        Some(1 | 2) => out.face_look.quality = TransparencyQuality::Nicer,
        _ => {}
    }
    let bg = &mut out.background;
    set_c(&mut bg.color, color(4000));
    set_c(&mut bg.sky_color, color(4001));
    set(&mut bg.sky, bool_(4002));
    set_c(&mut bg.ground_color, color(4003));
    set(&mut bg.ground, bool_(4004));
    if let Some(t) = int(4005) {
        bg.ground_transparency = (t as f32 / 100.0).clamp(0.0, 1.0);
    }
    set(&mut bg.ground_from_below, bool_(4006));
    set_c(&mut bg.horizon_color, color(4007));
    let mc = &mut out.modeling_colors;
    set_c(&mut mc.selected, color(7000));
    set_c(&mut mc.locked, color(7001));
    set_c(&mut mc.guides, color(7002));
    set_c(&mut mc.section_active, color(7003));
    set_c(&mut mc.section_inactive, color(7004));
    set_c(&mut mc.section_cut, color(7005));
    let ao = &mut out.ambient_occlusion;
    set(&mut ao.enabled, bool_(8100));
    if let Some(d) = float(8102).filter(|d| *d > 0.0) {
        ao.distance = d;
    }
    if let Some(i) = float(8103).filter(|i| (0.0..=1.0).contains(i)) {
        ao.intensity = i;
    }
    let texture = bool_(2007).unwrap_or(true);
    match int(2001) {
        Some(0) => out.face_style = FaceStyle::Wireframe,
        Some(1) => out.face_style = FaceStyle::HiddenLine,
        Some(2) if texture => out.face_style = FaceStyle::ShadedWithTextures,
        Some(2) => out.face_style = FaceStyle::Shaded,
        _ => {}
    }
}

/// File > Import of a `.skp`: adds the whole of `src` to `dst` as one new component
/// definition named `name` (made unique) and returns it. Materials and tags are matched
/// by name (a material or tag `dst` already has is used instead of the imported one;
/// SketchUp's handling of name clashes is unverified); the imported model's components
/// come along as definitions of their own. Scenes, styles and settings of `src` are
/// dropped.
pub fn merge_as_definition(dst: &mut Model, src: Model, name: &str) -> DefinitionId {
    let materials: Vec<MaterialId> = src
        .materials
        .into_iter()
        .map(
            |m| match dst.materials.iter().position(|x| x.name == m.name) {
                Some(i) => i,
                None => {
                    dst.materials.push(m);
                    dst.materials.len() - 1
                }
            },
        )
        .collect();
    let layers: Vec<LayerId> = src
        .layers
        .into_iter()
        .enumerate()
        .map(|(i, l)| {
            if i == 0 {
                return 0;
            }
            match dst.layers.iter().position(|x| x.name == l.name) {
                Some(k) => k,
                None => {
                    dst.layers.push(l);
                    dst.layers.len() - 1
                }
            }
        })
        .collect();
    // Keys first, so instances can point at definitions inserted later.
    let mut defs: HashMap<DefinitionId, DefinitionId> = HashMap::new();
    let src_defs: Vec<(DefinitionId, Definition)> = src.definitions.into_iter().collect();
    for (old, _) in &src_defs {
        defs.insert(*old, dst.definitions.insert(Definition::default()));
    }
    let remap = |ents: &mut Entities| {
        let mat = |m: &mut Option<MaterialId>| *m = m.and_then(|i| materials.get(i).copied());
        let lay = |l: &mut Option<LayerId>| {
            *l = l.and_then(|i| layers.get(i).copied()).filter(|&i| i != 0)
        };
        for f in ents.faces.values_mut() {
            mat(&mut f.attrs.material);
            mat(&mut f.attrs.back_material);
            lay(&mut f.attrs.layer);
        }
        for e in ents.edges.values_mut() {
            lay(&mut e.attrs.layer);
        }
        ents.instances.retain(|i| defs.contains_key(&i.definition));
        for i in &mut ents.instances {
            i.definition = defs[&i.definition];
            mat(&mut i.material);
            lay(&mut i.layer);
        }
        for t in &mut ents.texts {
            lay(&mut t.layer);
        }
        for d in &mut ents.dimensions {
            lay(&mut d.layer);
        }
    };
    for (old, mut d) in src_defs {
        remap(&mut d.entities);
        d.name = dst.unique_definition_name(&d.name);
        dst.definitions[defs[&old]] = d;
    }
    let mut root = src.root;
    remap(&mut root);
    let def = Definition {
        name: dst.unique_definition_name(name),
        entities: root,
        ..Default::default()
    };
    dst.definitions.insert(def)
}

fn entities(d: &skp::Definition, maps: &Maps) -> Entities {
    let mut ents = Entities::new();
    let mut vertices: HashMap<i64, VertexId> = HashMap::new();
    for (&id, v) in &d.vertices {
        vertices.insert(id, ents.add_vertex(DVec3::new(v.x, v.y, v.z)));
    }
    let mut edges: HashMap<i64, EdgeId> = HashMap::new();
    let layer = |name: &str| maps.layer(name);
    // One kernel curve per file curve; arcs get their arc data below.
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
            layer: layer(&e.layer),
            curve: e.curve_id.map(|c| {
                *curves.entry(c).or_insert_with(|| {
                    ents.curves.insert(crate::entities::Curve {
                        arc: None,
                        polygon: false,
                    })
                })
            }),
        };
        edges.insert(id, ents.add_raw_edge(a, b, attrs));
    }
    if !d.arc_curves.is_empty() {
        let mut members: HashMap<crate::entities::CurveId, Vec<EdgeId>> = HashMap::new();
        for (id, e) in &ents.edges {
            if let Some(c) = e.attrs.curve {
                members.entry(c).or_default().push(id);
            }
        }
        for (file_curve, &curve) in &curves {
            if let (Some(a), Some(m)) = (d.arc_curves.get(file_curve), members.get(&curve)) {
                ents.curves[curve].arc = arc_from_skp(&ents, m, a);
            }
        }
    }
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
            shadows: Default::default(),
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
            // The lock flag's place in the file is not known; not read.
            locked: false,
            layer: layer(&i.layer),
            material: material(i.material_id),
            attrs: Default::default(),
        });
    }
    ents.texts = texts(d, maps);
    ents.dimensions = dimensions(&d.dimensions, maps);
    ents.guides = guides(d);
    // Drawn around the definition's own geometry (the root's are placed around the
    // whole model by the caller).
    let (lo, hi) = ents.vertices.values().fold(
        (DVec3::splat(f64::MAX), DVec3::splat(f64::MIN)),
        |(lo, hi), v| (lo.min(v.pos), hi.max(v.pos)),
    );
    let center = if lo.x <= hi.x {
        (lo + hi) / 2.0
    } else {
        DVec3::ZERO
    };
    ents.section_planes = section_planes(&d.section_planes, center);
    ents
}

/// A file arc as the kernel's, when it is circular and every vertex of its edges lies on
/// it; `None` keeps the curve a plain curve (scaled arcs are ellipses).
fn arc_from_skp(
    ents: &Entities,
    curve_edges: &[EdgeId],
    a: &skp::ArcCurve,
) -> Option<crate::entities::Arc> {
    let (c, n, x, y) = (
        DVec3::from(a.center),
        DVec3::from(a.normal).try_normalize()?,
        DVec3::from(a.x_axis),
        DVec3::from(a.y_axis),
    );
    let r = x.length();
    let tol = 1e-6 * r.max(1.0);
    if r < 1e-9 || (n.cross(x) - y).length() > tol {
        return None;
    }
    let on_circle = curve_edges.iter().all(|&e| {
        let e = &ents.edges[e];
        [e.start, e.end].iter().all(|&v| {
            let d = ents.pos(v) - c;
            d.dot(n).abs() <= tol && (d.length() - r).abs() <= tol
        })
    });
    if !on_circle || curve_edges.is_empty() {
        return None;
    }
    // A full circle stores equal angles (SketchUp) or 0 and 2 pi (the SDK).
    let end = if a.end_angle == a.start_angle {
        a.start_angle + std::f64::consts::TAU
    } else {
        a.end_angle
    };
    Some(crate::entities::Arc {
        center: c,
        normal: n,
        x_axis: x / r,
        radius: r,
        start_angle: a.start_angle,
        end_angle: end,
        segments: curve_edges.len(),
    })
}

/// Section planes from their stored plane, drawn around `center`. Active state is not
/// read from the file yet.
fn section_planes(
    planes: &[skp::SectionPlane],
    center: DVec3,
) -> Vec<crate::entities::SectionPlane> {
    planes
        .iter()
        .filter_map(|s| {
            // a x + b y + c z + d = 0.
            let n = DVec3::new(s.plane[0], s.plane[1], s.plane[2]);
            let len = n.length();
            (len > 1e-12).then(|| {
                let (n, d) = (n / len, s.plane[3] / len);
                crate::entities::SectionPlane {
                    point: center - n * (n.dot(center) + d),
                    normal: n,
                    name: s.name.clone(),
                    hidden: s.hidden,
                    symbol: s.label.clone(),
                }
            })
        })
        .collect()
}

/// Dimensions with known endpoints. A dimension whose offset direction the reader
/// derived (legacy files) is built from it; the others go through
/// [`dimensions_from_skp`](crate::annotation::dimensions_from_skp) (VFF plane fields).
fn dimensions(dims: &[skp::Dimension], maps: &Maps) -> Vec<crate::annotation::Dimension> {
    dims.iter()
        .flat_map(|d| {
            let mut out = dimension(d);
            for x in &mut out {
                x.layer = maps.layer(&d.layer);
            }
            out
        })
        .collect()
}

fn dimension(d: &skp::Dimension) -> Vec<crate::annotation::Dimension> {
    match (d.a, d.b, d.offset_direction) {
        (Some(a), Some(b), Some(dir)) => {
            let (a, b) = (DVec3::from(a), DVec3::from(b));
            (a.distance(b) > crate::TOL)
                .then(|| crate::annotation::Dimension {
                    text: (!d.text.is_empty()).then(|| d.text.clone()),
                    hidden: d.hidden,
                    ..crate::annotation::Dimension::linear(a, b, DVec3::from(dir) * d.offset)
                })
                .into_iter()
                .collect()
        }
        _ => crate::annotation::dimensions_from_skp(std::slice::from_ref(d)),
    }
}

/// Leader and screen texts. A text anchored to an entity whose position is not
/// decoded (e.g. a face) keeps its label position, without a leader.
fn texts(d: &skp::Definition, maps: &Maps) -> Vec<crate::annotation::Text> {
    use crate::annotation::Text;
    d.texts
        .iter()
        .filter_map(|t| {
            let mut out = match (t.point, t.label_point, t.screen_position) {
                (Some(p), label, _) => {
                    let p = DVec3::from(p);
                    Text::leader(
                        p,
                        label.map_or(DVec3::ZERO, |l| DVec3::from(l) - p),
                        t.text.clone(),
                    )
                }
                (None, Some(l), _) => Text::leader(l.into(), DVec3::ZERO, t.text.clone()),
                (None, None, Some([x, y])) => Text::screen(x, y, t.text.clone()),
                (None, None, None) => return None,
            };
            out.hidden = t.hidden;
            out.layer = maps.layer(&t.layer);
            Some(out)
        })
        .collect()
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

#[cfg(test)]
mod merge_tests {
    use super::*;

    #[test]
    fn merged_model_becomes_a_definition_with_mapped_ids() {
        let mut src = Model::default();
        src.materials.push(Material {
            name: "Shared".into(),
            color: [1, 2, 3],
            opacity: 1.0,
            texture: None,
        });
        src.layers.push(Layer {
            name: "Walls".into(),
            color: [0; 3],
            visible: true,
            ..Layer::default()
        });
        let inner = src.definitions.insert(Definition {
            name: "Chair".into(),
            ..Default::default()
        });
        crate::add_line(&mut src.root, DVec3::ZERO, DVec3::X);
        src.root.instances.push(Instance {
            definition: inner,
            transform: DMat4::IDENTITY,
            name: String::new(),
            hidden: false,
            locked: false,
            layer: Some(1),
            material: Some(0),
            attrs: Default::default(),
        });
        let mut dst = Model::default();
        dst.materials.push(Material {
            name: "Other".into(),
            color: [9; 3],
            opacity: 1.0,
            texture: None,
        });
        dst.definitions.insert(Definition {
            name: "Chair".into(),
            ..Default::default()
        });
        let d = merge_as_definition(&mut dst, src, "house");
        let def = &dst.definitions[d];
        assert_eq!(def.name, "house");
        assert_eq!(def.entities.edges.len(), 1);
        let inst = &def.entities.instances[0];
        assert_eq!(inst.material, Some(1));
        assert_eq!(dst.materials[1].name, "Shared");
        assert_eq!(dst.layers[inst.layer.unwrap()].name, "Walls");
        assert_eq!(dst.definitions[inst.definition].name, "Chair#2");
        assert_eq!(dst.definitions.len(), 3);
    }
}
