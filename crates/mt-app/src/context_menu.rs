//! The viewport's right-click menu and the commands that hang off it (FEATURES.md 44,
//! 45, 46, 47, 51): Select submenu, Reverse / Orient Faces, Flip Along, Reset Scale /
//! Skew, Scale Definition, Divide, Weld Edges, Explode Curve, Zoom Selection, and Make
//! Group on empty space.
//!
//! A right click (no drag) picks the entity under the cursor; if it is not selected it
//! becomes the selection (SketchUp, unverified), a click on empty space clears it. The
//! menu lists the items for what is selected, greyed when they cannot apply. Item
//! order and wording are from memory of SketchUp 2026 (help page "Context menu items
//! for entities"); check in the trial.

use std::collections::{HashSet, VecDeque};

use eframe::egui;
use egui_wgpu::wgpu;
use glam::{DVec2, DVec3};
use mt_infer::Inference;
use mt_infer::units::{LengthUnit, parse_length};
use mt_kernel::{EdgeId, Entities, FaceId};
use mt_spec::MenuItem;

use crate::App;
use crate::document::Document;
use crate::document::{Owner, Scene};
use crate::modify::{Ctx, Hover, Marker, ModifyTool};
use crate::nav::Tool;
use crate::selection::{self, Mode};

/// Commands of this module (all in `IMPLEMENTED`).
pub(crate) const COMMANDS: &[&str] = &[
    "camera.zoom_selection",
    "edit.select_connected_faces",
    "edit.select_bounding_edges",
    "edit.select_all_connected",
    "edit.select_same_tag",
    "edit.select_same_material",
    "edit.select_connected_by_snaps",
    "edit.deselect_edges",
    "edit.deselect_faces",
    "edit.reverse_faces",
    "edit.orient_faces",
    "edit.flip_red",
    "edit.flip_green",
    "edit.flip_blue",
    "edit.reset_scale",
    "edit.reset_skew",
    "edit.scale_definition",
    "edit.divide",
    "edit.weld_edges",
    "edit.explode_curve",
];

/// State kept between frames: the command picked in the menu, run after the shell is
/// drawn (window commands need the shell, which is busy while the viewport draws).
#[derive(Default)]
pub(crate) struct ContextMenu {
    pending: Option<String>,
    /// The last right click was on the drawing axes (empty selection): the axes'
    /// menu (Place, Move, Reset, Hide).
    axes: bool,
}

/// What the selection holds, for building the menu and greying items.
#[derive(Default)]
struct Facts {
    faces: Vec<FaceId>,
    edges: Vec<EdgeId>,
    instances: Vec<usize>,
    /// Some instance is a component (not a group).
    component: bool,
    group: bool,
    hidden: bool,
    locked: bool,
    unlocked_instance: bool,
    curve: bool,
    /// Every selected edge belongs to the same curve (nothing to weld).
    one_curve: bool,
    divide: Option<EdgeId>,
    scaled: bool,
    skewed: bool,
    hidden_geometry_shown: bool,
    context_open: bool,
    /// The items of `context_extra`.
    extra: crate::context_extra::ExtraFacts,
    /// Right click on the drawing axes.
    axes: bool,
}

impl Facts {
    fn of(doc: &Document, items: &[Owner]) -> Self {
        let ents = doc.active();
        let mut f = Self {
            hidden_geometry_shown: doc.model.style.hidden_geometry,
            context_open: !doc.context.is_empty(),
            ..Default::default()
        };
        for &o in items {
            f.hidden |= is_hidden(ents, o);
            match o {
                Owner::Face(x) if ents.faces.contains_key(x) => f.faces.push(x),
                Owner::Edge(x) if ents.edges.contains_key(x) => {
                    f.edges.push(x);
                    f.curve |= ents.edges[x].attrs.curve.is_some();
                }
                Owner::Instance(i) => {
                    let Some(inst) = ents.instances.get(i) else {
                        continue;
                    };
                    f.instances.push(i);
                    let is_group = doc
                        .model
                        .definitions
                        .get(inst.definition)
                        .is_some_and(|d| d.is_group);
                    f.group |= is_group;
                    f.component |= !is_group;
                    if inst.locked {
                        f.locked = true;
                    } else {
                        f.unlocked_instance = true;
                        f.scaled |= mt_kernel::instance_ops::is_scaled(inst);
                        f.skewed |= mt_kernel::instance_ops::is_skewed(inst);
                    }
                }
                _ => {}
            }
        }
        f.divide = divide_target(ents, &f.edges);
        let curves: HashSet<_> = f.edges.iter().map(|&e| ents.edges[e].attrs.curve).collect();
        f.one_curve = curves.len() == 1 && curves.iter().all(Option::is_some);
        f
    }

    fn geometry(&self) -> bool {
        !self.faces.is_empty() || !self.edges.is_empty()
    }

    fn enabled(&self, id: &str) -> bool {
        if let Some(e) = self.extra.enabled(id) {
            return e;
        }
        let any = self.geometry() || !self.instances.is_empty();
        // Section planes, dimensions and texts can be erased, hidden and zoomed to.
        let annotation = !self.extra.sections.is_empty()
            || !self.extra.dims.is_empty()
            || !self.extra.texts.is_empty();
        match id {
            "edit.delete" | "edit.hide" | "camera.zoom_selection" if annotation => true,
            "edit.select_connected_faces" | "edit.select_all_connected" => self.geometry(),
            "edit.select_bounding_edges" | "edit.deselect_faces" => !self.faces.is_empty(),
            "edit.deselect_edges" => !self.edges.is_empty(),
            "edit.select_same_material" => !self.faces.is_empty() || !self.instances.is_empty(),
            "edit.select_connected_by_snaps" => !self.instances.is_empty(),
            "tool.snaps" => self.context_open || self.instances.len() == 1,
            "edit.reverse_faces"
            | "edit.orient_faces"
            | "edit.intersect_model"
            | "edit.intersect_selection" => !self.faces.is_empty(),
            "edit.flip_red" | "edit.flip_green" | "edit.flip_blue" => self.unlocked_instance,
            "edit.reset_scale" | "edit.scale_definition" => self.scaled,
            "edit.reset_skew" => self.skewed,
            "edit.divide" => self.divide.is_some(),
            "edit.weld_edges" => self.edges.len() >= 2 && !self.one_curve,
            "edit.explode_curve" => self.curve,
            "edit.explode" => self.unlocked_instance,
            "edit.make_unique" => self.component,
            "edit.intersect_context" => !self.faces.is_empty() && self.context_open,
            "edit.unhide_selected" => self.hidden,
            "edit.lock" => self.unlocked_instance,
            "edit.unlock_selected" => self.locked,
            // With nothing selected Make Group makes an empty group.
            "edit.create_group" => true,
            "edit.make_component"
            | "edit.delete"
            | "edit.hide"
            | "camera.zoom_selection"
            | "window.soften_edges" => any,
            _ => true,
        }
    }
}

fn is_hidden(ents: &Entities, o: Owner) -> bool {
    match o {
        Owner::Face(f) => ents.faces.get(f).is_some_and(|x| x.attrs.hidden),
        Owner::Edge(e) => ents.edges.get(e).is_some_and(|x| x.attrs.hidden),
        Owner::Instance(i) => ents.instances.get(i).is_some_and(|x| x.hidden),
        Owner::Section(i) => ents.section_planes.get(i).is_some_and(|x| x.hidden),
        Owner::Dimension(i) => ents.dimensions.get(i).is_some_and(|x| x.hidden),
        Owner::Text(i) => ents.texts.get(i).is_some_and(|x| x.hidden),
    }
}

/// The edge Divide works on: one straight edge, or the edges of one arc curve.
fn divide_target(ents: &Entities, edges: &[EdgeId]) -> Option<EdgeId> {
    let first = *edges.first()?;
    let curve = ents.edges.get(first)?.attrs.curve;
    let one = match curve {
        None => edges.len() == 1,
        Some(c) => edges
            .iter()
            .all(|e| ents.edges.get(*e).is_some_and(|x| x.attrs.curve == Some(c))),
    };
    (one && mt_kernel::curves::can_divide(ents, first)).then_some(first)
}

fn cmd(id: &str, label: &str) -> MenuItem {
    MenuItem::Command {
        id: id.to_owned(),
        label: label.to_owned(),
    }
}

fn sub(label: &str, items: Vec<MenuItem>) -> MenuItem {
    MenuItem::Submenu {
        label: label.to_owned(),
        items,
    }
}

fn sep() -> MenuItem {
    MenuItem::Separator { separator: true }
}

/// The menu for a selection with `f` (empty: the empty-space menu).
fn menu(f: &Facts, empty: bool) -> Vec<MenuItem> {
    if empty && f.axes {
        return vec![
            cmd("view.axes_place", "Place"),
            cmd("view.axes_move", "Move..."),
            cmd("view.axes_reset", "Reset"),
            cmd("view.axes_hide", "Hide"),
        ];
    }
    if empty {
        let mut v = vec![
            cmd("edit.select_all", "Select All"),
            sep(),
            cmd("edit.create_group", "Make Group"),
        ];
        // Inside a group or component (help: Snaps; position unverified).
        if f.context_open {
            v.push(cmd("tool.snaps", "Make/Edit Snaps"));
        }
        return v;
    }
    let mut v = vec![
        cmd("window.entity_info", "Entity Info"),
        cmd("edit.delete", "Erase"),
    ];
    if f.hidden && f.hidden_geometry_shown {
        v.push(cmd("edit.unhide_selected", "Unhide"));
    } else {
        v.push(cmd("edit.hide", "Hide"));
    }
    if !f.instances.is_empty() {
        if f.locked {
            v.push(cmd("edit.unlock_selected", "Unlock"));
        } else {
            v.push(cmd("edit.lock", "Lock"));
        }
    }
    v.push(sep());
    if !f.instances.is_empty() {
        if f.component {
            v.push(cmd("edit.make_unique", "Make Unique"));
        }
        v.push(cmd("edit.explode", "Explode"));
    }
    v.push(sub(
        "Select",
        vec![
            cmd("edit.select_connected_faces", "Connected Faces"),
            cmd("edit.select_bounding_edges", "Bounding Edges"),
            cmd("edit.select_all_connected", "All Connected"),
            cmd("edit.select_same_tag", "All with Same Tag"),
            cmd("edit.select_same_material", "All with Same Material"),
            cmd("edit.select_connected_by_snaps", "Connected by Snaps"),
            sep(),
            cmd("edit.deselect_edges", "Deselect Edges"),
            cmd("edit.deselect_faces", "Deselect Faces"),
            sep(),
            cmd("edit.invert_selection", "Invert Selection"),
        ],
    ));
    if !f.edges.is_empty() {
        v.push(cmd("edit.divide", "Divide"));
        v.push(cmd("edit.weld_edges", "Weld Edges"));
        v.push(cmd("edit.explode_curve", "Explode Curve"));
        if f.extra.curves {
            v.push(cmd("edit.find_center", "Find Center"));
            v.push(cmd("edit.convert_to_polygon", "Convert to Polygon"));
        }
    }
    let x = &f.extra;
    if !x.sections.is_empty() {
        v.push(cmd("edit.section_active", "Active Cut"));
        v.push(cmd("edit.section_reverse", "Reverse"));
        v.push(cmd("camera.align_view", "Align View"));
        v.push(cmd("edit.section_slice_group", "Create Group from Slice"));
        v.push(cmd(
            "edit.section_troubleshoot",
            "Troubleshoot Section Fill",
        ));
    }
    if !x.dims.is_empty() {
        v.push(sub(
            "Text Position",
            vec![
                cmd("edit.dim_text_start", "Outside Start"),
                cmd("edit.dim_text_center", "Center"),
                cmd("edit.dim_text_end", "Outside End"),
            ],
        ));
        if x.radial {
            v.push(sub(
                "Type",
                vec![
                    cmd("edit.dim_type_radius", "Radius"),
                    cmd("edit.dim_type_diameter", "Diameter"),
                ],
            ));
        }
    }
    if x.leader {
        v.push(sub(
            "Arrow",
            vec![
                cmd("edit.text_arrow_none", "None"),
                cmd("edit.text_arrow_dot", "Dot"),
                cmd("edit.text_arrow_closed", "Closed"),
                cmd("edit.text_arrow_open", "Open"),
            ],
        ));
        v.push(sub(
            "Leader",
            vec![
                cmd("edit.text_leader_view", "View Based"),
                cmd("edit.text_leader_pushpin", "Pushpin"),
                cmd("edit.text_leader_hidden", "Hidden"),
            ],
        ));
    }
    v.push(sep());
    if !f.instances.is_empty() {
        v.push(cmd("edit.reset_scale", "Reset Scale"));
        v.push(cmd("edit.reset_skew", "Reset Skew"));
        v.push(cmd("edit.scale_definition", "Scale Definition"));
    }
    if f.component {
        v.push(cmd("edit.change_axes", "Change Axes"));
        if x.glued {
            v.push(cmd("edit.unglue", "Unglue"));
        }
        v.push(cmd("edit.component_reload", "Reload..."));
        v.push(cmd("edit.component_save_as", "Save As..."));
        v.push(cmd("edit.select_instances", "Select Instances"));
        v.push(cmd("edit.replace_selected", "Replace Selected"));
    }
    if !f.instances.is_empty() || f.context_open {
        v.push(cmd("tool.snaps", "Make/Edit Snaps"));
    }
    v.push(cmd("edit.make_component", "Make Component..."));
    v.push(cmd("edit.create_group", "Make Group"));
    v.push(sub(
        "Intersect Faces",
        vec![
            cmd("edit.intersect_model", "With Model"),
            cmd("edit.intersect_selection", "With Selection"),
            cmd("edit.intersect_context", "With Context"),
        ],
    ));
    if !f.faces.is_empty() {
        v.push(cmd("edit.reverse_faces", "Reverse Faces"));
        v.push(cmd("edit.orient_faces", "Orient Faces"));
        if f.faces.len() == 1 {
            v.push(cmd("camera.align_view", "Align View"));
            v.push(cmd("edit.align_axes", "Align Axes"));
        }
    }
    if !f.faces.is_empty() || !f.instances.is_empty() {
        v.push(sub(
            "Area",
            vec![
                cmd("edit.area_selection", "Selection"),
                cmd("edit.area_tag", "Tag"),
                cmd("edit.area_material", "Material"),
            ],
        ));
    }
    if f.faces.len() == 1 {
        v.push(sub(
            "Texture",
            vec![cmd("tool.texture_position", "Position")],
        ));
    }
    if !f.instances.is_empty() {
        let whose = if f.group && !f.component {
            "Group's"
        } else {
            "Component's"
        };
        v.push(sub(
            "Flip Along",
            vec![
                cmd("edit.flip_red", &format!("{whose} Red")),
                cmd("edit.flip_green", &format!("{whose} Green")),
                cmd("edit.flip_blue", &format!("{whose} Blue")),
            ],
        ));
    }
    v.push(sep());
    if !f.edges.is_empty() || !f.instances.is_empty() || !f.faces.is_empty() {
        v.push(cmd("window.soften_edges", "Soften/Smooth Edges"));
    }
    v.push(cmd("camera.zoom_selection", "Zoom Selection"));
    v
}

/// Every item the right-click menus can show, for Help > Search: (id, label,
/// submenu path).
pub(crate) fn all_items() -> Vec<(String, String, String)> {
    let full = Facts {
        faces: vec![FaceId::default()],
        edges: vec![EdgeId::default()],
        instances: vec![0],
        component: true,
        group: true,
        extra: crate::context_extra::ExtraFacts {
            curves: true,
            sections: vec![0],
            dims: vec![0],
            radial: true,
            leader: true,
            glued: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let axes = Facts {
        axes: true,
        ..Default::default()
    };
    let mut out = Vec::new();
    fn walk(items: &[MenuItem], path: &str, out: &mut Vec<(String, String, String)>) {
        for i in items {
            match i {
                MenuItem::Command { id, label } => {
                    if !out.iter().any(|(x, _, _)| x == id) {
                        out.push((id.clone(), label.clone(), path.to_owned()));
                    }
                }
                MenuItem::Submenu { label, items } => {
                    walk(items, &format!("{path} > {label}"), out)
                }
                MenuItem::Separator { .. } => {}
            }
        }
    }
    walk(&menu(&full, false), "Context menu", &mut out);
    walk(&menu(&axes, true), "Context menu > Axes", &mut out);
    out
}

/// Bounds of the selected entities as drawn (world space).
pub(crate) fn selection_bounds(scene: &Scene, items: &[Owner]) -> Option<(DVec3, DVec3)> {
    let set: HashSet<Owner> = items.iter().copied().collect();
    let tris = scene
        .triangles
        .iter()
        .zip(&scene.tri_owner)
        .filter(|(_, o)| o.is_some_and(|o| set.contains(&o)))
        .flat_map(|(t, _)| *t);
    let segs = scene
        .edges
        .iter()
        .zip(&scene.edge_owner)
        .filter(|(_, o)| o.is_some_and(|o| set.contains(&o)))
        .flat_map(|(s, _)| [s.a, s.b])
        .map(|p| DVec3::new(p[0].into(), p[1].into(), p[2].into()));
    let mut it = tris.chain(segs);
    let first = it.next()?;
    Some(it.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
}

/// Select > Connected Faces: the faces reached from the selected faces and edges
/// through shared edges.
pub(crate) fn connected_faces(ents: &Entities, items: &[Owner]) -> Vec<Owner> {
    let mut seen: HashSet<FaceId> = HashSet::new();
    let mut queue: VecDeque<FaceId> = VecDeque::new();
    for o in items {
        let start: Vec<FaceId> = match *o {
            Owner::Face(f) if ents.faces.contains_key(f) => vec![f],
            Owner::Edge(e) => ents
                .edges
                .get(e)
                .map(|x| x.faces.clone())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        for f in start {
            if seen.insert(f) {
                queue.push_back(f);
            }
        }
    }
    let mut out = Vec::new();
    while let Some(f) = queue.pop_front() {
        out.push(Owner::Face(f));
        for l in ents.faces[f].loops() {
            for c in &l.coedges {
                for &g in &ents.edges[c.edge].faces {
                    if seen.insert(g) {
                        queue.push_back(g);
                    }
                }
            }
        }
    }
    out
}

/// Select > Bounding Edges: the edges of the selected faces' loops.
pub(crate) fn bounding_edges(ents: &Entities, items: &[Owner]) -> Vec<Owner> {
    let mut seen = HashSet::new();
    items
        .iter()
        .filter_map(|o| match *o {
            Owner::Face(f) => ents.faces.get(f),
            _ => None,
        })
        .flat_map(|f| f.loops().flat_map(|l| l.coedges.iter().map(|c| c.edge)))
        .filter(|e| seen.insert(*e))
        .map(Owner::Edge)
        .collect()
}

fn layer_of(ents: &Entities, o: Owner) -> Option<Option<usize>> {
    match o {
        Owner::Face(f) => ents.faces.get(f).map(|x| x.attrs.layer),
        Owner::Edge(e) => ents.edges.get(e).map(|x| x.attrs.layer),
        Owner::Instance(i) => ents.instances.get(i).map(|x| x.layer),
        _ => None,
    }
}

fn material_of(ents: &Entities, o: Owner) -> Option<Option<usize>> {
    match o {
        Owner::Face(f) => ents.faces.get(f).map(|x| x.attrs.material),
        Owner::Instance(i) => ents.instances.get(i).map(|x| x.material),
        _ => None,
    }
}

/// Every visible face, edge and instance of the context.
fn every(ents: &Entities) -> Vec<Owner> {
    ents.faces
        .iter()
        .filter(|(_, f)| !f.attrs.hidden)
        .map(|(id, _)| Owner::Face(id))
        .chain(
            ents.edges
                .iter()
                .filter(|(_, e)| !e.attrs.hidden)
                .map(|(id, _)| Owner::Edge(id)),
        )
        .chain(
            ents.instances
                .iter()
                .enumerate()
                .filter(|(_, i)| !i.hidden)
                .map(|(i, _)| Owner::Instance(i)),
        )
        .collect()
}

/// Select > All with Same Tag / Material: everything in the context sharing a tag
/// (material) with the selection. Untagged and the default material count as values.
/// Edges have no material.
pub(crate) fn same(ents: &Entities, items: &[Owner], material: bool) -> Vec<Owner> {
    let key = |o: Owner| {
        if material {
            material_of(ents, o)
        } else {
            layer_of(ents, o)
        }
    };
    let wanted: HashSet<Option<usize>> = items.iter().filter_map(|o| key(*o)).collect();
    every(ents)
        .into_iter()
        .filter(|o| key(*o).is_some_and(|k| wanted.contains(&k)))
        .collect()
}

/// Select > All Connected: everything connected to the selected faces and edges.
pub(crate) fn all_connected(ents: &Entities, items: &[Owner]) -> Vec<Owner> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for &o in items {
        if !matches!(o, Owner::Face(_) | Owner::Edge(_)) || seen.contains(&o) {
            continue;
        }
        for x in selection::expand(ents, o, 3) {
            if seen.insert(x) {
                out.push(x);
            }
        }
    }
    out
}

impl App {
    /// Right click in the viewport: picks, then shows the menu. Called by the viewport
    /// every frame.
    pub(crate) fn context_menu(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        viewport_px: DVec2,
        device: &wgpu::Device,
    ) {
        if response.secondary_clicked()
            && let Some(p) = response.interact_pointer_pos()
        {
            let ppp = ui.ctx().pixels_per_point();
            let cursor = DVec2::new(
                f64::from((p.x - rect.min.x) * ppp),
                f64::from((p.y - rect.min.y) * ppp),
            );
            let hit = selection::pick(
                &self.scene,
                self.bvh.as_ref(),
                &self.camera,
                cursor,
                viewport_px,
            );
            match (hit, &self.document) {
                (Some(h), Some(doc)) if !self.selection.contains(&h) => {
                    let items = selection::expand(doc.active(), h, 1);
                    self.select(device, Mode::Replace, items);
                }
                (None, _) => self.select(device, Mode::Replace, Vec::new()),
                _ => {}
            }
            self.ctx_menu.axes = hit.is_none() && self.near_axes(cursor, viewport_px);
        }
        let Some(doc) = &self.document else {
            return;
        };
        let items = self.selection.items().to_vec();
        let mut facts = Facts::of(doc, &items);
        facts.extra = self.extra_facts(doc);
        facts.axes = self.ctx_menu.axes;
        let list = menu(&facts, items.is_empty());
        let spec = &self.spec;
        let mut picked = None;
        response.context_menu(|ui| {
            ui.set_min_width(200.0);
            picked = mt_ui::menu_list(ui, &list, spec, &|id| facts.enabled(id), &|_| false);
        });
        if picked.is_some() {
            self.ctx_menu.pending = picked;
        }
    }

    /// The cursor (viewport pixels) is within a few pixels of a drawn axis line.
    fn near_axes(&self, cursor: DVec2, viewport_px: DVec2) -> bool {
        let Some(doc) = &self.document else {
            return false;
        };
        if !doc.model.style.axes {
            return false;
        }
        let a = crate::model_axes::axes_of(doc.model.axes);
        let len = self.camera.distance() * 0.5;
        let o = self.camera.project(a.origin, viewport_px);
        a.dirs.iter().any(|d| {
            let (Some(p), Some(q)) = (o, self.camera.project(a.origin + *d * len, viewport_px))
            else {
                return false;
            };
            let pq = q - p;
            let t = ((cursor - p).dot(pq) / pq.length_squared().max(1e-9)).clamp(0.0, 1.0);
            (p + pq * t).distance(cursor) <= 6.0
        })
    }

    /// Runs the command picked in the context menu (after the shell is drawn).
    pub(crate) fn context_menu_command(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(id) = self.ctx_menu.pending.take() else {
            return;
        };
        if let Some(shell) = &mut self.shell {
            shell.open_panel(&id);
        }
        self.run_command(ctx, frame, &id);
    }

    /// The commands of this module; false if `id` is not one.
    pub(crate) fn context_command(&mut self, frame: &eframe::Frame, id: &str) -> bool {
        let empty_group = id == "edit.create_group" && self.selection.items().is_empty();
        if !(COMMANDS.contains(&id) || empty_group) {
            return false;
        }
        let Some(rs) = frame.wgpu_render_state() else {
            return true;
        };
        let device = rs.device.clone();
        if id == "camera.zoom_selection" {
            return self.camera_command(id);
        }
        let Some(doc) = self.document.as_ref() else {
            return true;
        };
        let items = self.selection.items().to_vec();
        let facts = Facts::of(doc, &items);
        let ents = doc.active();
        let select = match id {
            "edit.select_connected_faces" => {
                Some([items.clone(), connected_faces(ents, &items)].concat())
            }
            "edit.select_bounding_edges" => {
                Some([items.clone(), bounding_edges(ents, &items)].concat())
            }
            "edit.select_all_connected" => Some(all_connected(ents, &items)),
            "edit.select_same_tag" => Some(same(ents, &items, false)),
            "edit.select_same_material" => Some(same(ents, &items, true)),
            "edit.select_connected_by_snaps" => Some(crate::component_snaps::connected(
                &doc.model.definitions,
                ents,
                &items,
            )),
            "edit.deselect_edges" => Some(
                items
                    .iter()
                    .copied()
                    .filter(|o| !matches!(o, Owner::Edge(_)))
                    .collect(),
            ),
            "edit.deselect_faces" => Some(
                items
                    .iter()
                    .copied()
                    .filter(|o| !matches!(o, Owner::Face(_)))
                    .collect(),
            ),
            _ => None,
        };
        if let Some(items) = select {
            self.select(&device, Mode::Replace, items);
            return true;
        }
        if id == "edit.divide" {
            if let Some(e) = facts.divide {
                self.tool = Tool::Modify("edit.divide");
                self.draw = None;
                self.modify = Some(Box::new(Divide::new(e)));
                self.erase_drag.clear();
                self.engine.unlock();
                self.inference = None;
            }
            return true;
        }
        let Some(doc) = self.document.as_mut() else {
            return true;
        };
        let unlocked: Vec<usize> = facts
            .instances
            .iter()
            .copied()
            .filter(|&i| !doc.active().instances[i].locked)
            .collect();
        let changed = match id {
            "edit.create_group" => {
                let mut made = None;
                doc.edit_model(|model, ents| {
                    let edit = crate::sandbox::group_edit(Entities::new(), glam::DMat4::IDENTITY);
                    made = edit(model, ents).first().copied();
                    true
                });
                // The empty group opens for drawing into (help: Grouping geometry).
                if let Some(Owner::Instance(i)) = made {
                    doc.open_instance(i);
                }
                self.selection.clear();
                true
            }
            "edit.reverse_faces" => {
                doc.edit(|ents| mt_kernel::orient::reverse_faces(ents, &facts.faces))
            }
            "edit.orient_faces" => doc.edit(|ents| {
                let mut any = false;
                for &f in &facts.faces {
                    any |= !mt_kernel::orient::orient_faces(ents, f).is_empty();
                }
                any
            }),
            "edit.flip_red" | "edit.flip_green" | "edit.flip_blue" => {
                let axis = match id {
                    "edit.flip_red" => 0,
                    "edit.flip_green" => 1,
                    _ => 2,
                };
                doc.edit_model(|model, ents| {
                    let mut any = false;
                    for &i in &unlocked {
                        any |= mt_kernel::instance_ops::flip_along(
                            &model.definitions,
                            &mut ents.instances[i],
                            axis,
                        );
                    }
                    any
                })
            }
            "edit.reset_scale" | "edit.reset_skew" => doc.edit(|ents| {
                let mut any = false;
                for &i in &unlocked {
                    let inst = &mut ents.instances[i];
                    any |= if id == "edit.reset_scale" {
                        mt_kernel::instance_ops::reset_scale(inst)
                    } else {
                        mt_kernel::instance_ops::reset_skew(inst)
                    };
                }
                any
            }),
            "edit.scale_definition" => doc.edit_model(|model, ents| {
                let mut any = false;
                for &i in &unlocked {
                    any |= mt_kernel::instance_ops::scale_definition(model, ents, i);
                }
                any
            }),
            "edit.weld_edges" => {
                doc.edit(|ents| !mt_kernel::curves::weld_edges(ents, &facts.edges).is_empty())
            }
            "edit.explode_curve" => {
                doc.edit(|ents| mt_kernel::curves::explode_curves(ents, &facts.edges))
            }
            _ => false,
        };
        if changed {
            let ents = self.document.as_ref().map(Document::active);
            if let Some(ents) = ents {
                self.selection.retain(|o| selection::exists(ents, o));
            }
            self.rebuild(&device);
        }
        true
    }
}

/// Divide: the division points follow the cursor along the edge (closer to an end:
/// more segments, the middle: two), a click divides; or type the count (`5` or `5s`)
/// or a segment length (`2.5`, `30cm`: the count that comes closest). Escape leaves it.
pub(crate) struct Divide {
    edge: EdgeId,
    count: usize,
    points: Vec<DVec3>,
    done: bool,
    made: Option<Vec<Owner>>,
}

/// Most segments the cursor picks (typed counts go up to the kernel's limit).
const CURSOR_MAX: usize = 100;
/// Pixels from the edge within which the cursor changes the count.
const NEAR_PX: f64 = 40.0;

impl Divide {
    pub(crate) fn new(edge: EdgeId) -> Self {
        Self {
            edge,
            count: 2,
            points: Vec::new(),
            done: false,
            made: None,
        }
    }

    fn apply(&mut self, ents: &mut Entities, n: usize) -> bool {
        let new = mt_kernel::curves::divide(ents, self.edge, n);
        self.done = true;
        if new.is_empty() {
            return false;
        }
        self.made = Some(new.into_iter().map(Owner::Edge).collect());
        true
    }
}

/// Segments for the cursor at fraction `t` of the way along the edge: two in the
/// middle, more towards either end.
pub(crate) fn count_at(t: f64, closed: bool) -> usize {
    let d = t.clamp(0.0, 1.0).min(1.0 - t.clamp(0.0, 1.0));
    let n = (0.5 / d.max(1e-3)).round() as usize;
    n.clamp(if closed { 3 } else { 2 }, CURSOR_MAX)
}

impl ModifyTool for Divide {
    fn status(&self) -> String {
        "Move along the edge to set the number of segments, click to divide, or type a \
         number of segments or a segment length."
            .to_owned()
    }

    fn measurement(&self, _cur: Option<&Inference>, _units: LengthUnit) -> (&'static str, String) {
        ("Segments", self.count.to_string())
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    fn click(&mut self, ctx: &mut Ctx, _inf: &Inference) -> bool {
        let n = self.count;
        self.apply(ctx.ents, n)
    }

    fn typed(
        &mut self,
        ctx: &mut Ctx,
        _cur: Option<&Inference>,
        text: &str,
        units: LengthUnit,
    ) -> bool {
        match typed_count(ctx.ents, self.edge, text, units) {
            Some(n) => self.apply(ctx.ents, n),
            None => false,
        }
    }

    fn preview(&self, _ents: &Entities, _cur: &Inference) -> Vec<Vec<DVec3>> {
        Vec::new()
    }

    fn cancel(&mut self) {
        self.done = true;
    }

    fn finished(&self) -> bool {
        self.done
    }

    fn take_selection(&mut self) -> Option<Vec<Owner>> {
        self.made.take()
    }

    fn hover(&mut self, h: &Hover) {
        if !h.ents.edges.contains_key(self.edge) {
            self.done = true;
            return;
        }
        // The edge (or arc) as drawn now, and where along it the cursor is.
        let Some(now) = curve_points(h.ents, self.edge) else {
            self.done = true;
            return;
        };
        let closed = now.first() == now.last() && now.len() > 2;
        if let Some(c) = h.cursor {
            let screen: Vec<Option<DVec2>> = now.iter().map(|p| (h.project)(*p)).collect();
            let lens: Vec<f64> = now.windows(2).map(|w| w[0].distance(w[1])).collect();
            let total: f64 = lens.iter().sum();
            let mut best: Option<(f64, f64)> = None; // (pixel distance, length along)
            let mut along = 0.0;
            for (i, w) in screen.windows(2).enumerate() {
                if let [Some(a), Some(b)] = w {
                    let ab = *b - *a;
                    let s = if ab.length_squared() > 0.0 {
                        ((c - *a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let d = (*a + ab * s).distance(c);
                    if best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, along + s * lens[i]));
                    }
                }
                along += lens[i];
            }
            if let Some((d, at)) = best
                && d <= NEAR_PX
                && total > 0.0
            {
                self.count = count_at(at / total, closed);
            }
        }
        self.points =
            mt_kernel::curves::divide_points(h.ents, self.edge, self.count).unwrap_or_default();
    }

    fn markers(&self, _cur: &Inference) -> Vec<Marker> {
        // Division points between the ends (a circle shows all of them).
        let n = self.points.len();
        if n < 2 {
            return Vec::new();
        }
        let closed = self.points.first() == self.points.last();
        let range = if closed { 0..n - 1 } else { 1..n - 1 };
        self.points[range]
            .iter()
            .map(|&point| Marker { point, hot: true })
            .collect()
    }
}

/// The segment count typed into Divide on edge `e`: a whole number (optionally
/// followed by `s`) is a count of two or more; anything else that reads as a length is
/// a segment length, giving the count that comes closest to it.
fn typed_count(ents: &Entities, e: EdgeId, text: &str, units: LengthUnit) -> Option<usize> {
    let text = text.trim();
    if let Ok(n) = text.trim_end_matches(['s', 'S']).trim().parse::<usize>() {
        return (n >= 2).then_some(n);
    }
    mt_kernel::curves::count_for_length(ents, e, parse_length(text, units)?)
}

/// The points along `e`'s divide target as it is now (its chain's vertices).
fn curve_points(ents: &Entities, e: EdgeId) -> Option<Vec<DVec3>> {
    let edges = match ents.edges.get(e)?.attrs.curve {
        Some(c) => ents.curve_edges(c),
        None => vec![e],
    };
    let (_, verts) = mt_kernel::curves::chain_of(ents, &edges)?;
    Some(verts.into_iter().map(|v| ents.pos(v)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_kernel::add_line;

    fn p(x: f64, y: f64, z: f64) -> DVec3 {
        DVec3::new(x, y, z)
    }

    /// Two squares sharing an edge, plus a loose edge.
    fn model() -> Entities {
        let mut e = Entities::new();
        let pts = [
            p(0.0, 0.0, 0.0),
            p(10.0, 0.0, 0.0),
            p(10.0, 10.0, 0.0),
            p(0.0, 10.0, 0.0),
        ];
        for i in 0..4 {
            add_line(&mut e, pts[i], pts[(i + 1) % 4]);
        }
        add_line(&mut e, p(10.0, 0.0, 0.0), p(20.0, 0.0, 0.0));
        add_line(&mut e, p(20.0, 0.0, 0.0), p(20.0, 10.0, 0.0));
        add_line(&mut e, p(20.0, 10.0, 0.0), p(10.0, 10.0, 0.0));
        add_line(&mut e, p(50.0, 0.0, 0.0), p(60.0, 0.0, 0.0));
        assert_eq!(e.faces.len(), 2);
        e
    }

    #[test]
    fn select_submenu() {
        let mut e = model();
        let f = e.faces.keys().next().unwrap();
        let faces = connected_faces(&e, &[Owner::Face(f)]);
        assert_eq!(faces.len(), 2);
        let edges = bounding_edges(&e, &[Owner::Face(f)]);
        assert_eq!(edges.len(), 4);
        // Everything but the loose edge: 2 faces and 7 edges.
        assert_eq!(all_connected(&e, &[Owner::Face(f)]).len(), 9);
        // Same tag: all untagged entities.
        assert_eq!(same(&e, &[Owner::Face(f)], false).len(), 2 + 8);
        // Same material: both default faces; then only the painted one.
        assert_eq!(same(&e, &[Owner::Face(f)], true).len(), 2);
        e.faces[f].attrs.material = Some(1);
        assert_eq!(same(&e, &[Owner::Face(f)], true), vec![Owner::Face(f)]);
    }

    #[test]
    fn divide_count_follows_the_cursor() {
        assert_eq!(count_at(0.5, false), 2);
        assert_eq!(count_at(0.5, true), 3);
        assert_eq!(count_at(0.25, false), 2);
        assert_eq!(count_at(0.1, false), 5);
        assert_eq!(count_at(0.9, false), 5);
        assert_eq!(count_at(0.0, false), CURSOR_MAX);
    }

    #[test]
    fn divide_takes_a_count_or_a_segment_length() {
        let e = model();
        // The loose edge is 10" long.
        let loose = e
            .edges
            .keys()
            .find(|&x| e.pos(e.edges[x].start).x == 50.0)
            .unwrap();
        let inch = LengthUnit::Inch;
        assert_eq!(typed_count(&e, loose, "4", inch), Some(4));
        assert_eq!(typed_count(&e, loose, "4s", inch), Some(4));
        assert_eq!(typed_count(&e, loose, "1", inch), None);
        assert_eq!(typed_count(&e, loose, "2.5", inch), Some(4));
        assert_eq!(typed_count(&e, loose, "3\"", inch), Some(3));
        assert_eq!(typed_count(&e, loose, "1'", inch), Some(2));
        assert_eq!(typed_count(&e, loose, "abc", inch), None);
    }

    #[test]
    fn menu_items_follow_the_selection() {
        let e = model();
        let ids = |m: &[MenuItem]| {
            fn walk(m: &[MenuItem], out: &mut Vec<String>) {
                for i in m {
                    match i {
                        MenuItem::Command { id, .. } => out.push(id.clone()),
                        MenuItem::Submenu { items, .. } => walk(items, out),
                        MenuItem::Separator { .. } => {}
                    }
                }
            }
            let mut out = Vec::new();
            walk(m, &mut out);
            out
        };
        let empty = ids(&menu(&Facts::default(), true));
        assert_eq!(empty, ["edit.select_all", "edit.create_group"]);
        assert!(Facts::default().enabled("edit.create_group"));
        let f = e.faces.keys().next().unwrap();
        let facts = Facts {
            faces: vec![f],
            ..Default::default()
        };
        let face_menu = ids(&menu(&facts, false));
        assert!(face_menu.contains(&"edit.reverse_faces".to_owned()));
        assert!(!face_menu.contains(&"edit.divide".to_owned()));
        assert!(!face_menu.contains(&"edit.flip_red".to_owned()));
        assert!(facts.enabled("edit.orient_faces"));
        assert!(!facts.enabled("edit.deselect_edges"));
        // Every command of the menus is a known command.
        for id in face_menu.iter().chain(&empty) {
            assert!(crate::IMPLEMENTED.contains(&id.as_str()), "{id}");
        }
        for id in COMMANDS {
            assert!(crate::IMPLEMENTED.contains(id), "{id}");
        }
        // A loose straight edge can be divided; welding needs two edges.
        let loose = e
            .edges
            .keys()
            .find(|&x| e.edges[x].faces.is_empty() && e.pos(e.edges[x].start).x >= 50.0)
            .unwrap();
        assert_eq!(divide_target(&e, &[loose]), Some(loose));
        let two: Vec<EdgeId> = e.edges.keys().take(2).collect();
        assert_eq!(divide_target(&e, &two), None);
    }
}
