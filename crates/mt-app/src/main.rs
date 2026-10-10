//! meshtard desktop application.

mod document;
#[cfg(target_os = "macos")]
mod macos_menu;
mod modify;
mod nav;
mod overlay;
mod selection;
mod tools;
mod tray;

use eframe::egui;
use egui_wgpu::wgpu;
use glam::{DVec2, DVec3};
use mt_kernel::bvh::{Bvh, RayIndex};
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
/// Models with fewer triangles build their pick index on the CPU: below this the GPU
/// round trip (mostly host-side flattening and PCIe transfer) costs more than it saves.
/// Measured on an RTX 4070 SUPER with `mt-cuda`'s `bench -- --sweep`: the GPU was 2x
/// slower at 20k triangles, even around 50k-100k, and 2-2.6x faster from 100k to 2M.
const GPU_MIN_TRIANGLES: usize = 100_000;

const IMPLEMENTED: &[&str] = &[
    "file.open",
    "file.new",
    "file.save",
    "file.save_as",
    "file.save_copy_as",
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
    "tool.tape_measure",
    "tool.push_pull",
    "tool.move",
    "tool.rotate",
    "tool.scale",
    "tool.offset",
    "tool.follow_me",
    "tool.paint_bucket",
    "edit.intersect_model",
    "edit.intersect_selection",
    "edit.create_group",
    "edit.make_component",
    "edit.close_group",
    "edit.explode",
    "edit.make_unique",
    "view.shadows",
    "view.section_planes",
    "view.section_cuts",
    "view.wireframe",
    "view.hidden_line",
    "view.shaded",
    "view.shaded_textures",
    "view.monochrome",
    "view.xray",
    "tool.section_plane",
    "edit.delete_guides",
    "edit.undo",
    "edit.redo",
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
    /// Pick index of `scene.triangles`; GPU-built for large models with `cuda`.
    bvh: Box<dyn RayIndex>,
    accel: mt_cuda::Accel,
    selection: Selection,
    /// Where a selection box drag started, in viewport pixels.
    box_start: Option<DVec2>,
    /// Snappable geometry and the inference state for drawing tools.
    geometry: mt_infer::Geometry,
    engine: mt_infer::Engine,
    inference: Option<mt_infer::Inference>,
    /// The active drawing tool, when `tool` is `Tool::Draw`.
    draw: Option<Box<dyn tools::DrawTool>>,
    /// Thick lines where the active section cuts the model.
    cut_lines: Vec<Segment>,
    /// The macOS menu bar (the shell draws the menus itself elsewhere).
    #[cfg(target_os = "macos")]
    native_menu: macos_menu::NativeMenu,
    /// Window title last sent (it changes when the model gets unsaved changes).
    last_title: String,
    /// Tray state the model does not hold.
    active_tag: usize,
    current_material: Option<usize>,
    current_component: Option<usize>,
    active_scene: Option<usize>,
    scene_tabs: bool,
    /// Open group or component space to world space (identity at the root).
    ctx_xf: glam::DMat4,
    /// The active modify tool, when `tool` is `Tool::Modify`.
    modify: Option<Box<dyn modify::ModifyTool>>,
    /// Edges picked during an Eraser drag, erased on release.
    erase_drag: Vec<Owner>,
    /// The direction lock comes from holding Shift (released with it).
    shift_lock: bool,
    /// Error from the last command, shown in the status bar until the next one.
    message: Option<String>,
}

impl App {
    fn new(spec: Spec, render_state: &egui_wgpu::RenderState) -> Self {
        let info = render_state.adapter.get_info();
        log::info!(
            "GPU: {} ({:?}, {:?}, driver {} {})",
            info.name,
            info.backend,
            info.device_type,
            info.driver,
            info.driver_info
        );
        let shortcuts = IMPLEMENTED
            .iter()
            .filter_map(|&id| {
                let shortcut = spec.keymap.shortcut(id)?;
                let key = egui::Key::from_name(&shortcut.key)?;
                Some((shortcut, key, id))
            })
            .collect();
        #[cfg(target_os = "macos")]
        let native_menu = macos_menu::NativeMenu::install(&spec, VIEW_TOGGLES);
        let mut app = Self {
            shell: Some({
                #[allow(unused_mut)]
                let mut shell = mt_ui::Shell::new(&spec);
                #[cfg(target_os = "macos")]
                shell.set_native_menu(true);
                shell
            }),
            camera: Camera::from_spec(&spec.default_model.camera),
            tool: Tool::Select,
            viewport: Viewport::new(&render_state.device, Style::from_spec(&spec.default_model)),
            texture: None,
            shortcuts,
            spec,
            document: None,
            scene: Scene::default(),
            bvh: Box::new(Bvh::default()),
            accel: {
                let accel = mt_cuda::Accel::new();
                log::info!(
                    "CUDA pick index for large models: {}",
                    if accel.is_gpu() { "on" } else { "off (CPU)" }
                );
                accel
            },
            selection: Selection::default(),
            box_start: None,
            geometry: mt_infer::Geometry::default(),
            engine: mt_infer::Engine::default(),
            inference: None,
            draw: None,
            modify: None,
            ctx_xf: glam::DMat4::IDENTITY,
            active_tag: 0,
            last_title: String::new(),
            #[cfg(target_os = "macos")]
            native_menu,
            cut_lines: Vec::new(),
            current_material: None,
            current_component: None,
            active_scene: None,
            scene_tabs: true,
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
        let triangles = self.scene.triangles.clone();
        let start = std::time::Instant::now();
        let gpu = self.accel.is_gpu() && triangles.len() >= GPU_MIN_TRIANGLES;
        self.bvh = if gpu {
            Box::new(self.accel.build_bvh(triangles))
        } else {
            Box::new(Bvh::build(triangles))
        };
        log::debug!(
            "pick index: {} triangles on the {} in {:?}",
            self.scene.triangles.len(),
            if gpu { "GPU" } else { "CPU" },
            start.elapsed()
        );
        let v3 = |p: [f32; 3]| DVec3::new(p[0].into(), p[1].into(), p[2].into());
        self.geometry = mt_infer::Geometry::from_edges(
            self.scene
                .edges
                .iter()
                .map(|s| [v3(s.a), v3(s.b)])
                .collect(),
        );
        if let Some(doc) = &self.document {
            for g in &doc.model.root.guides {
                match g.dir {
                    Some(d) => self.geometry.guide_lines.push((g.point, d)),
                    None => self.geometry.guide_points.push(g.point),
                }
            }
        }
        self.ctx_xf = self
            .document
            .as_ref()
            .map_or(glam::DMat4::IDENTITY, Document::context_transform);
        self.engine.reference_edge = None;
        self.apply_shadows();
        self.cut_lines.clear();
        let mut cut = None;
        if let (Some((p, n)), Some(doc)) = (self.scene.section, &self.document) {
            cut = Some(mt_render::SectionCut {
                point: p.as_vec3().to_array(),
                normal: n.as_vec3().to_array(),
            });
            let width = doc.model.style.section_cut_width;
            let f = |q: DVec3| q.as_vec3().to_array();
            self.cut_lines = mt_render::section_lines(&self.scene.triangles, p, n)
                .into_iter()
                .map(|[a, b]| Segment::line(f(a), f(b), [0.0, 0.0, 0.0, 1.0], [width, 0.0]))
                .collect();
        }
        self.viewport.set_section(cut);
        if let Some(doc) = &self.document {
            let st = &doc.model.style;
            let face_style = match st.face_style {
                mt_kernel::FaceStyle::Wireframe => mt_render::FaceStyle::Wireframe,
                mt_kernel::FaceStyle::HiddenLine => mt_render::FaceStyle::HiddenLine,
                mt_kernel::FaceStyle::Shaded => mt_render::FaceStyle::Shaded,
                mt_kernel::FaceStyle::ShadedWithTextures => {
                    mt_render::FaceStyle::ShadedWithTextures
                }
                mt_kernel::FaceStyle::Monochrome => mt_render::FaceStyle::Monochrome,
            };
            self.viewport.set_face_style(face_style, st.xray);
            let rgb = |c: [u8; 3]| c.map(|x| f32::from(x) / 255.0);
            self.viewport
                .set_monochrome_colors(rgb(doc.model.front_color), rgb(doc.model.back_color));
            let mut style = Style::from_spec(&self.spec.default_model);
            let st = &doc.model.style;
            style.profiles = (st.edges && st.profiles).then_some(st.profile_width);
            self.viewport.set_style(device, style);
        }
        self.viewport
            .set_faces(device, &self.scene.vertices, &self.scene.indices);
        self.update_highlight(device);
    }

    /// Passes the model's shadow settings to the viewport.
    fn apply_shadows(&mut self) {
        let shadows = self
            .document
            .as_ref()
            .map(|d| &d.model.shadows)
            .filter(|s| s.enabled)
            .map(|s| {
                // The settings hold no year; the current SketchUp default year is unverified.
                let sun = mt_render::sun_direction(
                    s.latitude,
                    s.longitude,
                    s.utc_offset,
                    2026,
                    s.month,
                    s.day,
                    s.hour,
                    s.minute,
                );
                mt_render::Shadows {
                    sun_dir: sun.as_vec3().to_array(),
                    light: s.light as f32,
                    dark: s.dark as f32,
                    on_faces: s.on_faces,
                    on_ground: s.on_ground,
                }
            });
        self.viewport.set_shadows(shadows);
    }

    /// Colors selected edges, dots selected faces and boxes selected components.
    fn update_highlight(&mut self, device: &wgpu::Device) {
        let mut edges = self.scene.edges.clone();
        let guides = self.scene.guides.clone();
        for (seg, owner) in edges.iter_mut().zip(&self.scene.edge_owner) {
            if owner.is_some_and(|o| self.selection.contains(&o)) {
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
            let Some(owner) = owner.filter(|o| self.selection.contains(o)) else {
                continue;
            };
            match owner {
                Owner::Face(_) => {
                    for k in 0..3 {
                        let i = self.scene.indices[3 * t + k] as usize;
                        vertices.push(self.scene.vertices[i]);
                    }
                }
                Owner::Instance(i) => self.scene.triangles[t].iter().for_each(|p| grow(i, *p)),
                Owner::Edge(_) | Owner::Section(_) => {}
            }
        }
        for (seg, owner) in self.scene.edges.iter().zip(&self.scene.edge_owner) {
            if let Some(owner @ Owner::Instance(i)) = owner
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
        edges.extend(guides);
        edges.extend(self.cut_lines.iter().copied());
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
            .filter_map(|o| *o)
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
                log::info!(
                    "opened {}: {} triangles, {} edges",
                    path.display(),
                    self.scene.triangles.len(),
                    self.scene.edges.len()
                );
                self.message = None;
            }
            Err(e) => {
                log::warn!("could not open {}: {e}", path.display());
                self.message = Some(format!("Could not open {}: {e}", path.display()));
            }
        }
    }

    fn run_command(&mut self, ctx: &egui::Context, frame: &eframe::Frame, id: &str) {
        if let Some(tool) = Tool::from_command(id) {
            self.tool = tool;
            self.draw = tools::for_command(id);
            self.modify = modify::for_command(id);
            if let Some(m) = self.modify.as_mut() {
                m.set_material(self.current_material);
            }
            self.erase_drag.clear();
            self.engine.unlock();
            self.inference = None;
        } else if id == "view.shadows" {
            if let Some(doc) = &mut self.document {
                // A view setting, not an undo step in this draft (SketchUp records
                // shadow toggles in undo, unverified).
                doc.model.shadows.enabled = !doc.model.shadows.enabled;
                self.apply_shadows();
            }
        } else if let Some(rs) = frame.wgpu_render_state()
            && id.starts_with("view.")
            && let Some(doc) = self.document.as_mut()
            && view_toggle(&mut doc.model.style, id)
        {
            // Display settings; not undo steps in this draft.
            let device = rs.device.clone();
            self.rebuild(&device);
        } else if id == "file.exit" {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if let Some(rs) = frame.wgpu_render_state()
            && matches!(
                id,
                "edit.select_all"
                    | "edit.select_none"
                    | "edit.delete"
                    | "edit.undo"
                    | "edit.redo"
                    | "edit.delete_guides"
                    | "edit.intersect_model"
                    | "edit.intersect_selection"
                    | "edit.create_group"
                    | "edit.make_component"
                    | "edit.close_group"
                    | "edit.explode"
                    | "edit.make_unique"
            )
        {
            let device = rs.device.clone();
            match id {
                "edit.select_all" => {
                    let all = self.all_entities();
                    self.select(&device, Mode::Replace, all);
                }
                "edit.select_none" => self.select(&device, Mode::Replace, Vec::new()),
                "edit.delete_guides" => {
                    if let Some(doc) = &mut self.document
                        && doc.edit(|root| !std::mem::take(&mut root.guides).is_empty())
                    {
                        self.rebuild(&device);
                    }
                }
                "edit.create_group" | "edit.make_component" => {
                    let sel = kernel_selection(self.selection.items());
                    let group = id == "edit.create_group";
                    if let Some(doc) = &mut self.document {
                        let mut made = None;
                        doc.edit_model(|model, ents| {
                            // SketchUp names group definitions Group#n and asks for a
                            // component name in a dialog (not implemented: Component#n).
                            let base = if group { "Group#1" } else { "Component#1" };
                            let name = model.unique_definition_name(base);
                            made = mt_kernel::group::make_group(
                                &mut model.definitions,
                                ents,
                                &sel,
                                name,
                                group,
                            );
                            made.is_some()
                        });
                        if let Some(i) = made {
                            self.selection.clear();
                            self.selection.apply(Mode::Replace, [Owner::Instance(i)]);
                            self.rebuild(&device);
                        }
                    }
                }
                "edit.close_group" => {
                    if let Some(doc) = &mut self.document
                        && doc.close_instance()
                    {
                        self.selection.clear();
                        self.rebuild(&device);
                    }
                }
                "edit.explode" | "edit.make_unique" => {
                    let mut instances: Vec<usize> = self
                        .selection
                        .items()
                        .iter()
                        .filter_map(|o| match o {
                            Owner::Instance(i) => Some(*i),
                            _ => None,
                        })
                        .collect();
                    instances.sort_unstable();
                    if let Some(doc) = &mut self.document
                        && !instances.is_empty()
                        && doc.edit_model(|model, ents| {
                            let mut any = false;
                            for &i in instances.iter().rev() {
                                any |= if id == "edit.explode" {
                                    mt_kernel::group::explode(&model.definitions, ents, i)
                                } else {
                                    let Some(d) = ents
                                        .instances
                                        .get(i)
                                        .and_then(|x| model.definitions.get(x.definition))
                                    else {
                                        continue;
                                    };
                                    let name = model.unique_definition_name(&d.name);
                                    mt_kernel::group::make_unique(
                                        &mut model.definitions,
                                        ents,
                                        i,
                                        name,
                                    )
                                };
                            }
                            any
                        })
                    {
                        self.selection.clear();
                        self.rebuild(&device);
                    }
                }
                "edit.intersect_model" | "edit.intersect_selection" => {
                    let faces: Vec<_> = self
                        .selection
                        .items()
                        .iter()
                        .filter_map(|o| match o {
                            Owner::Face(f) => Some(*f),
                            _ => None,
                        })
                        .collect();
                    if let Some(doc) = &mut self.document
                        && !faces.is_empty()
                        && doc.edit(|root| {
                            let with: Vec<_> = if id == "edit.intersect_model" {
                                root.faces.keys().collect()
                            } else {
                                faces.clone()
                            };
                            !mt_kernel::intersect_faces(root, &faces, &with).is_empty()
                        })
                    {
                        self.selection.clear();
                        self.rebuild(&device);
                    }
                }
                "edit.undo" | "edit.redo" => {
                    if let Some(doc) = &mut self.document
                        && (if id == "edit.undo" {
                            doc.undo()
                        } else {
                            doc.redo()
                        })
                    {
                        if let Some(d) = self.draw.as_mut() {
                            d.cancel();
                        }
                        if let Some(m) = self.modify.as_mut() {
                            m.cancel();
                        }
                        self.selection.clear();
                        self.rebuild(&device);
                    }
                }
                _ => {
                    if let Some(doc) = &mut self.document {
                        doc.delete(self.selection.items());
                        self.selection.clear();
                        self.rebuild(&device);
                    }
                }
            }
        } else if id == "file.new" {
            // SketchUp asks to save unsaved changes first (not implemented yet).
            if let Some(rs) = frame.wgpu_render_state() {
                let device = rs.device.clone();
                let doc = Document::untitled();
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(doc.title()));
                self.document = Some(doc);
                self.selection.clear();
                self.camera = Camera::from_spec(&self.spec.default_model.camera);
                self.rebuild(&device);
            }
        } else if matches!(id, "file.save" | "file.save_as" | "file.save_copy_as") {
            self.save(ctx, id);
        } else if id == "file.open" {
            let picked = rfd::FileDialog::new()
                .add_filter("SketchUp Models", &["skp"])
                .pick_file();
            if let (Some(path), Some(rs)) = (picked, frame.wgpu_render_state()) {
                self.open(ctx, &rs.device, &path);
            }
        }
    }

    /// File > Save, Save As, Save A Copy As. Files are written in the SketchUp 2017
    /// format (the only one the writer produces); SketchUp 2026 opens them.
    fn save(&mut self, ctx: &egui::Context, id: &str) {
        let Some(doc) = self.document.as_mut() else {
            return;
        };
        let path = if id == "file.save" && doc.has_file {
            Some(doc.path.clone())
        } else {
            let name = doc.path.file_name().map_or_else(
                || "Untitled.skp".into(),
                |n| n.to_string_lossy().into_owned(),
            );
            rfd::FileDialog::new()
                .add_filter("SketchUp Models", &["skp"])
                .set_file_name(name)
                .save_file()
                .map(|p| {
                    if p.extension().is_none() {
                        p.with_extension("skp")
                    } else {
                        p
                    }
                })
        };
        let Some(path) = path else {
            return;
        };
        let result = mt_kernel::export::to_skp(&doc.model)
            .map_err(|e| e.to_string())
            .and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string()));
        match result {
            Ok(()) => {
                if id != "file.save_copy_as" {
                    doc.mark_saved(&path);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Title(doc.title()));
                }
                self.message = Some(format!("Saved {} (SketchUp 2017 format).", path.display()));
            }
            Err(e) => self.message = Some(format!("Could not save {}: {e}", path.display())),
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
                    // On macOS the native menu bar owns shortcuts with modifiers.
                    if cfg!(target_os = "macos") && !plain {
                        return false;
                    }
                    !(typing && plain)
                        && i.key_pressed(*key)
                        // "Ctrl" in the keymap is Cmd on macOS (egui's `command`).
                        && m.command == s.ctrl
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
            Tool::Modify(_) => {
                self.modify_tool(ui, &response, rect, viewport_px, &render_state.device)
            }
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
            // Tools work in the open context's space.
            let xf = self.ctx_xf;
            let local = to_local(inf, xf);
            let mut previews = self
                .draw
                .as_ref()
                .map(|d| d.preview(&local))
                .unwrap_or_default();
            let mut anchor = self.draw.as_ref().and_then(|d| d.anchor());
            if let (Some(m), Some(doc)) = (&self.modify, &self.document) {
                previews = m.preview(doc.active(), &local);
                anchor = m.anchor();
            }
            for line in &mut previews {
                for p in line.iter_mut() {
                    *p = xf.transform_point3(*p);
                }
            }
            let anchor = anchor.map(|a| xf.transform_point3(a));
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
                self.engine
                    .lock_current(inf, self.ctx_xf.transform_point3(a));
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
        let anchor = draw.anchor().map(|a| self.ctx_xf.transform_point3(a));
        self.inference = cursor.map(|c| {
            let view = mt_infer::View {
                camera: &self.camera,
                viewport: viewport_px,
            };
            self.engine
                .infer(&self.geometry, self.bvh.as_ref(), view, c, anchor)
        });
        let (Some(inf), Some(doc)) = (self.inference, self.document.as_mut()) else {
            return;
        };
        let inf = to_local(&inf, self.ctx_xf);
        let primary = egui::PointerButton::Primary;
        let changed = if response.clicked_by(primary) {
            doc.edit(|root| draw.click(root, &inf))
        } else if response.drag_started_by(primary) {
            doc.edit(|root| draw.drag_start(root, &inf))
        } else if response.drag_stopped_by(primary) {
            doc.edit(|root| draw.drag_end(root, &inf))
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

    /// Mouse and keys of the modify tools (Push/Pull, Move, Rotate, Scale, Offset,
    /// Follow Me).
    fn modify_tool(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        viewport_px: DVec2,
        device: &wgpu::Device,
    ) {
        let ppp = ui.ctx().pixels_per_point();
        let (escape, ctrl, arrows) = ui.input(|i| {
            let k = |key| i.key_pressed(key);
            (
                k(egui::Key::Escape),
                i.modifiers.command,
                [
                    k(egui::Key::ArrowRight),
                    k(egui::Key::ArrowLeft),
                    k(egui::Key::ArrowUp),
                ],
            )
        });
        let Some(tool) = self.modify.as_mut() else {
            return;
        };
        for (pressed, axis) in arrows.into_iter().zip([
            mt_infer::Axis::Red,
            mt_infer::Axis::Green,
            mt_infer::Axis::Blue,
        ]) {
            if pressed {
                self.engine.toggle_axis_lock(axis);
            }
        }
        if escape {
            tool.cancel();
            self.engine.unlock();
        }
        if let Some(dir) = tool.constraint() {
            self.engine.lock_direction(
                self.ctx_xf.transform_vector3(dir),
                mt_infer::Kind::Perpendicular,
            );
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
        let anchor = tool.anchor().map(|a| self.ctx_xf.transform_point3(a));
        self.inference = cursor.map(|c| {
            let view = mt_infer::View {
                camera: &self.camera,
                viewport: viewport_px,
            };
            self.engine
                .infer(&self.geometry, self.bvh.as_ref(), view, c, anchor)
        });
        let (Some(inf), Some(c), Some(doc)) = (self.inference, cursor, self.document.as_mut())
        else {
            return;
        };
        let inf = to_local(&inf, self.ctx_xf);
        if !response.clicked_by(egui::PointerButton::Primary) {
            return;
        }
        let picked = selection::pick(&self.scene, self.bvh.as_ref(), &self.camera, c, viewport_px);
        let selection = self.selection.items().to_vec();
        let eye = self.ctx_xf.inverse().transform_point3(self.camera.eye);
        let changed = doc.edit(|root| {
            let mut ctx = modify::Ctx {
                ents: root,
                picked,
                selection: &selection,
                ctrl,
                eye,
            };
            tool.click(&mut ctx, &inf)
        });
        if tool.constraint().is_none() && tool.anchor().is_none() {
            self.engine.unlock();
        }
        if changed {
            self.engine.unlock();
            self.selection.clear();
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
            selection::pick(
                &app.scene,
                app.bvh.as_ref(),
                &app.camera,
                to_px(p),
                viewport_px,
            )
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
        // Esc closes the open group or component (unverified for SketchUp 2026).
        if ui.input(|i| i.key_pressed(egui::Key::Escape))
            && let Some(doc) = self.document.as_mut()
            && doc.close_instance()
        {
            self.selection.clear();
            self.rebuild(device);
        }

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
            let hit = selection::pick(
                &self.scene,
                self.bvh.as_ref(),
                &self.camera,
                cursor,
                viewport_px,
            );
            let Some(doc) = self.document.as_mut() else {
                return;
            };
            match hit {
                // Double click on a section plane activates or deactivates it.
                Some(Owner::Section(i)) if clicks == 2 => {
                    if doc.edit(|ents| {
                        ents.active_section = if ents.active_section == Some(i) {
                            None
                        } else {
                            Some(i)
                        };
                        true
                    }) {
                        self.rebuild(device);
                    }
                }
                // Double click on a group or component opens it for editing.
                Some(Owner::Instance(i)) if clicks == 2 => {
                    if doc.open_instance(i) {
                        self.selection.clear();
                        self.rebuild(device);
                    }
                }
                // A click outside the open group or component closes it (also on
                // empty space; unverified that SketchUp closes on empty space).
                None if !doc.context.is_empty() && mode == Mode::Replace => {
                    doc.close_instance();
                    self.selection.clear();
                    self.rebuild(device);
                }
                Some(h) => {
                    let items = selection::expand(doc.active(), h, clicks);
                    self.select(device, mode, items);
                }
                None => self.select(device, mode, Vec::new()),
            }
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
/// An inference in the open context's space.
fn to_local(inf: &mt_infer::Inference, ctx_xf: glam::DMat4) -> mt_infer::Inference {
    if ctx_xf == glam::DMat4::IDENTITY {
        return *inf;
    }
    let inv = ctx_xf.inverse();
    let p = |q: DVec3| inv.transform_point3(q);
    let normal_xf = glam::DMat3::from_mat4(ctx_xf).transpose();
    mt_infer::Inference {
        point: p(inf.point),
        guide: inf.guide.map(|(a, b)| (p(a), p(b))),
        normal: inf.normal.map(|n| (normal_xf * n).normalize_or_zero()),
        segment: inf.segment.map(|[a, b]| [p(a), p(b)]),
        ..*inf
    }
}

/// Applies a View menu display toggle; false if `id` is not one.
fn view_toggle(s: &mut mt_kernel::StyleSettings, id: &str) -> bool {
    use mt_kernel::FaceStyle as F;
    match id {
        "view.section_planes" => s.section_planes = !s.section_planes,
        "view.section_cuts" => s.section_cuts = !s.section_cuts,
        "view.xray" => s.xray = !s.xray,
        "view.wireframe" => s.face_style = F::Wireframe,
        "view.hidden_line" => s.face_style = F::HiddenLine,
        "view.shaded" => s.face_style = F::Shaded,
        "view.shaded_textures" => s.face_style = F::ShadedWithTextures,
        "view.monochrome" => s.face_style = F::Monochrome,
        _ => return false,
    }
    true
}

fn view_checked(s: &mt_kernel::StyleSettings, id: &str) -> bool {
    use mt_kernel::FaceStyle as F;
    match id {
        "view.section_planes" => s.section_planes,
        "view.section_cuts" => s.section_cuts,
        "view.xray" => s.xray,
        "view.wireframe" => s.face_style == F::Wireframe,
        "view.hidden_line" => s.face_style == F::HiddenLine,
        "view.shaded" => s.face_style == F::Shaded,
        "view.shaded_textures" => s.face_style == F::ShadedWithTextures,
        "view.monochrome" => s.face_style == F::Monochrome,
        _ => false,
    }
}

/// Menu commands shown with a check mark (native macOS menu).
#[cfg(target_os = "macos")]
const VIEW_TOGGLES: &[&str] = &[
    "view.shadows",
    "view.section_planes",
    "view.section_cuts",
    "view.xray",
    "view.wireframe",
    "view.hidden_line",
    "view.shaded",
    "view.shaded_textures",
    "view.monochrome",
];

/// The kernel's view of a selection.
fn kernel_selection(items: &[Owner]) -> mt_kernel::group::Selection {
    let mut sel = mt_kernel::group::Selection::default();
    for o in items {
        match *o {
            Owner::Edge(e) => sel.edges.push(e),
            Owner::Face(f) => sel.faces.push(f),
            Owner::Instance(i) => sel.instances.push(i),
            Owner::Section(_) => {}
        }
    }
    sel
}

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
        let root = doc.active();
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
                let group = def.is_some_and(|d| d.is_group);
                mt_ui::EntityInfo {
                    title: if group { "Group" } else { "Component" }.into(),
                    rows: vec![
                        ("Instance".into(), inst.name.clone()),
                        (
                            "Definition".into(),
                            def.map(|d| d.name.clone()).unwrap_or_default(),
                        ),
                    ],
                }
            }
            [Owner::Section(i)] if *i < root.section_planes.len() => mt_ui::EntityInfo {
                title: "Section Plane".into(),
                rows: vec![
                    ("Name".into(), root.section_planes[*i].name.clone()),
                    (
                        "Active".into(),
                        if root.active_section == Some(*i) {
                            "Yes"
                        } else {
                            "No"
                        }
                        .into(),
                    ),
                ],
            },
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
        // A .skp file dropped on the window opens it (SketchUp asks to save unsaved
        // changes first; not implemented yet).
        let dropped = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .find(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("skp")))
        });
        if let (Some(path), Some(rs)) = (dropped, frame.wgpu_render_state()) {
            let device = rs.device.clone();
            self.open(ui.ctx(), &device, &path);
        }
        if let Some(doc) = &self.document
            && doc.title() != self.last_title
        {
            self.last_title = doc.title();
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Title(self.last_title.clone()));
        }
        self.handle_shortcuts(ui.ctx(), frame, shell.is_typing());

        let info = self.entity_info();
        let status = self
            .message
            .clone()
            .unwrap_or_else(|| match (&self.draw, self.tool) {
                (Some(d), Tool::Draw(_)) => d.status(),
                (_, Tool::Modify(_)) if self.modify.is_some() => {
                    self.modify.as_ref().map(|m| m.status()).unwrap_or_default()
                }
                (_, t) => t.hint().to_owned(),
            });
        let units = self
            .document
            .as_ref()
            .map_or(mt_infer::units::LengthUnit::Inch, |d| d.units);
        let (label, value) = match (&self.draw, self.tool) {
            (Some(d), Tool::Draw(_)) => d.measurement(self.inference.as_ref(), units),
            (_, Tool::Modify(_)) if self.modify.is_some() => self
                .modify
                .as_ref()
                .map(|m| m.measurement(self.inference.as_ref(), units))
                .unwrap_or_default(),
            _ => ("Measurements", String::new()),
        };
        let shadows_on = self
            .document
            .as_ref()
            .is_some_and(|d| d.model.shadows.enabled);
        let style = self.document.as_ref().map(|d| d.model.style.clone());
        let can_undo = self.document.as_ref().is_some_and(Document::can_undo);
        let can_redo = self.document.as_ref().is_some_and(Document::can_redo);
        let spec = self.spec.clone();
        let ui_frame = mt_ui::Frame {
            spec: &spec,
            active_tool: self.tool.command(),
            status: &status,
            measurements_label: label,
            measurements_value: &value,
            is_enabled: &|id| match id {
                "edit.undo" => can_undo,
                "edit.redo" => can_redo,
                _ => IMPLEMENTED.contains(&id),
            },
            is_checked: &|id| {
                (id == "view.shadows" && shadows_on)
                    || style.as_ref().is_some_and(|s| view_checked(s, id))
            },
            entity_info: &info,
        };
        #[cfg(target_os = "macos")]
        self.native_menu
            .update(ui_frame.is_enabled, ui_frame.is_checked);
        shell.set_tray_data(self.tray_data());
        let events = shell.show(ui, &ui_frame, |ui| self.viewport_ui(ui, frame));
        #[cfg(target_os = "macos")]
        for id in macos_menu::NativeMenu::poll() {
            self.run_command(ui.ctx(), frame, &id);
        }
        self.shell = Some(shell);
        for event in events {
            match event {
                mt_ui::Event::Command(id) => self.run_command(ui.ctx(), frame, &id),
                mt_ui::Event::Measurements(text) => {
                    if let (Some(draw), Some(doc)) = (self.draw.as_mut(), self.document.as_mut())
                        && let local = self.inference.map(|i| to_local(&i, self.ctx_xf))
                        && doc.edit(|root| draw.typed(root, local.as_ref(), &text, units))
                        && let Some(rs) = frame.wgpu_render_state()
                    {
                        let device = rs.device.clone();
                        self.engine.unlock();
                        self.rebuild(&device);
                    } else if let (Some(tool), Some(doc)) =
                        (self.modify.as_mut(), self.document.as_mut())
                        && doc.edit(|root| {
                            let mut ctx = modify::Ctx {
                                ents: root,
                                picked: None,
                                selection: &[],
                                ctrl: false,
                                eye: DVec3::ZERO,
                            };
                            let local = self.inference.map(|i| to_local(&i, self.ctx_xf));
                            tool.typed(&mut ctx, local.as_ref(), &text, units)
                        })
                        && let Some(rs) = frame.wgpu_render_state()
                    {
                        let device = rs.device.clone();
                        self.engine.unlock();
                        self.selection.clear();
                        self.rebuild(&device);
                    }
                }
                e => {
                    if self.tray_event(e)
                        && let Some(rs) = frame.wgpu_render_state()
                    {
                        let device = rs.device.clone();
                        self.rebuild(&device);
                    }
                }
            }
        }
    }
}
