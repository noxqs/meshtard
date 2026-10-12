//! FEATURES.md 67 and leftovers of 41: Edit > Purge Unused, the image entity's
//! Export / Use As Material, Make Unique Texture, Undo / Redo named after the
//! operation, the status bar's geo-location pin and Model Info button, Model Info >
//! Rendering's anti-aliased textures, and the busy cursor during long commands.
//!
//! [`App::frame_hook`] runs once per frame after the window is drawn;
//! [`App::early_command`] runs first in `run_command`.

use std::collections::BTreeMap;

use mt_kernel::{FaceId, Material, Model};

use crate::App;
use crate::document::Owner;

/// Commands that block the window long enough for a busy cursor: the window draws
/// one frame with the wait cursor before running them (the app runs them on the UI
/// thread). Solid Tools' clicks are not covered.
pub(crate) const LONG_COMMANDS: &[&str] = &[
    "file.open",
    "file.revert",
    "file.save",
    "file.save_as",
    "file.save_copy_as",
    "file.import",
    "file.export_3d",
    "file.export_2d",
    "file.export_section",
    "file.export_animation",
    "edit.intersect_model",
    "edit.intersect_selection",
    "edit.intersect_context",
    "edit.purge_unused",
];

/// The commands this module adds (enabled in menus).
pub(crate) const COMMANDS: &[&str] = &[
    "edit.purge_unused",
    "edit.image_export",
    "edit.image_use_as_material",
    "edit.make_unique_texture",
    "view.hidden_objects",
    "window.model_info_geo",
];

#[derive(Default)]
pub(crate) struct State {
    /// Command id to menu label, for undo step names.
    labels: BTreeMap<String, String>,
    /// A long command waiting one frame for the busy cursor; `true` once that frame
    /// was drawn.
    busy: Option<(String, bool)>,
    /// The deferred command now running (not deferred again).
    running: Option<String>,
}

impl State {
    /// The wait cursor shows (a long command is about to run).
    pub(crate) fn busy(&self) -> bool {
        self.busy.is_some()
    }
}

/// Name of an undo step for command or tool `id`: its menu label without the
/// trailing dots (`"Make Component..."` gives `"Make Component"`).
pub(crate) fn operation_name(labels: &BTreeMap<String, String>, id: &str) -> String {
    labels
        .get(id)
        .map(|l| l.trim_end_matches('.').trim().to_owned())
        .unwrap_or_default()
}

/// `"Undo Move"`, or plain `"Undo"` for an unnamed step.
pub(crate) fn undo_label(verb: &str, name: Option<&str>) -> String {
    match name.filter(|n| !n.is_empty()) {
        Some(n) => format!("{verb} {n}"),
        None => verb.to_owned(),
    }
}

/// A material name not used in `model`: `base`, `base1`, `base2`, ...
pub(crate) fn unique_material_name(model: &Model, base: &str) -> String {
    let used = |n: &str| model.materials.iter().any(|m| m.name == n);
    if !used(base) {
        return base.to_owned();
    }
    (1..)
        .map(|i| format!("{base}{i}"))
        .find(|n| !used(n))
        .expect("a free name")
}

/// Make Unique Texture: each selected face side painted with a textured material
/// gets its own copy of the material. Returns how many copies were made.
pub(crate) fn make_unique_texture(
    model: &mut Model,
    ents: &mut mt_kernel::Entities,
    faces: &[FaceId],
) -> usize {
    let mut made = 0;
    for &f in faces {
        let Some(face) = ents.faces.get(f) else {
            continue;
        };
        let sides = [face.attrs.material, face.attrs.back_material];
        let mut copies: [Option<usize>; 2] = [None, None];
        for (k, m) in sides.iter().enumerate() {
            let Some(m) = *m else { continue };
            // Both sides with the same material share one copy.
            if k == 1 && sides[0] == Some(m) {
                copies[1] = copies[0];
                continue;
            }
            let Some(src) = model.materials.get(m).filter(|x| x.texture.is_some()) else {
                continue;
            };
            let copy = Material {
                name: unique_material_name(model, &src.name),
                ..src.clone()
            };
            model.materials.push(copy);
            copies[k] = Some(model.materials.len() - 1);
            made += 1;
        }
        let face = &mut ents.faces[f];
        if let Some(c) = copies[0] {
            face.attrs.material = Some(c);
        }
        if let Some(c) = copies[1] {
            face.attrs.back_material = Some(c);
        }
    }
    made
}

/// The material of image instance `i` of `ents` (its picture), if it is an image.
pub(crate) fn image_material(model: &Model, ents: &mt_kernel::Entities, i: usize) -> Option<usize> {
    let inst = ents.instances.get(i)?;
    let def = model
        .definitions
        .get(inst.definition)
        .filter(|d| d.is_image)?;
    def.entities.faces.values().find_map(|f| f.attrs.material)
}

/// File extension of image bytes (PNG, JPEG, BMP), `png` when unknown.
pub(crate) fn image_extension(data: &[u8]) -> &'static str {
    match image::guess_format(data) {
        Ok(image::ImageFormat::Jpeg) => "jpg",
        Ok(image::ImageFormat::Bmp) => "bmp",
        _ => "png",
    }
}

impl App {
    /// The selected image instance and its material.
    fn selected_image(&self) -> Option<(usize, usize)> {
        let doc = self.document.as_ref()?;
        self.selection.items().iter().find_map(|o| match *o {
            Owner::Instance(i) => image_material(&doc.model, doc.active(), i).map(|m| (i, m)),
            _ => None,
        })
    }

    fn selected_faces(&self) -> Vec<FaceId> {
        self.selection
            .items()
            .iter()
            .filter_map(|o| match *o {
                Owner::Face(f) => Some(f),
                _ => None,
            })
            .collect()
    }

    /// Whether a command of this module can run now (menus and context menus).
    pub(crate) fn small_enabled(&self, id: &str) -> Option<bool> {
        Some(match id {
            "edit.image_export" | "edit.image_use_as_material" => self.selected_image().is_some(),
            "edit.make_unique_texture" => {
                let Some(doc) = &self.document else {
                    return Some(false);
                };
                let ents = doc.active();
                self.selected_faces().iter().any(|&f| {
                    ents.faces.get(f).is_some_and(|face| {
                        [face.attrs.material, face.attrs.back_material]
                            .into_iter()
                            .flatten()
                            .any(|m| {
                                doc.model
                                    .materials
                                    .get(m)
                                    .is_some_and(|m| m.texture.is_some())
                            })
                    })
                })
            }
            "edit.purge_unused" | "window.model_info_geo" => self.document.is_some(),
            _ => return None,
        })
    }

    /// Runs first in `run_command`: names the undo step after the command, defers
    /// long commands one frame for the busy cursor, and runs this module's commands.
    /// Returns true when the command was handled (or deferred).
    pub(crate) fn early_command(
        &mut self,
        ctx: &egui::Context,
        frame: &eframe::Frame,
        id: &str,
    ) -> bool {
        if self.small.labels.is_empty() {
            self.small.labels = self.spec.commands();
        }
        if let Some(doc) = &mut self.document {
            doc.operation = operation_name(&self.small.labels, id);
        }
        if LONG_COMMANDS.contains(&id) && self.small.running.as_deref() != Some(id) {
            self.small.busy = Some((id.to_owned(), false));
            ctx.request_repaint();
            return true;
        }
        let rebuild = match id {
            "edit.purge_unused" => self.purge_unused(),
            "edit.image_export" => {
                self.image_export();
                false
            }
            "edit.image_use_as_material" => self.image_use_as_material(),
            "edit.make_unique_texture" => {
                let faces = self.selected_faces();
                self.document.as_mut().is_some_and(|doc| {
                    doc.edit_model(|m, ents| make_unique_texture(m, ents, &faces) > 0)
                })
            }
            "window.model_info_geo" => {
                self.open_model_info_at(crate::model_info::Page::GeoLocation);
                false
            }
            _ => return false,
        };
        if rebuild && let Some(rs) = frame.wgpu_render_state() {
            let device = rs.device.clone();
            self.rebuild(&device);
        }
        true
    }

    /// Edit > Purge Unused (SketchUp 2026 release notes): unused components,
    /// materials and tags, as one undo step.
    fn purge_unused(&mut self) -> bool {
        let Some(doc) = &mut self.document else {
            return false;
        };
        let mut purged = mt_kernel::purge::Purged::default();
        let changed = doc.edit_whole(|m| {
            purged = mt_kernel::purge::purge_all(m);
            purged.any()
        });
        self.message = Some(if changed {
            format!(
                "Purged {} component(s), {} material(s) and {} tag(s).",
                purged.definitions, purged.materials, purged.tags
            )
        } else {
            "Nothing to purge.".to_owned()
        });
        if changed {
            self.current_material = None;
            self.current_component = None;
        }
        changed
    }

    /// Image context menu > Export: saves the picture's file.
    fn image_export(&mut self) {
        let Some((_, m)) = self.selected_image() else {
            return;
        };
        let Some(doc) = &self.document else { return };
        let Some(tex) = doc.model.materials.get(m).and_then(|m| m.texture.clone()) else {
            return;
        };
        let ext = image_extension(&tex.data);
        let stem = std::path::Path::new(&tex.filename.replace('\\', "/"))
            .file_stem()
            .map_or_else(|| "image".to_owned(), |s| s.to_string_lossy().into_owned());
        let filters: &[(&str, &[&str])] = &[("Image", &[ext])];
        let Some(path) = crate::filedialog::save(filters, &format!("{stem}.{ext}")) else {
            return;
        };
        let _ = path.parent().map(std::fs::create_dir_all);
        match std::fs::write(&path, &tex.data) {
            Ok(()) => self.message = Some(format!("Exported {}", path.display())),
            Err(e) => self.file_error("export the image to", &path, e),
        }
    }

    /// Image context menu > Use As Material: a new material with the picture (one
    /// tile the image's size), made the Paint Bucket's material.
    fn image_use_as_material(&mut self) -> bool {
        let Some((_, m)) = self.selected_image() else {
            return false;
        };
        let Some(doc) = &mut self.document else {
            return false;
        };
        let mut index = None;
        let changed = doc.edit_whole(|model| {
            let Some(src) = model.materials.get(m).cloned() else {
                return false;
            };
            let name = unique_material_name(model, &src.name);
            model.materials.push(Material { name, ..src });
            index = Some(model.materials.len() - 1);
            true
        });
        if changed {
            self.current_material = index;
        }
        changed
    }

    /// Per-frame updates after the window is drawn: undo names in the Edit menu, the
    /// operation name for the next undo step, the Environments panel, the status
    /// bar's geo pin, texture filtering, and the busy cursor's deferred command.
    pub(crate) fn frame_hook(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if self.small.labels.is_empty() {
            self.small.labels = self.spec.commands();
        }
        let tool_name = operation_name(&self.small.labels, self.tool.command());
        if let Some(doc) = &mut self.document {
            doc.operation = tool_name;
        }
        let (undo, redo, located, smooth) = match &self.document {
            Some(d) => (
                undo_label("Undo", d.undo_name()),
                undo_label("Redo", d.redo_name()),
                d.model.geo.located,
                d.model.rendering.antialiased_textures,
            ),
            None => ("Undo".into(), "Redo".into(), false, true),
        };
        if let Some(rs) = frame.wgpu_render_state() {
            self.viewport.set_smooth_textures(&rs.device, smooth);
        }
        let panel_open = self
            .shell
            .as_ref()
            .is_some_and(|s| s.panel_expanded("window.environments"));
        let env = self.environment_data(ctx, panel_open);
        if let Some(shell) = &mut self.shell {
            shell.set_menu_label("edit.undo", Some(undo));
            shell.set_menu_label("edit.redo", Some(redo));
            shell.set_geo_located(located);
            shell.set_environment_data(env);
        }
        self.apply_environment();
        match self.small.busy.take() {
            Some((id, false)) => {
                ctx.set_cursor_icon(egui::CursorIcon::Wait);
                self.small.busy = Some((id, true));
                ctx.request_repaint();
            }
            Some((id, true)) => {
                ctx.set_cursor_icon(egui::CursorIcon::Wait);
                self.small.running = Some(id.clone());
                self.run_command(ctx, frame, &id);
                self.small.running = None;
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_kernel::Entities;

    #[test]
    fn names_and_labels() {
        let labels = mt_spec::Spec::builtin().unwrap().commands();
        assert_eq!(operation_name(&labels, "tool.move"), "Move");
        assert_eq!(
            operation_name(&labels, "edit.make_component"),
            "Make Component"
        );
        assert_eq!(operation_name(&labels, "nope"), "");
        assert_eq!(undo_label("Undo", Some("Move")), "Undo Move");
        assert_eq!(undo_label("Undo", Some("")), "Undo");
        assert_eq!(undo_label("Redo", None), "Redo");
        for id in ["edit.purge_unused", "view.hidden_objects"] {
            assert!(labels.contains_key(id), "{id} in menus.toml");
            assert!(COMMANDS.contains(&id));
        }
    }

    fn textured(name: &str) -> Material {
        Material {
            name: name.into(),
            color: [1, 2, 3],
            opacity: 1.0,
            texture: Some(mt_kernel::Texture {
                filename: "brick.png".into(),
                data: std::sync::Arc::from(&b"\x89PNG"[..]),
                width: 10.0,
                height: 10.0,
                colorize: false,
            }),
        }
    }

    #[test]
    fn make_unique_texture_copies_the_material_per_face() {
        let mut model = Model::default();
        model.materials.push(textured("Brick"));
        model.materials.push(Material {
            texture: None,
            ..textured("Plain")
        });
        let mut ents = Entities::new();
        let pts = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
            .map(|(x, y)| glam::DVec3::new(x, y, 0.0));
        for k in 0..4 {
            mt_kernel::add_line(&mut ents, pts[k], pts[(k + 1) % 4]);
        }
        let faces: Vec<FaceId> = ents.faces.keys().collect();
        assert_eq!(faces.len(), 1);
        ents.faces[faces[0]].attrs.material = Some(0);
        ents.faces[faces[0]].attrs.back_material = Some(0);
        assert_eq!(make_unique_texture(&mut model, &mut ents, &faces), 1);
        assert_eq!(model.materials.len(), 3);
        assert_eq!(model.materials[2].name, "Brick1");
        assert_eq!(ents.faces[faces[0]].attrs.material, Some(2));
        assert_eq!(ents.faces[faces[0]].attrs.back_material, Some(2));
        // A plain color is left alone.
        ents.faces[faces[0]].attrs.material = Some(1);
        ents.faces[faces[0]].attrs.back_material = None;
        assert_eq!(make_unique_texture(&mut model, &mut ents, &faces), 0);
        assert_eq!(unique_material_name(&model, "Brick"), "Brick2");
        assert_eq!(image_extension(b"\xff\xd8\xff\xe0"), "jpg");
        assert_eq!(image_extension(b"junk"), "png");
    }
}
