//! Help > About: the splash image, version, license, credits and build information.
//! (Help > Welcome, Knowledge Center and Contact Us open the project page; see
//! `presentation`.)

use eframe::egui;

use crate::App;

/// The About window's splash texture, loaded when first shown.
#[derive(Default)]
pub(crate) struct About {
    texture: Option<egui::TextureHandle>,
}

/// Build information lines.
pub(crate) fn build_info() -> Vec<(&'static str, String)> {
    vec![
        ("Version", env!("CARGO_PKG_VERSION").to_owned()),
        ("License", env!("CARGO_PKG_LICENSE").to_owned()),
        ("Target", env!("MT_BUILD_TARGET").to_owned()),
        ("Profile", env!("MT_BUILD_PROFILE").to_owned()),
        (
            "CUDA pick index",
            if cfg!(feature = "cuda") {
                "built in"
            } else {
                "not built"
            }
            .to_owned(),
        ),
    ]
}

impl App {
    pub(crate) fn about_ui(&mut self, ctx: &egui::Context) {
        let Some(about) = &mut self.present.about else {
            return;
        };
        if about.texture.is_none()
            && let Ok(img) =
                image::load_from_memory_with_format(crate::splash::IMAGE, image::ImageFormat::Png)
        {
            let img = img.to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            about.texture = Some(ctx.load_texture("mt_about", color, egui::TextureOptions::LINEAR));
        }
        let texture = about.texture.clone();
        let mut close = false;
        egui::Modal::new(egui::Id::new("mt_about")).show(ctx, |ui| {
            ui.set_width(420.0);
            crate::export::dialog_visuals(ui);
            if let Some(t) = &texture {
                let s = t.size_vec2();
                let k = (420.0 / s.x).min(1.0);
                ui.image((t.id(), s * k));
            }
            ui.add_space(6.0);
            ui.heading(format!("meshtard {}", env!("CARGO_PKG_VERSION")));
            ui.label("A 3D modeler for .skp files, written in Rust.");
            ui.add_space(6.0);
            egui::Grid::new("mt_about_grid")
                .num_columns(2)
                .spacing([10.0, 3.0])
                .show(ui, |ui| {
                    for (k, v) in build_info() {
                        ui.label(k);
                        ui.label(v);
                        ui.end_row();
                    }
                });
            ui.add_space(6.0);
            ui.label("meshtard is free software under the MIT License.");
            ui.label(
                "The .skp reader is a Rust port of OpenSKP \
                 (MIT, Copyright (c) 2024-2026 Ahsan Mehmood).",
            );
            ui.label(
                "SketchUp is a trademark of Trimble Inc.; meshtard is not affiliated with Trimble.",
            );
            ui.hyperlink(crate::presentation::PROJECT_URL);
            ui.add_space(10.0);
            if ui.button("Close").clicked()
                || ui.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Enter))
            {
                close = true;
            }
        });
        if close {
            self.present.about = None;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn build_info_names_version_and_license() {
        let info = super::build_info();
        assert_eq!(info[0].1, env!("CARGO_PKG_VERSION"));
        assert_eq!(info[1].1, "MIT");
        assert!(!info[2].1.is_empty());
    }
}
