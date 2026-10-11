//! File > Import: SketchUp models, OBJ and STL.
//!
//! - A `.skp` becomes a component definition of the open model (with the imported
//!   model's own components, materials and tags; see
//!   [`mt_kernel::import::merge_as_definition`]) and the Place Component tool starts
//!   with it, as SketchUp does. Adding the definition is one undo step, each placement
//!   another.
//! - OBJ and STL ask for options (units, Y up, merge coplanar faces), then become one
//!   group at the open context's origin, in one undo step, selected. Default units
//!   (STL millimeters, OBJ meters) and Y up for OBJ are guesses (unverified); MTL
//!   colors become materials, texture maps are not read.
//! - PNG and JPEG images become image entities placed with two clicks
//!   (`image_entity`); importing an image as a texture or a matched photo is not done.

use std::path::PathBuf;

use eframe::egui;
use mt_kernel::mesh_export::Units;
use mt_kernel::mesh_import;

use crate::App;
use crate::document::Owner;
use crate::selection::Mode;

/// The OBJ/STL options dialog while open.
#[derive(Debug, Clone)]
pub(crate) struct MeshImport {
    path: PathBuf,
    stl: bool,
    units: Units,
    y_up: bool,
    merge: bool,
}

const FILTERS: [(&str, &[&str]); 5] = [
    (
        "All Supported Types",
        &["skp", "obj", "stl", "png", "jpg", "jpeg"],
    ),
    ("Meshtard Models (*.skp)", &["skp"]),
    ("OBJ Files (*.obj)", &["obj"]),
    ("STL Files (*.stl)", &["stl"]),
    ("Images (*.png, *.jpg)", &["png", "jpg", "jpeg"]),
];

/// Reads and converts an OBJ or STL file. Returns the mesh materials and the faces
/// built with `first_material` as the model index of the first mesh material.
pub(crate) fn read_mesh(
    path: &std::path::Path,
    stl: bool,
    opts: &mesh_import::MeshOptions,
    first_material: usize,
) -> Result<(Vec<mesh_import::MeshMaterial>, mt_kernel::Entities), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mesh = if stl {
        mesh_import::parse_stl(&bytes)?
    } else {
        let text = String::from_utf8_lossy(&bytes);
        let dir = path.parent().unwrap_or(std::path::Path::new("."));
        mesh_import::parse_obj(&text, |name| std::fs::read_to_string(dir.join(name)).ok())?
    };
    let ids: Vec<usize> = (0..mesh.materials.len())
        .map(|i| first_material + i)
        .collect();
    let ents = mesh_import::to_entities(&mesh, opts, &ids);
    Ok((mesh.materials, ents))
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
            "obj" | "stl" => {
                let stl = ext == "stl";
                self.mesh_import = Some(MeshImport {
                    path,
                    stl,
                    units: if stl {
                        Units::Millimeters
                    } else {
                        Units::Meters
                    },
                    y_up: !stl,
                    merge: true,
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
            ..Default::default()
        }));
        self.selection.clear();
        self.rebuild(device);
        self.message = Some(format!("Imported {name}: click to place it."));
    }

    pub(crate) fn import_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(mut d) = self.mesh_import.take() else {
            return;
        };
        let title = if d.stl {
            "STL Import Options"
        } else {
            "OBJ Import Options"
        };
        let r = mt_ui::Dialog::new("import_mesh", title)
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
        let (materials, ents) = match read_mesh(&d.path, d.stl, &opts, doc.model.materials.len()) {
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
            m.materials
                .extend(materials.into_iter().map(|x| mt_kernel::Material {
                    name: x.name,
                    color: x.color,
                    opacity: x.opacity,
                    texture: None,
                }));
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
        let (mats, ents) = read_mesh(&dir.join("t.obj"), false, &opts, 3).unwrap();
        assert_eq!(mats.len(), 1);
        assert_eq!(mats[0].color, [0, 0, 255]);
        assert_eq!(ents.faces.len(), 1);
        assert_eq!(ents.faces.values().next().unwrap().attrs.material, Some(3));
        std::fs::remove_dir_all(&dir).ok();
    }
}
