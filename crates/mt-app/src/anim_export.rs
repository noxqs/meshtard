//! File > Export > Animation: the scene animation rendered frame by frame to a
//! numbered image sequence (`name0001.png`, `name0002.png`, ... next to the chosen
//! file name; JPEG when the name ends in .jpg).
//!
//! The timeline is what View > Animation > Play shows (`animation`): starting at the
//! first scene, each scene is held for the scene delay, then the camera eases to the
//! next one over the transition time (a jump with transitions off), with the tag
//! visibility of the scene it moves to. "Loop to starting scene" adds the move from
//! the last scene back to the first. Options after SketchUp's Animation Export Options
//! (from memory, unverified): resolution (width and height), frame rate, anti-alias and
//! loop to starting scene (off). Frames render one per UI frame with a progress bar and
//! a Cancel button; the camera and tag visibility come back afterwards.
//!
//! Video (MP4) is not written: there is no small pure-Rust H.264/MP4 encoder to embed,
//! so join the frames with a tool such as ffmpeg (`ffmpeg -framerate 24 -i
//! name%04d.png out.mp4`).

use eframe::egui;

use crate::App;

/// Export Animation options.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AnimOptions {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub antialias: bool,
    pub loop_to_start: bool,
}

impl Default for AnimOptions {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            fps: 24,
            antialias: true,
            loop_to_start: false,
        }
    }
}

/// One frame of the timeline: the camera and the scene whose tag visibility applies.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Frame {
    pub camera: mt_render::Camera,
    pub scene: usize,
}

/// The frames of the scene animation over `cameras` (one per scene) at `fps`.
pub(crate) fn timeline(
    cameras: &[mt_render::Camera],
    transition_secs: f64,
    delay_secs: f64,
    fps: u32,
    loop_to_start: bool,
) -> Vec<Frame> {
    let fps = f64::from(fps.max(1));
    let hold = (delay_secs.max(0.0) * fps).round() as usize;
    let moving = (transition_secs.max(0.0) * fps).round() as usize;
    let n = cameras.len();
    let mut out = Vec::new();
    for (i, cam) in cameras.iter().enumerate() {
        // The first scene shows at least once; later ones arrive with the transition.
        let frames = if i == 0 { hold.max(1) } else { hold };
        out.extend((0..frames).map(|_| Frame {
            camera: cam.clone(),
            scene: i,
        }));
        let next = if i + 1 < n {
            i + 1
        } else if loop_to_start && n > 1 {
            0
        } else {
            break;
        };
        let to = &cameras[next];
        if moving == 0 {
            out.push(Frame {
                camera: to.clone(),
                scene: next,
            });
        } else {
            out.extend((1..=moving).map(|k| Frame {
                camera: cam.interpolate(to, mt_render::ease_in_out(k as f64 / moving as f64)),
                scene: next,
            }));
        }
    }
    out
}

/// The file of frame `index` (0-based) for the chosen `path`: the stem with a
/// four-digit frame number from 1.
pub(crate) fn frame_path(path: &std::path::Path, index: usize) -> std::path::PathBuf {
    let stem = path
        .file_stem()
        .map_or_else(|| "frame".into(), |s| s.to_string_lossy().into_owned());
    let ext = path
        .extension()
        .map_or_else(|| "png".into(), |e| e.to_string_lossy().into_owned());
    path.with_file_name(format!("{stem}{:04}.{ext}", index + 1))
}

/// An export in progress.
pub(crate) struct AnimJob {
    path: std::path::PathBuf,
    opts: AnimOptions,
    frames: Vec<Frame>,
    next: usize,
    /// Tag visibility before the export, restored at the end.
    layers: Vec<bool>,
    camera: mt_render::Camera,
    /// The scene whose tag visibility is applied now.
    applied: Option<usize>,
    error: Option<String>,
}

impl App {
    pub(crate) fn open_export_animation(&mut self) {
        let scenes = self.document.as_ref().map_or(0, |d| d.model.pages.len());
        if scenes == 0 {
            self.message = Some("Add scenes to export an animation.".into());
            return;
        }
        self.present.anim = Some(self.present.last_anim.clone());
    }

    pub(crate) fn export_animation_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        self.animation_options_ui(ctx);
        self.animation_job_ui(ctx, frame);
    }

    fn animation_options_ui(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.present.anim.take() else {
            return;
        };
        let secs = self.animation.transition_secs();
        let delay = self.animation.delay_secs;
        let cameras = self.scene_cameras();
        let count = timeline(&cameras, secs, delay, d.fps, d.loop_to_start).len();
        let mut done = None;
        egui::Modal::new(egui::Id::new("mt_export_animation")).show(ctx, |ui| {
            ui.set_width(320.0);
            crate::export::dialog_visuals(ui);
            ui.strong("Export Animation Options");
            ui.add_space(6.0);
            egui::Grid::new("mt_anim_grid")
                .num_columns(3)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Width:");
                    ui.add(egui::DragValue::new(&mut d.width).range(16..=8192));
                    ui.label("pixels");
                    ui.end_row();
                    ui.label("Height:");
                    ui.add(egui::DragValue::new(&mut d.height).range(16..=8192));
                    ui.label("pixels");
                    ui.end_row();
                    ui.label("Frame Rate:");
                    ui.add(egui::DragValue::new(&mut d.fps).range(1..=120));
                    ui.label("frames/s");
                    ui.end_row();
                });
            ui.checkbox(&mut d.loop_to_start, "Loop to starting scene");
            ui.checkbox(&mut d.antialias, "Anti-alias rendering");
            ui.add_space(4.0);
            ui.weak(format!(
                "{count} frames ({:.1} s): transitions {secs:.1} s, delay {delay:.1} s \
                 (View > Animation > Settings). Saved as numbered PNG or JPEG images.",
                count as f64 / f64::from(d.fps.max(1))
            ));
            done = crate::presentation::ok_cancel(ui, "Export...");
        });
        match done {
            None => self.present.anim = Some(d),
            Some(false) => self.present.last_anim = d,
            Some(true) => {
                self.present.last_anim = d.clone();
                let name = self.export_file_name("png");
                let Some(path) = crate::filedialog::save(
                    &[
                        ("PNG Image Sequence", &["png"]),
                        ("JPEG Image Sequence", &["jpg", "jpeg"]),
                    ],
                    &name,
                ) else {
                    return;
                };
                let frames = timeline(&cameras, secs, delay, d.fps, d.loop_to_start);
                let layers = self.document.as_ref().map_or_else(Vec::new, |doc| {
                    doc.model.layers.iter().map(|l| l.visible).collect()
                });
                self.stop_playing();
                self.present.anim_job = Some(AnimJob {
                    path,
                    opts: d,
                    frames,
                    next: 0,
                    layers,
                    camera: self.camera.clone(),
                    applied: None,
                    error: None,
                });
            }
        }
    }

    /// Each scene's camera.
    fn scene_cameras(&self) -> Vec<mt_render::Camera> {
        self.document.as_ref().map_or_else(Vec::new, |doc| {
            doc.model
                .pages
                .iter()
                .map(|p| mt_render::Camera {
                    eye: p.eye,
                    target: p.target,
                    fov_deg: p.fov_deg,
                    perspective: p.perspective,
                    two_point: false,
                })
                .collect()
        })
    }

    /// Stops View > Animation > Play and any scene transition.
    fn stop_playing(&mut self) {
        self.animation.stop();
        self.transition = None;
    }

    /// Sets tag visibility: scene `scene`'s, or the saved `layers` with `None`.
    fn apply_scene_tags(&mut self, frame: &eframe::Frame, scene: Option<usize>, saved: &[bool]) {
        let Some(doc) = &mut self.document else {
            return;
        };
        let hidden = scene
            .and_then(|i| doc.model.pages.get(i))
            .map(|p| p.hidden_layers.clone());
        let mut changed = false;
        for (k, l) in doc.model.layers.iter_mut().enumerate() {
            let v = match &hidden {
                Some(h) => !h.contains(&k),
                None => saved.get(k).copied().unwrap_or(l.visible),
            };
            changed |= l.visible != v;
            l.visible = v;
        }
        if changed && let Some(rs) = frame.wgpu_render_state() {
            let device = rs.device.clone();
            self.rebuild(&device);
        }
    }

    fn animation_job_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(mut job) = self.present.anim_job.take() else {
            return;
        };
        let total = job.frames.len();
        let mut cancel = false;
        // Render frames for up to ~1/10 s per UI frame.
        let start = std::time::Instant::now();
        while job.error.is_none() && job.next < total && start.elapsed().as_millis() < 100 {
            let f = job.frames[job.next].clone();
            if job.applied != Some(f.scene) {
                let saved = job.layers.clone();
                self.apply_scene_tags(frame, Some(f.scene), &saved);
                job.applied = Some(f.scene);
            }
            let size = [job.opts.width.max(1), job.opts.height.max(1)];
            let factor = if job.opts.antialias && size.iter().all(|&s| s * 2 <= 8192) {
                2
            } else {
                1
            };
            let ppp = ctx.pixels_per_point() * factor as f32;
            let Some(img) = self.render_view(frame, &f.camera, size.map(|s| s * factor), ppp)
            else {
                job.error = Some("no GPU renderer".into());
                break;
            };
            let img = if factor > 1 {
                image::imageops::resize(
                    &img,
                    size[0],
                    size[1],
                    image::imageops::FilterType::Triangle,
                )
            } else {
                img
            };
            let path = frame_path(&job.path, job.next);
            if let Err(e) = crate::export::encode_image(&img, &path)
                .and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string()))
            {
                job.error = Some(format!("{}: {e}", path.display()));
                break;
            }
            job.next += 1;
        }
        if job.next < total && job.error.is_none() {
            egui::Modal::new(egui::Id::new("mt_anim_progress")).show(ctx, |ui| {
                ui.set_width(320.0);
                crate::export::dialog_visuals(ui);
                ui.strong("Exporting Animation");
                ui.add_space(6.0);
                ui.label(format!("Frame {} of {total}", job.next + 1));
                ui.add(
                    egui::ProgressBar::new(job.next as f32 / total.max(1) as f32)
                        .show_percentage()
                        .desired_width(300.0),
                );
                ui.add_space(6.0);
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
            if !cancel {
                ctx.request_repaint();
                self.present.anim_job = Some(job);
                return;
            }
        }
        // Done, failed or cancelled: put the view back.
        let saved = job.layers.clone();
        self.apply_scene_tags(frame, None, &saved);
        self.camera = job.camera.clone();
        let first = frame_path(&job.path, 0);
        self.message = Some(match (&job.error, cancel) {
            (Some(e), _) => format!("Could not export the animation: {e}"),
            (None, true) => format!(
                "Animation export cancelled after {} of {total} frames.",
                job.next
            ),
            (None, false) => format!(
                "Exported {total} frames ({}x{}, {} fps) from {}.",
                job.opts.width,
                job.opts.height,
                job.opts.fps,
                first.display()
            ),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    fn cam(x: f64) -> mt_render::Camera {
        mt_render::Camera {
            eye: DVec3::new(x, -100.0, 50.0),
            target: DVec3::new(x, 0.0, 0.0),
            fov_deg: 35.0,
            perspective: true,
            two_point: false,
        }
    }

    #[test]
    fn timeline_holds_and_moves() {
        let cams = [cam(0.0), cam(10.0), cam(20.0)];
        // 1 s delay and 2 s transitions at 10 fps: 10 + 20 + 10 + 20 + 10 frames.
        let f = timeline(&cams, 2.0, 1.0, 10, false);
        assert_eq!(f.len(), 70);
        assert_eq!(f[0].camera, cams[0]);
        assert_eq!(f[9].camera, cams[0]);
        assert_eq!(f[29].camera, cams[1], "transition ends at the scene");
        assert_eq!(f[10].scene, 1, "tags of the scene moved to");
        assert!(f[15].camera.eye.x > 0.0 && f[15].camera.eye.x < 10.0);
        assert_eq!(f.last().unwrap().camera, cams[2]);
        // Looping adds the way back.
        assert_eq!(timeline(&cams, 2.0, 1.0, 10, true).len(), 90);
        // Transitions off: one jump frame per scene.
        let jumps = timeline(&cams, 0.0, 1.0, 10, false);
        assert_eq!(jumps.len(), 10 + 1 + 10 + 1 + 10);
        // No delay: the first scene still shows.
        assert_eq!(timeline(&cams[..1], 2.0, 0.0, 10, false).len(), 1);
    }

    #[test]
    fn frames_are_numbered_from_one() {
        let p = std::path::Path::new("/tmp/out/walk.png");
        assert_eq!(
            frame_path(p, 0),
            std::path::Path::new("/tmp/out/walk0001.png")
        );
        assert_eq!(
            frame_path(p, 41),
            std::path::Path::new("/tmp/out/walk0042.png")
        );
    }
}
