//! File > Import: SketchUp models, OBJ, STL, FBX, PLY, 3MF and 3DS.
//!
//! - A `.skp` becomes a component definition of the open model (with the imported
//!   model's own components, materials and tags; see
//!   [`mt_kernel::import::merge_as_definition`]) and the Place Component tool starts
//!   with it, as SketchUp does. Adding the definition is one undo step, each placement
//!   another.
//! - Mesh files ask for options (units, Y up, merge coplanar faces; PLY: colors as
//!   materials), then become one group at the open context's origin, in one undo
//!   step, selected. Units and axes default to what the file records (FBX, 3MF), else
//!   to per-format guesses ([`formats::Kind::default_hints`], unverified). Materials
//!   come with their colors and PNG/JPEG textures (OBJ `map_Kd`, FBX, PLY, 3MF, 3DS);
//!   a textured material's color becomes the image's average color. See
//!   [`mt_kernel::formats`].
//! - PNG and JPEG images become image entities placed with two clicks
//!   (`image_entity`); importing an image as a texture or a matched photo is not done.

use std::path::PathBuf;

use eframe::egui;
use mt_kernel::formats::{self, Kind};
use mt_kernel::mesh_export::Units;
use mt_kernel::mesh_import;

use crate::App;
use crate::document::Owner;
use crate::selection::Mode;

/// The mesh import options dialog while open.
#[derive(Debug, Clone)]
pub(crate) struct MeshImport {
    path: PathBuf,
    kind: Kind,
    units: Units,
    y_up: bool,
    merge: bool,
    /// PLY: vertex and face colors become materials.
    colors: bool,
}

const FILTERS: [(&str, &[&str]); 9] = [
    (
        "All Supported Types",
        &[
            "skp", "obj", "stl", "fbx", "ply", "3mf", "3ds", "png", "jpg", "jpeg",
        ],
    ),
    ("Meshtard Models (*.skp)", &["skp"]),
    ("OBJ Files (*.obj)", &["obj"]),
    ("STL Files (*.stl)", &["stl"]),
    ("FBX Files (*.fbx)", &["fbx"]),
    ("PLY Files (*.ply)", &["ply"]),
    ("3MF Files (*.3mf)", &["3mf"]),
    ("3DS Files (*.3ds)", &["3ds"]),
    ("Images (*.png, *.jpg)", &["png", "jpg", "jpeg"]),
];

/// Reads `path` as `kind`, with files it names (MTL libraries, textures) read
/// relative to its folder.
fn read_file(
    path: &std::path::Path,
    kind: Kind,
    colors: bool,
) -> Result<(mesh_import::Mesh, formats::Hints), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let dir = path
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf();
    let read = move |name: &str| {
        let p = std::path::Path::new(name);
        let p = if p.is_absolute() {
            p.to_path_buf()
        } else {
            dir.join(p)
        };
        std::fs::read(p).ok()
    };
    formats::read_mesh(kind, &bytes, &read, colors)
}

/// Reads and converts a mesh file. Returns the model materials and the faces built
/// with `first_material` as the model index of the first mesh material.
pub(crate) fn read_mesh(
    path: &std::path::Path,
    kind: Kind,
    opts: &mesh_import::MeshOptions,
    first_material: usize,
    colors: bool,
) -> Result<(Vec<mt_kernel::Material>, mt_kernel::Entities), String> {
    let (mesh, _) = read_file(path, kind, colors)?;
    let ids: Vec<usize> = (0..mesh.materials.len())
        .map(|i| first_material + i)
        .collect();
    let ents = mesh_import::to_entities(&mesh, opts, &ids);
    let materials = mesh
        .materials
        .iter()
        .map(|m| {
            let mut mat = formats::model_material(m);
            // Shaded mode shows a textured material's color: the image's average.
            if let Some(img) = mat
                .texture
                .as_ref()
                .and_then(|t| image::load_from_memory(&t.data).ok())
            {
                mat.color = crate::image_entity::average_color(&img.to_rgb8());
            }
            mat
        })
        .collect();
    Ok((materials, ents))
}

impl App {
    /// File > Import: asks for a file, then imports it or asks for options.
    pub(crate) fn import_command(&mut self, device: &egui_wgpu::wgpu::Device) {
        let Some(path) = crate::filedialog::open(&FILTERS) else {
            return;
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        match ext.as_str() {
            "skp" => self.import_skp(&path, device),
            "png" | "jpg" | "jpeg" => self.import_image(&path),
            _ if Kind::from_extension(&ext).is_some() => {
                let kind = Kind::from_extension(&ext).expect("checked");
                let (mut units, mut y_up) = kind.default_hints();
                // FBX and 3MF record their units and axes.
                if matches!(kind, Kind::Fbx | Kind::ThreeMf) {
                    match read_file(&path, kind, false) {
                        Ok((_, hints)) => {
                            units = hints.units.unwrap_or(units);
                            y_up = hints.y_up.unwrap_or(y_up);
                        }
                        Err(e) => {
                            self.file_error("import", &path, e);
                            return;
                        }
                    }
                }
                self.mesh_import = Some(MeshImport {
                    path,
                    kind,
                    units,
                    y_up,
                    merge: true,
                    colors: true,
                });
            }
            _ => {
                self.message = Some(format!("Cannot import {}.", path.display()));
            }
        }
    }

    fn import_skp(&mut self, path: &std::path::Path, device: &egui_wgpu::wgpu::Device) {
        let src = match mt_skp::read(path) {
            Ok(s) => s,
            Err(e) => {
                self.file_error("import", path, e);
                return;
            }
        };
        let model = mt_kernel::import::from_skp(&src);
        let name = path
            .file_stem()
            .map_or_else(|| "Import".into(), |s| s.to_string_lossy().into_owned());
        let Some(doc) = &mut self.document else {
            return;
        };
        let mut def = None;
        doc.edit_model(|m, _| {
            def = Some(mt_kernel::import::merge_as_definition(m, model, &name));
            true
        });
        let Some(definition) = def else { return };
        if let Some(d) = self.draw.as_mut() {
            d.cancel();
        }
        self.tool = crate::nav::Tool::Modify("tool.place_component");
        self.draw = None;
        self.modify = Some(Box::new(crate::modify::PlaceComponent {
            definition,
            glue: mt_kernel::GlueTo::None,
        }));
        self.selection.clear();
        self.rebuild(device);
        self.message = Some(format!("Imported {name}: click to place it."));
    }

    pub(crate) fn import_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(mut d) = self.mesh_import.take() else {
            return;
        };
        let title = format!("{} Import Options", d.kind.name());
        let r = mt_ui::Dialog::new("import_mesh", &title)
            .width(320.0)
            .show(ctx, |ui| {
                crate::export::dialog_visuals(ui);
                ui.label(
                    d.path
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                );
                ui.add_space(8.0);
                egui::Grid::new("mt_import_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Units:");
                        egui::ComboBox::from_id_salt("mt_import_units")
                            .selected_text(d.units.name())
                            .show_ui(ui, |ui| {
                                for u in Units::ALL {
                                    ui.selectable_value(&mut d.units, u, u.name());
                                }
                            });
                        ui.end_row();
                    });
                ui.add_space(6.0);
                ui.checkbox(&mut d.y_up, "File is Y up (swap YZ)");
                ui.checkbox(&mut d.merge, "Merge coplanar faces");
                if d.kind == Kind::Ply {
                    ui.checkbox(&mut d.colors, "Import colors as materials");
                }
                crate::presentation::ok_cancel(ui, "Import")
            });
        let done = crate::presentation::answer(r);
        match done {
            None => self.mesh_import = Some(d),
            Some(false) => {}
            Some(true) => {
                if let Some(rs) = frame.wgpu_render_state() {
                    let device = rs.device.clone();
                    self.import_mesh(&d, &device);
                }
            }
        }
    }

    fn import_mesh(&mut self, d: &MeshImport, device: &egui_wgpu::wgpu::Device) {
        let Some(doc) = &mut self.document else {
            return;
        };
        let opts = mesh_import::MeshOptions {
            inches_per_unit: 1.0 / d.units.per_inch(),
            y_up: d.y_up,
            merge_coplanar: d.merge,
        };
        let first = doc.model.materials.len();
        let (materials, ents) = match read_mesh(&d.path, d.kind, &opts, first, d.colors) {
            Ok(x) => x,
            Err(e) => {
                self.file_error("import", &d.path, e);
                return;
            }
        };
        if ents.faces.is_empty() {
            self.message = Some(format!("No faces in {}.", d.path.display()));
            return;
        }
        let faces = ents.faces.len();
        let mut placed = None;
        doc.edit_model(|m, active| {
            m.materials.extend(materials);
            let name = m.unique_definition_name("Group#1");
            let def = m.definitions.insert(mt_kernel::Definition {
                name,
                entities: ents,
                is_group: true,
                ..Default::default()
            });
            active.instances.push(mt_kernel::Instance {
                definition: def,
                transform: glam::DMat4::IDENTITY,
                name: String::new(),
                hidden: false,
                locked: false,
                layer: None,
                material: None,
                attrs: Default::default(),
            });
            placed = Some(active.instances.len() - 1);
            true
        });
        if let Some(i) = placed {
            self.selection.clear();
            self.selection.apply(Mode::Replace, [Owner::Instance(i)]);
        }
        self.rebuild(device);
        self.message = Some(format!("Imported {} ({faces} faces).", d.path.display()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_obj_with_mtl_next_to_it() {
        let dir = std::env::temp_dir().join(format!("mt_import_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("t.mtl"), "newmtl Blue\nKd 0 0 1\n").unwrap();
        std::fs::write(
            dir.join("t.obj"),
            "mtllib t.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl Blue\nf 1 2 3\n",
        )
        .unwrap();
        let opts = mesh_import::MeshOptions {
            inches_per_unit: 1.0,
            y_up: false,
            merge_coplanar: true,
        };
        let (mats, ents) = read_mesh(&dir.join("t.obj"), Kind::Obj, &opts, 3, true).unwrap();
        assert_eq!(mats.len(), 1);
        assert_eq!(mats[0].color, [0, 0, 255]);
        assert_eq!(ents.faces.len(), 1);
        assert_eq!(ents.faces.values().next().unwrap().attrs.material, Some(3));
        std::fs::remove_dir_all(&dir).ok();
    }
}
