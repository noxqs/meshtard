//! meshtard desktop application.

mod document;
mod nav;

use eframe::egui;
use egui_wgpu::wgpu;
use glam::DVec2;
use mt_render::{Camera, Style, Viewport};
use mt_spec::{MenuItem, Shortcut, Spec};

use document::Document;
use nav::Tool;

/// Touchpad scroll distance, in points, that counts as one wheel notch.
const POINTS_PER_NOTCH: f32 = 40.0;

/// Commands that do something today. Every other menu item is shown disabled.
const IMPLEMENTED: &[&str] = &[
    "file.open",
    "file.exit",
    "tool.select",
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
    document: Option<Document>,
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
        Self {
            camera: Camera::from_spec(&spec.default_model.camera),
            tool: Tool::Select,
            viewport: Viewport::new(&render_state.device, Style::from_spec(&spec.default_model)),
            texture: None,
            shortcuts,
            spec,
            document: None,
            message: None,
        }
    }

    fn open(&mut self, ctx: &egui::Context, device: &wgpu::Device, path: &std::path::Path) {
        match Document::open(path) {
            Ok(doc) => {
                let scene = doc.scene();
                self.viewport.set_edges(device, &scene.edges);
                self.viewport
                    .set_faces(device, &scene.vertices, &scene.indices);
                if let Some(camera) = doc
                    .saved_camera()
                    .or_else(|| doc.extents_camera(self.camera.fov_deg))
                {
                    self.camera = camera;
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(doc.title()));
                self.document = Some(doc);
                self.message = None;
            }
            Err(e) => self.message = Some(format!("Could not open {}: {e}", path.display())),
        }
    }

    fn run_command(&mut self, ctx: &egui::Context, frame: &eframe::Frame, id: &str) {
        if let Some(tool) = Tool::from_command(id) {
            self.tool = tool;
        } else if id == "file.exit" {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if id == "file.open" {
            let picked = rfd::FileDialog::new()
                .add_filter("SketchUp Models", &["skp"])
                .pick_file();
            if let (Some(path), Some(rs)) = (picked, frame.wgpu_render_state()) {
                self.open(ctx, &rs.device, &path);
            }
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let pressed: Vec<&'static str> = ctx.input(|i| {
            self.shortcuts
                .iter()
                .filter(|(s, key, _)| {
                    let m = i.modifiers;
                    i.key_pressed(*key) && m.ctrl == s.ctrl && m.shift == s.shift && m.alt == s.alt
                })
                .map(|(_, _, id)| *id)
                .collect()
        });
        for id in pressed {
            self.run_command(ctx, frame, id);
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        let mut clicked = None;
        egui::MenuBar::new().ui(ui, |ui| {
            for menu in &self.spec.menus.menu {
                ui.menu_button(&menu.label, |ui| {
                    menu_items(ui, &menu.items, &self.spec, &mut clicked);
                });
            }
        });
        if let Some(id) = clicked {
            self.run_command(ui.ctx(), frame, &id);
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

fn menu_items(ui: &mut egui::Ui, items: &[MenuItem], spec: &Spec, clicked: &mut Option<String>) {
    for item in items {
        match item {
            MenuItem::Separator { .. } => {
                ui.separator();
            }
            MenuItem::Submenu { label, items } => {
                ui.menu_button(label, |ui| menu_items(ui, items, spec, clicked));
            }
            MenuItem::Command { id, label } => {
                let mut button = egui::Button::new(label);
                if let Some(shortcut) = spec.keymap.shortcut(id) {
                    button = button.shortcut_text(shortcut.to_string());
                }
                let enabled = IMPLEMENTED.contains(&id.as_str());
                if ui.add_enabled(enabled, button).clicked() {
                    *clicked = Some(id.clone());
                    ui.close();
                }
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.handle_shortcuts(ui.ctx(), frame);
        egui::Panel::top("menu_bar").show(ui, |ui| self.menu_bar(ui, frame));
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            match &self.message {
                Some(m) => ui.colored_label(egui::Color32::DARK_RED, m),
                None => ui.label(self.tool.hint()),
            };
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.viewport_ui(ui, frame));
    }
}
