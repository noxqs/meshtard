//! Saves a [`Model`] as a SketchUp 2017 (legacy v17) `.skp` through `mt_skp::write`.
//!
//! What is written: materials (color, opacity, PNG/JPEG textures), tags, component and
//! group definitions with their instances (nested too), faces with holes, front/back
//! materials, positioned textures (as three pins), tags and hidden state, every edge
//! with its soft/smooth/hidden flags, curves, guides, section planes, dimensions and
//! leader texts (root only; see `annotation::write_root` for what of them is lost).
//!
//! Known gaps (format support missing in the writer, see `mt_skp::write`):
//! - Arcs and circles are written as polyline curves through their actual vertices, so
//!   they reopen as curves, not as arcs editable by radius. (The writer's true arcs
//!   compute their own points, which would not coincide with the model's vertices.)
//! - Style settings are saved as the style items whose meaning is known (see
//!   [`style_items`]); fog, Component Edit hiding and the Monochrome face style are
//!   not (their items are unknown). Units go in through [`to_skp_with`].
//! - Edge tags, the saved view (files open in the writer's Iso view), scenes,
//!   shadow settings, model axes, instance locks, two-point perspective and the
//!   active section plane are not saved: where the v17 format keeps them is not
//!   known (no SketchUp-saved sample has them).
//! - Guides and section planes inside definitions are not saved (the writer has no
//!   definition-level calls for them).
//! - Section planes are saved as their plane only (as SketchUp does); the point
//!   they are drawn around is recomputed on import.

use std::collections::{HashMap, HashSet};

use glam::{DMat4, DVec3};
use mt_skp::OptionValue;
use mt_skp::write::{self as w, EdgeFlags, Placement, SkpBuilder, WriteError};

use crate::entities::{DefinitionId, EdgeId, Entities, Instance};
use crate::model::{FaceStyle, Model, StyleSettings};

/// Encodes `model` as `.skp` bytes.
pub fn to_skp(model: &Model) -> Result<Vec<u8>, WriteError> {
    to_skp_with(model, &[])
}

/// A named option of the file's options manager: section, key, value (e.g.
/// `UnitsOptions`, `LengthUnit`, `Int(2)`); see `mt_skp::Model::named_options`.
pub type NamedOption<'a> = (&'a str, &'a str, OptionValue);

/// [`to_skp`] that also writes `options` (units and snapping kept outside the
/// kernel model).
pub fn to_skp_with(model: &Model, options: &[NamedOption]) -> Result<Vec<u8>, WriteError> {
    let mut b = SkpBuilder::new();
    b.set_face_colors(model.front_color, model.back_color);
    for (id, value) in style_items(&model.style) {
        b.set_style_item(id, value);
    }
    for &(section, key, value) in options {
        b.set_named_option(section, key, value);
    }
    // A model with nothing at all is saved as the blank document.
    if is_empty(model) {
        return b.to_empty_document();
    }
    write_model(&mut b, model)?;
    b.to_bytes()
}

/// SketchUp style items (`style.xml` ids, OpenSKP `docs/API_DESIGN.md`) for the
/// style settings whose item is known. [`crate::import`] reads the same ids.
///
/// RenderMode (2001) values 0 Wireframe, 1 Hidden Line, 2 Shaded (2007 Texture
/// telling the two shaded styles apart, as corpus styles show) follow the menu
/// order and are unverified; Monochrome's value is unknown, so a Monochrome model
/// leaves the face style items as they are. EdgeDisplayMode (1000) is written 1 / 0
/// for Edges on / off (unverified).
pub fn style_items(s: &StyleSettings) -> Vec<(i32, OptionValue)> {
    use OptionValue::{Bool, Int};
    let px = |w: f32| Int(w.round().max(1.0) as i32);
    let [r, g, b] = s.section_fill_color;
    let mut items = vec![
        (1000, Int(i32::from(s.edges))),
        (1004, Bool(s.extension)),
        (1005, px(s.extension_length)),
        (1006, Bool(s.profiles)),
        (1007, px(s.profile_width)),
        (1008, Bool(s.depth_cue)),
        (1009, px(s.depth_cue_width)),
        (1015, Bool(s.back_edges)),
        (2004, Bool(s.xray)),
        (7008, Bool(s.axes)),
        // DrawHidden (pre-2020 key) and DrawHiddenGeometry.
        (7010, Bool(s.hidden_geometry)),
        (7017, Bool(s.hidden_geometry)),
        (7012, Bool(!s.guides)),
        (
            7013,
            Int(i32::from(s.section_planes) | i32::from(s.section_cuts) << 1),
        ),
        (7014, px(s.section_cut_width)),
        (7015, Bool(s.section_fill)),
        (7016, Int(i32::from_le_bytes([r, g, b, 0xff]))),
    ];
    let render = match s.face_style {
        FaceStyle::Wireframe => Some((0, false)),
        FaceStyle::HiddenLine => Some((1, false)),
        FaceStyle::Shaded => Some((2, false)),
        FaceStyle::ShadedWithTextures => Some((2, true)),
        FaceStyle::Monochrome => None,
    };
    if let Some((mode, texture)) = render {
        items.push((2001, Int(mode)));
        items.push((2007, Bool(texture)));
    }
    items
}

/// Writes `model` into `b` (exposed for deterministic tests).
fn is_empty(model: &Model) -> bool {
    let r = &model.root;
    r.edges.is_empty()
        && r.faces.is_empty()
        && r.instances.is_empty()
        && r.guides.is_empty()
        && r.section_planes.is_empty()
        && r.dimensions.is_empty()
        && r.texts.is_empty()
        && model.materials.is_empty()
        && model.layers.len() <= 1
        && model.definitions.is_empty()
}

pub fn write_model(b: &mut SkpBuilder, model: &Model) -> Result<(), WriteError> {
    // Real models have faces a little off their plane; accept what the kernel does.
    // Corpus files have faces up to about 0.004" off their plane (Untitled.skp);
    // 0.01" accepts them while still rejecting real warps.
    b.set_plane_tolerance(0.01);
    b.set_allow_empty_root(true);
    b.set_face_colors(model.front_color, model.back_color);
    let mut materials = Vec::new();
    for (i, m) in model.materials.iter().enumerate() {
        // Names must be unique in the file; duplicates get a suffix.
        let name = if model.materials[..i].iter().any(|x| x.name == m.name) {
            format!("{}#{i}", m.name)
        } else {
            m.name.clone()
        };
        let [r, g, bl] = m.color;
        // Textures the writer accepts (PNG, JPEG) are embedded; others (BMP) are saved
        // as the plain color. The colorize flag and color of textured materials are not
        // written (the writer takes neither).
        let texture = m.texture.as_ref().filter(|t| {
            t.data.starts_with(b"\x89PNG\r\n\x1a\n") || t.data.starts_with(b"\xff\xd8\xff")
        });
        let id = match texture {
            Some(t) => {
                let size = |s: f64| (s > 1e-9).then_some(s);
                b.add_texture_material(&w::TextureMaterial {
                    width: size(t.width),
                    height: size(t.height),
                    opacity: Some(m.opacity),
                    ..w::TextureMaterial::new(&name, &t.data, &t.filename)
                })?
            }
            None => b.add_material_with_opacity(&name, [r, g, bl, 255], Some(m.opacity))?,
        };
        materials.push(id);
    }
    let mut layers = vec![None];
    for l in model.layers.iter().skip(1) {
        let [r, g, bl] = l.color;
        layers.push(Some(b.add_layer(
            &l.name,
            Some([r, g, bl, 255]),
            l.visible,
        )?));
    }
    let maps = Maps { materials, layers };

    // Every definition (groups too, so group copies sharing one keep sharing it),
    // dependencies first.
    let mut order = Vec::new();
    let mut seen = HashSet::new();
    for id in model.definitions.keys() {
        visit(model, id, &mut seen, &mut order);
    }
    let mut written: HashMap<DefinitionId, w::DefinitionId> = HashMap::new();
    for id in order {
        let d = &model.definitions[id];
        let name = unique_name(&d.name, &written);
        let desc = (!d.description.is_empty()).then_some(d.description.as_str());
        let mut db = b.add_component_definition(&name, desc)?;
        write_body(&mut DefSink(&mut db), model, &d.entities, &maps, &written)?;
        written.insert(id, db.finish()?);
    }

    let root = &model.root;
    b.begin_root()?;
    write_body(&mut RootSink(b), model, root, &maps, &written)?;
    for g in &root.guides {
        let p = g.point.to_array();
        match g.dir {
            Some(dir) => b.add_construction_line(w::GuideLine::Infinite {
                point: p,
                direction: dir.to_array(),
            })?,
            None => b.add_construction_point(p)?,
        }
    }
    for s in &root.section_planes {
        b.add_section_plane(s.point.to_array(), s.normal.to_array())?;
    }
    crate::annotation::write_root(b, root)?;
    Ok(())
}

struct Maps {
    materials: Vec<w::MaterialId>,
    /// Index 0 (Untagged) is `None`.
    layers: Vec<Option<w::LayerId>>,
}

impl Maps {
    fn material(&self, m: Option<usize>) -> Option<w::MaterialId> {
        m.and_then(|i| self.materials.get(i).cloned())
    }

    fn layer(&self, l: Option<usize>) -> Option<w::LayerId> {
        l.and_then(|i| self.layers.get(i).cloned().flatten())
    }
}

fn visit(
    model: &Model,
    id: DefinitionId,
    seen: &mut HashSet<DefinitionId>,
    out: &mut Vec<DefinitionId>,
) {
    if !seen.insert(id) {
        return;
    }
    let Some(d) = model.definitions.get(id) else {
        return;
    };
    for inst in &d.entities.instances {
        visit(model, inst.definition, seen, out);
    }
    out.push(id);
}

fn unique_name(name: &str, written: &HashMap<DefinitionId, w::DefinitionId>) -> String {
    let taken = |n: &str| written.values().any(|d| d.name() == n);
    if !taken(name) {
        return name.to_owned();
    }
    (2..)
        .map(|i| format!("{name}#{i}"))
        .find(|n| !taken(n))
        .expect("unbounded")
}

fn placement(inst: &Instance, maps: &Maps) -> Placement {
    let xf: DMat4 = inst.transform;
    let mut m = [0.0; 9];
    for r in 0..3 {
        for c in 0..3 {
            m[3 * r + c] = xf.col(c)[r];
        }
    }
    Placement {
        name: (!inst.name.is_empty()).then(|| inst.name.clone()),
        translation: xf.w_axis.truncate().to_array(),
        matrix: Some(m),
        rotation: None,
        material: maps.material(inst.material),
        layer: maps.layer(inst.layer),
        attributes: Vec::new(),
        hidden: inst.hidden,
    }
}

/// Where a body (root or definition contents) goes.
trait Sink {
    fn edge(&mut self, a: DVec3, b: DVec3, flags: EdgeFlags) -> Result<(), WriteError>;
    fn polyline(&mut self, p: &w::Polyline) -> Result<(), WriteError>;
    fn face(&mut self, f: &w::Face) -> Result<(), WriteError>;
    fn instance(
        &mut self,
        def: &w::DefinitionId,
        p: &Placement,
        group: bool,
    ) -> Result<(), WriteError>;
    fn image(&mut self, def: &w::DefinitionId, p: &Placement) -> Result<(), WriteError>;
}

struct RootSink<'a>(&'a mut SkpBuilder);
struct DefSink<'a, 'b>(&'a mut w::DefinitionBuilder<'b>);

impl Sink for RootSink<'_> {
    fn edge(&mut self, a: DVec3, b: DVec3, flags: EdgeFlags) -> Result<(), WriteError> {
        self.0.add_edge(a.to_array(), b.to_array(), flags)
    }
    fn polyline(&mut self, p: &w::Polyline) -> Result<(), WriteError> {
        self.0.add_polyline(p)
    }
    fn face(&mut self, f: &w::Face) -> Result<(), WriteError> {
        self.0.add_face(f)
    }
    fn instance(
        &mut self,
        def: &w::DefinitionId,
        p: &Placement,
        group: bool,
    ) -> Result<(), WriteError> {
        if group {
            self.0.add_group_instance(def, p)
        } else {
            self.0.add_instance(def, p)
        }
    }
    fn image(&mut self, def: &w::DefinitionId, p: &Placement) -> Result<(), WriteError> {
        self.0.add_image_instance(def, p)
    }
}

impl Sink for DefSink<'_, '_> {
    fn edge(&mut self, a: DVec3, b: DVec3, flags: EdgeFlags) -> Result<(), WriteError> {
        self.0.add_edge(a.to_array(), b.to_array(), flags)
    }
    fn polyline(&mut self, p: &w::Polyline) -> Result<(), WriteError> {
        self.0.add_polyline(p)
    }
    fn face(&mut self, f: &w::Face) -> Result<(), WriteError> {
        self.0.add_face(f)
    }
    fn instance(
        &mut self,
        def: &w::DefinitionId,
        p: &Placement,
        group: bool,
    ) -> Result<(), WriteError> {
        if group {
            self.0.add_group_instance(def, p)
        } else {
            self.0.add_instance(def, p)
        }
    }
    fn image(&mut self, def: &w::DefinitionId, p: &Placement) -> Result<(), WriteError> {
        self.0.add_image_instance(def, p)
    }
}

fn flags(ents: &Entities, e: EdgeId) -> EdgeFlags {
    let a = &ents.edges[e].attrs;
    EdgeFlags {
        hidden: a.hidden,
        soft: a.soft,
        smooth: a.smooth,
    }
}

/// Edges first (curves as polylines, then the rest one by one, so each keeps its own
/// flags), then faces, which reuse those edges, then instances.
fn write_body(
    sink: &mut dyn Sink,
    model: &Model,
    ents: &Entities,
    maps: &Maps,
    written: &HashMap<DefinitionId, w::DefinitionId>,
) -> Result<(), WriteError> {
    let mut done: HashSet<EdgeId> = HashSet::new();
    for c in ents.curves.keys() {
        let edges = ents.curve_edges(c);
        let Some((points, closed)) = chain(ents, &edges) else {
            continue;
        };
        let mut p = w::Polyline::new(points.iter().map(|p| p.to_array()));
        p.closed = closed;
        p.edges = flags(ents, edges[0]);
        sink.polyline(&p)?;
        done.extend(edges);
    }
    for (id, e) in &ents.edges {
        if !done.contains(&id) {
            sink.edge(ents.pos(e.start), ents.pos(e.end), flags(ents, id))?;
        }
    }
    for face in ents.faces.values() {
        let pts = |l| {
            ents.loop_points(l)
                .into_iter()
                .map(|p| p.to_array())
                .collect::<Vec<_>>()
        };
        let f = w::Face {
            outer: pts(&face.outer),
            holes: face.holes.iter().map(pts).collect(),
            material: maps.material(face.attrs.material),
            back_material: maps.material(face.attrs.back_material),
            layer: maps.layer(face.attrs.layer),
            hidden: face.attrs.hidden,
            front_uv: uv_pins(model, ents, face, false),
            back_uv: uv_pins(model, ents, face, true),
            ..w::Face::default()
        };
        sink.face(&f)?;
    }
    for inst in &ents.instances {
        let Some(d) = model.definitions.get(inst.definition) else {
            continue;
        };
        if let Some(def) = written.get(&inst.definition) {
            // An image entity is a `CImage` placing its definition (the textured
            // rectangle, written like any other definition).
            if d.is_image {
                sink.image(def, &placement(inst, maps))?;
            } else {
                sink.instance(def, &placement(inst, maps), d.is_group)?;
            }
        }
    }
    Ok(())
}

/// Three texture pins reproducing a positioned texture on one side of `face` (`None`
/// for the default projection or an untextured side). Pins fit an affine mapping, so
/// distorted (projective) positioning is saved approximately.
fn uv_pins(
    model: &Model,
    ents: &Entities,
    face: &crate::entities::Face,
    back: bool,
) -> Option<[w::UvPin; 3]> {
    let uv = face.attrs.uv.as_deref()?;
    (if back { uv.back } else { uv.front })?;
    let m = if back {
        face.attrs.back_material
    } else {
        face.attrs.material
    };
    let mapping = crate::texture::face_mapping(face, back, model.materials.get(m?)?)?;
    let (xr, yr) = mt_skp::uv::face_uv_basis(face.normal.to_array());
    let p0 = *ents.loop_points(&face.outer).first()?;
    let pin = |p: DVec3| w::UvPin {
        point: p.to_array(),
        uv: mapping.uv(p.to_array()),
    };
    Some([
        pin(p0),
        pin(p0 + DVec3::from_array(xr) * 12.0),
        pin(p0 + DVec3::from_array(yr) * 12.0),
    ])
}

/// The points of a curve's edges in chain order, and whether it closes.
fn chain(ents: &Entities, edges: &[EdgeId]) -> Option<(Vec<DVec3>, bool)> {
    if edges.is_empty() {
        return None;
    }
    let set: HashSet<EdgeId> = edges.iter().copied().collect();
    // Start at a vertex used by only one curve edge (an open end), else anywhere.
    let degree = |v| {
        ents.vertices[v]
            .edges
            .iter()
            .filter(|e| set.contains(e))
            .count()
    };
    let first = &ents.edges[edges[0]];
    let start = edges
        .iter()
        .flat_map(|&e| [ents.edges[e].start, ents.edges[e].end])
        .find(|&v| degree(v) == 1)
        .unwrap_or(first.start);
    let mut used = HashSet::new();
    let mut v = start;
    let mut points = vec![ents.pos(v)];
    while let Some(&e) = ents.vertices[v]
        .edges
        .iter()
        .find(|e| set.contains(e) && !used.contains(*e))
    {
        used.insert(e);
        v = ents.edges[e].other(v);
        points.push(ents.pos(v));
    }
    if used.len() != set.len() {
        return None; // Branching curve: leave its edges to the plain-edge pass.
    }
    let closed = v == start && points.len() > 2;
    if closed {
        points.pop();
    }
    Some((points, closed))
}
