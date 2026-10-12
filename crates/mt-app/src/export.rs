//! File > Export > 2D Graphic and 3D Model.
//!
//! 2D Graphic renders the current view offscreen ([`mt_render::Viewport::render_image`])
//! and saves PNG or JPEG (by the chosen extension). Options, after SketchUp's export
//! options (from memory, unverified): "Use view size" (on) or a width and height with
//! the view's aspect kept, "Transparent background" (PNG only; the sky, ground and
//! background are left out, so ground shadows go too) and "Anti-alias" (on; renders at
//! twice the size and scales down; the view's own 4x multisampling is always on).
//! Line widths stay in screen pixels. Selection highlights are left out; the fading of
//! geometry outside an open group is kept as on screen. Dimension and text labels
//! (painted over the view by the UI) are not in the image.
//!
//! 3D Model writes OBJ (+ MTL and texture images next to it), binary or ASCII STL or
//! COLLADA through [`mt_kernel::mesh_export`], glTF / GLB through
//! [`mt_kernel::gltf_io`], and FBX, PLY (binary or ASCII, texture coordinates), 3MF,
//! VRML, KMZ, 3DS, USDZ or IFC through [`mt_kernel::formats`]; the options dialog picks the
//! format and its options before the file dialog. Per-format option defaults
//! (two-sided faces for COLLADA and KMZ, Y up for OBJ, VRML and USDZ) are from memory,
//! unverified. "Export only current selection" (every format, as in SketchUp's
//! exporters) writes the selected entities of the open context, in model coordinates,
//! instead of the whole model.

use eframe::egui;
use mt_kernel::formats::{self, ExportKind};
use mt_kernel::mesh_export::{self, Units};

use crate::App;
use crate::document::Owner;

/// Export 2D Graphic options.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Export2d {
    pub use_view_size: bool,
    pub width: u32,
    pub height: u32,
    pub transparent: bool,
    pub antialias: bool,
}

impl Default for Export2d {
    fn default() -> Self {
        Self {
            use_view_size: true,
            width: 1920,
            height: 1080,
            transparent: false,
            antialias: true,
        }
    }
}

/// Export 3D Model formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format3d {
    Obj,
    Stl,
    Dae,
    Glb,
    Gltf,
    Other(ExportKind),
}

impl Format3d {
    const ALL: [Self; 13] = [
        Self::Other(ExportKind::ThreeDs),
        Self::Dae,
        Self::Other(ExportKind::Fbx),
        Self::Glb,
        Self::Gltf,
        Self::Other(ExportKind::Ifc),
        Self::Other(ExportKind::Kmz),
        Self::Obj,
        Self::Other(ExportKind::Ply),
        Self::Stl,
        Self::Other(ExportKind::ThreeMf),
        Self::Other(ExportKind::Usdz),
        Self::Other(ExportKind::Vrml),
    ];

    /// glTF keeps groups and components as nodes (meters, Y up).
    fn keeps_hierarchy(self) -> bool {
        matches!(self, Self::Glb | Self::Gltf)
    }

    /// IFC keeps groups and components as building element proxies.
    fn keeps_objects(self) -> bool {
        self == Self::Other(ExportKind::Ifc)
    }

    fn label(self) -> &'static str {
        match self {
            Self::Obj => "OBJ File (*.obj)",
            Self::Stl => "STL File (*.stl)",
            Self::Dae => "COLLADA File (*.dae)",
            Self::Glb => "glTF Binary (*.glb)",
            Self::Gltf => "glTF File (*.gltf)",
            Self::Other(ExportKind::Fbx) => "FBX File (*.fbx)",
            Self::Other(ExportKind::Kmz) => "Google Earth File (*.kmz)",
            Self::Other(ExportKind::Ply) => "PLY File (*.ply)",
            Self::Other(ExportKind::ThreeMf) => "3MF File (*.3mf)",
            Self::Other(ExportKind::Vrml) => "VRML File (*.wrl)",
            Self::Other(ExportKind::ThreeDs) => "3DS File (*.3ds)",
            Self::Other(ExportKind::Usdz) => "USDZ File (*.usdz)",
            Self::Other(ExportKind::Ifc) => "IFC File (*.ifc)",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Obj => "obj",
            Self::Stl => "stl",
            Self::Dae => "dae",
            Self::Glb => "glb",
            Self::Gltf => "gltf",
            Self::Other(k) => k.extension(),
        }
    }

    /// Whether the format has a length unit to choose (COLLADA and KMZ are inches,
    /// glTF meters).
    fn has_units(self) -> bool {
        !matches!(
            self,
            Self::Dae | Self::Glb | Self::Gltf | Self::Other(ExportKind::Kmz)
        )
    }

    fn has_two_sided(self) -> bool {
        !matches!(
            self,
            Self::Stl | Self::Other(ExportKind::Ply | ExportKind::ThreeMf | ExportKind::Ifc)
        )
    }

    fn has_y_up(self) -> bool {
        matches!(
            self,
            Self::Obj
                | Self::Other(
                    ExportKind::Fbx | ExportKind::Vrml | ExportKind::Ply | ExportKind::Usdz
                )
        )
    }

    fn has_textures(self) -> bool {
        matches!(
            self,
            Self::Other(
                ExportKind::Fbx
                    | ExportKind::Vrml
                    | ExportKind::ThreeMf
                    | ExportKind::Ply
                    | ExportKind::ThreeDs
                    | ExportKind::Usdz
            )
        )
    }

    /// STL and PLY are binary or ASCII.
    fn has_ascii(self) -> bool {
        matches!(self, Self::Stl | Self::Other(ExportKind::Ply))
    }

    /// The options a format starts with when picked: two-sided faces and Y up.
    fn defaults(self) -> (bool, bool) {
        match self {
            Self::Dae | Self::Other(ExportKind::Kmz) => (true, false),
            Self::Obj | Self::Other(ExportKind::Vrml | ExportKind::Usdz) => (false, true),
            _ => (false, false),
        }
    }
}

/// Export 3D Model options.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Export3d {
    pub format: Format3d,
    /// OBJ and STL coordinates' unit; `None` for the model's units.
    pub units: Option<Units>,
    pub two_sided: bool,
    pub y_up: bool,
    /// Write texture images and coordinates (FBX, VRML, 3MF, PLY, 3DS, USDZ).
    pub textures: bool,
    /// STL and PLY: ASCII instead of binary.
    pub ascii: bool,
    /// Export only the selection of the open context.
    pub selection_only: bool,
}

impl Default for Export3d {
    fn default() -> Self {
        Self {
            // SketchUp's own exchange format first (unverified as the default type).
            format: Format3d::Dae,
            units: None,
            // On for COLLADA, off for OBJ (reset when the format changes; SketchUp's
            // per-format defaults are from memory, unverified).
            two_sided: true,
            y_up: true,
            textures: true,
            ascii: false,
            selection_only: false,
        }
    }
}

/// Export dialogs: the open one, if any, and the last used options.
#[derive(Debug, Clone, Default)]
pub(crate) struct Exports {
    d2: Option<Export2d>,
    d3: Option<Export3d>,
    last2d: Export2d,
    last3d: Export3d,
}

/// The shell theme draws flat widgets; dialog fields need visible borders.
pub(crate) fn dialog_visuals(ui: &mut egui::Ui) {
    let border = egui::Stroke::new(1.0, egui::Color32::from_gray(150));
    let w = &mut ui.visuals_mut().widgets;
    w.inactive.bg_stroke = border;
    w.inactive.bg_fill = egui::Color32::WHITE;
    w.inactive.weak_bg_fill = egui::Color32::from_gray(235);
    w.hovered.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(0, 120, 215));
    ui.visuals_mut().extreme_bg_color = egui::Color32::WHITE;
}

/// The kernel unit for a document unit.
pub(crate) fn kernel_units(u: mt_infer::units::LengthUnit) -> Units {
    use mt_infer::units::LengthUnit as L;
    match u {
        L::Inch => Units::Inches,
        L::Foot => Units::Feet,
        L::Millimeter => Units::Millimeters,
        L::Centimeter => Units::Centimeters,
        L::Meter => Units::Meters,
    }
}

/// Encodes `img` for `path`'s extension (JPEG for .jpg/.jpeg, else PNG).
pub(crate) fn encode_image(
    img: &image::RgbaImage,
    path: &std::path::Path,
) -> Result<Vec<u8>, String> {
    let jpeg = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"));
    let mut out = std::io::Cursor::new(Vec::new());
    if jpeg {
        let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
            .encode_image(&rgb)
            .map_err(|e| e.to_string())?;
    } else {
        img.write_to(&mut out, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
    }
    Ok(out.into_inner())
}

/// Writes an export's files: `main` at `path`, the others next to it by name.
fn write_files(
    path: &std::path::Path,
    main: &[u8],
    others: &[(String, &[u8])],
) -> Result<(), String> {
    std::fs::write(path, main).map_err(|e| e.to_string())?;
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    for (name, data) in others {
        std::fs::write(dir.join(name), data).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Encodes the model in `opts.format` and writes it (with MTL and textures) at `path`.
pub(crate) fn export_3d(
    model: &mt_kernel::Model,
    opts: &Export3d,
    model_units: Units,
    path: &std::path::Path,
) -> Result<(), String> {
    let base = path
        .file_stem()
        .map_or_else(|| "model".into(), |s| s.to_string_lossy().into_owned());
    let units = opts.units.unwrap_or(model_units);
    match opts.format {
        Format3d::Obj => {
            let files = mesh_export::to_obj(
                model,
                &mesh_export::ObjOptions {
                    units,
                    two_sided: opts.two_sided,
                    y_up: opts.y_up,
                    base_name: base.clone(),
                },
            );
            let mut others: Vec<(String, &[u8])> =
                vec![(format!("{base}.mtl"), files.mtl.as_bytes())];
            others.extend(files.textures.iter().map(|(n, d)| (n.clone(), &d[..])));
            write_files(path, files.obj.as_bytes(), &others)
        }
        Format3d::Stl if opts.ascii => write_files(
            path,
            mesh_export::to_stl_ascii(model, units, &base).as_bytes(),
            &[],
        ),
        Format3d::Stl => write_files(path, &mesh_export::to_stl(model, units), &[]),
        Format3d::Dae => {
            let files = mesh_export::to_dae(
                model,
                &mesh_export::DaeOptions {
                    two_sided: opts.two_sided,
                    base_name: base,
                },
            );
            let others: Vec<(String, &[u8])> = files
                .textures
                .iter()
                .map(|(n, d)| (n.clone(), &d[..]))
                .collect();
            write_files(path, files.dae.as_bytes(), &others)
        }
        Format3d::Glb | Format3d::Gltf => {
            let bytes = mt_kernel::gltf_io::to_gltf(
                model,
                &mt_kernel::gltf_io::GltfOptions {
                    binary: opts.format == Format3d::Glb,
                    two_sided: opts.two_sided,
                },
            );
            write_files(path, &bytes, &[])
        }
        Format3d::Other(kind) => {
            let files = formats::export(
                kind,
                model,
                &formats::ExportOptions {
                    units,
                    two_sided: opts.two_sided,
                    y_up: opts.y_up,
                    textures: opts.textures,
                    base_name: base,
                    ascii: opts.ascii,
                },
            )?;
            let others: Vec<(String, &[u8])> = files
                .others
                .iter()
                .map(|(n, d)| (n.clone(), &d[..]))
                .collect();
            write_files(path, &files.main, &others)
        }
    }
}

/// "Export only current selection": the model with only the selected entities of the
/// open context (`active`, placed by `xf`) at its root.
pub(crate) fn selection_model(
    model: &mt_kernel::Model,
    active: &mt_kernel::Entities,
    items: &[Owner],
    xf: glam::DMat4,
) -> mt_kernel::Model {
    let sel = crate::kernel_selection(items);
    let pick = |f: fn(&Owner) -> Option<usize>| items.iter().filter_map(f).collect::<Vec<_>>();
    let dims = pick(|o| match o {
        Owner::Dimension(i) => Some(*i),
        _ => None,
    });
    let texts = pick(|o| match o {
        Owner::Text(i) => Some(*i),
        _ => None,
    });
    let root = mt_kernel::clipboard::copy(active, &sel, &dims, &texts, xf);
    mt_kernel::Model {
        root,
        definitions: model.definitions.clone(),
        materials: model.materials.clone(),
        layers: model.layers.clone(),
        front_color: model.front_color,
        back_color: model.back_color,
        ..Default::default()
    }
}

impl App {
    /// File > Export > 2D Graphic / 3D Model: opens the options dialog.
    pub(crate) fn export_command(&mut self, id: &str) {
        if id == "file.export_2d" {
            let mut d = self.exports.last2d.clone();
            if d.use_view_size || d.width == 0 {
                d.width = self.viewport_px.x.max(1.0) as u32;
                d.height = self.viewport_px.y.max(1.0) as u32;
            }
            self.exports.d2 = Some(d);
        } else {
            self.exports.d3 = Some(self.exports.last3d.clone());
        }
    }

    pub(crate) fn export_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        self.export_2d_ui(ctx, frame);
        self.export_3d_ui(ctx);
    }

    fn export_2d_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(mut d) = self.exports.d2.take() else {
            return;
        };
        let view = [
            self.viewport_px.x.max(1.0) as u32,
            self.viewport_px.y.max(1.0) as u32,
        ];
        let aspect = f64::from(view[0]) / f64::from(view[1]);
        let r = mt_ui::Dialog::new("export_2d", "Export 2D Graphic")
            .width(320.0)
            .show(ctx, |ui| {
                dialog_visuals(ui);
                ui.strong("Image Size");
                ui.add_space(4.0);
                ui.checkbox(&mut d.use_view_size, "Use view size");
                if d.use_view_size {
                    d.width = view[0];
                    d.height = view[1];
                }
                ui.add_space(4.0);
                egui::Grid::new("mt_export_2d_grid")
                    .num_columns(3)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        // Width and height keep the view's aspect.
                        ui.label("Width:");
                        let w = ui.add_enabled(
                            !d.use_view_size,
                            egui::DragValue::new(&mut d.width).range(16..=8192),
                        );
                        ui.label("pixels");
                        ui.end_row();
                        if w.changed() {
                            d.height = ((f64::from(d.width) / aspect).round() as u32).max(1);
                        }
                        ui.label("Height:");
                        let h = ui.add_enabled(
                            !d.use_view_size,
                            egui::DragValue::new(&mut d.height).range(16..=8192),
                        );
                        ui.label("pixels");
                        ui.end_row();
                        if h.changed() {
                            d.width = ((f64::from(d.height) * aspect).round() as u32).max(1);
                        }
                    });
                ui.add_space(10.0);
                ui.strong("Rendering");
                ui.add_space(4.0);
                ui.checkbox(&mut d.antialias, "Anti-alias");
                ui.checkbox(&mut d.transparent, "Transparent background (PNG)");
                crate::presentation::ok_cancel(ui, "Export...")
            });
        let done = crate::presentation::answer(r);
        match done {
            None => self.exports.d2 = Some(d),
            Some(false) => self.exports.last2d = d,
            Some(true) => {
                self.exports.last2d = d.clone();
                let name = self.export_name("png");
                let Some(path) = crate::filedialog::save(
                    &[("PNG Image", &["png"]), ("JPEG Image", &["jpg", "jpeg"])],
                    &name,
                ) else {
                    return;
                };
                if let Some(rs) = frame.wgpu_render_state() {
                    let (device, queue) = (rs.device.clone(), rs.queue.clone());
                    let ppp = ctx.pixels_per_point();
                    let result = self
                        .render_2d(&device, &queue, &d, ppp, &path)
                        .and_then(|img| encode_image(&img, &path))
                        .and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string()));
                    self.message = Some(match result {
                        Ok(()) => format!("Exported {}.", path.display()),
                        Err(e) => format!("Could not export {}: {e}", path.display()),
                    });
                }
            }
        }
    }

    /// Renders the view for Export 2D Graphic, without selection highlights.
    fn render_2d(
        &mut self,
        device: &egui_wgpu::wgpu::Device,
        queue: &egui_wgpu::wgpu::Queue,
        d: &Export2d,
        ppp: f32,
        path: &std::path::Path,
    ) -> Result<image::RgbaImage, String> {
        let png = !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"));
        let size = [d.width.max(1), d.height.max(1)];
        let max = device.limits().max_texture_dimension_2d;
        let factor = if d.antialias && size.iter().all(|&s| s * 2 <= max) {
            2
        } else {
            1
        };
        let selection = std::mem::take(&mut self.selection);
        self.update_highlight(device);
        let img = self.viewport.render_image(
            device,
            queue,
            &self.camera,
            size.map(|s| s * factor),
            ppp * factor as f32,
            d.transparent && png,
        );
        self.selection = selection;
        self.update_highlight(device);
        Ok(if factor > 1 {
            image::imageops::resize(
                &img,
                size[0],
                size[1],
                image::imageops::FilterType::Triangle,
            )
        } else {
            img
        })
    }

    /// The model's file stem with `ext`.
    fn export_name(&self, ext: &str) -> String {
        let stem = self
            .document
            .as_ref()
            .and_then(|d| d.path.file_stem())
            .map_or_else(|| "Untitled".into(), |s| s.to_string_lossy().into_owned());
        format!("{stem}.{ext}")
    }

    fn export_3d_ui(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.exports.d3.take() else {
            return;
        };
        let model_units = self
            .document
            .as_ref()
            .map_or(Units::Inches, |doc| kernel_units(doc.units));
        let has_selection = !self.selection.items().is_empty();
        let r = mt_ui::Dialog::new("export_3d", "Export 3D Model")
            .width(320.0)
            .show(ctx, |ui| {
                dialog_visuals(ui);
                egui::Grid::new("mt_export_3d_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Format:");
                        egui::ComboBox::from_id_salt("mt_export_format")
                            .selected_text(d.format.label())
                            .show_ui(ui, |ui| {
                                for f in Format3d::ALL {
                                    if ui.selectable_value(&mut d.format, f, f.label()).changed() {
                                        (d.two_sided, d.y_up) = f.defaults();
                                    }
                                }
                            });
                        ui.end_row();
                        if d.format.has_units() {
                            ui.label("Units:");
                            let label = |u: Option<Units>| match u {
                                None => format!("Model Units ({})", model_units.name()),
                                Some(u) => u.name().to_owned(),
                            };
                            egui::ComboBox::from_id_salt("mt_export_units")
                                .selected_text(label(d.units))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut d.units, None, label(None));
                                    for u in Units::ALL {
                                        ui.selectable_value(&mut d.units, Some(u), label(Some(u)));
                                    }
                                });
                            ui.end_row();
                        }
                    });
                ui.add_space(8.0);
                ui.add_enabled(
                    has_selection,
                    egui::Checkbox::new(&mut d.selection_only, "Export only current selection"),
                );
                if d.format.has_ascii() {
                    ui.horizontal(|ui| {
                        ui.label("File format:");
                        ui.radio_value(&mut d.ascii, false, "Binary");
                        ui.radio_value(&mut d.ascii, true, "ASCII");
                    });
                }
                if d.format == Format3d::Other(ExportKind::Vrml) {
                    // SketchUp's VRML wording: the inverse of two-sided faces.
                    let mut ignore = !d.two_sided;
                    ui.checkbox(&mut ignore, "Ignore back of face material");
                    d.two_sided = !ignore;
                } else if d.format.has_two_sided() {
                    ui.checkbox(&mut d.two_sided, "Export two-sided faces");
                }
                if d.format.has_textures() {
                    ui.checkbox(&mut d.textures, "Export texture maps");
                }
                if d.format == Format3d::Other(ExportKind::Vrml) {
                    ui.checkbox(&mut d.y_up, "Use VRML standard orientation");
                } else if d.format.has_y_up() {
                    ui.checkbox(&mut d.y_up, "Swap YZ coordinates (Y is up)");
                }
                ui.add_space(8.0);
                ui.weak(if d.format.keeps_hierarchy() {
                    "Hidden geometry is not exported; groups and components are kept as nodes (meters, Y up)."
                } else if d.format.keeps_objects() {
                    "Hidden geometry is not exported; groups and components become IfcBuildingElementProxy objects on one storey."
                } else {
                    "Hidden geometry is not exported; groups and components are flattened."
                });
                crate::presentation::ok_cancel(ui, "Export...")
            });
        let done = crate::presentation::answer(r);
        match done {
            None => self.exports.d3 = Some(d),
            Some(false) => self.exports.last3d = d,
            Some(true) => {
                self.exports.last3d = d.clone();
                let ext = d.format.extension();
                let name = self.export_name(ext);
                let Some(path) = crate::filedialog::save(&[(d.format.label(), &[ext])], &name)
                else {
                    return;
                };
                let Some(doc) = &self.document else { return };
                let part;
                let model = if d.selection_only && has_selection {
                    part = selection_model(
                        &doc.model,
                        doc.active(),
                        self.selection.items(),
                        self.ctx_xf,
                    );
                    &part
                } else {
                    &doc.model
                };
                self.message = Some(match export_3d(model, &d, model_units, &path) {
                    Ok(()) => format!("Exported {}.", path.display()),
                    Err(e) => format!("Could not export {}: {e}", path.display()),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_and_jpeg_by_extension() {
        let img = image::RgbaImage::from_pixel(4, 3, image::Rgba([10, 200, 30, 255]));
        let png = encode_image(&img, std::path::Path::new("a.png")).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        let jpg = encode_image(&img, std::path::Path::new("a.JPG")).unwrap();
        assert!(jpg.starts_with(&[0xff, 0xd8]));
        let back = image::load_from_memory(&jpg).unwrap();
        assert_eq!((back.width(), back.height()), (4, 3));
    }

    #[test]
    fn export_3d_writes_obj_with_mtl() {
        let dir = std::env::temp_dir().join(format!("mt_export_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut m = mt_kernel::Model::default();
        let p = |x, y| glam::DVec3::new(x, y, 0.0);
        for (a, b) in [
            (p(0., 0.), p(1., 0.)),
            (p(1., 0.), p(1., 1.)),
            (p(1., 1.), p(0., 1.)),
            (p(0., 1.), p(0., 0.)),
        ] {
            mt_kernel::add_line(&mut m.root, a, b);
        }
        let opts = Export3d {
            format: Format3d::Obj,
            two_sided: false,
            ..Default::default()
        };
        let path = dir.join("square.obj");
        export_3d(&m, &opts, Units::Inches, &path).unwrap();
        let obj = std::fs::read_to_string(&path).unwrap();
        assert!(obj.contains("mtllib square.mtl"));
        assert_eq!(obj.lines().filter(|l| l.starts_with("f ")).count(), 2);
        assert!(dir.join("square.mtl").exists());
        let stl = dir.join("square.stl");
        export_3d(
            &m,
            &Export3d {
                format: Format3d::Stl,
                ..opts
            },
            Units::Millimeters,
            &stl,
        )
        .unwrap();
        assert_eq!(std::fs::metadata(&stl).unwrap().len(), 84 + 2 * 50);
        export_3d(
            &m,
            &Export3d {
                format: Format3d::Stl,
                ascii: true,
                ..opts.clone()
            },
            Units::Millimeters,
            &stl,
        )
        .unwrap();
        let text = std::fs::read_to_string(&stl).unwrap();
        assert!(text.starts_with("solid square"));
        assert_eq!(text.matches("facet normal").count(), 2);
        // The other formats write files the importers read back.
        for kind in ExportKind::ALL {
            let path = dir.join(format!("square.{}", kind.extension()));
            export_3d(
                &m,
                &Export3d {
                    format: Format3d::Other(kind),
                    ..opts.clone()
                },
                Units::Inches,
                &path,
            )
            .unwrap();
            let bytes = std::fs::read(&path).unwrap();
            let none = |_: &str| None;
            let back = match kind {
                ExportKind::Fbx => Some(formats::Kind::Fbx),
                ExportKind::Ply => Some(formats::Kind::Ply),
                ExportKind::ThreeMf => Some(formats::Kind::ThreeMf),
                ExportKind::ThreeDs => Some(formats::Kind::ThreeDs),
                ExportKind::Vrml | ExportKind::Kmz | ExportKind::Usdz => None,
                ExportKind::Ifc => {
                    let (back, _) = formats::ifc::import(&bytes, &Default::default()).unwrap();
                    assert_eq!(crate::import::count_faces(&back, &back.root), 1);
                    continue;
                }
            };
            match back {
                Some(k) => {
                    let (mesh, _) = formats::read_mesh(k, &bytes, &none, true).unwrap();
                    assert_eq!(mesh.polygons.len(), 2, "{kind:?}");
                }
                None => assert!(!bytes.is_empty()),
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn selection_model_keeps_only_the_selection() {
        let mut m = mt_kernel::Model::default();
        for x0 in [0.0, 10.0] {
            let c = [(0., 0.), (1., 0.), (1., 1.), (0., 1.)]
                .map(|(x, y)| glam::DVec3::new(x0 + x, y, 0.0));
            for i in 0..4 {
                mt_kernel::add_line(&mut m.root, c[i], c[(i + 1) % 4]);
            }
        }
        assert_eq!(m.root.faces.len(), 2);
        let far = m
            .root
            .faces
            .iter()
            .find(|(_, f)| m.root.loop_points(&f.outer)[0].x > 5.0)
            .map(|(id, _)| id)
            .unwrap();
        let xf = glam::DMat4::from_translation(glam::DVec3::Z);
        let part = selection_model(&m, &m.root, &[Owner::Face(far)], xf);
        assert_eq!(part.root.faces.len(), 1);
        assert_eq!(part.root.edges.len(), 4);
        // In model coordinates (the open context's transform applied).
        assert!(
            part.root
                .vertices
                .values()
                .all(|v| v.pos.x > 5.0 && v.pos.z == 1.0)
        );
        assert_eq!(mt_kernel::mesh_export::triangles(&part).len(), 2);
    }
}
