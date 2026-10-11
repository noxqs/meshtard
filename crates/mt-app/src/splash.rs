//! The splash screen shown while the app starts: the image at its own size (smaller
//! if the window is), centered in the window, with the start-up status near its
//! bottom edge, until start-up is done (a model passed on the
//! command line opens on a background thread meanwhile), shown for at least
//! `MIN_SECS`, then fades out. SketchUp shows its splash in a separate borderless
//! window before the main window; here it is drawn inside the main window.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::App;
use crate::document::Document;

pub(crate) const IMAGE: &[u8] = include_bytes!("../assets/splash.png");
/// Shortest time the splash stays up (unverified; long enough to be seen).
const MIN_SECS: f32 = 1.2;
const FADE_SECS: f32 = 0.35;

pub struct Splash {
    started: Instant,
    texture: Option<egui::TextureHandle>,
    /// When start-up finished and the fade began.
    fading_since: Option<Instant>,
    done: bool,
}

/// A model opening on a background thread.
pub struct Loading {
    pub path: PathBuf,
    rx: Receiver<Result<Document, String>>,
}

impl Loading {
    pub fn start(path: PathBuf) -> Self {
        let (tx, rx) = channel();
        let p = path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Document::open(&p).map_err(|e| e.to_string()));
        });
        Self { path, rx }
    }
}

impl Splash {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            texture: None,
            fading_since: None,
            done: false,
        }
    }

    fn texture(&mut self, ctx: &egui::Context) -> Option<&egui::TextureHandle> {
        if self.texture.is_none() {
            let img = image::load_from_memory_with_format(IMAGE, image::ImageFormat::Png)
                .ok()?
                .to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            self.texture = Some(ctx.load_texture("mt_splash", color, egui::TextureOptions::LINEAR));
        }
        self.texture.as_ref()
    }
}

impl App {
    /// Picks up a model finished loading in the background.
    pub(crate) fn poll_loading(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(loading) = &self.loading else { return };
        let Ok(result) = loading.rx.try_recv() else {
            ctx.request_repaint_after(Duration::from_millis(50));
            return;
        };
        let path = self.loading.take().map(|l| l.path).unwrap_or_default();
        let Some(rs) = frame.wgpu_render_state() else {
            return;
        };
        let device = rs.device.clone();
        match result {
            Ok(doc) => self.install(ctx, &device, &path, doc),
            Err(e) => {
                log::warn!("could not open {}: {e}", path.display());
                self.message = Some(format!("Could not open {}: {e}", path.display()));
            }
        }
    }

    /// Draws the splash over the window while it is up.
    pub(crate) fn splash_ui(&mut self, ctx: &egui::Context) {
        if self.splash.done {
            return;
        }
        let elapsed = self.splash.started.elapsed().as_secs_f32();
        if self.loading.is_none() && elapsed >= MIN_SECS && self.splash.fading_since.is_none() {
            self.splash.fading_since = Some(Instant::now());
        }
        let alpha = match self.splash.fading_since {
            Some(t) => 1.0 - t.elapsed().as_secs_f32() / FADE_SECS,
            None => 1.0,
        };
        if alpha <= 0.0 {
            self.splash.done = true;
            self.splash.texture = None;
            return;
        }
        let screen = ctx.content_rect();
        let Some((id, size)) = self.splash.texture(ctx).map(|t| (t.id(), t.size_vec2())) else {
            self.splash.done = true;
            return;
        };
        // The image's own pixel size, shrunk only to fit the window.
        let size = size / ctx.pixels_per_point();
        let scale = (screen.width() / size.x)
            .min(screen.height() / size.y)
            .min(1.0);
        let img = egui::Rect::from_center_size(screen.center(), size * scale);
        let status = match &self.loading {
            Some(l) => format!(
                "Opening {}...",
                l.path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
            ),
            None => "Starting...".to_owned(),
        };
        let a = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
        egui::Area::new(egui::Id::new("mt_splash"))
            .order(egui::Order::Foreground)
            .fixed_pos(img.min)
            .interactable(true)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(img.size(), egui::Sense::click_and_drag());
                // Soft shadow under the splash, like the dialogs.
                let shadow = egui::Shadow {
                    offset: [0, 6],
                    blur: 24,
                    spread: 0,
                    color: egui::Color32::from_black_alpha((f32::from(a) * 70.0 / 255.0) as u8),
                };
                ui.painter().add(shadow.as_shape(rect, 0));
                ui.painter().image(
                    id,
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::from_white_alpha(a),
                );
                ui.painter().text(
                    egui::pos2(rect.center().x, rect.bottom() - 16.0),
                    egui::Align2::CENTER_BOTTOM,
                    status,
                    egui::FontId::proportional(13.0),
                    egui::Color32::from_rgba_unmultiplied(40, 40, 40, a),
                );
                let border = egui::Color32::from_gray(110);
                ui.painter().rect_stroke(
                    rect,
                    0.0,
                    egui::Stroke::new(1.0, border.gamma_multiply(f32::from(a) / 255.0)),
                    egui::StrokeKind::Inside,
                );
            });
        ctx.request_repaint();
    }
}
