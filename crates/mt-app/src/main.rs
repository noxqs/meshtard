//! meshtard desktop application.

mod about;
mod anim_export;
mod animation;
mod annot;
mod camera_cmd;
mod camera_tools;
mod click_style;
mod commands;
mod component_behavior;
mod component_dialog;
mod component_snaps;
mod confirm;
mod context_extra;
mod context_menu;
mod cursor;
mod document;
mod entity_info;
mod environments;
mod errors;
mod export;
mod extensions;
mod filedialog;
mod fix_problems;
mod flip;
mod graphics;
mod grips;
mod image_entity;
mod import;
mod lasso;
mod library;
mod locks;
#[cfg(target_os = "macos")]
mod macos_menu;
mod match_photo;
mod material_edit;
mod model_axes;
mod model_info;
mod modify;
mod move_extra;
mod nav;
mod overlay;
mod paint;
mod pick_tool;
mod preferences;
mod prefs_pages;
mod presentation;
mod print;
mod report;
mod sandbox;
mod sandbox_tools;
mod search;
mod section_export;
mod selection;
mod settings;
mod small_items;
mod snap;
mod soften;
mod solid_tools;
mod splash;
mod spring;
mod style_view;
mod templates;
mod text3d;
mod texture_position;
mod tool_extra;
mod tools;
mod tray;
mod tray_more;
mod welcome;

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
/// Selection box of a locked group or component (red, as in SketchUp).
const LOCKED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

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
    "tool.lasso",
    "tool.snaps",
    "edit.select_connected_by_snaps",
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
    "tool.protractor",
    "tool.dimensions",
    "tool.text",
    "tool.push_pull",
    "tool.move",
    "tool.rotate",
    "tool.scale",
    "tool.offset",
    "tool.follow_me",
    "tool.paint_bucket",
    "tool.flip",
    "tool.texture_position",
    "edit.intersect_model",
    "edit.intersect_selection",
    "edit.create_group",
    "edit.make_component",
    "edit.close_group",
    "edit.explode",
    "edit.make_unique",
    // Context menu commands (context_menu.rs).
    "camera.zoom_selection",
    "edit.select_connected_faces",
    "edit.select_bounding_edges",
    "edit.select_all_connected",
    "edit.select_same_tag",
    "edit.select_same_material",
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
    "view.shadows",
    "view.section_planes",
    "view.section_cuts",
    "view.section_fill",
    "view.wireframe",
    "view.hidden_line",
    "view.shaded",
    "view.shaded_textures",
    "view.monochrome",
    "view.xray",
    "tool.section_plane",
    "edit.delete_guides",
    "edit.purge_unused",
    "edit.image_export",
    "edit.image_use_as_material",
    "edit.make_unique_texture",
    "window.model_info_geo",
    "edit.undo",
    "edit.redo",
    "edit.select_all",
    "edit.select_none",
    "edit.delete",
    "edit.cut",
    "edit.copy",
    "edit.paste",
    "edit.paste_in_place",
    "camera.orbit",
    "camera.pan",
    "camera.zoom",
    "camera.zoom_extents",
    "camera.view_top",
    "camera.view_bottom",
    "camera.view_front",
    "camera.view_back",
    "camera.view_left",
    "camera.view_right",
    "camera.view_iso",
    "camera.parallel",
    "camera.perspective",
    "camera.previous",
    "camera.next",
    "camera.zoom_window",
    "window.tray_show",
    "window.entity_info",
    "window.materials",
    "window.components",
    "window.styles",
    "window.tags",
    "window.outliner",
    "window.scenes",
    "window.shadows",
    "window.environments",
    "window.instructor",
    "view.toolbars",
    "edit.invert_selection",
    "edit.hide",
    "edit.unhide_selected",
    "edit.unhide_last",
    "edit.unhide_all",
    "edit.lock",
    "edit.unlock_selected",
    "edit.unlock_all",
    "view.axes",
    "view.guides",
    "view.hidden_geometry",
    "view.hidden_objects",
    "view.scene_tabs",
    "view.edges",
    "view.back_edges",
    "view.profiles",
    "view.depth_cue",
    "view.extension",
    "view.fog",
    "window.fog",
    "file.export_2d",
    "file.export_3d",
    "file.import",
    "view.anim_add_scene",
    "view.anim_update_scene",
    "view.anim_delete_scene",
    "view.anim_previous",
    "view.anim_next",
    "view.anim_play",
    "view.anim_settings",
    "tool.outer_shell",
    "tool.solid_intersect",
    "tool.solid_union",
    "tool.solid_subtract",
    "tool.solid_trim",
    "tool.solid_split",
    "tool.tag",
    "tool.axes",
    "file.revert",
    "view.hide_rest_of_model",
    "view.hide_similar_components",
    "edit.intersect_context",
    "window.soften_edges",
    "camera.position",
    "camera.walk",
    "camera.look_around",
    "camera.field_of_view",
    "camera.two_point",
    "window.model_info",
    "window.preferences",
    "tool.sandbox_from_scratch",
    "tool.sandbox_from_contours",
    "tool.sandbox_smoove",
    "tool.sandbox_stamp",
    "tool.sandbox_drape",
    "tool.sandbox_add_detail",
    "tool.sandbox_flip_edge",
    "tool.text_3d",
    "file.export_section",
    "file.export_animation",
    "file.print",
    "file.print_preview",
    "file.print_setup",
    "file.new_from_template",
    "file.save_as_template",
    "file.generate_report",
    "window.manage_trays",
    "help.about",
    "help.welcome",
    "help.knowledge_center",
    "help.contact",
    // More context menu commands (context_extra.rs) and Help > Search.
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
    "help.search",
    "window.3d_warehouse",
    "camera.match_new_photo",
    // Extensions (extensions/): Extension Manager, Warehouse, Console.
    "extensions.manager",
    "extensions.warehouse",
    "window.extension_console",
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
            .with_app_id("meshtard")
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
                    .expect("icon.png is a valid PNG"),
            )
            .with_inner_size([1280.0, 800.0]),
        renderer: eframe::Renderer::Wgpu,
        // Preferences > Graphics > GPU (after a restart).
        wgpu_options: graphics::wgpu_options(graphics::saved_gpu()),
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
            // A model given on the command line opens in the background behind the
            // splash screen.
            app.loading = open_path.map(splash::Loading::start);
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
    /// Ctrl held in the previous frame (modify tools toggle on press).
    ctrl_was_down: bool,
    /// The Make Component dialog while open.
    make_component: Option<component_dialog::MakeComponent>,
    /// An action waiting for the "Save changes?" answer.
    pending: Option<confirm::Pending>,
    /// The window may close (the question was answered or not needed).
    closing: bool,
    /// The splash screen while the app starts.
    splash: splash::Splash,
    /// A model opening on a background thread (the command-line file).
    loading: Option<splash::Loading>,
    /// Window title last sent (it changes when the model gets unsaved changes).
    last_title: String,
    /// Tray state the model does not hold.
    active_tag: usize,
    current_material: Option<usize>,
    current_component: Option<usize>,
    active_scene: Option<usize>,
    scene_tabs: bool,
    /// Camera move to an activated scene or view, in progress.
    transition: Option<tray::Transition>,
    camera_history: camera_cmd::History,
    /// Zoom Window: where the drag started (physical pixels).
    zoom_box: Option<DVec2>,
    /// The viewport size in pixels at the last frame.
    viewport_px: DVec2,
    /// Open group or component space to world space (identity at the root).
    ctx_xf: glam::DMat4,
    /// The active modify tool, when `tool` is `Tool::Modify`.
    modify: Option<Box<dyn modify::ModifyTool>>,
    /// Definition-space bounds of the open context's top-level instances.
    inst_bounds: std::collections::HashMap<usize, (DVec3, DVec3)>,
    /// Targets drawn live by Move, Rotate or Scale: targets, transform (context
    /// space), copy.
    live: Option<(Vec<Owner>, glam::DMat4, bool)>,
    /// Edges picked during an Eraser drag, erased on release.
    erase_drag: Vec<Owner>,
    /// What Shift and Alt locked (released with them; `locks`).
    shift_lock: locks::ModifierLocks,
    /// Error from the last command, shown in the status bar until the next one.
    message: Option<String>,
    /// Edit > Copy / Cut: a copy of the selection in world coordinates (internal; the
    /// system clipboard only gets a short text so Ctrl+V reaches the app).
    clipboard: Option<mt_kernel::Entities>,
    /// The clipboard drawn while Paste places it (world coordinates), and the
    /// transform it is drawn with.
    ghost: Option<Scene>,
    ghost_xf: Option<glam::DMat4>,
    /// File > Export dialogs and their last options.
    exports: export::Exports,
    /// File > Import options for an OBJ or STL file, while asked.
    mesh_import: Option<import::MeshImport>,
    /// View > Animation settings and playback.
    animation: animation::Animation,
    /// The Solid Tools or Tag tool, when `tool` is `Tool::Pick`.
    pick: Option<pick_tool::PickTool>,
    /// A camera tool held by its shortcut key (see `spring`).
    spring: Option<spring::Spring>,
    /// An error dialog waiting to be closed.
    error: Option<errors::ErrorMessage>,
    /// Window > Soften Edges.
    soften: soften::SoftenPanel,
    /// Position Camera, Walk, Look Around and Field of View.
    camera_tools: camera_tools::CameraTools,
    /// Window > Model Info while open.
    model_info: Option<model_info::ModelInfo>,
    /// Window > Preferences and the settings it keeps.
    prefs: preferences::Prefs,
    /// File > Export Section Slice and Animation, Print, templates, report, About.
    present: presentation::Presentation,
    /// The viewport's right-click menu.
    ctx_menu: context_menu::ContextMenu,
    /// Texture Position and externally edited textures.
    textures: texture_position::Textures,
    /// Dialogs and state of the extra context items, tool modifiers and Search.
    extra: context_extra::Extra,
    /// The Lasso tool's outline while drawn.
    lasso: lasso::Lasso,
    /// The Move tool's grips on group and component boxes.
    grips: grips::Grips,
    /// Faces and edges stretched by the running Move, Rotate or Scale (drawn live).
    stretch: move_extra::Stretch,
    /// Window > Model Library (3D Warehouse equivalent).
    library: library::Library,
    /// Window > Environments library.
    envs: environments::Environments,
    /// FEATURES.md 67 items' state (`small_items`).
    small: small_items::State,
    /// Camera > Match New Photo / Edit Matched Photo and the scene photo shown.
    match_photo: match_photo::MatchPhoto,
    /// Extensions: Rhai scripts, the Extension Manager and Warehouse.
    extensions: extensions::Extensions,
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
        let mut spec = spec;
        let mut prefs = preferences::load(&mut spec);
        let samples = prefs.set_adapter(&render_state.adapter);
        graphics::write_log(&graphics::log_text(
            &info,
            prefs.more.gpu.as_deref(),
            samples,
            &prefs.samples_supported,
        ));
        let shortcuts = preferences::shortcut_table(&spec, IMPLEMENTED);
        #[cfg(target_os = "macos")]
        let native_menu = macos_menu::NativeMenu::install(&spec, VIEW_TOGGLES);
        let mut app = Self {
            shell: Some({
                #[allow(unused_mut)]
                let mut shell = mt_ui::Shell::new(&spec);
                #[cfg(target_os = "macos")]
                shell.set_native_menu(true);
                if let Some(layout) = settings::load_toolbar_layout() {
                    shell.set_toolbar_layout(layout);
                }
                if let Some(trays) = settings::load_tray_layout() {
                    shell.set_tray_layout(trays);
                }
                shell
            }),
            camera: Camera::from_spec(&spec.default_model.camera),
            tool: Tool::Select,
            viewport: Viewport::with_samples(
                &render_state.device,
                Style::from_spec(&spec.default_model),
                samples,
            ),
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
            inst_bounds: Default::default(),
            live: None,
            ctx_xf: glam::DMat4::IDENTITY,
            active_tag: 0,
            last_title: String::new(),
            splash: splash::Splash::new(),
            loading: None,
            pending: None,
            make_component: None,
            ctrl_was_down: false,
            closing: false,
            #[cfg(target_os = "macos")]
            native_menu,
            cut_lines: Vec::new(),
            current_material: None,
            current_component: None,
            active_scene: None,
            scene_tabs: true,
            transition: None,
            camera_history: camera_cmd::History::default(),
            zoom_box: None,
            viewport_px: DVec2::ZERO,
            erase_drag: Vec::new(),
            shift_lock: locks::ModifierLocks::default(),
            message: None,
            clipboard: None,
            ghost: None,
            ghost_xf: None,
            exports: Default::default(),
            mesh_import: None,
            animation: Default::default(),
            pick: None,
            spring: None,
            error: None,
            soften: Default::default(),
            camera_tools: Default::default(),
            model_info: None,
            prefs,
            present: Default::default(),
            ctx_menu: Default::default(),
            textures: Default::default(),
            extra: Default::default(),
            lasso: Default::default(),
            grips: Default::default(),
            stretch: Default::default(),
            library: Default::default(),
            envs: Default::default(),
            small: Default::default(),
            match_photo: Default::default(),
            extensions: Default::default(),
        };
        // SketchUp starts with an empty model ready to draw in.
        app.document = Some(Document::untitled());
        app.rebuild(&render_state.device);
        app.load_extensions();
        app
    }

    /// Rebuilds the drawn scene and pick index after the model changed.
    fn rebuild(&mut self, device: &wgpu::Device) {
        self.sync_facing();
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
        self.ctx_xf = self
            .document
            .as_ref()
            .map_or(glam::DMat4::IDENTITY, Document::context_transform);
        self.geometry = snap::geometry(&self.scene, self.document.as_ref(), self.ctx_xf);
        self.engine.geometry_changed();
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
            self.geometry.section_lines = mt_render::section_lines(&self.scene.triangles, p, n);
            let cut_color = style_view::rgba(doc.model.style.modeling_colors.section_cut);
            self.cut_lines = (self.geometry.section_lines.iter().copied())
                .map(|[a, b]| Segment::line(f(a), f(b), cut_color, [width, 0.0]))
                .collect();
        }
        self.viewport.set_section(cut);
        // Section fill (View > Section Fill), capping the cut where it closes.
        let mut fill: Vec<[[f32; 3]; 3]> = Vec::new();
        let mut fill_color = [0.25; 3];
        if let (Some((p, n)), Some(doc)) = (self.scene.section, &self.document)
            && doc.model.style.section_fill
        {
            fill_color = doc
                .model
                .style
                .section_fill_color
                .map(|x| f32::from(x) / 255.0);
            fill = mt_render::section_fill(&self.scene.triangles, p, n)
                .iter()
                .map(|t| t.map(|q| q.as_vec3().to_array()))
                .collect();
        }
        self.viewport.set_section_fill(device, &fill, fill_color);
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
            style.profiles = st.profiles.then_some(st.profile_width);
            style.edges = st.edges;
            style.back_edges = st.back_edges;
            style.depth_cue = st.depth_cue.then_some(st.depth_cue_width);
            style.extension = st.extension.then_some(st.extension_length);
            style.fog = st.fog.enabled.then(|| mt_render::Fog {
                start: st.fog.start,
                end: st.fog.end,
                color: (!st.fog.use_background).then(|| rgb(st.fog.color)),
            });
            if !st.axes {
                style.axes = None;
            }
            style_view::apply_render(&doc.model, &mut style);
            model_axes::apply(&doc.model, &mut style, &mut self.engine);
            self.viewport.set_style(device, style);
        }
        self.viewport
            .set_faces(device, &self.scene.vertices, &self.scene.indices);
        let casters = self.scene.shadow_casters();
        let caster_indices: &[u32] = if casters.is_some() {
            &self.scene.indices
        } else {
            &[]
        };
        self.viewport
            .set_shadow_casters(device, casters.as_deref().unwrap_or(&[]), caster_indices);
        self.viewport.set_textures(&self.scene.textures);
        self.inst_bounds = match &self.document {
            Some(doc) => {
                let ents = doc.active();
                let xf = self.ctx_xf;
                selection::instance_bounds(&self.scene, |i| {
                    ents.instances
                        .get(i)
                        .map(|inst| doc.drawn_transform(xf * inst.transform, inst))
                })
            }
            None => Default::default(),
        };
        self.live = None;
        self.ghost_xf = None;
        self.viewport.set_preview_faces(device, &[], &[]);
        self.update_highlight(device);
    }

    /// The sun the Shadows settings place (or Environments > Set Sun Location), when
    /// shadows or "Use sun for shading" are on.
    fn sun_dir(&self) -> Option<DVec3> {
        let d = self.document.as_ref()?;
        let s = &d.model.shadows;
        // "Use sun for shading": sun lighting without shadows.
        if !(s.enabled || s.use_sun_for_shading) {
            return None;
        }
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
        let sun = mt_render::rotate_north(sun, d.model.geo.north_angle);
        // Environments > Set Sun Location replaces the sun.
        Some(environments::sun_override(&d.model.environment).unwrap_or(sun))
    }

    /// Passes the model's shadow settings to the viewport.
    fn apply_shadows(&mut self) {
        self.apply_environment();
        let sun = self.sun_dir();
        let shadows = self
            .document
            .as_ref()
            .map(|d| &d.model.shadows)
            .zip(sun)
            .map(|(s, sun)| mt_render::Shadows {
                sun_dir: sun.as_vec3().to_array(),
                light: s.light as f32,
                dark: s.dark as f32,
                on_faces: s.on_faces && s.enabled,
                on_ground: s.on_ground && s.enabled,
            });
        self.viewport.set_shadows(shadows);
    }

    /// Colors selected edges, dots selected faces and boxes selected components (in
    /// the component's axes, as SketchUp does). Targets drawn live by a modify tool
    /// (`self.live`) are drawn at their transformed position.
    fn update_highlight(&mut self, device: &wgpu::Device) {
        // Live targets: owner set, world-space transform, copy.
        let live = self.live.as_ref().map(|(t, xf, copy)| {
            let set: std::collections::HashSet<Owner> = t.iter().copied().collect();
            (set, self.ctx_xf * *xf * self.ctx_xf.inverse(), *copy)
        });
        let moved = |o: &Option<Owner>| match (&live, o) {
            (Some((set, xf, copy)), Some(o)) if set.contains(o) => Some((*xf, *copy)),
            _ => None,
        };
        let tp = |xf: glam::DMat4, p: [f32; 3]| -> [f32; 3] {
            xf.transform_point3(glam::Vec3::from(p).as_dvec3())
                .as_vec3()
                .into()
        };
        // Styles > Modeling colors.
        let (sel_color, lock_color) = self.document.as_ref().map_or((SELECTED, LOCKED), |d| {
            let mc = &d.model.style.modeling_colors;
            (style_view::rgba(mc.selected), style_view::rgba(mc.locked))
        });
        let mut edges = Vec::with_capacity(self.scene.edges.len());
        let mut copies = Vec::new();
        // Locked groups and components are highlighted red instead of blue.
        let highlight = |o: &Option<Owner>| match (o, &self.document) {
            (Some(o), Some(doc)) if doc.is_locked(*o) => lock_color,
            _ => sel_color,
        };
        // Compatibility > Bounding box only: selected groups and components show
        // their box, not their edges.
        let box_only = self.prefs.more.bbox_highlight;
        for (seg, owner) in self.scene.edges.iter().zip(&self.scene.edge_owner) {
            let mut seg = *seg;
            let selected = owner.is_some_and(|o| {
                self.selection.contains(&o) && !(box_only && matches!(o, Owner::Instance(_)))
            });
            let color = if selected {
                highlight(owner)
            } else {
                sel_color
            };
            if let Some(o) = owner
                && let (Some(a), Some(b)) = (self.stretch.map(o, seg.a), self.stretch.map(o, seg.b))
            {
                (seg.a, seg.b) = (a, b);
            }
            match moved(owner) {
                Some((xf, copy)) => {
                    let mut c = seg;
                    (c.a, c.b) = (tp(xf, c.a), tp(xf, c.b));
                    if selected {
                        c.color = color;
                    }
                    if copy {
                        copies.push(c);
                        if selected {
                            seg.color = color;
                        }
                    } else {
                        seg = c;
                    }
                }
                None if selected => seg.color = color,
                None => {}
            }
            edges.push(seg);
        }
        edges.extend(copies);
        let mut vertices: Vec<FaceVertex> = Vec::new();
        for (t, owner) in self.scene.tri_owner.iter().enumerate() {
            let Some(owner @ Owner::Face(_)) = owner.filter(|o| self.selection.contains(o)) else {
                continue;
            };
            let xf = moved(&Some(owner)).map(|(xf, _)| xf);
            for k in 0..3 {
                let mut v = self.scene.vertices[self.scene.indices[3 * t + k] as usize];
                if let Some(xf) = xf {
                    v.position = tp(xf, v.position);
                }
                v.position = self.stretch.map(&owner, v.position).unwrap_or(v.position);
                vertices.push(v);
            }
        }
        // Selected components and groups show their bounding box in their own axes.
        if let Some(doc) = &self.document {
            let ents = doc.active();
            for o in self.selection.items() {
                let Owner::Instance(i) = *o else { continue };
                let (Some(inst), Some((min, max))) =
                    (ents.instances.get(i), self.inst_bounds.get(&i))
                else {
                    continue;
                };
                let mut xf = doc.drawn_transform(self.ctx_xf * inst.transform, inst);
                if let Some((m, false)) = moved(&Some(*o)) {
                    xf = m * xf;
                }
                let b = selection::OrientedBox::from_aabb(*min, *max).transformed(xf);
                let color = if inst.locked { lock_color } else { sel_color };
                for [a, c] in b.edges() {
                    let f = |p: DVec3| p.as_vec3().to_array();
                    edges.push(Segment::line(f(a), f(c), color, [1.0, 0.0]));
                }
            }
        }
        let indices: Vec<u32> = (0..vertices.len() as u32).collect();
        self.viewport
            .set_selected_faces(device, &vertices, &indices);
        if let (Some(ghost), Some(xf)) = (&self.ghost, self.ghost_xf) {
            edges.extend(ghost.edges.iter().map(|s| {
                let mut s = *s;
                (s.a, s.b) = (tp(xf, s.a), tp(xf, s.b));
                s
            }));
        }
        edges.extend(self.scene.guides.iter().copied());
        edges.extend(self.cut_lines.iter().copied());
        self.viewport.set_edges(device, &edges);
    }

    /// Draws the targets of Move, Rotate or Scale at the transformed position while
    /// the tool runs and hides them at the original one (copies keep them). Only the
    /// targets are transformed per change; the rest of the model is re-sent once when
    /// the operation starts and ends (no rebuild).
    fn update_live(&mut self, device: &wgpu::Device) {
        self.update_ghost(device);
        let want = match (&self.modify, &self.inference, self.tool) {
            (Some(m), Some(inf), Tool::Modify(_)) => m
                .live(&to_local(inf, self.ctx_xf))
                .map(|(t, xf, copy)| (t.to_vec(), xf, copy)),
            _ => None,
        }
        .or_else(|| self.grips.live());
        if want == self.live {
            return;
        }
        let stretch = move_extra::live_stretch(self.document.as_ref(), &want, self.ctx_xf);
        let hidden = |l: &Option<(Vec<Owner>, glam::DMat4, bool)>| {
            l.as_ref().filter(|l| !l.2).map(|l| l.0.clone())
        };
        let hide = hidden(&want);
        if hidden(&self.live) != hide {
            match &hide {
                Some(targets) => {
                    let set: std::collections::HashSet<Owner> = targets.iter().copied().collect();
                    let indices: Vec<u32> = self
                        .scene
                        .indices
                        .chunks(3)
                        .zip(&self.scene.tri_owner)
                        .filter(|(_, o)| {
                            !o.is_some_and(|o| set.contains(&o) || stretch.contains(&o))
                        })
                        .flat_map(|(t, _)| t.iter().copied())
                        .collect();
                    self.viewport
                        .set_faces(device, &self.scene.vertices, &indices);
                }
                None => self
                    .viewport
                    .set_faces(device, &self.scene.vertices, &self.scene.indices),
            }
        }
        match &want {
            Some((targets, xf, _)) => {
                let set: std::collections::HashSet<Owner> = targets.iter().copied().collect();
                let w = self.ctx_xf * *xf * self.ctx_xf.inverse();
                let n = glam::DMat3::from_mat4(w).inverse().transpose();
                let f = |p: [f32; 3]| glam::Vec3::from(p).as_dvec3();
                let mut vertices = Vec::new();
                for (t, owner) in self.scene.tri_owner.iter().enumerate() {
                    if let Some(o) = owner.filter(|o| stretch.contains(o)) {
                        let tri = [0, 1, 2]
                            .map(|k| self.scene.vertices[self.scene.indices[3 * t + k] as usize]);
                        vertices.extend(stretch.triangle(&o, tri));
                        continue;
                    }
                    if !owner.is_some_and(|o| set.contains(&o)) {
                        continue;
                    }
                    for k in 0..3 {
                        let mut v = self.scene.vertices[self.scene.indices[3 * t + k] as usize];
                        v.position = w.transform_point3(f(v.position)).as_vec3().into();
                        v.normal = (n * f(v.normal)).normalize_or_zero().as_vec3().into();
                        vertices.push(v);
                    }
                }
                let indices: Vec<u32> = (0..vertices.len() as u32).collect();
                self.viewport.set_preview_faces(device, &vertices, &indices);
            }
            None => self.viewport.set_preview_faces(device, &[], &[]),
        }
        self.live = want;
        self.stretch = stretch;
        self.update_highlight(device);
    }

    /// Draws the clipboard where Paste would place it (faces as preview faces, edges
    /// with the model's), re-sent only when the cursor moves it.
    fn update_ghost(&mut self, device: &wgpu::Device) {
        let want = match (&self.modify, &self.inference, self.tool, &self.ghost) {
            (Some(m), Some(inf), Tool::Modify(_), Some(_)) => m
                .ghost(&to_local(inf, self.ctx_xf))
                .map(|xf| self.ctx_xf * xf),
            _ => None,
        };
        if want == self.ghost_xf {
            return;
        }
        self.ghost_xf = want;
        match (&self.ghost, want) {
            (Some(ghost), Some(w)) => {
                let n = glam::DMat3::from_mat4(w).inverse().transpose();
                let f = |p: [f32; 3]| glam::Vec3::from(p).as_dvec3();
                let vertices: Vec<FaceVertex> = ghost
                    .indices
                    .iter()
                    .map(|&i| {
                        let mut v = ghost.vertices[i as usize];
                        v.position = w.transform_point3(f(v.position)).as_vec3().into();
                        v.normal = (n * f(v.normal)).normalize_or_zero().as_vec3().into();
                        v
                    })
                    .collect();
                let indices: Vec<u32> = (0..vertices.len() as u32).collect();
                self.viewport.set_preview_faces(device, &vertices, &indices);
            }
            _ => self.viewport.set_preview_faces(device, &[], &[]),
        }
        self.update_highlight(device);
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
            Ok(doc) => self.install(ctx, device, path, doc),
            Err(e) => self.file_error("open", path, e),
        }
    }

    /// Makes an opened document the current one.
    fn install(
        &mut self,
        ctx: &egui::Context,
        device: &wgpu::Device,
        path: &std::path::Path,
        doc: Document,
    ) {
        {
            {
                if let Some(camera) = doc
                    .saved_camera()
                    .or_else(|| doc.extents_camera(self.camera.fov_deg))
                {
                    self.camera = camera;
                }
                self.transition = None;
                self.camera_history.clear();
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(doc.title()));
                self.document = Some(doc);
                self.after_open();
                self.clipboard_changed_document();
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
        }
    }

    fn run_command(&mut self, ctx: &egui::Context, frame: &eframe::Frame, id: &str) {
        if self.early_command(ctx, frame, id) {
            return;
        }
        if id == "tool.snaps"
            && let Some(rs) = frame.wgpu_render_state()
            && !self.snaps_command(&rs.device.clone())
        {
            return;
        }
        if let Some(tool) = Tool::from_command(id) {
            self.tool = tool;
            self.draw = tools::for_command(id);
            self.modify = modify::for_command(id);
            self.start_pick_tool(frame, id);
            if let Some(m) = self.modify.as_mut() {
                m.set_material(self.current_material);
            }
            self.sandbox_activate(frame, id);
            self.erase_drag.clear();
            self.shift_lock.reset(&mut self.engine);
            self.inference = None;
            self.extra.change_axes = None;
        } else if id == "help.search" {
            self.open_search();
        } else if id == "edit.undo" && self.undo_cancels_tool() {
        } else if id == "view.shadows" {
            if let Some(doc) = &mut self.document {
                // A view setting, not an undo step in this draft (SketchUp records
                // shadow toggles in undo, unverified).
                doc.model.shadows.enabled = !doc.model.shadows.enabled;
                self.apply_shadows();
            }
        } else if let Some(rs) = frame.wgpu_render_state()
            && id.starts_with("view.anim_")
        {
            if self.animation_command(id) {
                let device = rs.device.clone();
                self.rebuild(&device);
            }
        } else if matches!(id, "file.export_2d" | "file.export_3d") {
            self.export_command(id);
        } else if let Some(rs) = frame.wgpu_render_state()
            && id == "file.import"
        {
            let device = rs.device.clone();
            self.import_command(&device);
        } else if id == "view.scene_tabs" {
            self.scene_tabs = !self.scene_tabs;
        } else if let Some(rs) = frame.wgpu_render_state()
            && id.starts_with("view.")
            && let Some(doc) = self.document.as_mut()
            && view_toggle(&mut doc.model.style, id)
        {
            // Display settings; not undo steps in this draft.
            let device = rs.device.clone();
            self.rebuild(&device);
        } else if self.camera_command(id)
            || self.context_command(frame, id)
            || self.context_extra_command(frame, id)
            || self.extra_command(ctx, frame, id)
            || self.m13_command(ctx, frame, id)
            || self.presentation_command(ctx, frame, id)
            || self.texture_command(frame, id)
            || self.library_command(ctx, id)
            || self.match_photo_command(id)
            || self.extensions_command(ctx, frame, id)
        {
        } else if id == "window.model_info" {
            self.open_model_info();
        } else if id == "window.preferences" {
            self.open_preferences();
        } else if id == "file.exit" {
            self.request(ctx, frame, confirm::Pending::Exit);
        } else if let Some(rs) = frame.wgpu_render_state()
            && matches!(
                id,
                "edit.cut" | "edit.copy" | "edit.paste" | "edit.paste_in_place"
            )
        {
            let device = rs.device.clone();
            self.clipboard_command(ctx, &device, id);
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
                    | "edit.invert_selection"
                    | "edit.hide"
                    | "edit.unhide_selected"
                    | "edit.unhide_last"
                    | "edit.unhide_all"
                    | "edit.lock"
                    | "edit.unlock_selected"
                    | "edit.unlock_all"
            )
        {
            let device = rs.device.clone();
            match id {
                "edit.select_all" => {
                    let all = self.all_entities();
                    self.select(&device, Mode::Replace, all);
                }
                "edit.select_none" => self.select(&device, Mode::Replace, Vec::new()),
                "edit.invert_selection" => {
                    let all = self.all_entities();
                    let inverted = all
                        .into_iter()
                        .filter(|o| !self.selection.contains(o))
                        .collect();
                    self.select(&device, Mode::Replace, inverted);
                }
                "edit.hide"
                | "edit.unhide_selected"
                | "edit.unhide_last"
                | "edit.unhide_all"
                | "edit.lock"
                | "edit.unlock_selected"
                | "edit.unlock_all" => {
                    let items = self.selection.items().to_vec();
                    let Some(doc) = &mut self.document else {
                        return;
                    };
                    let changed = match id {
                        "edit.hide" => doc.hide(&items),
                        "edit.unhide_selected" => doc.unhide(&items),
                        "edit.unhide_last" => doc.unhide_last(),
                        "edit.unhide_all" => doc.unhide_all(),
                        "edit.lock" => doc.lock(&items, true),
                        "edit.unlock_selected" => doc.lock(&items, false),
                        _ => doc.unlock_all(),
                    };
                    if changed {
                        // Hidden entities leave the selection unless they stay visible
                        // (View > Hidden Geometry); locking keeps it.
                        if id == "edit.hide"
                            && !doc.model.style.hidden_geometry
                            && !doc.model.style.hidden_objects
                        {
                            self.selection.clear();
                        }
                        self.rebuild(&device);
                    }
                }
                "edit.delete_guides" => {
                    if let Some(doc) = &mut self.document
                        && doc.edit(|root| !std::mem::take(&mut root.guides).is_empty())
                    {
                        self.rebuild(&device);
                    }
                }
                "edit.make_component" => {
                    let sel = kernel_selection(self.selection.items());
                    self.open_make_component(sel);
                }
                "edit.create_group" => {
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
                    if id == "edit.explode"
                        && let Some(doc) = &self.document
                    {
                        // Locked groups and components are not exploded.
                        instances.retain(|&i| !doc.is_locked(Owner::Instance(i)));
                    }
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
            self.request(ctx, frame, confirm::Pending::New);
        } else if matches!(id, "file.save" | "file.save_as" | "file.save_copy_as") {
            self.save_command(ctx, id);
        } else if id == "file.open" {
            self.request(ctx, frame, confirm::Pending::Open(None));
        }
    }

    /// Edit > Cut, Copy, Paste and Paste In Place.
    ///
    /// Copy keeps the selection; Cut copies, then deletes like Delete (one undo step).
    /// Paste starts placing the clipboard with the cursor ([`modify::Paste`]); Paste In
    /// Place inserts it at the coordinates it was copied from, in the open context. The
    /// pasted entities become the selection.
    fn clipboard_command(&mut self, ctx: &egui::Context, device: &wgpu::Device, id: &str) {
        let Some(doc) = &mut self.document else {
            return;
        };
        match id {
            "edit.cut" | "edit.copy" => {
                let items = self.selection.items().to_vec();
                let sel = kernel_selection(&items);
                let pick = |f: fn(&Owner) -> Option<usize>| items.iter().filter_map(f).collect();
                let dims: Vec<usize> = pick(|o| match o {
                    Owner::Dimension(i) => Some(*i),
                    _ => None,
                });
                let texts: Vec<usize> = pick(|o| match o {
                    Owner::Text(i) => Some(*i),
                    _ => None,
                });
                let clip =
                    mt_kernel::clipboard::copy(doc.active(), &sel, &dims, &texts, self.ctx_xf);
                if mt_kernel::clipboard::is_empty(&clip) {
                    return;
                }
                // egui turns Ctrl+V into a paste event only when the system clipboard
                // holds text: leave a note there.
                ctx.copy_text("meshtard entities".into());
                self.clipboard = Some(clip);
                if id == "edit.cut" {
                    doc.delete(&items);
                    self.selection.clear();
                    self.rebuild(device);
                }
            }
            _ => {
                let Some(mut clip) = self.clipboard.clone() else {
                    return;
                };
                // Instances of definitions that are gone or would contain themselves
                // are left out.
                clip.instances.retain(|i| {
                    doc.model.definitions.contains_key(i.definition)
                        && !doc.in_context_path(i.definition)
                });
                if mt_kernel::clipboard::is_empty(&clip) {
                    self.message = Some("Nothing to paste here.".into());
                    return;
                }
                if let Some(d) = self.draw.as_mut() {
                    d.cancel();
                }
                if id == "edit.paste_in_place" {
                    let to_local = self.ctx_xf.inverse();
                    let mut pasted = None;
                    if doc.edit(|ents| {
                        pasted = Some(mt_kernel::clipboard::paste(ents, &clip, to_local, |_| true));
                        true
                    }) {
                        let owners = pasted.map(|p| modify::pasted_owners(&p));
                        self.selection.clear();
                        self.selection
                            .apply(Mode::Replace, owners.unwrap_or_default());
                        self.rebuild(device);
                    }
                    return;
                }
                let Some((grab, _)) =
                    mt_kernel::clipboard::clip_bounds(&clip, &doc.model.definitions)
                else {
                    return;
                };
                self.ghost = Some(doc.ghost_scene(&clip));
                self.ghost_xf = None;
                self.tool = Tool::Modify("edit.paste");
                self.draw = None;
                self.modify = Some(Box::new(modify::Paste::new(clip, grab, self.ctx_xf)));
                self.erase_drag.clear();
                self.engine.unlock();
                self.inference = None;
                self.selection.clear();
                self.update_highlight(device);
            }
        }
    }

    /// The clipboard's instances name definitions of the model they were copied
    /// from: another document cannot use them (loose geometry and annotations stay).
    fn clipboard_changed_document(&mut self) {
        if let Some(c) = &mut self.clipboard {
            c.instances.clear();
            if mt_kernel::clipboard::is_empty(c) {
                self.clipboard = None;
            }
        }
    }

    /// File > New from the built-in template, without asking (see `confirm`).
    fn new_document_builtin(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        {
            if let Some(rs) = frame.wgpu_render_state() {
                let device = rs.device.clone();
                let doc = Document::untitled();
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(doc.title()));
                self.document = Some(doc);
                self.clipboard_changed_document();
                self.selection.clear();
                self.camera = Camera::from_spec(&self.spec.default_model.camera);
                self.transition = None;
                self.camera_history.clear();
                self.rebuild(&device);
            }
        }
    }

    /// File > Save, Save As, Save A Copy As. Files are written in the SketchUp 2017
    /// format (the only one the writer produces); SketchUp 2026 opens them.
    /// Returns whether the file was written.
    fn save(&mut self, ctx: &egui::Context, id: &str) -> bool {
        let Some(doc) = self.document.as_ref() else {
            return false;
        };
        let path = if id == "file.save" && doc.has_file {
            Some(doc.path.clone())
        } else {
            let name = doc.path.file_name().map_or_else(
                || "Untitled.skp".into(),
                |n| n.to_string_lossy().into_owned(),
            );
            filedialog::save(&[("Meshtard Models", &["skp"])], &name)
        };
        let Some(path) = path else {
            return false;
        };
        // Preferences > General: Fix Problems, backup.
        self.before_save(&path);
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        let result = doc
            .to_skp()
            .map_err(|e| e.to_string())
            .and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string()));
        match result {
            Ok(()) => {
                self.prefs.after_save(&path);
                if id != "file.save_copy_as" {
                    doc.mark_saved(&path);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Title(doc.title()));
                }
                self.message = Some(format!("Saved {} (.skp 2017 format).", path.display()));
                true
            }
            Err(e) => {
                self.file_error("save", &path, e);
                false
            }
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context, frame: &eframe::Frame, typing: bool) {
        self.spring_update(ctx);
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let pressed: Vec<(&'static str, Option<egui::Key>)> = ctx.input(|i| {
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
                .map(|(_, key, id)| (*id, Some(*key)))
                .chain(
                    // egui turns Ctrl+X, Ctrl+C and Ctrl+V into clipboard events instead
                    // of key presses (Ctrl+V only while the system clipboard holds
                    // text). On macOS the native menu owns these shortcuts.
                    i.events
                        .iter()
                        .filter(|_| !cfg!(target_os = "macos"))
                        .filter_map(|e| match e {
                            egui::Event::Cut => Some(("edit.cut", None)),
                            egui::Event::Copy => Some(("edit.copy", None)),
                            egui::Event::Paste(_) => Some(("edit.paste", None)),
                            _ => None,
                        }),
                )
                .collect()
        });
        for (id, key) in pressed {
            if let Some(key) = key
                && !self.spring_press(ctx, id, key)
            {
                continue;
            }
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
            ctrl: ui.input(|i| i.modifiers.command),
            wheel: if response.hovered() {
                f64::from(notches) * self.prefs.wheel_sign()
            } else {
                0.0
            },
        };
        self.animate_camera(ui.ctx());
        let before = self.camera.clone();
        // Match Photo editing takes the mouse (no navigation while lining up).
        let photo = self.match_photo_input(ui, &response, rect, viewport_px);
        if !photo {
            nav::apply(self.tool, &mut self.camera, &input, viewport_px);
        }
        self.cancel_transition_if_moved(&before);
        self.viewport_px = viewport_px;
        if self.transition.is_none() {
            self.camera_history.track(&before, &self.camera);
        }
        // Texture Position takes the mouse while it runs (the tools below then see a
        // navigation tool, which does nothing here).
        let texpos =
            self.texture_position_input(ui, &response, rect, viewport_px, &render_state.device);
        match if texpos || photo {
            Tool::Pan
        } else {
            self.tool
        } {
            Tool::Select => {
                self.select_tool(ui, &response, rect, viewport_px, &render_state.device)
            }
            Tool::Lasso => self.lasso_tool(ui, &response, rect, viewport_px, &render_state.device),
            Tool::Draw(_) => self.draw_tool(ui, &response, rect, viewport_px, &render_state.device),
            Tool::Modify(_) => {
                self.modify_tool(ui, &response, rect, viewport_px, &render_state.device)
            }
            Tool::Eraser => {
                self.eraser_tool(ui, &response, rect, viewport_px, &render_state.device)
            }
            Tool::Pick(_) => self.pick_tool(ui, &response, rect, viewport_px, &render_state.device),
            Tool::ZoomWindow => {
                self.inference = None;
                self.zoom_window_tool(ui, &response, rect, viewport_px);
            }
            Tool::PositionCamera | Tool::Walk => {
                self.inference = None;
                self.camera_tool(ui, &response, rect, viewport_px);
            }
            _ => self.inference = None,
        }
        self.context_menu(ui, &response, rect, viewport_px, &render_state.device);
        self.orbit_double_click(ui, &response, rect, viewport_px);
        self.tool_cursor(ui, &response, rect, viewport_px);
        self.update_facing(&render_state.device);
        self.update_live(&render_state.device);

        let photo_behind = self.match_photo_behind();
        self.viewport.set_transparent_background(photo_behind);
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
        drop(renderer);
        if self.match_photo_under(ui, rect) {
            ui.painter().image(
                id,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        self.match_photo_over(ui, rect);
        self.draw_selection_box(ui, &response, rect);
        self.crosshairs(ui, &response, rect, viewport_px);
        self.draw_lasso(ui, &response, rect);
        self.draw_zoom_box(ui, &response, rect);
        self.pick_overlay(ui, rect);
        self.context_extra_paint(ui, rect);
        self.grips_paint(ui, rect, viewport_px, &render_state.device);
        self.snaps_paint(ui, rect, viewport_px);
        self.texture_position_paint(ui, rect, viewport_px);
        if self.annotations(ui, rect, viewport_px) {
            self.rebuild(&render_state.device);
        }
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
            if let Some(m) = &self.modify {
                // Flip's planes, under the inference marker.
                let planes: Vec<_> = m
                    .planes(&local)
                    .into_iter()
                    .map(|p| modify::Plane {
                        corners: p.corners.map(|c| xf.transform_point3(c)),
                        ..p
                    })
                    .collect();
                overlay::planes(&ui.painter().with_clip_rect(rect), to_screen, &planes);
            }
            overlay::paint(
                &ui.painter().with_clip_rect(rect),
                to_screen,
                anchor,
                &previews,
                inf,
            );
            if let Some(m) = &self.modify {
                let markers: Vec<_> = m
                    .markers(&local)
                    .into_iter()
                    .map(|k| (xf.transform_point3(k.point), k.hot))
                    .collect();
                overlay::markers(&ui.painter().with_clip_rect(rect), to_screen, &markers);
            }
        }
    }

    /// Paints dimension and text labels, and the Text tool's editor. Returns whether
    /// the model changed.
    fn annotations(&mut self, ui: &egui::Ui, rect: egui::Rect, viewport_px: DVec2) -> bool {
        let ppp = ui.ctx().pixels_per_point();
        let camera = &self.camera;
        let to_screen = |p: DVec3| {
            camera
                .project(p, viewport_px)
                .map(|q| rect.min + egui::vec2(q.x as f32, q.y as f32) / ppp)
        };
        let selection = &self.selection;
        annot::paint(
            &ui.painter().with_clip_rect(rect),
            &to_screen,
            rect,
            &self.scene.annotations,
            &|o| selection.contains(o),
        );
        match (self.draw.as_mut(), self.document.as_mut()) {
            (Some(draw), Some(doc)) => {
                annot::text_editor(ui, rect, &to_screen, self.ctx_xf, draw.as_mut(), doc)
            }
            _ => false,
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
        let (keys, shift, alt) = ui.input(|i| {
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
                i.modifiers.alt,
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
            self.shift_lock
                .release(&mut self.engine, &mut locks::Draw(draw.as_mut()));
            draw.cancel();
            self.shift_lock.reset(&mut self.engine);
        }
        // Shift locks the direction or the plane, Alt cycles the Line tool's
        // inferences (see `locks`).
        let anchor = draw.anchor().map(|a| self.ctx_xf.transform_point3(a));
        let is_line = self.tool == Tool::Draw("tool.line");
        self.shift_lock.update(
            &mut self.engine,
            &mut locks::Draw(draw.as_mut()),
            self.inference.as_ref(),
            anchor,
            locks::Keys {
                shift,
                alt,
                shift_locks: true,
                alt_cycles: is_line && anchor.is_some(),
            },
        );

        // A drag starts where the button went down.
        let press = response
            .drag_started_by(egui::PointerButton::Primary)
            .then(|| ui.input(|i| i.pointer.press_origin()))
            .flatten();
        let cursor = press
            .or(response.hover_pos())
            .or(response.interact_pointer_pos())
            .map(|p| {
                DVec2::new(
                    f64::from((p.x - rect.min.x) * ppp),
                    f64::from((p.y - rect.min.y) * ppp),
                )
            });
        self.draw_tool_pre(ui, cursor, viewport_px);
        let Some(draw) = self.draw.as_mut() else {
            return;
        };
        let anchor = draw.anchor().map(|a| self.ctx_xf.transform_point3(a));
        let now = ui.input(|i| i.time);
        let ctx_xf = self.ctx_xf;
        self.inference = cursor.map(|c| {
            let view = mt_infer::View {
                camera: &self.camera,
                viewport: viewport_px,
            };
            let inf = self
                .engine
                .infer_at(&self.geometry, self.bvh.as_ref(), view, c, anchor, now);
            // The tool's shape hints (Square, Half Circle...), in its own space.
            draw.hint(&to_local(&inf, ctx_xf))
                .map_or(inf, |h| to_world(&h, ctx_xf))
        });
        let (Some(inf), Some(doc)) = (self.inference, self.document.as_mut()) else {
            return;
        };
        let inf = to_local(&inf, self.ctx_xf);
        let primary = egui::PointerButton::Primary;
        // Preferences > Drawing > Click Style (`click_style`); Freehand only drags.
        let style = if self.tool == Tool::Draw("tool.freehand") {
            click_style::ClickStyle::ClickDragRelease
        } else {
            self.prefs.drawing.click_style
        };
        let changed = match self.prefs.pointer.gesture(style, response, press) {
            click_style::Gesture::Click => doc.edit(|root| draw.click(root, &inf)),
            click_style::Gesture::DragStart => {
                let before = draw.anchor();
                let changed = doc.edit(|root| draw.drag_start(root, &inf));
                self.prefs
                    .pointer
                    .started(draw.anchor() != before || changed);
                changed
            }
            click_style::Gesture::DragEnd => doc.edit(|root| draw.drag_end(root, &inf)),
            click_style::Gesture::None => {
                if response.dragged_by(primary) {
                    draw.drag_move(&inf);
                }
                false
            }
        };
        // "Continue line drawing" off: a drawn segment ends the Line tool's chain.
        if changed
            && self.tool == Tool::Draw("tool.line")
            && !style.line_continues(self.prefs.drawing.continue_line)
        {
            draw.cancel();
        }
        let changed = self.draw_tool_post(response, &inf) | changed;
        if changed {
            self.shift_lock.after_change(&mut self.engine);
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
        let (escape, ctrl, arrows, down, shift, alt) = ui.input(|i| {
            let k = |key| i.key_pressed(key);
            (
                k(egui::Key::Escape),
                i.modifiers.command,
                [
                    k(egui::Key::ArrowRight),
                    k(egui::Key::ArrowLeft),
                    k(egui::Key::ArrowUp),
                ],
                k(egui::Key::ArrowDown),
                i.modifiers.shift,
                i.modifiers.alt,
            )
        });
        // Pressing Ctrl (Option on macOS in SketchUp; unverified) toggles copy / new
        // faces; holding it is no longer needed.
        let ctrl_down = ctrl && !self.ctrl_was_down;
        self.ctrl_was_down = ctrl;
        if self.modify.as_ref().is_some_and(|m| m.finished()) {
            self.modify = None;
            self.ghost = None;
            self.tool = Tool::Select;
            self.engine.unlock();
            self.inference = None;
            return;
        }
        if self.grips_frame(ui, response, rect, viewport_px, device) {
            return;
        }
        let Some(tool) = self.modify.as_mut() else {
            return;
        };
        if ctrl_down {
            tool.toggle_ctrl();
        }
        for (pressed, axis) in arrows.into_iter().zip([
            mt_infer::Axis::Red,
            mt_infer::Axis::Green,
            mt_infer::Axis::Blue,
        ]) {
            if pressed && !tool.arrow(axis) {
                self.engine.toggle_axis_lock(axis);
            }
        }
        // Down: parallel to the reference edge, as in the drawing tools.
        if down && tool.direction_locks() {
            self.engine.toggle_parallel_lock(&self.geometry);
        }
        if escape {
            self.shift_lock
                .release(&mut self.engine, &mut locks::Modify(tool.as_mut()));
            tool.cancel();
            self.shift_lock.reset(&mut self.engine);
            if tool.finished() {
                ui.ctx().request_repaint();
            }
        }
        // Shift locks the direction (or Rotate's plane) where the tool has no own use
        // for it and no constraint of its own (Push/Pull, Flip drags).
        let anchor = tool.anchor().map(|a| self.ctx_xf.transform_point3(a));
        let shift_locks =
            tool.direction_locks() && !tool.shift_is_modifier() && tool.constraint().is_none();
        self.shift_lock.update(
            &mut self.engine,
            &mut locks::Modify(tool.as_mut()),
            self.inference.as_ref(),
            anchor,
            locks::Keys {
                shift,
                alt,
                shift_locks,
                alt_cycles: false,
            },
        );
        if let Some(dir) = tool.constraint() {
            self.engine.lock_direction(
                self.ctx_xf.transform_vector3(dir),
                mt_infer::Kind::Perpendicular,
            );
        }
        // A drag starts where the button went down (a grip under the press).
        let press = response
            .drag_started_by(egui::PointerButton::Primary)
            .then(|| ui.input(|i| i.pointer.press_origin()))
            .flatten();
        let cursor = press
            .or(response.hover_pos())
            .or(response.interact_pointer_pos())
            .map(|p| {
                DVec2::new(
                    f64::from((p.x - rect.min.x) * ppp),
                    f64::from((p.y - rect.min.y) * ppp),
                )
            });
        let anchor = tool.anchor().map(|a| self.ctx_xf.transform_point3(a));
        let now = ui.input(|i| i.time);
        self.inference = cursor.map(|c| {
            let view = mt_infer::View {
                camera: &self.camera,
                viewport: viewport_px,
            };
            self.engine
                .infer_at(&self.geometry, self.bvh.as_ref(), view, c, anchor, now)
        });
        // A grabbed bounding box grip is Move's base point.
        if let (Some(p), Some(i)) = (self.grips.take_base(), self.inference.as_mut()) {
            i.point = self.ctx_xf.transform_point3(p);
        }
        // Locked groups and components are left out of what modify tools act on.
        let unlocked: Vec<Owner> = self
            .selection
            .items()
            .iter()
            .copied()
            .filter(|o| !self.document.as_ref().is_some_and(|d| d.is_locked(*o)))
            .collect();
        let local = self.inference.map(|i| to_local(&i, self.ctx_xf));
        tool.input(&modify::Input {
            held: ui.input(|i| cursor::Mods {
                ctrl: i.modifiers.command,
                shift: i.modifiers.shift,
                alt: i.modifiers.alt,
            }),
            press: response.drag_started_by(egui::PointerButton::Primary),
            ray: cursor.map(|c| {
                let r = self.camera.ray(c, viewport_px);
                let inv = self.ctx_xf.inverse();
                let d = inv.transform_vector3(r.dir).normalize_or_zero();
                (inv.transform_point3(r.origin), d)
            }),
            cur: local.as_ref(),
            material: cursor.and_then(|c| {
                let r = self.camera.ray(c, viewport_px);
                let hit = self.bvh.ray(r.origin, r.dir)?;
                let [a, b, c] = self.scene.triangles.get(hit.triangle)?;
                let back = (*b - *a).cross(*c - *a).dot(r.dir) > 0.0;
                let m = self.scene.tri_material.get(hit.triangle)?;
                Some(m[usize::from(back)])
            }),
        });
        if let Some(doc) = &self.document {
            let (camera, xf, bounds) = (&self.camera, self.ctx_xf, &self.inst_bounds);
            tool.hover(&modify::Hover {
                ents: doc.active(),
                selection: &unlocked,
                instance_bounds: &|i| bounds.get(&i).copied(),
                project: &|p| camera.project(xf.transform_point3(p), viewport_px),
                cursor,
            });
        }
        let snaps = component_snaps::Frame {
            camera: &self.camera,
            ctx_xf: self.ctx_xf,
            scene: &self.scene,
            bvh: self.bvh.as_ref(),
            cursor,
            viewport_px,
            alt,
        };
        if component_snaps::frame(tool.as_mut(), self.document.as_mut(), snaps) {
            self.rebuild(device);
            return;
        }
        let (Some(inf), Some(c), Some(doc)) = (self.inference, cursor, self.document.as_mut())
        else {
            return;
        };
        let inf = to_local(&inf, self.ctx_xf);
        // A drag works like two clicks: press, then release (SketchUp's click-drag),
        // as Preferences > Drawing > Click Style says (`click_style`).
        let gesture = self
            .prefs
            .pointer
            .gesture(self.prefs.drawing.click_style, response, press);
        let anchor_before = tool.anchor();
        match gesture {
            click_style::Gesture::Click | click_style::Gesture::DragStart => {}
            click_style::Gesture::DragEnd if anchor_before.is_some() => {}
            _ => return,
        }
        let double = response.double_clicked();
        let mut picked =
            selection::pick(&self.scene, self.bvh.as_ref(), &self.camera, c, viewport_px);
        if picked.is_some_and(|o| doc.is_locked(o)) {
            picked = None;
        }
        // Preferences > Drawing > Disable Pre-Pick on Push/Pull.
        let selection =
            if self.prefs.drawing.no_prepick && self.tool == Tool::Modify("tool.push_pull") {
                Vec::new()
            } else {
                unlocked
            };
        let eye = self.ctx_xf.inverse().transform_point3(self.camera.eye);
        let changed = doc.edit(|root| {
            let mut ctx = modify::Ctx {
                ents: root,
                picked,
                selection: &selection,
                // Ctrl works as a toggle (above), not while held.
                ctrl: false,
                eye,
            };
            let clicked = tool.click(&mut ctx, &inf);
            clicked | (double && tool.double_click(&mut ctx, &inf))
        });
        if gesture == click_style::Gesture::DragStart {
            self.prefs
                .pointer
                .started(tool.anchor() != anchor_before || changed);
        }
        // Paint Bucket with Alt: the sampled material becomes current.
        if let Some(m) = tool.take_sampled_material() {
            self.current_material = m;
        }
        // Set Component Axes finished: make the component with the picked axes.
        if let Some((d, axes)) = tool.take_axes() {
            self.modify = None;
            self.tool = Tool::Select;
            self.engine.unlock();
            self.create_component(d, Some(axes), device);
            return;
        }
        if let Some(axes) = tool.take_model_axes() {
            self.set_model_axes(axes, device);
            return;
        }
        if tool.constraint().is_none() && tool.anchor().is_none() {
            self.shift_lock.after_change(&mut self.engine);
        }
        if changed {
            self.shift_lock.after_change(&mut self.engine);
            if let Some(items) = tool.take_selection() {
                ui.ctx().request_repaint();
                self.selection.clear();
                self.selection.apply(Mode::Replace, items);
            } else if tool.keeps_selection() {
                let ents = doc.active();
                self.selection.retain(|o| selection::exists(ents, o));
            } else {
                self.selection.clear();
            }
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
        let mods = ui.input(|i| i.modifiers);
        // Erasing one edge of a curve erases the whole curve, so the drag highlights all
        // of it (hiding and softening work per edge).
        let whole_curve = |app: &Self, o: Owner| -> Vec<Owner> {
            let curve = match (o, &app.document) {
                (Owner::Edge(e), Some(doc)) if !mods.shift && !mods.command => {
                    doc.active().edges.get(e).and_then(|x| x.attrs.curve)
                }
                _ => None,
            };
            let Some(c) = curve else {
                return vec![o];
            };
            let ents = app.document.as_ref().map(Document::active);
            let mut out = vec![o];
            for owner in app.scene.edge_owner.iter().flatten() {
                if let Owner::Edge(e) = *owner
                    && !out.contains(owner)
                    && ents
                        .and_then(|x| x.edges.get(e))
                        .and_then(|x| x.attrs.curve)
                        == Some(c)
                {
                    out.push(*owner);
                }
            }
            out
        };
        let mut done: Vec<Owner> = Vec::new();
        if response.clicked_by(primary) {
            done.extend(pos.and_then(|p| pick(self, p)));
        } else if response.dragged_by(primary) || response.drag_started_by(primary) {
            // Picks along the pointer's path since the last frame, so a fast drag does
            // not skip edges.
            let mut added = false;
            if let Some(p) = pos {
                let delta = response.drag_delta();
                let steps = (delta.length() / 3.0).ceil().max(1.0) as usize;
                for k in 0..steps {
                    let q = p - delta * (k as f32 / steps as f32);
                    if let Some(o) = pick(self, q)
                        && !self.erase_drag.contains(&o)
                    {
                        for o in whole_curve(self, o) {
                            if !self.erase_drag.contains(&o) {
                                self.erase_drag.push(o);
                            }
                        }
                        added = true;
                    }
                }
            }
            if added {
                // Highlighted in the selection color until the button is released.
                self.select(device, Mode::Replace, self.erase_drag.clone());
            }
        } else if response.drag_stopped_by(primary) {
            done = std::mem::take(&mut self.erase_drag);
            self.selection.clear();
        }
        if done.is_empty() {
            return;
        }
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
/// An inference from the open context's space back to world space.
fn to_world(inf: &mt_infer::Inference, ctx_xf: glam::DMat4) -> mt_infer::Inference {
    if ctx_xf == glam::DMat4::IDENTITY {
        return *inf;
    }
    let p = |q: DVec3| ctx_xf.transform_point3(q);
    let normal_xf = glam::DMat3::from_mat4(ctx_xf).inverse().transpose();
    mt_infer::Inference {
        point: p(inf.point),
        guide: inf.guide.map(|(a, b)| (p(a), p(b))),
        normal: inf.normal.map(|n| (normal_xf * n).normalize_or_zero()),
        segment: inf.segment.map(|[a, b]| [p(a), p(b)]),
        ..*inf
    }
}

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
        "view.section_fill" => s.section_fill = !s.section_fill,
        "view.xray" => s.xray = !s.xray,
        "view.axes" => s.axes = !s.axes,
        "view.guides" => s.guides = !s.guides,
        "view.hidden_geometry" => s.hidden_geometry = !s.hidden_geometry,
        "view.hidden_objects" => s.hidden_objects = !s.hidden_objects,
        "view.wireframe" => s.face_style = F::Wireframe,
        "view.hidden_line" => s.face_style = F::HiddenLine,
        "view.shaded" => s.face_style = F::Shaded,
        "view.shaded_textures" => s.face_style = F::ShadedWithTextures,
        "view.monochrome" => s.face_style = F::Monochrome,
        "view.edges" => s.edges = !s.edges,
        "view.back_edges" => s.back_edges = !s.back_edges,
        "view.profiles" => s.profiles = !s.profiles,
        "view.depth_cue" => s.depth_cue = !s.depth_cue,
        "view.extension" => s.extension = !s.extension,
        "view.fog" => s.fog.enabled = !s.fog.enabled,
        "view.hide_rest_of_model" => s.hide_rest_of_model = !s.hide_rest_of_model,
        "view.hide_similar_components" => s.hide_similar_components = !s.hide_similar_components,
        _ => return false,
    }
    true
}

fn view_checked(s: &mt_kernel::StyleSettings, id: &str) -> bool {
    use mt_kernel::FaceStyle as F;
    match id {
        "view.section_planes" => s.section_planes,
        "view.section_cuts" => s.section_cuts,
        "view.section_fill" => s.section_fill,
        "view.xray" => s.xray,
        "view.axes" => s.axes,
        "view.guides" => s.guides,
        "view.hidden_geometry" => s.hidden_geometry,
        "view.hidden_objects" => s.hidden_objects,
        "view.wireframe" => s.face_style == F::Wireframe,
        "view.hidden_line" => s.face_style == F::HiddenLine,
        "view.shaded" => s.face_style == F::Shaded,
        "view.shaded_textures" => s.face_style == F::ShadedWithTextures,
        "view.monochrome" => s.face_style == F::Monochrome,
        "view.edges" => s.edges,
        "view.back_edges" => s.back_edges,
        "view.profiles" => s.profiles,
        "view.depth_cue" => s.depth_cue,
        "view.extension" => s.extension,
        "view.fog" => s.fog.enabled,
        "view.hide_rest_of_model" => s.hide_rest_of_model,
        "view.hide_similar_components" => s.hide_similar_components,
        _ => false,
    }
}

/// Menu commands shown with a check mark (native macOS menu).
#[cfg(target_os = "macos")]
const VIEW_TOGGLES: &[&str] = &[
    "view.shadows",
    "view.scene_tabs",
    "view.hidden_geometry",
    "view.hidden_objects",
    "view.axes",
    "view.guides",
    "view.section_planes",
    "view.section_cuts",
    "view.section_fill",
    "view.xray",
    "view.wireframe",
    "view.hidden_line",
    "view.shaded",
    "view.shaded_textures",
    "view.monochrome",
    "view.edges",
    "view.back_edges",
    "view.profiles",
    "view.depth_cue",
    "view.extension",
    "view.fog",
    "view.hide_rest_of_model",
    "view.hide_similar_components",
];

/// The kernel's view of a selection.
fn kernel_selection(items: &[Owner]) -> mt_kernel::group::Selection {
    let mut sel = mt_kernel::group::Selection::default();
    for o in items {
        match *o {
            Owner::Edge(e) => sel.edges.push(e),
            Owner::Face(f) => sel.faces.push(f),
            Owner::Instance(i) => sel.instances.push(i),
            Owner::Section(_) | Owner::Dimension(_) | Owner::Text(_) => {}
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

impl eframe::App for App {
    fn on_exit(&mut self) {
        self.preferences_exit();
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let Some(mut shell) = self.shell.take() else {
            return;
        };
        shell.capture_typing(ui.ctx());
        self.intercept_close(ui.ctx(), frame);
        // A .skp file dropped on the window opens it (after asking to save changes).
        let dropped = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .find(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("skp")))
        });
        if let Some(path) = dropped {
            self.request(ui.ctx(), frame, confirm::Pending::Open(Some(path)));
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
                // The Line tool's inference mode (Alt cycles it after the first click).
                (Some(d), Tool::Draw("tool.line")) if d.anchor().is_some() => format!(
                    "{} Alt = cycle inferences: {}.",
                    d.status(),
                    self.engine.filter.label()
                ),
                (Some(d), Tool::Draw(_)) => d.status(),
                (_, Tool::Modify(_)) if self.modify.is_some() => {
                    self.modify.as_ref().map(|m| m.status()).unwrap_or_default()
                }
                (_, Tool::Pick(_)) => self.pick_status(),
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
            _ => self.camera_tool_measurement(units),
        };
        let shadows_on = self
            .document
            .as_ref()
            .is_some_and(|d| d.model.shadows.enabled);
        let style = self.document.as_ref().map(|d| d.model.style.clone());
        let can_undo = self.document.as_ref().is_some_and(Document::can_undo);
        let can_redo = self.document.as_ref().is_some_and(Document::can_redo);
        let has_selection = !self.selection.items().is_empty();
        let can_paste = self.clipboard.is_some();
        let has_file = self.document.as_ref().is_some_and(|d| d.has_file);
        let can_go_back = self.camera_history.can_go_back();
        let can_go_forward = self.camera_history.can_go_forward();
        let can_edit_photo = self.can_edit_matched_photo();
        let camera = self.camera.clone();
        let scene_tabs = self.scene_tabs;
        let playing = self.animation.is_playing();
        let tray_on = shell.tray_visible();
        let spec = self.spec.clone();
        let small_on: Vec<(&str, bool)> = small_items::COMMANDS
            .iter()
            .map(|id| (*id, self.small_enabled(id).unwrap_or(true)))
            .collect();
        let ext_ids = self.extensions.command_ids();
        let ui_frame = mt_ui::Frame {
            spec: &spec,
            active_tool: self.tool.command(),
            status: &status,
            measurements_label: label,
            measurements_value: &value,
            is_enabled: &|id| match id {
                "edit.undo" => can_undo,
                "edit.redo" => can_redo,
                "edit.cut" | "edit.copy" => has_selection,
                "edit.paste" | "edit.paste_in_place" => can_paste,
                "camera.previous" => can_go_back,
                "camera.next" => can_go_forward,
                "camera.edit_matched_photo" => can_edit_photo,
                "file.revert" => has_file,
                _ => small_on.iter().find(|(c, _)| *c == id).map_or_else(
                    || IMPLEMENTED.contains(&id) || ext_ids.iter().any(|x| x == id),
                    |x| x.1,
                ),
            },
            is_checked: &|id| {
                camera_cmd::is_checked(&camera, id)
                    || (id == "view.shadows" && shadows_on)
                    || (id == "view.scene_tabs" && scene_tabs)
                    || (id == "view.anim_play" && playing)
                    || (id == "window.tray_show" && tray_on)
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
        self.frame_hook(ui.ctx(), frame);
        self.context_menu_command(ui.ctx(), frame);
        self.poll_loading(ui.ctx(), frame);
        self.splash_ui(ui.ctx());
        self.confirm_ui(ui.ctx(), frame);
        self.error_ui(ui.ctx());
        self.make_component_ui(ui.ctx(), frame);
        self.export_ui(ui.ctx(), frame);
        self.import_ui(ui.ctx(), frame);
        self.text3d_ui(ui.ctx(), frame);
        self.animation_ui(ui.ctx(), frame);
        self.model_info_ui(ui.ctx(), frame);
        self.preferences_ui(ui.ctx(), frame);
        self.presentation_ui(ui.ctx(), frame);
        self.context_extra_ui(ui.ctx(), frame);
        self.tool_extra_ui(ui.ctx(), frame);
        self.search_ui(ui.ctx(), frame);
        self.library_ui(ui.ctx(), frame);
        self.match_photo_ui(ui.ctx(), frame);
        self.extensions_ui(ui.ctx(), frame);
        for event in events {
            match event {
                mt_ui::Event::Command(id) => self.run_command(ui.ctx(), frame, &id),
                mt_ui::Event::ToolbarLayoutChanged => {
                    if let Some(shell) = &self.shell {
                        settings::save_toolbar_layout(shell.toolbar_layout());
                    }
                }
                mt_ui::Event::TrayLayoutChanged => {
                    if let Some(shell) = &self.shell {
                        settings::save_tray_layout(shell.tray_layout());
                    }
                }
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
                    } else {
                        self.camera_tool_typed(&text, units);
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
        self.apply_model_edit(frame);
        if self.poll_external_texture(ui.ctx())
            && let Some(rs) = frame.wgpu_render_state()
        {
            let device = rs.device.clone();
            self.rebuild(&device);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_toggles_flip_and_check() {
        let mut s = mt_kernel::StyleSettings::default();
        for id in [
            "view.axes",
            "view.guides",
            "view.hidden_geometry",
            "view.hidden_objects",
        ] {
            let before = view_checked(&s, id);
            assert!(view_toggle(&mut s, id));
            assert_eq!(view_checked(&s, id), !before, "{id}");
            assert!(IMPLEMENTED.contains(&id));
        }
        assert!(!s.axes && !s.guides && s.hidden_geometry && s.hidden_objects);
        assert!(
            !view_toggle(&mut s, "view.scene_tabs"),
            "app state, not style"
        );
    }
}

#[cfg(test)]
mod edge_style_tests {
    use super::{IMPLEMENTED, view_checked, view_toggle};

    #[test]
    fn edge_style_and_fog_toggle_and_check() {
        let mut s = mt_kernel::StyleSettings::default();
        let ids = [
            "view.edges",
            "view.back_edges",
            "view.profiles",
            "view.depth_cue",
            "view.extension",
            "view.fog",
        ];
        // Edges and Profiles on by default, the rest off.
        let defaults: Vec<bool> = ids.iter().map(|id| view_checked(&s, id)).collect();
        assert_eq!(defaults, [true, false, true, false, false, false]);
        for (id, was) in ids.iter().zip(defaults) {
            assert!(IMPLEMENTED.contains(id), "{id} enabled");
            assert!(view_toggle(&mut s, id), "{id} is a toggle");
            assert_eq!(view_checked(&s, id), !was, "{id} flipped");
            assert!(view_toggle(&mut s, id));
            assert_eq!(view_checked(&s, id), was, "{id} back");
        }
        // Each toggle changes only its own flag.
        view_toggle(&mut s, "view.back_edges");
        let after: Vec<bool> = ids.iter().map(|id| view_checked(&s, id)).collect();
        assert_eq!(after, [true, true, true, false, false, false]);
        assert!(IMPLEMENTED.contains(&"window.fog"));
    }
}
