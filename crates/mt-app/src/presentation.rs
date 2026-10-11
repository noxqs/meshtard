//! The M13 file and help commands (FEATURES.md Tier 3): Export Section Slice
//! (`section_export`), Export Animation (`anim_export`), Print Setup / Preview / Print
//! (`print`), templates (`templates`), Generate Report (`report`) and About and the
//! help links (`about`). This module holds their dialog state and routes commands and
//! per-frame UI to them.

use eframe::egui;

use crate::App;

/// Dialog and job state of the M13 commands.
#[derive(Default)]
pub(crate) struct Presentation {
    pub section: Option<crate::section_export::SectionOptions>,
    pub last_section: crate::section_export::SectionOptions,
    pub anim: Option<crate::anim_export::AnimOptions>,
    pub last_anim: crate::anim_export::AnimOptions,
    pub anim_job: Option<crate::anim_export::AnimJob>,
    pub print_setup: crate::print::PrintSetup,
    pub print_setup_dialog: Option<crate::print::PrintSetup>,
    pub print_preview: Option<crate::print::Preview>,
    pub save_template: Option<crate::templates::SaveTemplate>,
    pub new_template: Option<crate::templates::NewFromTemplate>,
    pub report: Option<crate::report::ReportDialog>,
    pub about: Option<crate::about::About>,
}

/// The `ok` and Cancel buttons at the bottom of a dialog; Enter presses `ok`.
/// `Some(true)` for `ok`, `Some(false)` for Cancel.
pub(crate) fn ok_cancel(ui: &mut egui::Ui, ok: &str) -> Option<bool> {
    mt_ui::dialog::buttons(ui, &[ok, "Cancel"], Some(0)).map(|i| i == 0)
}

/// A dialog's answer from [`ok_cancel`]: its close button and Escape count as Cancel.
pub(crate) fn answer(r: mt_ui::DialogResponse<Option<bool>>) -> Option<bool> {
    if r.closed { Some(false) } else { r.inner }
}

/// Opens `target` (a file or URL) with the system's default application: `open` on
/// macOS, `cmd /C start` on Windows, `xdg-open` elsewhere. Does not wait for it.
pub(crate) fn open_with_system(target: &str) -> std::io::Result<()> {
    let mut cmd = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    cmd.arg(target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

/// The project page the Help menu's Welcome, Knowledge Center and Contact Us open.
pub(crate) const PROJECT_URL: &str = "https://github.com/noxqs/meshtard";

impl App {
    /// Runs an M13 command; false if `id` is not one.
    pub(crate) fn presentation_command(
        &mut self,
        ctx: &egui::Context,
        frame: &eframe::Frame,
        id: &str,
    ) -> bool {
        match id {
            "file.export_section" => self.open_export_section(),
            "file.export_animation" => self.open_export_animation(),
            "file.print_setup" => {
                self.present.print_setup_dialog = Some(self.present.print_setup.clone());
            }
            "file.print_preview" => self.open_print_preview(ctx, frame),
            "file.print" => self.print(ctx, frame),
            "file.save_as_template" => self.open_save_template(),
            "file.new_from_template" => self.open_new_from_template(),
            "file.generate_report" => self.open_report(),
            "help.about" => self.present.about = Some(Default::default()),
            // The shell opens the dialog for its own menu bar; the macOS menu bar's
            // commands come here only.
            "window.manage_trays" => {
                if cfg!(target_os = "macos")
                    && let Some(shell) = &mut self.shell
                {
                    shell.open_trays_dialog();
                }
            }
            "help.welcome" => self.open_welcome(),
            "help.knowledge_center" | "help.contact" => {
                if let Err(e) = open_with_system(PROJECT_URL) {
                    self.message = Some(format!("Could not open {PROJECT_URL}: {e}"));
                }
            }
            _ => return false,
        }
        true
    }

    /// Per frame: the M13 dialogs and the animation export job.
    pub(crate) fn presentation_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        self.export_section_ui(ctx);
        self.export_animation_ui(ctx, frame);
        self.print_ui(ctx, frame);
        self.templates_ui(ctx, frame);
        self.report_ui(ctx);
        self.about_ui(ctx);
    }

    /// The model's file stem with `ext` (`Untitled` before the first save).
    pub(crate) fn export_file_name(&self, ext: &str) -> String {
        format!("{}.{ext}", self.model_stem())
    }

    pub(crate) fn model_stem(&self) -> String {
        self.document
            .as_ref()
            .and_then(|d| d.path.file_stem())
            .map_or_else(|| "Untitled".into(), |s| s.to_string_lossy().into_owned())
    }

    /// Renders `camera`'s view offscreen at `size` pixels without selection
    /// highlights (as Export 2D Graphic does). `ppp` scales line widths.
    pub(crate) fn render_view(
        &mut self,
        frame: &eframe::Frame,
        camera: &mt_render::Camera,
        size: [u32; 2],
        ppp: f32,
    ) -> Option<image::RgbaImage> {
        let rs = frame.wgpu_render_state()?;
        let (device, queue) = (rs.device.clone(), rs.queue.clone());
        let selection = std::mem::take(&mut self.selection);
        self.update_highlight(&device);
        let img = self
            .viewport
            .render_image(&device, &queue, camera, size, ppp, false);
        self.selection = selection;
        self.update_highlight(&device);
        Some(img)
    }
}
