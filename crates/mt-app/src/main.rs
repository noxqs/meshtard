//! meshtard desktop application.

mod document;
mod nav;
mod overlay;
mod selection;
mod tools;

use eframe::egui;
use egui_wgpu::wgpu;
use glam::{DVec2, DVec3};
use mt_kernel::bvh::Bvh;
use mt_render::{Camera, FaceVertex, Segment, Style, Viewport};
use mt_spec::{Shortcut, Spec};

use document::{Document, Owner, Scene};
use nav::Tool;
use selection::{Mode, Rect, Selection};

/// SketchUp's selection highlight color (draft: plain blue).
const SELECTED: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// Touchpad scroll distance, in points, that counts as one wheel notch.
const POINTS_PER_NOTCH: f32 = 40.0;

/// Commands that do something today. Every other menu item is shown disabled.
const IMPLEMENTED: &[&str] = &[
    "file.open",
    "file.exit",
    "tool.select",
    "tool.line",
    "tool.rectangle",
    "tool.rotated_rectangle",
    "tool.circle",
    "tool.polygon",
    "tool.arc",
    "tool.arc_2pt",
    "tool.arc_3pt",
    "tool.pie",
    "tool.freehand",
    "tool.eraser",
    "edit.select_all",
    "edit.select_none",
    "edit.delete",
    "camera.orbit",
    "camera.pan",
    "camera.zoom",
];

fn main() -> eframe::Result {
    env_logger::init();
    let spec = match std::env::var_os("MESHTARD_SPEC_DIR") {
        Some(dir) => Spec::load_dir(dir.as_ref()),
        None => Spec::builtin(),
    }
    .unwrap_or_else(|e| {
        eprintln!("spec error: {e}");
        std::process::exit(1);
    });

    let open_path = std::env::args_os().nth(1).map(std::path::PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Untitled - meshtard")
            .with_inner_size([1280.0, 800.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "meshtard",
        options,
        Box::new(|cc| {
            let render_state = cc
                .wgpu_render_state
                .as_ref()
                .ok_or("wgpu renderer not available")?;
            // SketchUp on Windows uses the light system theme; full styling comes in M3.
            cc.egui_ctx.set_theme(egui::Theme::Light);
            let mut app = App::new(spec, render_state);
            if let Some(path) = open_path {
                app.open(&cc.egui_ctx, &render_state.device, &path);
            }
            Ok(Box::new(app))
        }),
    )
}

struct App {
    spec: Spec,
    camera: Camera,
    tool: Tool,
    viewport: Viewport,
    texture: Option<(egui::TextureId, [u32; 2])>,
    /// Shortcut for each implemented command, parsed once.
    shortcuts: Vec<(Shortcut, egui::Key, &'static str)>,
    /// The window around the viewport; taken out while it draws (it calls back into
    /// the app for the viewport).
    shell: Option<mt_ui::Shell>,
    document: Option<Document>,
    /// Drawn geometry of the document and its pick index.
    scene: Scene,
    bvh: Bvh,
    selection: Selection,
    /// Where a selection box drag started, in viewport pixels.
    box_start: Option<DVec2>,
    /// Snappable geometry and the inference state for drawing tools.
    geometry: mt_infer::Geometry,
    engine: mt_infer::Engine,
    inference: Option<mt_infer::Inference>,
    /// The active drawing tool, when `tool` is `Tool::Draw`.
    draw: Option<Box<dyn tools::DrawTool>>,
    /// Edges picked during an Eraser drag, erased on release.
    erase_drag: Vec<Owner>,
    /// The direction lock comes from holding Shift (released with it).
    shift_lock: bool,
    /// Error from the last command, shown in the status bar until the next one.
    message: Option<String>,
}

impl App {
    fn new(spec: Spec, render_state: &egui_wgpu::RenderState) -> Self {
        let shortcuts = IMPLEMENTED
            .iter()
            .filter_map(|&id| {
                let shortcut = spec.keymap.shortcut(id)?;
                let key = egui::Key::from_name(&shortcut.key)?;
                Some((shortcut, key, id))
            })
            .collect();
        let mut app = Self {
            shell: Some(mt_ui::Shell::new(&spec)),
            camera: Camera::from_spec(&spec.default_model.camera),
            tool: Tool::Select,
            viewport: Viewport::new(&render_state.device, Style::from_spec(&spec.default_model)),
            texture: None,
            shortcuts,
            spec,
            document: None,
            scene: Scene::default(),
            bvh: Bvh::default(),
            selection: Selection::default(),
            box_start: None,
            geometry: mt_infer::Geometry::default(),
            engine: mt_infer::Engine::default(),
            inference: None,
            draw: None,
            erase_drag: Vec::new(),
            shift_lock: false,
            message: None,
        };
        // SketchUp starts with an empty model ready to draw in.
        app.document = Some(Document::untitled());
        app.rebuild(&render_state.device);
        app
    }

    /// Rebuilds the drawn scene and pick index after the model changed.
    fn rebuild(&mut self, device: &wgpu::Device) {
        self.scene = self
            .document
            .as_ref()
            .map(Document::scene)
            .unwrap_or_default();
        self.bvh = Bvh::build(self.scene.triangles.clone());
        let v3 = |p: [f32; 3]| DVec3::new(p[0].into(), p[1].into(), p[2].into());
        self.geometry = mt_infer::Geometry::from_edges(
            self.scene
                .edges
                .iter()
                .map(|s| [v3(s.a), v3(s.b)])
                .collect(),
        );
        self.engine.reference_edge = None;
        self.viewport
            .set_faces(device, &self.scene.vertices, &self.scene.indices);
        self.update_highlight(device);
    }

    /// Colors selected edges, dots selected faces and boxes selected components.
    fn update_highlight(&mut self, device: &wgpu::Device) {
        let mut edges = self.scene.edges.clone();
        for (seg, owner) in edges.iter_mut().zip(&self.scene.edge_owner) {
            if self.selection.contains(owner) {
                seg.color = SELECTED;
            }
        }
        let mut boxes: std::collections::HashMap<usize, (glam::DVec3, glam::DVec3)> =
            Default::default();
        let mut grow = |i: usize, p: glam::DVec3| {
            let b = boxes.entry(i).or_insert((p, p));
            b.0 = b.0.min(p);
            b.1 = b.1.max(p);
        };
        let mut vertices: Vec<FaceVertex> = Vec::new();
        for (t, owner) in self.scene.tri_owner.iter().enumerate() {
            if !self.selection.contains(owner) {
                continue;
            }
            match owner {
                Owner::Face(_) => {
                    for k in 0..3 {
                        let i = self.scene.indices[3 * t + k] as usize;
                        vertices.push(self.scene.vertices[i]);
                    }
                }
                Owner::Instance(i) => self.scene.triangles[t].iter().for_each(|p| grow(*i, *p)),
                Owner::Edge(_) => {}
            }
        }
        for (seg, owner) in self.scene.edges.iter().zip(&self.scene.edge_owner) {
            if let Owner::Instance(i) = owner
                && self.selection.contains(owner)
            {
                for p in [seg.a, seg.b] {
                    grow(*i, glam::Vec3::from(p).as_dvec3());
                }
            }
        }
        // Selected components and groups show their bounding box (world-aligned for now;
        // SketchUp aligns it with the component's axes).
        for (min, max) in boxes.values() {
            let c = |x: bool, y: bool, z: bool| {
                let p = glam::DVec3::new(
                    if x { max.x } else { min.x },
                    if y { max.y } else { min.y },
                    if z { max.z } else { min.z },
                );
                p.as_vec3().into()
            };
            for (a, b) in [
                ((0, 0, 0), (1, 0, 0)),
                ((0, 1, 0), (1, 1, 0)),
                ((0, 0, 1), (1, 0, 1)),
                ((0, 1, 1), (1, 1, 1)),
                ((0, 0, 0), (0, 1, 0)),
                ((1, 0, 0), (1, 1, 0)),
                ((0, 0, 1), (0, 1, 1)),
                ((1, 0, 1), (1, 1, 1)),
                ((0, 0, 0), (0, 0, 1)),
                ((1, 0, 0), (1, 0, 1)),
                ((0, 1, 0), (0, 1, 1)),
                ((1, 1, 0), (1, 1, 1)),
            ] {
                let p = |(x, y, z): (i32, i32, i32)| c(x == 1, y == 1, z == 1);
                edges.push(Segment::line(p(a), p(b), SELECTED, [1.0, 0.0]));
            }
        }
        let indices: Vec<u32> = (0..vertices.len() as u32).collect();
        self.viewport
            .set_selected_faces(device, &vertices, &indices);
        self.viewport.set_edges(device, &edges);
    }

    fn select(&mut self, device: &wgpu::Device, mode: Mode, items: Vec<Owner>) {
        self.selection.apply(mode, items);
        self.update_highlight(device);
    }

    /// Every visible top-level entity.
    fn all_entities(&self) -> Vec<Owner> {
        let mut seen = std::collections::HashSet::new();
        self.scene
            .tri_owner
            .iter()
            .chain(&self.scene.edge_owner)
            .copied()
            .filter(|o| seen.insert(*o))
            .collect()
    }

    fn open(&mut self, ctx: &egui::Context, device: &wgpu::Device, path: &std::path::Path) {
        match Document::open(path) {
            Ok(doc) => {
                if let Some(camera) = doc
                    .saved_camera()
                    .or_else(|| doc.extents_camera(self.camera.fov_deg))
                {
                    self.camera = camera;
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(doc.title()));
                self.document = Some(doc);
                self.selection.clear();
                self.rebuild(device);
                self.message = None;
            }
            Err(e) => self.message = Some(format!("Could not open {}: {e}", path.display())),
        }
    }

    fn run_command(&mut self, ctx: &egui::Context, frame: &eframe::Frame, id: &str) {
        if let Some(tool) = Tool::from_command(id) {
            self.tool = tool;
            self.draw = tools::for_command(id);
            self.erase_drag.clear();
            self.engine.unlock();
            self.inference = None;
        } else if id == "file.exit" {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if let Some(rs) = frame.wgpu_render_state()
            && matches!(id, "edit.select_all" | "edit.select_none" | "edit.delete")
        {
            let device = rs.device.clone();
            match id {
                "edit.select_all" => {
                    let all = self.all_entities();
                    self.select(&device, Mode::Replace, all);
                }
                "edit.select_none" => self.select(&device, Mode::Replace, Vec::new()),
                _ => {
                    if let Some(doc) = &mut self.document {
                        doc.delete(self.selection.items());
                        self.selection.clear();
                        self.rebuild(&device);
                    }
                }
            }
        } else if id == "file.open" {
            let picked = rfd::FileDialog::new()
                .add_filter("SketchUp Models", &["skp"])
                .pick_file();
            if let (Some(path), Some(rs)) = (picked, frame.wgpu_render_state()) {
                self.open(ctx, &rs.device, &path);
            }
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context, frame: &eframe::Frame, typing: bool) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let pressed: Vec<&'static str> = ctx.input(|i| {
            self.shortcuts
                .iter()
                .filter(|(s, key, _)| {
                    let m = i.modifiers;
                    // While typing into the Measurements box, plain keys are text.
                    let plain = !s.ctrl && !s.alt;
                    !(typing && plain)
                        && i.key_pressed(*key)
                        && m.ctrl == s.ctrl
                        && m.shift == s.shift
                        && m.alt == s.alt
                })
                .map(|(_, _, id)| *id)
                .collect()
        });
        for id in pressed {
            self.run_command(ctx, frame, id);
        }
    }

    fn viewport_ui(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        let Some(render_state) = frame.wgpu_render_state() else {
            return;
        };
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let ppp = ui.ctx().pixels_per_point();
        let size = [
            (rect.width() * ppp).round().max(1.0) as u32,
            (rect.height() * ppp).round().max(1.0) as u32,
        ];
        let viewport_px = DVec2::new(f64::from(size[0]), f64::from(size[1]));

        let to_px = |v: egui::Vec2| DVec2::new(f64::from(v.x * ppp), f64::from(v.y * ppp));
        let (shift, notches) = ui.input(|i| (i.modifiers.shift, wheel_notches(&i.raw.events)));
        let input = nav::Input {
            cursor: response.hover_pos().map(|p| to_px(p - rect.min)),
            left_drag: if response.dragged_by(egui::PointerButton::Primary) {
                to_px(response.drag_delta())
            } else {
                DVec2::ZERO
            },
            middle_drag: if response.dragged_by(egui::PointerButton::Middle) {
                to_px(response.drag_delta())
            } else {
                DVec2::ZERO
            },
            shift,
            wheel: if response.hovered() {
                f64::from(notches)
            } else {
                0.0
            },
        };
        nav::apply(self.tool, &mut self.camera, &input, viewport_px);
        match self.tool {
            Tool::Select => {
                self.select_tool(ui, &response, rect, viewport_px, &render_state.device)
            }
            Tool::Draw(_) => self.draw_tool(ui, &response, rect, viewport_px, &render_state.device),
            Tool::Eraser => {
                self.eraser_tool(ui, &response, rect, viewport_px, &render_state.device)
            }
            _ => self.inference = None,
        }

        self.viewport.render(
            &render_state.device,
            &render_state.queue,
            &self.camera,
            size,
            ppp,
        );
        let view = self.viewport.color_view().expect("rendered above");
        let mut renderer = render_state.renderer.write();
        let id = match self.texture {
            Some((id, old)) if old == size => id,
            Some((id, _)) => {
                renderer.update_egui_texture_from_wgpu_texture(
                    &render_state.device,
                    view,
                    wgpu::FilterMode::Nearest,
                    id,
                );
                id
            }
            None => renderer.register_native_texture(
                &render_state.device,
                view,
                wgpu::FilterMode::Nearest,
            ),
        };
        self.texture = Some((id, size));
        ui.painter().image(
            id,
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        self.draw_selection_box(ui, &response, rect);
        if let Some(inf) = &self.inference {
            let camera = &self.camera;
            let to_screen = |p: DVec3| {
                camera
                    .project(p, viewport_px)
                    .map(|q| rect.min + egui::vec2(q.x as f32, q.y as f32) / ppp)
            };
            let previews = self
                .draw
                .as_ref()
                .map(|d| d.preview(inf))
                .unwrap_or_default();
            let anchor = self.draw.as_ref().and_then(|d| d.anchor());
            overlay::paint(
                &ui.painter().with_clip_rect(rect),
                to_screen,
                anchor,
                &previews,
                inf,
            );
        }
    }

    /// Mouse and keys of the drawing tools.
    fn draw_tool(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        viewport_px: DVec2,
        device: &wgpu::Device,
    ) {
        let ppp = ui.ctx().pixels_per_point();
        let (keys, shift) = ui.input(|i| {
            let k = |key| i.key_pressed(key);
            (
                [
                    k(egui::Key::ArrowRight),
                    k(egui::Key::ArrowLeft),
                    k(egui::Key::ArrowUp),
                    k(egui::Key::ArrowDown),
                    k(egui::Key::Escape),
                ],
                i.modifiers.shift,
            )
        });
        let [right, left, up, down, escape] = keys;
        let Some(draw) = self.draw.as_mut() else {
            return;
        };
        // Arrow keys lock the drawing plane of shape tools, the direction otherwise.
        for (pressed, axis) in [
            (right, mt_infer::Axis::Red),
            (left, mt_infer::Axis::Green),
            (up, mt_infer::Axis::Blue),
        ] {
            if pressed && !draw.lock_plane(axis) {
                self.engine.toggle_axis_lock(axis);
            }
        }
        if down {
            self.engine.toggle_parallel_lock(&self.geometry);
        }
        if escape {
            draw.cancel();
            self.engine.unlock();
        }
        // Holding Shift locks the current direction; releasing it unlocks.
        if shift && !self.shift_lock {
            if let (Some(inf), Some(a)) = (&self.inference, draw.anchor()) {
                self.engine.lock_current(inf, a);
            }
            self.shift_lock = self.engine.is_locked();
        } else if !shift && self.shift_lock {
            self.engine.unlock();
            self.shift_lock = false;
        }

        let cursor = response
            .hover_pos()
            .or(response.interact_pointer_pos())
            .map(|p| {
                DVec2::new(
                    f64::from((p.x - rect.min.x) * ppp),
                    f64::from((p.y - rect.min.y) * ppp),
                )
            });
        let anchor = draw.anchor();
        self.inference = cursor.map(|c| {
            let view = mt_infer::View {
                camera: &self.camera,
                viewport: viewport_px,
            };
            self.engine
                .infer(&self.geometry, &self.bvh, view, c, anchor)
        });
        let (Some(inf), Some(doc)) = (self.inference, self.document.as_mut()) else {
            return;
        };
        let root = &mut doc.model.root;
        let primary = egui::PointerButton::Primary;
        let changed = if response.clicked_by(primary) {
            draw.click(root, &inf)
        } else if response.drag_started_by(primary) {
            draw.drag_start(root, &inf)
        } else if response.drag_stopped_by(primary) {
            draw.drag_end(root, &inf)
        } else {
            if response.dragged_by(primary) {
                draw.drag_move(&inf);
            }
            false
        };
        if changed {
            self.engine.unlock();
            self.shift_lock = false;
            self.rebuild(device);
        }
    }

    /// The Eraser: click or drag over edges and components to erase them. Shift hides
    /// instead, Ctrl softens and smooths, Ctrl+Shift unsoftens.
    fn eraser_tool(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        viewport_px: DVec2,
        device: &wgpu::Device,
    ) {
        self.inference = None;
        let ppp = ui.ctx().pixels_per_point();
        let to_px = |p: egui::Pos2| {
            DVec2::new(
                f64::from((p.x - rect.min.x) * ppp),
                f64::from((p.y - rect.min.y) * ppp),
            )
        };
        let pick = |app: &Self, p: egui::Pos2| {
            selection::pick(&app.scene, &app.bvh, &app.camera, to_px(p), viewport_px)
                .filter(|o| !matches!(o, Owner::Face(_)))
        };
        let primary = egui::PointerButton::Primary;
        let pos = response.interact_pointer_pos();
        let mut done: Vec<Owner> = Vec::new();
        if response.clicked_by(primary) {
            done.extend(pos.and_then(|p| pick(self, p)));
        } else if response.dragged_by(primary) || response.drag_started_by(primary) {
            if let Some(o) = pos.and_then(|p| pick(self, p))
                && !self.erase_drag.contains(&o)
            {
                self.erase_drag.push(o);
                self.select(device, Mode::Replace, self.erase_drag.clone());
            }
        } else if response.drag_stopped_by(primary) {
            done = std::mem::take(&mut self.erase_drag);
            self.selection.clear();
        }
        if done.is_empty() {
            return;
        }
        let mods = ui.input(|i| i.modifiers);
        if let Some(doc) = &mut self.document {
            doc.erase(&done, mods.shift, mods.command);
        }
        self.rebuild(device);
    }

    /// Clicks and box drags of the Select tool.
    fn select_tool(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        viewport_px: DVec2,
        device: &wgpu::Device,
    ) {
        let ppp = ui.ctx().pixels_per_point();
        let to_px = |p: egui::Pos2| {
            DVec2::new(
                f64::from((p.x - rect.min.x) * ppp),
                f64::from((p.y - rect.min.y) * ppp),
            )
        };
        let mods = ui.input(|i| i.modifiers);
        let mode = Mode::from_modifiers(mods.command, mods.shift);

        if response.drag_started_by(egui::PointerButton::Primary) {
            self.box_start = ui.input(|i| i.pointer.press_origin()).map(to_px);
        }
        if response.drag_stopped_by(egui::PointerButton::Primary)
            && let (Some(start), Some(end)) = (
                self.box_start.take(),
                response.interact_pointer_pos().map(to_px),
            )
        {
            // Left to right: window. Right to left: crossing.
            let window = end.x >= start.x;
            let items = selection::box_select(
                &self.scene,
                &self.camera,
                viewport_px,
                Rect::from_corners(start, end),
                window,
            );
            self.select(device, mode, items);
            return;
        }
        if response.clicked_by(egui::PointerButton::Primary)
            && let Some(cursor) = response.interact_pointer_pos().map(to_px)
        {
            let clicks = if response.triple_clicked() {
                3
            } else if response.double_clicked() {
                2
            } else {
                1
            };
            let hit = selection::pick(&self.scene, &self.bvh, &self.camera, cursor, viewport_px);
            let items = match (hit, &self.document) {
                (Some(h), Some(doc)) => selection::expand(&doc.model.root, h, clicks),
                _ => Vec::new(),
            };
            self.select(device, mode, items);
        }
    }

    /// The rubber band: solid for a window selection, dashed for crossing.
    fn draw_selection_box(&self, ui: &egui::Ui, response: &egui::Response, rect: egui::Rect) {
        let (Some(_), Some(start), Some(end)) = (
            self.box_start,
            ui.input(|i| i.pointer.press_origin()),
            response
                .hover_pos()
                .or(ui.input(|i| i.pointer.latest_pos())),
        ) else {
            return;
        };
        let r = egui::Rect::from_two_pos(start, end).intersect(rect);
        let stroke = egui::Stroke::new(1.0, egui::Color32::BLACK);
        if end.x >= start.x {
            ui.painter()
                .rect_stroke(r, 0.0, stroke, egui::StrokeKind::Inside);
        } else {
            let corners = [
                r.left_top(),
                r.right_top(),
                r.right_bottom(),
                r.left_bottom(),
                r.left_top(),
            ];
            ui.painter()
                .extend(egui::Shape::dashed_line(&corners, stroke, 4.0, 3.0));
        }
    }
}

/// Wheel movement this frame in notches; positive is away from the user (zoom in).
fn wheel_notches(events: &[egui::Event]) -> f32 {
    events
        .iter()
        .map(|e| match e {
            egui::Event::MouseWheel { unit, delta, .. } => match unit {
                egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => delta.y,
                egui::MouseWheelUnit::Point => delta.y / POINTS_PER_NOTCH,
            },
            _ => 0.0,
        })
        .sum()
}

impl App {
    /// What the Entity Info panel shows for the current selection.
    fn entity_info(&self) -> mt_ui::EntityInfo {
        let items = self.selection.items();
        let Some(doc) = &self.document else {
            return mt_ui::EntityInfo::no_selection();
        };
        let root = &doc.model.root;
        let inches = |v: f64| format!("{v:.3}\"");
        // All selected edges belong to one curve: describe the curve.
        let curve = items.first().and_then(|o| match o {
            Owner::Edge(e) => root.edges.get(*e).and_then(|x| x.attrs.curve),
            _ => None,
        });
        if let Some(c) = curve
            && let Some(cv) = root.curves.get(c)
            && items.iter().all(|o| matches!(o, Owner::Edge(e) if root.edges.get(*e).and_then(|x| x.attrs.curve) == Some(c)))
        {
            let units = doc.units;
            let segments = ("Segments".to_owned(), items.len().to_string());
            return match cv.arc {
                Some(arc) => mt_ui::EntityInfo {
                    title: if arc.is_circle() { "Circle" } else { "Arc" }.into(),
                    rows: vec![("Radius".into(), units.format(arc.radius)), segments],
                },
                None => mt_ui::EntityInfo { title: "Curve".into(), rows: vec![segments] },
            };
        }
        match items {
            [] => mt_ui::EntityInfo::no_selection(),
            [Owner::Edge(e)] if root.edges.contains_key(*e) => {
                let edge = &root.edges[*e];
                let len = root.pos(edge.start).distance(root.pos(edge.end));
                mt_ui::EntityInfo {
                    title: "Edge".into(),
                    rows: vec![
                        ("Length".into(), inches(len)),
                        (
                            "Soft".into(),
                            if edge.attrs.soft { "Yes" } else { "No" }.into(),
                        ),
                    ],
                }
            }
            [Owner::Face(f)] if root.faces.contains_key(*f) => {
                let tris = mt_kernel::tessellate::triangulate(root, *f);
                let area: f64 = tris
                    .iter()
                    .map(|[a, b, c]| (*b - *a).cross(*c - *a).length() / 2.0)
                    .sum();
                mt_ui::EntityInfo {
                    title: "Face".into(),
                    rows: vec![("Area".into(), format!("{area:.3} sq in"))],
                }
            }
            [Owner::Instance(i)] if *i < root.instances.len() => {
                let inst = &root.instances[*i];
                let def = doc.model.definitions.get(inst.definition);
                mt_ui::EntityInfo {
                    title: "Component".into(),
                    rows: vec![
                        ("Instance".into(), inst.name.clone()),
                        (
                            "Definition".into(),
                            def.map(|d| d.name.clone()).unwrap_or_default(),
                        ),
                    ],
                }
            }
            _ => mt_ui::EntityInfo {
                title: format!("{} Entities", items.len()),
                rows: Vec::new(),
            },
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let Some(mut shell) = self.shell.take() else {
            return;
        };
        shell.capture_typing(ui.ctx());
        self.handle_shortcuts(ui.ctx(), frame, shell.is_typing());

        let info = self.entity_info();
        let status = self
            .message
            .clone()
            .unwrap_or_else(|| match (&self.draw, self.tool) {
                (Some(d), Tool::Draw(_)) => d.status(),
                (_, t) => t.hint().to_owned(),
            });
        let units = self
            .document
            .as_ref()
            .map_or(mt_infer::units::LengthUnit::Inch, |d| d.units);
        let (label, value) = match (&self.draw, self.tool) {
            (Some(d), Tool::Draw(_)) => d.measurement(self.inference.as_ref(), units),
            _ => ("Measurements", String::new()),
        };
        let spec = self.spec.clone();
        let ui_frame = mt_ui::Frame {
            spec: &spec,
            active_tool: self.tool.command(),
            status: &status,
            measurements_label: label,
            measurements_value: &value,
            is_enabled: &|id| IMPLEMENTED.contains(&id),
            is_checked: &|_| false,
            entity_info: &info,
        };
        let events = shell.show(ui, &ui_frame, |ui| self.viewport_ui(ui, frame));
        self.shell = Some(shell);
        for event in events {
            match event {
                mt_ui::Event::Command(id) => self.run_command(ui.ctx(), frame, &id),
                mt_ui::Event::Measurements(text) => {
                    if let (Some(draw), Some(doc)) = (self.draw.as_mut(), self.document.as_mut())
                        && draw.typed(&mut doc.model.root, self.inference.as_ref(), &text, units)
                        && let Some(rs) = frame.wgpu_render_state()
                    {
                        let device = rs.device.clone();
                        self.engine.unlock();
                        self.rebuild(&device);
                    }
                }
            }
        }
    }
}
