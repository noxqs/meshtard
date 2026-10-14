//! File > Import: SketchUp models, glTF/GLB, DXF, IFC, OBJ, STL, FBX, PLY, 3MF, 3DS
//! and images.
//!
//! - A `.skp` becomes a component definition of the open model (with the imported
//!   model's own components, materials and tags; see
//!   [`mt_kernel::import::merge_as_definition`]) and the Place Component tool starts
//!   with it, as SketchUp does. Adding the definition is one undo step, each placement
//!   another.
//! - glTF / GLB ([`mt_kernel::gltf_io`]) ask whether to merge coplanar faces, then come
//!   in the same way as a `.skp`: one component (its node hierarchy as nested groups
//!   and components inside, materials with base color textures) placed with Place
//!   Component.
//! - DXF ([`mt_kernel::dxf`]) asks for units (from the file by default) and whether
//!   layers become groups (default, as SketchUp 2026's importer does) or tags, then
//!   becomes one component instance at the origin of the open context (drawings are
//!   often far from the origin), selected, in one undo step. DWG is a closed format and
//!   is not read; PDF import is not done.
//! - IFC / IFCZIP ([`formats::ifc`], geometry only) asks whether to merge coplanar
//!   faces (units come from the file), then becomes one component instance at the
//!   open context's origin, selected, in one undo step, like DXF: the project, site,
//!   building, storey and element hierarchy as nested groups, mapped items as
//!   components. The status bar says how many objects had geometry and what was
//!   skipped.
//! - Mesh files (OBJ, STL, FBX, PLY, 3MF, 3DS) ask for options (units, Y up, merge
//!   coplanar faces; PLY: colors as materials), then become one group at the open
//!   context's origin, in one undo step, selected; an FBX file's models become groups
//!   nested inside it ([`formats::scene`]). Units and axes default to what the
//!   file records (FBX, 3MF), else to per-format guesses
//!   ([`formats::Kind::default_hints`], unverified). Materials come with their colors
//!   and PNG/JPEG textures (OBJ `map_Kd`, FBX, PLY, 3MF, 3DS); a textured material's
//!   color becomes the image's average color. See [`mt_kernel::formats`].
//! - PNG and JPEG images ask "Use as: Image / Texture / New Matched Photo" (SketchUp
//!   has this choice in its import dialog; the default Image is from memory). Image: an
//!   image entity placed with two clicks (`image_entity`). Texture: two clicks on a face
//!   set the size and position of one tile; the face side gets a new material with the
//!   picture. New Matched Photo: as Camera > Match New Photo (`match_photo`).

use std::path::{Path, PathBuf};

use eframe::egui;
use mt_kernel::dxf::LayerMode;
use mt_kernel::formats;
use mt_kernel::mesh_export::Units;
use mt_kernel::mesh_import;

use crate::App;
use crate::document::Owner;
use crate::selection::Mode;

/// What an import options dialog is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// OBJ, STL, FBX, PLY, 3MF, 3DS.
    Mesh(formats::Kind),
    Gltf,
    Dxf,
    Ifc,
    Image,
}

/// The import options dialog while open.
#[derive(Debug, Clone)]
pub(crate) struct MeshImport {
    path: PathBuf,
    kind: Kind,
    /// Mesh units; DXF units when `units_from_file` is off.
    units: Units,
    units_from_file: bool,
    y_up: bool,
    merge: bool,
    layers: LayerMode,
    /// Images: "Use as texture" (else an image entity).
    as_texture: bool,
    /// Images: "Use as new matched photo" (wins over `as_texture`).
    as_photo: bool,
    /// PLY: vertex and face colors become materials.
    colors: bool,
}

impl MeshImport {
    fn new(path: PathBuf, kind: Kind) -> Self {
        let (units, y_up) = match kind {
            Kind::Mesh(k) => k.default_hints(),
            Kind::Dxf => (Units::Millimeters, false),
            Kind::Gltf | Kind::Ifc | Kind::Image => (Units::Meters, false),
        };
        Self {
            path,
            kind,
            units,
            units_from_file: true,
            y_up,
            merge: true,
            layers: LayerMode::Groups,
            as_texture: false,
            as_photo: false,
            colors: true,
        }
    }
}

const FILTERS: [(&str, &[&str]); 12] = [
    (
        "All Supported Types",
        &[
            "skp", "gltf", "glb", "dxf", "ifc", "ifczip", "obj", "stl", "fbx", "ply", "3mf", "3ds",
            "png", "jpg", "jpeg",
        ],
    ),
    ("Meshtard Models (*.skp)", &["skp"]),
    ("glTF Files (*.gltf, *.glb)", &["gltf", "glb"]),
    ("AutoCAD DXF Files (*.dxf)", &["dxf"]),
    ("IFC Files (*.ifc, *.ifczip)", &["ifc", "ifczip"]),
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
    path: &Path,
    kind: formats::Kind,
    colors: bool,
) -> Result<(formats::scene::Scene, formats::Hints), String> {
    crate::errors::guard(|| read_file_unguarded(path, kind, colors))
}

fn read_file_unguarded(
    path: &Path,
    kind: formats::Kind,
    colors: bool,
) -> Result<(formats::scene::Scene, formats::Hints), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let read = move |name: &str| {
        let p = Path::new(name);
        let p = if p.is_absolute() {
            p.to_path_buf()
        } else {
            dir.join(p)
        };
        std::fs::read(p).ok()
    };
    formats::read_scene(kind, &bytes, &read, colors)
}

/// Reads a mesh file: the model materials for its materials and the mesh (with its
/// node hierarchy, FBX) to build with [`formats::scene::to_entities`].
pub(crate) fn read_mesh(
    path: &Path,
    kind: formats::Kind,
    colors: bool,
) -> Result<(Vec<mt_kernel::Material>, formats::scene::Scene), String> {
    let (scene, _) = read_file(path, kind, colors)?;
    let materials = scene
        .mesh
        .materials
        .iter()
        .map(|m| {
            let mut mat = formats::model_material(m);
            // Shaded mode shows a textured material's color: the image's average.
            if let Some(img) = mat
                .texture
                .as_ref()
                .and_then(|t| mt_kernel::images::decode(&t.data).ok())
            {
                mat.color = crate::image_entity::average_color(&img.to_rgb8());
            }
            mat
        })
        .collect();
    Ok((materials, scene))
}

/// Moves `model` so its bounding box's low corner is at the origin. Place Component
/// holds a definition by its origin, and a glTF scene can lie far from its own (a
/// model exported from a drawing away from the origin): the copy would land that far
/// from the click.
pub(crate) fn origin_to_corner(model: &mut mt_kernel::Model) {
    let mut lo = glam::DVec3::splat(f64::INFINITY);
    let mut add = |p: glam::DVec3| lo = lo.min(p);
    for (_, v) in model.root.vertices.iter() {
        add(v.pos);
    }
    for i in &model.root.instances {
        if let Some((a, b)) =
            mt_kernel::instance_ops::definition_bounds(&model.definitions, i.definition)
        {
            for k in 0..8 {
                let c = glam::DVec3::new(
                    if k & 1 == 0 { a.x } else { b.x },
                    if k & 2 == 0 { a.y } else { b.y },
                    if k & 4 == 0 { a.z } else { b.z },
                );
                add(i.transform.transform_point3(c));
            }
        }
    }
    if lo.is_finite() && lo != glam::DVec3::ZERO {
        model.root = mt_kernel::instance_ops::transform_entities(
            &model.root,
            glam::DMat4::from_translation(-lo),
        );
    }
}

/// Reads a `.gltf` / `.glb` file (buffers and images next to it) into a model, with
/// textured materials given their image's average color and aspect ratio.
pub(crate) fn read_gltf(path: &Path, merge: bool) -> Result<mt_kernel::Model, String> {
    crate::errors::guard(|| read_gltf_unguarded(path, merge))
}

fn read_gltf_unguarded(path: &Path, merge: bool) -> Result<mt_kernel::Model, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let resolve = move |name: &str| -> Option<Vec<u8>> {
        // Only files in the model's folder or below.
        let rel = Path::new(name);
        if rel.is_absolute()
            || rel
                .components()
                .any(|c| c == std::path::Component::ParentDir)
        {
            return None;
        }
        std::fs::read(dir.join(rel)).ok()
    };
    let mut model = mt_kernel::gltf_io::import(&bytes, &resolve, merge)?;
    for m in &mut model.materials {
        let Some(t) = m.texture.as_mut() else {
            continue;
        };
        if let Ok(img) = mt_kernel::images::decode(&t.data) {
            let img = img.to_rgb8();
            let avg = crate::image_entity::average_color(&img);
            // The base color factor tints the texture.
            m.color = [0, 1, 2].map(|k| ((u32::from(avg[k]) * u32::from(m.color[k])) / 255) as u8);
            let (w, h) = img.dimensions();
            t.height = t.width * f64::from(h.max(1)) / f64::from(w.max(1));
        }
    }
    Ok(model)
}

/// Reads a DXF file into a model with the chosen units and layer handling.
pub(crate) fn read_dxf(
    path: &Path,
    units: Option<Units>,
    layers: LayerMode,
) -> Result<mt_kernel::Model, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    crate::errors::guard(|| {
        let drawing = mt_kernel::dxf::parse(&bytes)?;
        Ok::<_, String>(mt_kernel::dxf::to_model(
            &drawing,
            &mt_kernel::dxf::DxfOptions {
                inches_per_unit: units.map(|u| 1.0 / u.per_inch()),
                layers,
            },
        ))
    })
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map_or_else(|| "Import".into(), |s| s.to_string_lossy().into_owned())
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
        let kind = match ext.as_str() {
            "skp" => return self.import_skp(&path, device),
            "png" | "jpg" | "jpeg" => Kind::Image,
            "gltf" | "glb" => Kind::Gltf,
            "dxf" => Kind::Dxf,
            "ifc" | "ifczip" => Kind::Ifc,
            "dwg" => {
                self.message =
                    Some("DWG is a closed format and cannot be imported; save it as DXF.".into());
                return;
            }
            other => match formats::Kind::from_extension(other) {
                Some(k) => Kind::Mesh(k),
                None => {
                    self.message = Some(format!("Cannot import {}.", path.display()));
                    return;
                }
            },
        };
        let mut d = MeshImport::new(path, kind);
        // FBX and 3MF record their units and axes.
        if let Kind::Mesh(k @ (formats::Kind::Fbx | formats::Kind::ThreeMf)) = kind {
            match read_file(&d.path, k, false) {
                Ok((_, hints)) => {
                    d.units = hints.units.unwrap_or(d.units);
                    d.y_up = hints.y_up.unwrap_or(d.y_up);
                }
                Err(e) => {
                    self.file_error("import", &d.path, e);
                    return;
                }
            }
        }
        self.mesh_import = Some(d);
    }

    fn import_skp(&mut self, path: &Path, device: &egui_wgpu::wgpu::Device) {
        let model = match crate::errors::read_skp_model(path) {
            Ok(m) => m,
            Err(e) => {
                self.file_error("import", path, e);
                return;
            }
        };
        self.place_imported(model, &file_stem(path), device);
    }

    /// Adds `model` as a component definition named `name` and starts Place Component
    /// with it.
    fn place_imported(
        &mut self,
        model: mt_kernel::Model,
        name: &str,
        device: &egui_wgpu::wgpu::Device,
    ) {
        self.place_imported_with(model, name, device, |_| {});
    }

    /// [`Self::place_imported`], with `f` changing the new definition in the same
    /// undo step (the model library stores the license and source there).
    pub(crate) fn place_imported_with(
        &mut self,
        model: mt_kernel::Model,
        name: &str,
        device: &egui_wgpu::wgpu::Device,
        f: impl FnOnce(&mut mt_kernel::Definition),
    ) {
        let Some(doc) = &mut self.document else {
            return;
        };
        let mut def = None;
        doc.edit_model(|m, _| {
            let id = mt_kernel::import::merge_as_definition(m, model, name);
            if let Some(d) = m.definitions.get_mut(id) {
                f(d);
            }
            def = Some(id);
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
        let title = match d.kind {
            Kind::Mesh(k) => format!("{} Import Options", k.name()),
            Kind::Gltf => "glTF Import Options".into(),
            Kind::Dxf => "DXF Import Options".into(),
            Kind::Ifc => "IFC Import Options".into(),
            Kind::Image => "Import Image".into(),
        };
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
                match d.kind {
                    Kind::Mesh(k) => {
                        units_row(ui, &mut d.units, None);
                        ui.add_space(6.0);
                        ui.checkbox(&mut d.y_up, "File is Y up (swap YZ)");
                        ui.checkbox(&mut d.merge, "Merge coplanar faces");
                        if k == formats::Kind::Ply {
                            ui.checkbox(&mut d.colors, "Import colors as materials");
                        }
                    }
                    Kind::Gltf => {
                        ui.checkbox(&mut d.merge, "Merge coplanar faces");
                        ui.weak("glTF is in meters, Y up.");
                    }
                    Kind::Ifc => {
                        ui.checkbox(&mut d.merge, "Merge coplanar faces");
                        ui.weak("Units come from the file. Openings are not cut.");
                    }
                    Kind::Dxf => {
                        units_row(ui, &mut d.units, Some(&mut d.units_from_file));
                        ui.add_space(6.0);
                        ui.label("Layers become:");
                        ui.radio_value(&mut d.layers, LayerMode::Groups, "Groups");
                        ui.radio_value(&mut d.layers, LayerMode::Tags, "Tags");
                    }
                    Kind::Image => {
                        ui.label("Use image as:");
                        let mut choice = if d.as_photo {
                            2
                        } else {
                            usize::from(d.as_texture)
                        };
                        ui.radio_value(&mut choice, 0, "Image");
                        ui.radio_value(&mut choice, 1, "Texture");
                        ui.radio_value(&mut choice, 2, "New Matched Photo");
                        d.as_texture = choice == 1;
                        d.as_photo = choice == 2;
                    }
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
                    match d.kind {
                        Kind::Mesh(_) => self.import_mesh(&d, &device),
                        Kind::Gltf => match read_gltf(&d.path, d.merge) {
                            Ok(m) if m.root.instances.is_empty() => {
                                self.message = Some(format!("No faces in {}.", d.path.display()));
                            }
                            Ok(mut m) => {
                                origin_to_corner(&mut m);
                                self.place_imported(m, &file_stem(&d.path), &device);
                            }
                            Err(e) => self.file_error("import", &d.path, e),
                        },
                        Kind::Dxf => self.import_dxf(&d, &device),
                        Kind::Ifc => self.import_ifc(&d, &device),
                        Kind::Image if d.as_photo => self.match_new_photo(&d.path),
                        Kind::Image => self.import_image(&d.path, d.as_texture),
                    }
                }
            }
        }
    }

    fn import_dxf(&mut self, d: &MeshImport, device: &egui_wgpu::wgpu::Device) {
        let units = (!d.units_from_file).then_some(d.units);
        let model = match read_dxf(&d.path, units, d.layers) {
            Ok(m) => m,
            Err(e) => {
                self.file_error("import", &d.path, e);
                return;
            }
        };
        if model.root.instances.is_empty() && model.root.edges.is_empty() {
            self.message = Some(format!("Nothing to import in {}.", d.path.display()));
            return;
        }
        self.add_at_origin(model, &file_stem(&d.path), device);
        self.message = Some(format!("Imported {}.", d.path.display()));
    }

    fn import_ifc(&mut self, d: &MeshImport, device: &egui_wgpu::wgpu::Device) {
        let read = std::fs::read(&d.path).map_err(|e| e.to_string());
        let opts = formats::ifc::IfcOptions {
            merge_coplanar: d.merge,
        };
        // Texture images named by the file, relative to its folder.
        let dir = d.path.parent().map(Path::to_path_buf).unwrap_or_default();
        let images = move |name: &str| std::fs::read(dir.join(name)).ok();
        let (model, report) = match read
            .and_then(|b| crate::errors::guard(|| formats::ifc::import_with(&b, &opts, &images)))
        {
            Ok(x) => x,
            Err(e) => {
                self.file_error("import", &d.path, e);
                return;
            }
        };
        if model.root.instances.is_empty() {
            self.message = Some(format!(
                "No geometry in {} ({}).",
                d.path.display(),
                report.summary()
            ));
            return;
        }
        self.add_at_origin(model, &file_stem(&d.path), device);
        self.message = Some(format!(
            "Imported {}: {}.",
            d.path.display(),
            report.summary()
        ));
    }

    /// Adds `model` as a component definition named `name` with one instance at the
    /// open context's origin, selected, in one undo step.
    fn add_at_origin(
        &mut self,
        model: mt_kernel::Model,
        name: &str,
        device: &egui_wgpu::wgpu::Device,
    ) {
        let Some(doc) = &mut self.document else {
            return;
        };
        let mut placed = None;
        // A file's geo-location (IFC sites) is taken when the model has none.
        let (geo, lat, lon) = (model.geo, model.shadows.latitude, model.shadows.longitude);
        doc.edit_model(|m, active| {
            if !m.geo.located && m.geo.map_origin == glam::DVec3::ZERO {
                if geo.located {
                    m.shadows.latitude = lat;
                    m.shadows.longitude = lon;
                    m.geo = geo;
                } else if geo.map_origin != glam::DVec3::ZERO || geo.epsg != 0 {
                    m.geo.map_origin = geo.map_origin;
                    m.geo.epsg = geo.epsg;
                    m.geo.north_angle = geo.north_angle;
                }
            }
            let def = mt_kernel::import::merge_as_definition(m, model, name);
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
        let Kind::Mesh(kind) = d.kind else {
            return;
        };
        let (materials, scene) = match read_mesh(&d.path, kind, d.colors) {
            Ok(x) => x,
            Err(e) => {
                self.file_error("import", &d.path, e);
                return;
            }
        };
        let mut faces = 0;
        let mut placed = None;
        let mut failed = None;
        doc.edit_model(|m, active| {
            let first = m.materials.len();
            let ids: Vec<usize> = (first..first + materials.len()).collect();
            m.materials.extend(materials);
            // FBX models become groups inside the import's group. A panic building
            // the faces leaves the model as it was (errors::guard).
            let defs: std::collections::HashSet<_> = m.definitions.keys().collect();
            let ents = match crate::errors::guard(|| {
                Ok::<_, String>(formats::scene::to_entities(m, &scene, &opts, &ids))
            }) {
                Ok(ents) => ents,
                Err(e) => {
                    m.definitions.retain(|k, _| defs.contains(&k));
                    m.materials.truncate(first);
                    failed = Some(e);
                    return false;
                }
            };
            faces = count_faces(m, &ents);
            if faces == 0 {
                m.materials.truncate(first);
                return false;
            }
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
        if let Some(e) = failed {
            self.file_error("import", &d.path, e);
            return;
        }
        if faces == 0 {
            self.message = Some(format!("No faces in {}.", d.path.display()));
            return;
        }
        if let Some(i) = placed {
            self.selection.clear();
            self.selection.apply(Mode::Replace, [Owner::Instance(i)]);
        }
        self.rebuild(device);
        self.message = Some(format!("Imported {} ({faces} faces).", d.path.display()));
    }
}

/// Faces of `ents` and of the groups and components in it (each definition once).
pub(crate) fn count_faces(model: &mt_kernel::Model, ents: &mt_kernel::Entities) -> usize {
    let mut seen = std::collections::BTreeSet::new();
    let mut stack: Vec<&mt_kernel::Entities> = vec![ents];
    let mut n = 0;
    while let Some(e) = stack.pop() {
        n += e.faces.len();
        for i in &e.instances {
            if seen.insert(i.definition)
                && let Some(d) = model.definitions.get(i.definition)
            {
                stack.push(&d.entities);
            }
        }
    }
    n
}

/// The Units row of an import dialog; with `from_file`, a "From file" choice first.
fn units_row(ui: &mut egui::Ui, units: &mut Units, mut from_file: Option<&mut bool>) {
    egui::Grid::new("mt_import_grid")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Units:");
            let file = from_file.as_deref().copied().unwrap_or(false);
            let text = if file { "From file" } else { units.name() };
            egui::ComboBox::from_id_salt("mt_import_units")
                .selected_text(text)
                .show_ui(ui, |ui| {
                    if let Some(f) = from_file.as_deref_mut()
                        && ui.selectable_label(*f, "From file").clicked()
                    {
                        *f = true;
                    }
                    for u in Units::ALL {
                        let on = !file && *units == u;
                        if ui.selectable_label(on, u.name()).clicked() {
                            *units = u;
                            if let Some(f) = from_file.as_deref_mut() {
                                *f = false;
                            }
                        }
                    }
                });
            ui.end_row();
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mt_import_{tag}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_obj_with_mtl_next_to_it() {
        let dir = temp_dir("obj");
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
        let (mats, scene) = read_mesh(&dir.join("t.obj"), formats::Kind::Obj, true).unwrap();
        let mut model = mt_kernel::Model::default();
        let ents = formats::scene::to_entities(&mut model, &scene, &opts, &[3]);
        assert_eq!(mats.len(), 1);
        assert_eq!(mats[0].color, [0, 0, 255]);
        assert_eq!(ents.faces.len(), 1);
        assert_eq!(ents.faces.values().next().unwrap().attrs.material, Some(3));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn imported_gltf_is_held_by_its_low_corner() {
        // A box 1 m wide, 50 m from the scene's origin.
        let gltf = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
          "nodes":[{"mesh":0,"translation":[50,2,-3]}],
          "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
          "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3",
            "min":[0,0,0],"max":[1,0,1]}],
          "bufferViews":[{"buffer":0,"byteLength":36}],
          "buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAAAAAAIA/"}]}"#;
        let mut m = mt_kernel::gltf_io::import(gltf.as_bytes(), &|_| None, false).unwrap();
        origin_to_corner(&mut m);
        let i = &m.root.instances[0];
        let (a, b) =
            mt_kernel::instance_ops::definition_bounds(&m.definitions, i.definition).unwrap();
        let (a, b) = (
            i.transform.transform_point3(a),
            i.transform.transform_point3(b),
        );
        let lo = a.min(b);
        assert!(lo.abs().max_element() < 1e-9, "{lo}");
        // Size is kept: 1 m in x.
        assert!(((a - b).x.abs() - 1.0 / 0.0254).abs() < 1e-6);
    }

    #[test]
    fn glb_export_reads_back_with_texture_aspect() {
        let dir = temp_dir("glb");
        let mut m = mt_kernel::Model::default();
        let c =
            [(0., 0.), (10., 0.), (10., 10.), (0., 10.)].map(|(x, y)| glam::DVec3::new(x, y, 0.0));
        for i in 0..4 {
            mt_kernel::add_line(&mut m.root, c[i], c[(i + 1) % 4]);
        }
        let mut png = Vec::new();
        image::RgbImage::from_pixel(4, 2, image::Rgb([200, 100, 0]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        m.materials.push(mt_kernel::Material {
            name: "Orange".into(),
            color: [1, 2, 3],
            opacity: 1.0,
            texture: Some(mt_kernel::Texture {
                filename: "o.png".into(),
                data: png.into(),
                width: 10.0,
                height: 5.0,
                colorize: false,
            }),
        });
        for f in m.root.faces.values_mut() {
            f.attrs.material = Some(0);
            f.attrs.back_material = Some(0);
        }
        let path = dir.join("m.glb");
        crate::export::export_3d(
            &m,
            &crate::export::Export3d {
                format: crate::export::Format3d::Glb,
                ..Default::default()
            },
            Units::Inches,
            &path,
        )
        .unwrap();
        let back = read_gltf(&path, true).unwrap();
        let mat = &back.materials[0];
        assert_eq!(mat.color, [200, 100, 0]);
        let t = mat.texture.as_ref().unwrap();
        assert!((t.height / t.width - 0.5).abs() < 1e-9);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reads_dxf_with_units_choice() {
        let dir = temp_dir("dxf");
        let mut text = String::from("0\nSECTION\n2\nENTITIES\n");
        for (a, b) in [
            ((0, 0), (1, 0)),
            ((1, 0), (1, 1)),
            ((1, 1), (0, 1)),
            ((0, 1), (0, 0)),
        ] {
            text += &format!(
                "0\nLINE\n8\nPlan\n10\n{}\n20\n{}\n30\n0\n11\n{}\n21\n{}\n31\n0\n",
                a.0, a.1, b.0, b.1
            );
        }
        text += "0\nENDSEC\n0\nEOF\n";
        std::fs::write(dir.join("p.dxf"), text).unwrap();
        let m = read_dxf(&dir.join("p.dxf"), Some(Units::Feet), LayerMode::Groups).unwrap();
        assert_eq!(m.root.instances.len(), 1);
        let def = &m.definitions[m.root.instances[0].definition];
        assert_eq!(def.entities.faces.len(), 1);
        let max = def
            .entities
            .vertices
            .values()
            .map(|v| v.pos.x)
            .fold(0.0, f64::max);
        assert!((max - 12.0).abs() < 1e-9);
        std::fs::remove_dir_all(&dir).ok();
    }
}
