//! More right-click items (FEATURES.md 55): Find Center, Convert to Polygon, Align View
//! and Align Axes, the Area submenu, section plane items (Reverse, Active Cut, Align
//! View, Create Group from Slice, Troubleshoot Section Fill), dimension Text Position
//! and Type, leader text Arrow and Leader, the drawing axes' items (Place, Move...,
//! Reset, Hide) and component items (Change Axes, Unglue, Save As, Reload, Select
//! Instances, Replace Selected).
//!
//! Item names follow the help page "Context menu items for entities" and the pages it
//! links [doc]; where an item sits in the menu, the dialogs' layout and the exact
//! results (which faces Area > Tag counts, where Align Axes puts the origin, the Move
//! Sketching Context dialog's rotation order) are from memory, unverified.

use std::collections::HashMap;

use eframe::egui;
use egui_wgpu::wgpu;
use glam::{DMat4, DVec3};
use mt_infer::units::{LengthUnit, parse_length};
use mt_kernel::model_data::{Endpoint, LeaderKind};
use mt_kernel::{DefinitionId, DimensionKind, Entities, FaceId, Model, TextPosition};

use crate::App;
use crate::document::Owner;
use crate::nav::Tool;
use crate::selection::{self, Mode};

/// Commands of this module (all in `IMPLEMENTED`).
pub(crate) const COMMANDS: &[&str] = &[
    "edit.find_center",
    "edit.convert_to_polygon",
    "camera.align_view",
    "edit.align_axes",
    "edit.area_selection",
    "edit.area_tag",
    "edit.area_material",
    "edit.section_active",
    "edit.section_reverse",
    "edit.section_slice_group",
    "edit.section_troubleshoot",
    "edit.dim_text_start",
    "edit.dim_text_center",
    "edit.dim_text_end",
    "edit.dim_type_radius",
    "edit.dim_type_diameter",
    "edit.text_arrow_none",
    "edit.text_arrow_dot",
    "edit.text_arrow_closed",
    "edit.text_arrow_open",
    "edit.text_leader_view",
    "edit.text_leader_pushpin",
    "edit.text_leader_hidden",
    "view.axes_place",
    "view.axes_move",
    "view.axes_reset",
    "view.axes_hide",
    "edit.change_axes",
    "edit.unglue",
    "edit.component_save_as",
    "edit.component_reload",
    "edit.select_instances",
    "edit.replace_selected",
];

/// Dialogs and markers of this module, and of the tool modifiers (`tool_extra`).
#[derive(Default)]
pub(crate) struct Extra {
    /// Area dialog: title and the formatted total.
    pub area: Option<(String, String)>,
    /// Move Sketching Context dialog: x, y, z offsets and red, green, blue rotations.
    pub axes_move: Option<[String; 6]>,
    /// Troubleshoot Section Fill: where the cut's loops are open (world space), and
    /// the model state they belong to (they go when the model changes).
    pub open_ends: Option<(u64, Vec<DVec3>)>,
    /// Change Axes is running for this instance of the active context.
    pub change_axes: Option<usize>,
    /// Tape Measure rescale waiting for the answer: factor and the message.
    pub rescale: Option<(f64, String)>,
    /// A section plane placed, waiting for its name and symbol.
    pub section: Option<mt_kernel::SectionPlane>,
    /// "Don't ask again" was ticked in the section name prompt.
    pub section_no_prompt: bool,
    /// Help > Search popup.
    pub search: Option<crate::search::Search>,
    /// Ctrl is down in a drawing tool: `true` while no other key was pressed with it
    /// (a tap toggles the tool's Ctrl mode on release).
    pub ctrl_tap: Option<bool>,
}

/// What the selection holds for this module's items.
#[derive(Default, Debug)]
pub(crate) struct ExtraFacts {
    /// Selected edges that belong to a curve (Find Center).
    pub curves: bool,
    /// Some selected curve is an arc or circle not yet a polygon.
    pub convertible: bool,
    /// Exactly one face selected (Align View / Align Axes).
    pub one_face: Option<FaceId>,
    pub sections: Vec<usize>,
    pub dims: Vec<usize>,
    /// Some selected dimension is radial.
    pub radial: bool,
    pub texts: Vec<usize>,
    /// Some selected text has a leader.
    pub leader: bool,
    /// Components (not groups) selected: their definitions.
    pub definitions: Vec<DefinitionId>,
    /// Some selected instance is glued.
    pub glued: bool,
    /// Exactly one unlocked component instance.
    pub one_component: Option<usize>,
    /// A component is picked in the Components panel (Replace Selected).
    pub current_component: bool,
}

impl ExtraFacts {
    pub fn of(model: &Model, ents: &Entities, items: &[Owner], current: Option<usize>) -> Self {
        let mut f = Self {
            current_component: current.is_some(),
            ..Default::default()
        };
        let mut faces = Vec::new();
        let mut components = Vec::new();
        for &o in items {
            match o {
                Owner::Edge(e) => {
                    if let Some(c) = ents.edges.get(e).and_then(|x| x.attrs.curve) {
                        f.curves = true;
                        f.convertible |= ents
                            .curves
                            .get(c)
                            .is_some_and(|c| c.arc.is_some() && !c.polygon);
                    }
                }
                Owner::Face(x) => faces.push(x),
                Owner::Section(i) if i < ents.section_planes.len() => f.sections.push(i),
                Owner::Dimension(i) if i < ents.dimensions.len() => {
                    f.dims.push(i);
                    f.radial |= ents.dimensions[i].kind != DimensionKind::Linear;
                }
                Owner::Text(i) if i < ents.texts.len() => {
                    f.texts.push(i);
                    f.leader |= !ents.texts[i].screen;
                }
                Owner::Instance(i) => {
                    let Some(inst) = ents.instances.get(i) else {
                        continue;
                    };
                    f.glued |= inst.attrs.glued;
                    let def = model.definitions.get(inst.definition);
                    if def.is_some_and(|d| !d.is_group && !d.is_image) {
                        components.push(i);
                        if !f.definitions.contains(&inst.definition) {
                            f.definitions.push(inst.definition);
                        }
                    }
                }
                _ => {}
            }
        }
        if let [x] = faces[..] {
            f.one_face = Some(x);
        }
        if let [i] = components[..]
            && items.len() == 1
            && !ents.instances[i].locked
        {
            f.one_component = Some(i);
        }
        f
    }

    /// Whether `id` (one of [`COMMANDS`]) applies; `None` for other commands.
    pub fn enabled(&self, id: &str) -> Option<bool> {
        Some(match id {
            "edit.find_center" => self.curves,
            "edit.convert_to_polygon" => self.convertible,
            "camera.align_view" => self.one_face.is_some() || self.sections.len() == 1,
            "edit.align_axes" => self.one_face.is_some(),
            "edit.area_selection" | "edit.area_tag" | "edit.area_material" => {
                self.one_face.is_some() || !self.definitions.is_empty()
            }
            "edit.section_active"
            | "edit.section_reverse"
            | "edit.section_slice_group"
            | "edit.section_troubleshoot" => self.sections.len() == 1,
            "edit.dim_text_start" | "edit.dim_text_center" | "edit.dim_text_end" => {
                !self.dims.is_empty()
            }
            "edit.dim_type_radius" | "edit.dim_type_diameter" => self.radial,
            id if id.starts_with("edit.text_") => self.leader,
            "edit.change_axes" | "edit.component_save_as" | "edit.component_reload" => {
                self.one_component.is_some()
            }
            "edit.unglue" => self.glued,
            "edit.select_instances" => !self.definitions.is_empty(),
            "edit.replace_selected" => !self.definitions.is_empty() && self.current_component,
            id if COMMANDS.contains(&id) => true,
            _ => return None,
        })
    }
}

/// Area of face `f` with its points mapped by `xf` (holes subtracted).
pub fn face_area_xf(ents: &Entities, f: FaceId, xf: DMat4) -> f64 {
    let Some(face) = ents.faces.get(f) else {
        return 0.0;
    };
    let area = |pts: Vec<DVec3>| {
        // Newell's formula: half the length of the summed cross products.
        let n = pts.len();
        let mut v = DVec3::ZERO;
        for i in 0..n {
            v += pts[i].cross(pts[(i + 1) % n]);
        }
        v.length() / 2.0
    };
    let map = |l| {
        ents.loop_points(l)
            .into_iter()
            .map(|p| xf.transform_point3(p))
            .collect()
    };
    area(map(&face.outer)) - face.holes.iter().map(|h| area(map(h))).sum::<f64>()
}

/// Sums the areas of the faces of `ents` (and of the instances inside it, drawn with
/// their transforms) that `keep` accepts.
pub fn area_where(
    model: &Model,
    ents: &Entities,
    xf: DMat4,
    keep: &dyn Fn(&mt_kernel::Face) -> bool,
) -> f64 {
    let mut total: f64 = ents
        .faces
        .iter()
        .filter(|(_, f)| keep(f))
        .map(|(id, _)| face_area_xf(ents, id, xf))
        .sum();
    for inst in &ents.instances {
        if let Some(d) = model.definitions.get(inst.definition) {
            total += area_where(model, &d.entities, xf * inst.transform, keep);
        }
    }
    total
}

/// Area > Selection: the selected faces and every face inside selected groups and
/// components.
pub fn selection_area(model: &Model, ents: &Entities, items: &[Owner], xf: DMat4) -> f64 {
    items
        .iter()
        .map(|o| match *o {
            Owner::Face(f) => face_area_xf(ents, f, xf),
            Owner::Instance(i) => ents
                .instances
                .get(i)
                .and_then(|inst| {
                    let d = model.definitions.get(inst.definition)?;
                    Some(area_where(model, &d.entities, xf * inst.transform, &|_| {
                        true
                    }))
                })
                .unwrap_or(0.0),
            _ => 0.0,
        })
        .sum()
}

/// Find Center: the center of each selected curve (an arc's or circle's center, else
/// the mean of a curve's vertices, as for a polygon).
pub fn curve_centers(ents: &Entities, items: &[Owner]) -> Vec<DVec3> {
    let mut out: Vec<DVec3> = Vec::new();
    let mut seen = Vec::new();
    for o in items {
        let Owner::Edge(e) = *o else { continue };
        let Some(c) = ents.edges.get(e).and_then(|x| x.attrs.curve) else {
            continue;
        };
        if seen.contains(&c) {
            continue;
        }
        seen.push(c);
        let center = match ents.curves.get(c).and_then(|x| x.arc) {
            Some(arc) => arc.center,
            None => {
                let mut verts: Vec<_> = ents
                    .curve_edges(c)
                    .iter()
                    .flat_map(|&e| [ents.edges[e].start, ents.edges[e].end])
                    .collect();
                verts.sort();
                verts.dedup();
                if verts.is_empty() {
                    continue;
                }
                verts.iter().map(|&v| ents.pos(v)).sum::<DVec3>() / verts.len() as f64
            }
        };
        out.push(center);
    }
    out
}

/// The ends of open loops among cut segments: points where an odd number of segments
/// meet (closed loops meet in pairs). Points closer than `tol` are one point.
pub fn open_ends(segments: &[[DVec3; 2]], tol: f64) -> Vec<DVec3> {
    let key = |p: DVec3| {
        let k = (p / tol).round();
        (k.x as i64, k.y as i64, k.z as i64)
    };
    let mut count: HashMap<(i64, i64, i64), (DVec3, usize)> = HashMap::new();
    for s in segments {
        if s[0].distance(s[1]) <= tol {
            continue;
        }
        for p in s {
            count.entry(key(*p)).or_insert((*p, 0)).1 += 1;
        }
    }
    let mut out: Vec<DVec3> = count
        .into_values()
        .filter(|(_, n)| n % 2 == 1)
        .map(|(p, _)| p)
        .collect();
    out.sort_by(|a, b| {
        a.x.total_cmp(&b.x)
            .then(a.y.total_cmp(&b.y))
            .then(a.z.total_cmp(&b.z))
    });
    out
}

/// Type > Radius / Diameter of a radial dimension (`a` the center for a radius, the
/// opposite point for a diameter).
pub fn set_radial_kind(d: &mut mt_kernel::Dimension, kind: DimensionKind) -> bool {
    match (d.kind, kind) {
        (DimensionKind::Radius, DimensionKind::Diameter) => {
            d.a = 2.0 * d.a - d.b;
        }
        (DimensionKind::Diameter, DimensionKind::Radius) => {
            d.a = (d.a + d.b) / 2.0;
        }
        _ => return false,
    }
    d.kind = kind;
    true
}

/// Change Axes: the definition's entities move by `m` (old definition space to new)
/// and every instance of it takes `m`'s inverse, so nothing moves in the model.
pub fn change_definition_axes(model: &mut Model, def: DefinitionId, m: DMat4) -> bool {
    let Some(d) = model.definitions.get_mut(def) else {
        return false;
    };
    let ents = &mut d.entities;
    let all = all_owners(ents);
    crate::modify::apply(ents, &all, m, false);
    for g in &mut ents.guides {
        g.point = m.transform_point3(g.point);
        g.dir = g.dir.map(|v| m.transform_vector3(v).normalize_or_zero());
    }
    let inv = m.inverse();
    let fix = |ents: &mut Entities| {
        for i in &mut ents.instances {
            if i.definition == def {
                i.transform *= inv;
            }
        }
    };
    fix(&mut model.root);
    for d in model.definitions.values_mut() {
        fix(&mut d.entities);
    }
    true
}

/// Every top-level entity of `ents`.
pub fn all_owners(ents: &Entities) -> Vec<Owner> {
    ents.faces
        .keys()
        .map(Owner::Face)
        .chain(ents.edges.keys().map(Owner::Edge))
        .chain((0..ents.instances.len()).map(Owner::Instance))
        .chain((0..ents.section_planes.len()).map(Owner::Section))
        .chain((0..ents.dimensions.len()).map(Owner::Dimension))
        .chain((0..ents.texts.len()).map(Owner::Text))
        .collect()
}

/// Scales everything in `ents` about its origin (Tape Measure rescale).
pub fn scale_all(ents: &mut Entities, factor: f64) {
    let xf = DMat4::from_scale(DVec3::splat(factor));
    let all = all_owners(ents);
    crate::modify::apply(ents, &all, xf, false);
    for g in &mut ents.guides {
        g.point *= factor;
    }
}

/// Save As for a component: a model whose root is the definition's contents, with the
/// definitions and materials it uses.
pub fn definition_model(model: &Model, def: DefinitionId) -> Option<Model> {
    let d = model.definitions.get(def)?;
    let mut out = model.clone();
    out.root = d.entities.clone();
    out.pages.clear();
    mt_kernel::purge::purge_definitions(&mut out);
    // Purging can leave nested definitions only the dropped ones used.
    while mt_kernel::purge::purge_definitions(&mut out) > 0 {}
    Some(out)
}

/// Reload: the definition's contents become `src`'s whole model (its materials, tags
/// and components merged by name as File > Import does).
pub fn reload_definition(model: &mut Model, def: DefinitionId, src: Model) -> bool {
    if !model.definitions.contains_key(def) {
        return false;
    }
    let tmp = mt_kernel::import::merge_as_definition(model, src, "Reload");
    let Some(new) = model.definitions.remove(tmp) else {
        return false;
    };
    model.definitions[def].entities = new.entities;
    true
}

/// The axes after the Move Sketching Context dialog: offsets along and rotations
/// (degrees) about the current red, green and blue axes, applied red, green, blue.
pub fn moved_axes(axes: DMat4, offset: DVec3, degrees: DVec3) -> DMat4 {
    let r = degrees * (std::f64::consts::PI / 180.0);
    axes * DMat4::from_translation(offset)
        * DMat4::from_rotation_x(r.x)
        * DMat4::from_rotation_y(r.y)
        * DMat4::from_rotation_z(r.z)
}

impl App {
    /// Runs a command of this module; false if `id` is not one.
    pub(crate) fn context_extra_command(&mut self, frame: &eframe::Frame, id: &str) -> bool {
        if !COMMANDS.contains(&id) {
            return false;
        }
        let Some(rs) = frame.wgpu_render_state() else {
            return true;
        };
        let device = rs.device.clone();
        match id {
            "view.axes_place" => {
                self.run_tool("tool.axes");
                return true;
            }
            "view.axes_hide" => {
                if let Some(doc) = self.document.as_mut() {
                    doc.model.style.axes = false;
                }
                self.rebuild(&device);
                return true;
            }
            "view.axes_move" => {
                self.extra.axes_move = Some(Default::default());
                return true;
            }
            "edit.change_axes" => {
                let Some(doc) = self.document.as_ref() else {
                    return true;
                };
                let f = self.extra_facts(doc);
                if let Some(i) = f.one_component {
                    self.run_tool("tool.axes");
                    self.extra.change_axes = Some(i);
                    self.message =
                        Some("Change Axes: click the origin, then red and green.".into());
                }
                return true;
            }
            _ => {}
        }
        let Some(doc) = self.document.as_ref() else {
            return true;
        };
        let items = self.selection.items().to_vec();
        let f = self.extra_facts(doc);
        let units = doc.units;
        let xf = self.ctx_xf;
        match id {
            "camera.align_view" => {
                let ents = doc.active();
                let target = match (f.one_face, f.sections.first()) {
                    (Some(face), _) => ents.faces.get(face).map(|x| {
                        let pts = ents.loop_points(&x.outer);
                        let c = pts.iter().sum::<DVec3>() / pts.len().max(1) as f64;
                        (c, x.normal)
                    }),
                    // Looking at the plane against its arrows, at the cut.
                    (None, Some(&s)) => {
                        let sp = &ents.section_planes[s];
                        Some((sp.point, -sp.normal))
                    }
                    _ => None,
                };
                if let Some((c, n)) = target {
                    let c = xf.transform_point3(c);
                    let n = glam::DMat3::from_mat4(xf).inverse().transpose() * n;
                    let n = n.normalize_or(DVec3::Z);
                    // From the side the camera is on now (a ground face's front
                    // points down; which side SketchUp takes is unverified).
                    let n = if (self.camera.eye - c).dot(n) < 0.0 {
                        -n
                    } else {
                        n
                    };
                    let mut cam = self.camera.clone();
                    let dist = cam.distance();
                    cam.target = c;
                    cam.eye = c + n * dist;
                    cam.two_point = false;
                    self.go(cam);
                }
                return true;
            }
            "edit.area_selection" | "edit.area_tag" | "edit.area_material" => {
                let ents = doc.active();
                let model = &doc.model;
                let face = f.one_face.and_then(|x| ents.faces.get(x));
                let (title, area) = match id {
                    "edit.area_selection" => {
                        ("Area".to_owned(), selection_area(model, ents, &items, xf))
                    }
                    "edit.area_tag" => {
                        let layer = face.map(|x| x.attrs.layer).unwrap_or(None);
                        let name = layer
                            .and_then(|l| model.layers.get(l))
                            .map_or("Untagged".to_owned(), |l| l.name.clone());
                        let total = area_where(model, &model.root, DMat4::IDENTITY, &|x| {
                            x.attrs.layer == layer
                        });
                        (format!("Area of Tag \"{name}\""), total)
                    }
                    _ => {
                        let mat = face.map(|x| x.attrs.material).unwrap_or(None);
                        let name = mat
                            .and_then(|m| model.materials.get(m))
                            .map_or("Default".to_owned(), |m| m.name.clone());
                        // Faces painted with it on either side count once.
                        let total = area_where(model, &model.root, DMat4::IDENTITY, &|x| {
                            x.attrs.material == mat || x.attrs.back_material == mat
                        });
                        (format!("Area of Material \"{name}\""), total)
                    }
                };
                self.extra.area = Some((title, crate::annot::area_label(area, units)));
                return true;
            }
            "edit.section_troubleshoot" => {
                if let Some(&s) = f.sections.first() {
                    let sp = &doc.active().section_planes[s];
                    let (p, n) = (
                        xf.transform_point3(sp.point),
                        (glam::DMat3::from_mat4(xf).inverse().transpose() * sp.normal)
                            .normalize_or(DVec3::Z),
                    );
                    let segs = mt_render::section_lines(&self.scene.triangles, p, n);
                    let ends = open_ends(&segs, 1e-4);
                    self.message = Some(if ends.is_empty() {
                        "Troubleshoot Section Fill: every loop of the cut is closed.".into()
                    } else {
                        format!(
                            "Troubleshoot Section Fill: {} open end(s) marked in red.",
                            ends.len()
                        )
                    });
                    self.extra.open_ends = Some((doc.state(), ends));
                }
                return true;
            }
            "edit.select_instances" => {
                let ents = doc.active();
                let sel: Vec<Owner> = ents
                    .instances
                    .iter()
                    .enumerate()
                    .filter(|(_, i)| f.definitions.contains(&i.definition) && !i.hidden)
                    .map(|(i, _)| Owner::Instance(i))
                    .collect();
                self.select(&device, Mode::Replace, sel);
                return true;
            }
            "edit.component_save_as" => {
                self.component_save_as(&f);
                return true;
            }
            "edit.component_reload" => {
                self.component_reload(&f, &device);
                return true;
            }
            _ => {}
        }
        let current = self.current_component;
        let slice = match (id, f.sections.first()) {
            ("edit.section_slice_group", Some(&s)) => {
                let sp = &doc.active().section_planes[s];
                let p = xf.transform_point3(sp.point);
                let n = (glam::DMat3::from_mat4(xf).inverse().transpose() * sp.normal)
                    .normalize_or(DVec3::Z);
                Some(mt_render::section_lines(&self.scene.triangles, p, n))
            }
            _ => None,
        };
        let Some(doc) = self.document.as_mut() else {
            return true;
        };
        let inv = xf.inverse();
        let changed = match id {
            "edit.find_center" => doc.edit(|ents| {
                let centers = curve_centers(ents, &items);
                for c in &centers {
                    ents.guides.push(mt_kernel::Guide {
                        point: *c,
                        dir: None,
                    });
                }
                !centers.is_empty()
            }),
            "edit.convert_to_polygon" => doc.edit(|ents| {
                let mut any = false;
                for o in &items {
                    if let Owner::Edge(e) = *o
                        && let Some(c) = ents.edges.get(e).and_then(|x| x.attrs.curve)
                        && let Some(curve) = ents.curves.get_mut(c)
                        && curve.arc.is_some()
                        && !curve.polygon
                    {
                        curve.polygon = true;
                        any = true;
                    }
                }
                any
            }),
            "edit.align_axes" => {
                let frame = f.one_face.and_then(|face| {
                    let ents = doc.active();
                    let x = ents.faces.get(face)?;
                    let pts = ents.loop_points(&x.outer);
                    let (o, next) = (*pts.first()?, *pts.get(1)?);
                    let red = (next - o).normalize_or_zero();
                    let green = x.normal.cross(red);
                    let local = crate::component_dialog::axes_frame(o, o + red, o + green)?;
                    crate::model_axes::world_frame(xf, local)
                });
                match frame {
                    Some(frame) => doc.edit_model(|m, _| {
                        let changed = m.axes != frame;
                        m.axes = frame;
                        changed
                    }),
                    None => false,
                }
            }
            "edit.section_active" | "edit.section_reverse" => doc.edit(|ents| {
                let Some(&s) = f.sections.first() else {
                    return false;
                };
                if id == "edit.section_reverse" {
                    ents.section_planes[s].normal = -ents.section_planes[s].normal;
                } else {
                    ents.active_section = if ents.active_section == Some(s) {
                        None
                    } else {
                        Some(s)
                    };
                }
                true
            }),
            "edit.section_slice_group" => {
                let segs = slice.unwrap_or_default();
                let mut g = Entities::new();
                for [a, b] in &segs {
                    let (a, b) = (inv.transform_point3(*a), inv.transform_point3(*b));
                    if a.distance(b) > mt_kernel::TOL {
                        mt_kernel::add_line(&mut g, a, b);
                    }
                }
                // Only the cut's edges (closed loops would make faces).
                let faces: Vec<FaceId> = g.faces.keys().collect();
                for x in faces {
                    g.remove_face(x);
                }
                if g.edges.is_empty() {
                    false
                } else {
                    let edit = crate::sandbox::group_edit(g, DMat4::IDENTITY);
                    let mut made = Vec::new();
                    let changed = doc.edit_model(|model, ents| {
                        made = edit(model, ents);
                        true
                    });
                    self.selection.clear();
                    self.selection.apply(Mode::Replace, made);
                    changed
                }
            }
            "edit.dim_text_start" | "edit.dim_text_center" | "edit.dim_text_end" => {
                let pos = match id {
                    "edit.dim_text_start" => TextPosition::OutsideStart,
                    "edit.dim_text_center" => TextPosition::Center,
                    _ => TextPosition::OutsideEnd,
                };
                doc.edit(|ents| {
                    let mut any = false;
                    for &i in &f.dims {
                        let d = &mut ents.dimensions[i];
                        any |= d.kind == DimensionKind::Linear && d.text_position != pos;
                        d.text_position = pos;
                    }
                    any
                })
            }
            "edit.dim_type_radius" | "edit.dim_type_diameter" => {
                let kind = if id == "edit.dim_type_radius" {
                    DimensionKind::Radius
                } else {
                    DimensionKind::Diameter
                };
                doc.edit(|ents| {
                    let mut any = false;
                    for &i in &f.dims {
                        any |= set_radial_kind(&mut ents.dimensions[i], kind);
                    }
                    any
                })
            }
            id if id.starts_with("edit.text_") => {
                let arrow = match id {
                    "edit.text_arrow_none" => Some(Endpoint::None),
                    "edit.text_arrow_dot" => Some(Endpoint::Dot),
                    "edit.text_arrow_closed" => Some(Endpoint::ClosedArrow),
                    "edit.text_arrow_open" => Some(Endpoint::OpenArrow),
                    _ => None,
                };
                let leader = match id {
                    "edit.text_leader_view" => Some(LeaderKind::ViewBased),
                    "edit.text_leader_pushpin" => Some(LeaderKind::Pushpin),
                    "edit.text_leader_hidden" => Some(LeaderKind::Hidden),
                    _ => None,
                };
                // The text's own style, starting from the model's (Model Info > Text).
                let default_style = doc.model.text_style.clone();
                doc.edit(|ents| {
                    let mut any = false;
                    for &i in &f.texts {
                        let t = &mut ents.texts[i];
                        if t.screen {
                            continue;
                        }
                        let mut style = t.style.clone().unwrap_or_else(|| default_style.clone());
                        if let Some(a) = arrow {
                            style.endpoint = a;
                        }
                        if let Some(l) = leader {
                            style.leader = l;
                        }
                        any |= t.style.as_ref() != Some(&style);
                        t.style = Some(style);
                    }
                    any
                })
            }
            "view.axes_reset" => doc.edit_model(|m, _| {
                let changed = m.axes != DMat4::IDENTITY;
                m.axes = DMat4::IDENTITY;
                changed
            }),
            "edit.unglue" => doc.edit(|ents| {
                let mut any = false;
                for o in &items {
                    if let Owner::Instance(i) = *o
                        && let Some(inst) = ents.instances.get_mut(i)
                    {
                        any |= inst.attrs.glued;
                        inst.attrs.glued = false;
                    }
                }
                any
            }),
            "edit.replace_selected" => {
                let target = current.and_then(|c| {
                    crate::tray::component_definitions(&doc.model)
                        .get(c)
                        .copied()
                });
                match target {
                    Some(def) if !doc.in_context_path(def) => doc.edit(|ents| {
                        let mut any = false;
                        for o in &items {
                            if let Owner::Instance(i) = *o
                                && let Some(inst) = ents.instances.get_mut(i)
                                && !inst.locked
                                && inst.definition != def
                            {
                                inst.definition = def;
                                any = true;
                            }
                        }
                        any
                    }),
                    _ => false,
                }
            }
            _ => false,
        };
        if changed {
            if let Some(ents) = self
                .document
                .as_ref()
                .map(crate::document::Document::active)
            {
                self.selection.retain(|o| selection::exists(ents, o));
            }
            self.rebuild(&device);
        }
        true
    }

    pub(crate) fn extra_facts(&self, doc: &crate::document::Document) -> ExtraFacts {
        ExtraFacts::of(
            &doc.model,
            doc.active(),
            self.selection.items(),
            self.current_component,
        )
    }

    /// Switches to a tool as its toolbar button does.
    pub(crate) fn run_tool(&mut self, id: &'static str) {
        if let Some(tool) = Tool::from_command(id) {
            self.tool = tool;
            self.draw = crate::tools::for_command(id);
            self.modify = crate::modify::for_command(id);
            self.shift_lock.reset(&mut self.engine);
            self.inference = None;
            self.extra.change_axes = None;
        }
    }

    /// Change Axes picked the new axes (open-context space): moves the definition so
    /// its origin and axes are there. Returns false when Change Axes is not running.
    pub(crate) fn finish_change_axes(&mut self, local: DMat4, device: &wgpu::Device) -> bool {
        let Some(i) = self.extra.change_axes.take() else {
            return false;
        };
        self.modify = None;
        self.tool = Tool::Select;
        self.engine.unlock();
        self.message = None;
        let Some(doc) = self.document.as_mut() else {
            return true;
        };
        let Some(inst) = doc.active().instances.get(i) else {
            return true;
        };
        let (def, t) = (inst.definition, inst.transform);
        // Old definition space -> context -> new definition space.
        let m = local.inverse() * t;
        if doc.edit_whole(|model| change_definition_axes(model, def, m)) {
            self.rebuild(device);
        }
        true
    }

    fn component_save_as(&mut self, f: &ExtraFacts) {
        let Some(doc) = self.document.as_ref() else {
            return;
        };
        let Some(i) = f.one_component else { return };
        let def = doc.active().instances[i].definition;
        let name = doc.model.definitions[def].name.clone();
        let Some(model) = definition_model(&doc.model, def) else {
            return;
        };
        let Some(path) = crate::filedialog::save(
            &[("Meshtard Models (*.skp)", &["skp"])],
            &format!("{name}.skp"),
        ) else {
            return;
        };
        match mt_kernel::export::to_skp(&model).map(|bytes| std::fs::write(&path, bytes)) {
            Ok(Ok(())) => self.message = Some(format!("Saved {name} to {}.", path.display())),
            Ok(Err(e)) => self.file_error("save", &path, e),
            Err(e) => self.file_error("save", &path, e),
        }
    }

    fn component_reload(&mut self, f: &ExtraFacts, device: &wgpu::Device) {
        let Some(i) = f.one_component else { return };
        let Some(path) = crate::filedialog::open(&[("Meshtard Models (*.skp)", &["skp"])]) else {
            return;
        };
        let src = match mt_skp::read(&path) {
            Ok(s) => mt_kernel::import::from_skp(&s),
            Err(e) => {
                self.file_error("open", &path, e);
                return;
            }
        };
        let Some(doc) = self.document.as_mut() else {
            return;
        };
        let def = doc.active().instances[i].definition;
        if doc.in_context_path(def) {
            return;
        }
        if doc.edit_whole(|m| reload_definition(m, def, src)) {
            self.selection.clear();
            self.rebuild(device);
        }
    }

    /// Red circles at Troubleshoot Section Fill's open ends (until the model changes).
    pub(crate) fn context_extra_paint(&mut self, ui: &egui::Ui, rect: egui::Rect) {
        let state = self.document.as_ref().map(crate::document::Document::state);
        let Some((s, ends)) = &self.extra.open_ends else {
            return;
        };
        if Some(*s) != state {
            self.extra.open_ends = None;
            return;
        }
        let ppp = ui.ctx().pixels_per_point();
        let painter = ui.painter().with_clip_rect(rect);
        for p in ends {
            if let Some(q) = self.camera.project(*p, self.viewport_px) {
                let c = rect.min + egui::vec2(q.x as f32, q.y as f32) / ppp;
                painter.circle_stroke(c, 8.0, egui::Stroke::new(2.0, egui::Color32::RED));
            }
        }
    }

    /// The Area and Move Sketching Context dialogs.
    pub(crate) fn context_extra_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if let Some((title, value)) = self.extra.area.clone() {
            let r = mt_ui::Dialog::new("area", &title)
                .width(260.0)
                .show(ctx, |ui| {
                    ui.label(egui::RichText::new(format!("Area: {value}")).size(13.0));
                    mt_ui::dialog::buttons(ui, &["OK"], Some(0)).is_some()
                });
            if r.closed || r.inner {
                self.extra.area = None;
            }
        }
        let units = self.document.as_ref().map_or(LengthUnit::Inch, |d| d.units);
        if let Some(mut v) = self.extra.axes_move.take() {
            for (k, x) in v.iter_mut().enumerate() {
                if x.is_empty() {
                    *x = if k < 3 { units.format(0.0) } else { "0".into() };
                }
            }
            let r = mt_ui::Dialog::new("axes_move", "Move Sketching Context")
                .width(300.0)
                .show(ctx, |ui| {
                    crate::export::dialog_visuals(ui);
                    egui::Grid::new("mt_axes_move")
                        .num_columns(4)
                        .spacing([8.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("");
                            for h in ["X", "Y", "Z"] {
                                ui.label(h);
                            }
                            ui.end_row();
                            for (row, label) in ["Move", "Rotate"].iter().enumerate() {
                                ui.label(*label);
                                for k in 0..3 {
                                    ui.add_sized(
                                        [70.0, 20.0],
                                        egui::TextEdit::singleline(&mut v[row * 3 + k]),
                                    );
                                }
                                ui.end_row();
                            }
                        });
                    crate::presentation::ok_cancel(ui, "OK")
                });
            match crate::presentation::answer(r) {
                None => self.extra.axes_move = Some(v),
                Some(false) => {}
                Some(true) => {
                    let len = |s: &str| parse_length(s, units);
                    let deg = |s: &str| s.trim().replace(',', ".").parse::<f64>().ok();
                    let parsed = (|| {
                        Some((
                            DVec3::new(len(&v[0])?, len(&v[1])?, len(&v[2])?),
                            DVec3::new(deg(&v[3])?, deg(&v[4])?, deg(&v[5])?),
                        ))
                    })();
                    match parsed {
                        Some((o, r)) => {
                            if let Some(doc) = self.document.as_mut()
                                && doc.edit_model(|m, _| {
                                    m.axes = moved_axes(m.axes, o, r);
                                    true
                                })
                                && let Some(rs) = frame.wgpu_render_state()
                            {
                                let device = rs.device.clone();
                                self.rebuild(&device);
                            }
                        }
                        None => {
                            self.message = Some("Move Sketching Context: invalid value.".into());
                            self.extra.axes_move = Some(v);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_kernel::add_line;

    fn p(x: f64, y: f64, z: f64) -> DVec3 {
        DVec3::new(x, y, z)
    }

    fn square(e: &mut Entities, o: DVec3, s: f64) -> FaceId {
        let pts = [o, o + p(s, 0.0, 0.0), o + p(s, s, 0.0), o + p(0.0, s, 0.0)];
        for i in 0..4 {
            add_line(e, pts[i], pts[(i + 1) % 4]);
        }
        e.faces
            .iter()
            .find(|(_, f)| e.loop_points(&f.outer).iter().all(|q| q.x >= o.x - 1e-9))
            .map(|(id, _)| id)
            .unwrap()
    }

    #[test]
    fn area_of_faces_and_instances() {
        let mut m = Model::default();
        let f = square(&mut m.root, DVec3::ZERO, 10.0);
        assert!((face_area_xf(&m.root, f, DMat4::IDENTITY) - 100.0).abs() < 1e-9);
        // A component of a 2x2 square, placed scaled by 3: 36.
        let mut inner = Entities::new();
        square(&mut inner, DVec3::ZERO, 2.0);
        let d = m.definitions.insert(mt_kernel::Definition {
            name: "C".into(),
            entities: inner,
            ..Default::default()
        });
        m.root.instances.push(mt_kernel::Instance {
            definition: d,
            transform: DMat4::from_scale(DVec3::splat(3.0)),
            name: String::new(),
            hidden: false,
            locked: false,
            layer: None,
            material: None,
            attrs: Default::default(),
        });
        let sel = [Owner::Face(f), Owner::Instance(0)];
        let a = selection_area(&m, &m.root, &sel, DMat4::IDENTITY);
        assert!((a - 136.0).abs() < 1e-9, "{a}");
        m.root.faces[f].attrs.layer = Some(1);
        let tagged = area_where(&m, &m.root, DMat4::IDENTITY, &|x| x.attrs.layer == Some(1));
        assert!((tagged - 100.0).abs() < 1e-9);
        let untagged = area_where(&m, &m.root, DMat4::IDENTITY, &|x| x.attrs.layer.is_none());
        assert!((untagged - 36.0).abs() < 1e-9);
    }

    #[test]
    fn find_center_and_open_ends() {
        let mut e = Entities::new();
        let arc = mt_kernel::Arc {
            center: p(5.0, 5.0, 0.0),
            normal: DVec3::Z,
            x_axis: DVec3::X,
            radius: 3.0,
            start_angle: 0.0,
            end_angle: std::f64::consts::TAU,
            segments: 12,
        };
        let edges = mt_kernel::add_curve(&mut e, &arc.points(), Some(arc));
        let c = curve_centers(&e, &[Owner::Edge(edges[0]), Owner::Edge(edges[3])]);
        assert_eq!(c.len(), 1);
        assert!((c[0] - p(5.0, 5.0, 0.0)).length() < 1e-9);
        let facts = ExtraFacts::of(&Model::default(), &e, &[Owner::Edge(edges[0])], None);
        assert!(facts.convertible && facts.enabled("edit.find_center") == Some(true));
        // A hexagon drawn as a free curve: the mean of its vertices.
        let mut h = Entities::new();
        let mut pts: Vec<DVec3> = (0..6)
            .map(|k| {
                let a = k as f64 * std::f64::consts::TAU / 6.0;
                p(10.0 + a.cos(), a.sin(), 0.0)
            })
            .collect();
        pts.push(pts[0]);
        let edges = mt_kernel::add_curve(&mut h, &pts, None);
        let c = curve_centers(&h, &[Owner::Edge(edges[0])]);
        assert!((c[0] - p(10.0, 0.0, 0.0)).length() < 1e-9);

        // A closed triangle and an open polyline: the open one's two ends.
        let segs = [
            [p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)],
            [p(1.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
            [p(0.0, 1.0, 0.0), p(0.0, 0.0, 0.0)],
            [p(5.0, 0.0, 0.0), p(6.0, 0.0, 0.0)],
            [p(6.0, 0.0, 0.0), p(6.0, 1.0, 0.0)],
        ];
        assert_eq!(
            open_ends(&segs, 1e-6),
            vec![p(5.0, 0.0, 0.0), p(6.0, 1.0, 0.0)]
        );
    }

    #[test]
    fn radial_type_and_text_position() {
        let mut d = mt_kernel::Dimension {
            kind: DimensionKind::Radius,
            ..mt_kernel::Dimension::linear(p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0), DVec3::ZERO)
        };
        assert!(set_radial_kind(&mut d, DimensionKind::Diameter));
        assert_eq!(d.a, p(-2.0, 0.0, 0.0));
        assert!((d.value() - 4.0).abs() < 1e-12);
        assert!(set_radial_kind(&mut d, DimensionKind::Radius));
        assert_eq!(d.a, DVec3::ZERO);
        assert!(!set_radial_kind(&mut d, DimensionKind::Radius));
        let mut l = mt_kernel::Dimension::linear(DVec3::ZERO, p(10.0, 0.0, 0.0), p(0.0, 1.0, 0.0));
        assert_eq!(l.label_point(), p(5.0, 1.0, 0.0));
        l.text_position = TextPosition::OutsideEnd;
        assert_eq!(l.label_point(), p(12.0, 1.0, 0.0));
        assert_eq!(l.segments(0.0).len(), 4);
    }

    #[test]
    fn change_axes_keeps_the_model_in_place() {
        let mut m = Model::default();
        let mut inner = Entities::new();
        add_line(&mut inner, p(1.0, 0.0, 0.0), p(2.0, 0.0, 0.0));
        let d = m.definitions.insert(mt_kernel::Definition {
            entities: inner,
            ..Default::default()
        });
        let t = DMat4::from_translation(p(10.0, 0.0, 0.0));
        for _ in 0..2 {
            m.root.instances.push(mt_kernel::Instance {
                definition: d,
                transform: t,
                name: String::new(),
                hidden: false,
                locked: false,
                layer: None,
                material: None,
                attrs: Default::default(),
            });
        }
        let world = |m: &Model| {
            let e = &m.definitions[d].entities;
            let v = e.vertices.values().next().unwrap().pos;
            m.root.instances[1].transform.transform_point3(v)
        };
        let before = world(&m);
        // New axes at the context point (11, 0, 0), turned 90 degrees.
        let local = DMat4::from_translation(p(11.0, 0.0, 0.0)) * DMat4::from_rotation_z(1.0);
        assert!(change_definition_axes(&mut m, d, local.inverse() * t));
        assert!((world(&m) - before).length() < 1e-9);
        assert!(
            (m.root.instances[0].transform.transform_point3(DVec3::ZERO) - p(11.0, 0.0, 0.0))
                .length()
                < 1e-9
        );
    }

    #[test]
    fn save_as_and_reload_definition() {
        let mut m = Model::default();
        let mut inner = Entities::new();
        add_line(&mut inner, DVec3::ZERO, p(1.0, 0.0, 0.0));
        let d = m.definitions.insert(mt_kernel::Definition {
            name: "Part".into(),
            entities: inner,
            ..Default::default()
        });
        add_line(&mut m.root, p(5.0, 5.0, 0.0), p(9.0, 5.0, 0.0));
        let out = definition_model(&m, d).unwrap();
        assert_eq!(out.root.edges.len(), 1);
        assert!(
            out.definitions.is_empty(),
            "the saved part itself is not kept"
        );
        let bytes = mt_kernel::export::to_skp(&out).unwrap();
        assert!(!bytes.is_empty());

        let mut src = Model::default();
        add_line(&mut src.root, DVec3::ZERO, p(0.0, 3.0, 0.0));
        add_line(&mut src.root, p(0.0, 3.0, 0.0), p(4.0, 3.0, 0.0));
        let defs = m.definitions.len();
        assert!(reload_definition(&mut m, d, src));
        assert_eq!(m.definitions[d].entities.edges.len(), 2);
        assert_eq!(m.definitions.len(), defs);
    }

    #[test]
    fn moved_and_scaled() {
        let a = moved_axes(DMat4::IDENTITY, p(1.0, 2.0, 3.0), p(0.0, 0.0, 90.0));
        assert!((a.transform_point3(DVec3::ZERO) - p(1.0, 2.0, 3.0)).length() < 1e-12);
        assert!((a.transform_vector3(DVec3::X) - DVec3::Y).length() < 1e-12);
        let mut e = Entities::new();
        add_line(&mut e, p(1.0, 0.0, 0.0), p(2.0, 0.0, 0.0));
        e.guides.push(mt_kernel::Guide {
            point: p(0.0, 1.0, 0.0),
            dir: None,
        });
        scale_all(&mut e, 2.0);
        let xs: Vec<f64> = e.vertices.values().map(|v| v.pos.x).collect();
        assert!(xs.contains(&2.0) && xs.contains(&4.0));
        assert_eq!(e.guides[0].point, p(0.0, 2.0, 0.0));
        for id in COMMANDS {
            assert!(crate::IMPLEMENTED.contains(id), "{id}");
        }
    }
}
