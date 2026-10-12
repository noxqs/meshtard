//! Window > Model Info: a window with SketchUp's pages, alphabetically: Animation,
//! Components, Credits, Dimensions, File, Geo-location, Statistics, Text, Units.
//!
//! Units changes apply at once, each one undo step, and are saved in the .skp's
//! `UnitsOptions` (area and volume units too; their codes are unverified). Length
//! snapping rounds inferred lengths (`mt_infer::Engine::length_snap`), angle
//! snapping Rotate's and the Protractor's angles near multiples of the snap angle,
//! and angle precision every angle shown in the Measurements box.
//!
//! Components (fades, show component axes), Dimensions, Text and Geo-location
//! (FEATURES.md 61) are model data that is not saved in .skp files yet. Dimensions
//! and Text set the style of new labels; Update Selected gives the selected ones
//! the current style. Label fonts are the UI font at the chosen size (no font
//! choice). Geo-location's latitude and longitude are the Shadows settings'; the
//! north angle turns the sun.

use eframe::egui;
use mt_infer::units::{LengthFormat, LengthUnit, UnitsOptions, parse_length};
use mt_kernel::purge;

use crate::App;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Page {
    Animation,
    Components,
    Credits,
    Dimensions,
    File,
    GeoLocation,
    Rendering,
    Statistics,
    Text,
    #[default]
    Units,
}

impl Page {
    const ALL: [Self; 10] = [
        Self::Animation,
        Self::Components,
        Self::Credits,
        Self::Dimensions,
        Self::File,
        Self::GeoLocation,
        Self::Rendering,
        Self::Statistics,
        Self::Text,
        Self::Units,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Animation => "Animation",
            Self::Components => "Components",
            Self::Credits => "Credits",
            Self::Dimensions => "Dimensions",
            Self::File => "File",
            Self::GeoLocation => "Geo-location",
            Self::Rendering => "Rendering",
            Self::Statistics => "Statistics",
            Self::Text => "Text",
            Self::Units => "Units",
        }
    }
}

/// The open Model Info window.
#[derive(Debug, Clone, Default)]
pub(crate) struct ModelInfo {
    page: Page,
    /// The snap length field while edited.
    snap_text: Option<String>,
    /// File page: size and saved version, read when the page opens.
    file: Option<(Option<u64>, String)>,
    /// Geo-location's latitude and longitude fields while edited.
    geo_text: Option<(String, String)>,
}

/// Font sizes offered for dimension and text labels (points; SketchUp opens a font
/// dialog instead).
const FONT_SIZES: [f32; 14] = [
    8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 16.0, 18.0, 20.0, 24.0, 28.0, 36.0, 48.0,
];

/// A font size choice; returns the size picked.
fn font_size_combo(ui: &mut egui::Ui, id: &str, size: f32) -> f32 {
    let mut s = size;
    egui::ComboBox::from_id_salt(id)
        .selected_text(format!("{s} pt"))
        .show_ui(ui, |ui| {
            for f in FONT_SIZES {
                ui.selectable_value(&mut s, f, format!("{f} pt"));
            }
        });
    s
}

/// An endpoint choice; returns the one picked.
fn endpoint_combo(
    ui: &mut egui::Ui,
    id: &str,
    e: mt_kernel::model_data::Endpoint,
) -> mt_kernel::model_data::Endpoint {
    let mut e = e;
    egui::ComboBox::from_id_salt(id)
        .selected_text(e.label())
        .show_ui(ui, |ui| {
            for x in mt_kernel::model_data::Endpoint::ALL {
                ui.selectable_value(&mut e, x, x.label());
            }
        });
    e
}

/// SketchUp's angle snapping choices (unverified).
const SNAP_ANGLES: [f64; 10] = [0.5, 1.0, 5.0, 7.5, 10.0, 15.0, 22.5, 30.0, 45.0, 90.0];

/// Text of a precision choice, e.g. `0.00mm` or `1/16"`.
pub(crate) fn precision_label(format: LengthFormat, unit: LengthUnit, p: u8) -> String {
    if format.is_fractional() {
        if p == 0 {
            "1\"".into()
        } else {
            format!("1/{}\"", 1u32 << p)
        }
    } else {
        let unit = format.fixed_unit().unwrap_or(unit);
        let zeros = "0".repeat(usize::from(p));
        let dot = if p > 0 { "." } else { "" };
        format!("0{dot}{zeros}{}", unit.suffix())
    }
}

/// Units after choosing `format`: every format but Decimal fixes the unit.
pub(crate) fn with_format(
    unit: LengthUnit,
    opts: UnitsOptions,
    format: LengthFormat,
) -> (LengthUnit, UnitsOptions) {
    let unit = format.fixed_unit().unwrap_or(unit);
    // Fractional precision counts halvings up to 1/64; decimals up to 6 places.
    let precision = if format.is_fractional() != opts.format.is_fractional() {
        if format.is_fractional() { 4 } else { 2 }
    } else {
        opts.precision
    };
    (
        unit,
        UnitsOptions {
            format,
            precision,
            ..opts
        },
    )
}

impl App {
    pub(crate) fn open_model_info(&mut self) {
        if self.model_info.is_none() {
            self.model_info = Some(ModelInfo::default());
        }
    }

    /// Opens Model Info on `page` (the status bar's geo-location pin).
    pub(crate) fn open_model_info_at(&mut self, page: Page) {
        let info = self.model_info.get_or_insert_with(ModelInfo::default);
        info.page = page;
        info.file = None;
    }

    pub(crate) fn model_info_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        // The open model's display options for every length shown.
        mt_infer::units::set_display(self.document.as_ref().map(|d| d.unit_options));
        // Length snapping for the inference engine (in the model's units).
        self.engine.length_snap = self
            .document
            .as_ref()
            .map(|d| d.unit_options)
            .filter(|o| o.length_snapping && o.snap_length > 0.0)
            .map(|o| o.snap_length);
        let Some(mut info) = self.model_info.take() else {
            return;
        };
        let mut rebuild = false;
        // A fixed-height body: the page list on the left, the page (scrolling when
        // long) on the right.
        let height = (ctx.content_rect().height() - 160.0).clamp(200.0, 380.0);
        let r = mt_ui::Dialog::new("model_info", "Model Info")
            .width(520.0)
            .modal(false)
            .show(ctx, |ui| {
                crate::export::dialog_visuals(ui);
                let size = egui::vec2(ui.available_width(), height);
                let layout = egui::Layout::left_to_right(egui::Align::Min);
                ui.allocate_ui_with_layout(size, layout, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(110.0);
                        for page in Page::ALL {
                            if ui
                                .selectable_label(info.page == page, page.label())
                                .clicked()
                            {
                                info.page = page;
                                info.file = None;
                            }
                        }
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("mt_mi_page")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                rebuild |= self.model_info_page(ui, &mut info);
                            });
                    });
                });
            });
        if rebuild && let Some(rs) = frame.wgpu_render_state() {
            let device = rs.device.clone();
            self.rebuild(&device);
        }
        if !r.closed {
            self.model_info = Some(info);
        }
    }

    /// One page. Returns whether the drawn scene must be rebuilt.
    fn model_info_page(&mut self, ui: &mut egui::Ui, info: &mut ModelInfo) -> bool {
        let Some(doc) = self.document.as_mut() else {
            ui.label("No model is open.");
            return false;
        };
        match info.page {
            Page::Units => {
                let (mut unit, mut opts) = (doc.units, doc.unit_options);
                ui.strong("Length Units");
                egui::Grid::new("mt_mi_length")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label("Format");
                        let mut format = opts.format;
                        egui::ComboBox::from_id_salt("mt_mi_format")
                            .selected_text(format.label())
                            .show_ui(ui, |ui| {
                                for f in LengthFormat::ALL {
                                    ui.selectable_value(&mut format, f, f.label());
                                }
                            });
                        if format != opts.format {
                            (unit, opts) = with_format(unit, opts, format);
                        }
                        ui.end_row();
                        ui.label("Unit");
                        ui.add_enabled_ui(opts.format == LengthFormat::Decimal, |ui| {
                            egui::ComboBox::from_id_salt("mt_mi_unit")
                                .selected_text(unit.label())
                                .show_ui(ui, |ui| {
                                    for u in LengthUnit::ALL {
                                        ui.selectable_value(&mut unit, u, u.label());
                                    }
                                });
                        });
                        ui.end_row();
                        ui.label("Precision");
                        egui::ComboBox::from_id_salt("mt_mi_precision")
                            .selected_text(precision_label(opts.format, unit, opts.precision))
                            .show_ui(ui, |ui| {
                                for p in 0..=6 {
                                    let text = precision_label(opts.format, unit, p);
                                    ui.selectable_value(&mut opts.precision, p, text);
                                }
                            });
                        ui.end_row();
                    });
                ui.checkbox(&mut opts.length_snapping, "Enable length snapping");
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    let text = info
                        .snap_text
                        .get_or_insert_with(|| unit.format(opts.snap_length));
                    let r = ui.add_enabled(
                        opts.length_snapping,
                        egui::TextEdit::singleline(text).desired_width(100.0),
                    );
                    if r.lost_focus() {
                        if let Some(v) = parse_length(text, unit).filter(|v| *v > 0.0) {
                            opts.snap_length = v;
                        }
                        info.snap_text = None;
                    }
                });
                ui.add_space(8.0);
                ui.strong("Angle Units");
                egui::Grid::new("mt_mi_angle")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label("Precision");
                        let shown = |p: u8| {
                            let zeros = "0".repeat(usize::from(p));
                            if p == 0 {
                                "0".into()
                            } else {
                                format!("0.{zeros}")
                            }
                        };
                        egui::ComboBox::from_id_salt("mt_mi_angle_precision")
                            .selected_text(shown(opts.angle_precision))
                            .show_ui(ui, |ui| {
                                for p in 0..=3 {
                                    ui.selectable_value(&mut opts.angle_precision, p, shown(p));
                                }
                            });
                        ui.end_row();
                    });
                ui.checkbox(&mut opts.angle_snapping, "Enable angle snapping");
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    ui.add_enabled_ui(opts.angle_snapping, |ui| {
                        egui::ComboBox::from_id_salt("mt_mi_snap_angle")
                            .selected_text(format!("{}", opts.snap_angle))
                            .show_ui(ui, |ui| {
                                for a in SNAP_ANGLES {
                                    ui.selectable_value(&mut opts.snap_angle, a, format!("{a}"));
                                }
                            });
                    });
                });
                ui.add_space(8.0);
                ui.strong("Area and Volume Units");
                egui::Grid::new("mt_mi_area").num_columns(2).show(ui, |ui| {
                    ui.label("Area");
                    egui::ComboBox::from_id_salt("mt_mi_area_unit")
                        .selected_text(opts.area_unit.label())
                        .show_ui(ui, |ui| {
                            for u in mt_infer::units::AreaUnit::ALL {
                                ui.selectable_value(&mut opts.area_unit, u, u.label());
                            }
                        });
                    ui.end_row();
                    ui.label("Area precision");
                    egui::ComboBox::from_id_salt("mt_mi_area_precision")
                        .selected_text(decimals(opts.area_precision))
                        .show_ui(ui, |ui| {
                            for p in 0..=6 {
                                ui.selectable_value(&mut opts.area_precision, p, decimals(p));
                            }
                        });
                    ui.end_row();
                    ui.label("Volume");
                    egui::ComboBox::from_id_salt("mt_mi_volume_unit")
                        .selected_text(opts.volume_unit.label())
                        .show_ui(ui, |ui| {
                            for u in mt_infer::units::VolumeUnit::ALL {
                                ui.selectable_value(&mut opts.volume_unit, u, u.label());
                            }
                        });
                    ui.end_row();
                    ui.label("Volume precision");
                    egui::ComboBox::from_id_salt("mt_mi_volume_precision")
                        .selected_text(decimals(opts.volume_precision))
                        .show_ui(ui, |ui| {
                            for p in 0..=6 {
                                ui.selectable_value(&mut opts.volume_precision, p, decimals(p));
                            }
                        });
                    ui.end_row();
                });
                ui.add_space(8.0);
                ui.strong("Display");
                let mut show_units = !opts.suppress_units;
                ui.add_enabled(
                    opts.format == LengthFormat::Decimal,
                    egui::Checkbox::new(&mut show_units, "Display units format"),
                );
                opts.suppress_units = !show_units;
                ui.add_enabled(
                    opts.format == LengthFormat::Architectural,
                    egui::Checkbox::new(&mut opts.force_inch_display, "Force display of 0\""),
                );
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(
                        "Saved in the .skp file. Length snapping rounds inferred lengths; \
                         angle snapping applies near multiples of the snap angle.",
                    )
                    .small()
                    .weak(),
                );
                if doc.set_units(unit, opts) {
                    info.snap_text = None;
                    // Dimension labels show the new format.
                    return true;
                }
                false
            }
            Page::Statistics => {
                let s = purge::statistics(&doc.model);
                ui.strong("Entire model");
                egui::Grid::new("mt_mi_stats")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        for (name, n) in [
                            ("Edges", s.edges),
                            ("Faces", s.faces),
                            ("Component Instances", s.component_instances),
                            ("Groups", s.groups),
                            ("Guides", s.guides),
                            ("Dimensions", s.dimensions),
                            ("Text", s.texts),
                            ("Section Planes", s.section_planes),
                            ("Component Definitions", s.component_definitions),
                            ("Materials", s.materials),
                            ("Tags", s.tags),
                        ] {
                            ui.label(name);
                            ui.label(n.to_string());
                            ui.end_row();
                        }
                    });
                ui.add_space(8.0);
                if ui.button("Fix Problems").clicked() {
                    let mut report = crate::fix_problems::Report::default();
                    let changed = doc.edit_whole(|model| {
                        report = crate::fix_problems::fix_model(model);
                        report.fixed > 0
                    });
                    self.message = Some(report.message());
                    if changed {
                        self.selection.clear();
                        return true;
                    }
                }
                if ui.button("Purge Unused").clicked() {
                    let mut purged = purge::Purged::default();
                    let changed = doc.edit_whole(|model| {
                        purged = purge::purge_all(model);
                        purged.any()
                    });
                    self.message = Some(format!(
                        "Purged {} component definitions, {} materials and {} tags.",
                        purged.definitions, purged.materials, purged.tags
                    ));
                    if changed {
                        // Indices into the purged lists may be stale.
                        self.current_material = None;
                        self.current_component = None;
                        self.active_tag = 0;
                        return true;
                    }
                }
                false
            }
            Page::File => {
                let path = doc.path.clone();
                let has_file = doc.has_file;
                let (size, version) = info.file.get_or_insert_with(|| {
                    if !has_file {
                        return (None, "Not saved yet".into());
                    }
                    let size = std::fs::metadata(&path).ok().map(|m| m.len());
                    let head = std::fs::File::open(&path).ok().and_then(|f| {
                        use std::io::Read;
                        let mut b = Vec::new();
                        f.take(512).read_to_end(&mut b).ok().map(|_| b)
                    });
                    (size, head.map_or("unknown".into(), |h| mt_skp::version(&h)))
                });
                egui::Grid::new("mt_mi_file").num_columns(2).show(ui, |ui| {
                    ui.label("Name");
                    ui.label(
                        path.file_name()
                            .map_or("Untitled".into(), |n| n.to_string_lossy()),
                    );
                    ui.end_row();
                    ui.label("Location");
                    ui.label(if has_file {
                        path.parent()
                            .map_or(String::new(), |p| p.display().to_string())
                    } else {
                        String::new()
                    });
                    ui.end_row();
                    ui.label("Size");
                    ui.label(size.map_or(String::new(), |s| format!("{} KB", s.div_ceil(1024))));
                    ui.end_row();
                    ui.label("Version saved");
                    ui.label(version.as_str());
                    ui.end_row();
                    ui.label("Saves as");
                    ui.label(".skp 2017");
                    ui.end_row();
                });
                false
            }
            Page::Credits => {
                ui.strong("Model Author");
                ui.label("No author is claimed for this model.");
                ui.add_space(6.0);
                ui.strong("Component Authors");
                let n = doc
                    .model
                    .definitions
                    .values()
                    .filter(|d| !d.is_group)
                    .count();
                ui.label(format!(
                    "{n} component definitions; no author credits are stored."
                ));
                false
            }
            Page::Animation => {
                let a = &mut self.animation;
                ui.strong("Scene Transitions");
                ui.checkbox(&mut a.transitions, "Enable scene transitions");
                ui.horizontal(|ui| {
                    ui.add_enabled(
                        a.transitions,
                        egui::DragValue::new(&mut a.transition_secs)
                            .range(0.0..=60.0)
                            .speed(0.1),
                    );
                    ui.label("seconds");
                });
                ui.strong("Scene Delay");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut a.delay_secs)
                            .range(0.0..=60.0)
                            .speed(0.1),
                    );
                    ui.label("seconds");
                });
                false
            }
            Page::Components => self.components_page(ui),
            Page::Dimensions => self.dimensions_page(ui),
            Page::Text => self.text_page(ui),
            Page::GeoLocation => self.geo_page(ui, info),
            Page::Rendering => {
                // SketchUp's Rendering page (help: "Use Anti-Aliased Textures"); its
                // other options are not built.
                let r = &mut doc.model.rendering;
                ui.checkbox(&mut r.antialiased_textures, "Use Anti-Aliased Textures")
                    .on_hover_text("Smooth textures (on) or sharp texels (off)");
                false
            }
        }
    }
}

/// `0`, `0.0`, `0.00`... for a number of decimals.
fn decimals(p: u8) -> String {
    if p == 0 {
        "0".into()
    } else {
        format!("0.{}", "0".repeat(usize::from(p)))
    }
}

impl App {
    /// Model Info > Components: fade (or hide) the rest of the model and similar
    /// components while editing one, and show component axes. Slider drags are not
    /// undo steps (display settings, as View > Component Edit).
    fn components_page(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        let m = &mut doc.model;
        let before = (
            m.edit_fade,
            m.style.hide_similar_components,
            m.style.hide_rest_of_model,
        );
        ui.strong("Component/Group Editing");
        egui::Grid::new("mt_mi_components")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Fade similar");
                ui.add(egui::Slider::new(&mut m.edit_fade.similar, 0.0..=1.0).show_value(false));
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut m.style.hide_similar_components, "Hide");
                ui.end_row();
                ui.label("Fade rest of model");
                ui.add(egui::Slider::new(&mut m.edit_fade.rest, 0.0..=1.0).show_value(false));
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut m.style.hide_rest_of_model, "Hide");
                ui.end_row();
            });
        ui.add_space(8.0);
        ui.strong("Component Axes");
        ui.checkbox(&mut m.edit_fade.show_axes, "Show component axes");
        ui.label(
            egui::RichText::new("Kept in the model, not saved in the .skp file.")
                .small()
                .weak(),
        );
        before
            != (
                m.edit_fade,
                m.style.hide_similar_components,
                m.style.hide_rest_of_model,
            )
    }

    /// Selected dimensions or texts of the open context.
    fn selected_annotations(&self, dims: bool) -> Vec<usize> {
        use crate::document::Owner;
        self.selection
            .items()
            .iter()
            .filter_map(|o| match (*o, dims) {
                (Owner::Dimension(i), true) | (Owner::Text(i), false) => Some(i),
                _ => None,
            })
            .collect()
    }

    /// Model Info > Dimensions: text size and alignment, endpoints; Select All
    /// Dimensions and Update Selected Dimensions.
    fn dimensions_page(&mut self, ui: &mut egui::Ui) -> bool {
        use mt_kernel::model_data::DimensionAlign;
        let selected = self.selected_annotations(true);
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        let mut s = doc.model.dimension_style;
        ui.strong("Text");
        ui.horizontal(|ui| {
            ui.label("Size");
            s.font_size = font_size_combo(ui, "mt_mi_dim_font", s.font_size);
        });
        ui.add_space(6.0);
        ui.strong("Leader Lines");
        ui.horizontal(|ui| {
            ui.label("Endpoints");
            s.endpoint = endpoint_combo(ui, "mt_mi_dim_end", s.endpoint);
        });
        ui.add_space(6.0);
        ui.strong("Dimension");
        ui.radio_value(&mut s.align, DimensionAlign::Screen, "Align to screen");
        let along = s.align != DimensionAlign::Screen;
        ui.horizontal(|ui| {
            if ui.radio(along, "Align to dimension line").clicked() && !along {
                s.align = DimensionAlign::Above;
            }
            ui.add_enabled_ui(along, |ui| {
                egui::ComboBox::from_id_salt("mt_mi_dim_align")
                    .selected_text(match s.align {
                        DimensionAlign::Centered => "Centered",
                        DimensionAlign::Outside => "Outside",
                        _ => "Above",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut s.align, DimensionAlign::Above, "Above");
                        ui.selectable_value(&mut s.align, DimensionAlign::Centered, "Centered");
                        ui.selectable_value(&mut s.align, DimensionAlign::Outside, "Outside");
                    });
            });
        });
        let mut rebuild = false;
        if s != doc.model.dimension_style {
            rebuild |= doc.edit_model(|m, _| {
                m.dimension_style = s;
                true
            });
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Select All Dimensions").clicked() {
                let n = doc.active().dimensions.len();
                self.selection.apply(
                    crate::selection::Mode::Replace,
                    (0..n).map(crate::document::Owner::Dimension),
                );
                rebuild = true;
            }
            if ui
                .add_enabled(
                    !selected.is_empty(),
                    egui::Button::new("Update Selected Dimensions"),
                )
                .clicked()
            {
                rebuild |= doc.edit(|ents| {
                    let mut any = false;
                    for &i in &selected {
                        if let Some(d) = ents.dimensions.get_mut(i) {
                            any |= d.style != Some(s);
                            d.style = Some(s);
                        }
                    }
                    any
                });
            }
        });
        ui.label(
            egui::RichText::new(
                "Dimensions without their own style follow these settings. Kept in the \
                 model, not saved in the .skp file.",
            )
            .small()
            .weak(),
        );
        rebuild
    }

    /// Model Info > Text: size and color, leader type and endpoint; Select All Screen
    /// Text and Update Selected Text.
    fn text_page(&mut self, ui: &mut egui::Ui) -> bool {
        use mt_kernel::model_data::LeaderKind;
        let selected = self.selected_annotations(false);
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        let mut s = doc.model.text_style;
        ui.strong("Text");
        ui.horizontal(|ui| {
            ui.label("Size");
            s.font_size = font_size_combo(ui, "mt_mi_text_font", s.font_size);
            ui.label("Color");
            ui.color_edit_button_srgb(&mut s.color);
        });
        ui.add_space(6.0);
        ui.strong("Leader Lines");
        egui::Grid::new("mt_mi_leader")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Endpoint");
                s.endpoint = endpoint_combo(ui, "mt_mi_text_end", s.endpoint);
                ui.end_row();
                ui.label("Leader");
                egui::ComboBox::from_id_salt("mt_mi_text_leader")
                    .selected_text(s.leader.label())
                    .show_ui(ui, |ui| {
                        for k in LeaderKind::ALL {
                            ui.selectable_value(&mut s.leader, k, k.label());
                        }
                    });
                ui.end_row();
            });
        let mut rebuild = false;
        if s != doc.model.text_style {
            rebuild |= doc.edit_model(|m, _| {
                m.text_style = s;
                true
            });
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Select All Screen Text").clicked() {
                let items: Vec<_> = (doc.active().texts.iter().enumerate())
                    .filter(|(_, t)| t.screen)
                    .map(|(i, _)| crate::document::Owner::Text(i))
                    .collect();
                self.selection.apply(crate::selection::Mode::Replace, items);
                rebuild = true;
            }
            if ui
                .add_enabled(
                    !selected.is_empty(),
                    egui::Button::new("Update Selected Text"),
                )
                .clicked()
            {
                rebuild |= doc.edit(|ents| {
                    let mut any = false;
                    for &i in &selected {
                        if let Some(t) = ents.texts.get_mut(i) {
                            any |= t.style != Some(s);
                            t.style = Some(s);
                        }
                    }
                    any
                });
            }
        });
        ui.label(
            egui::RichText::new(
                "Texts without their own style follow these settings. Kept in the model, \
                 not saved in the .skp file.",
            )
            .small()
            .weak(),
        );
        rebuild
    }

    /// Model Info > Geo-location: manual latitude and longitude (the Shadows
    /// settings') and the north angle, used by the sun. Clear Location forgets it.
    fn geo_page(&mut self, ui: &mut egui::Ui, info: &mut ModelInfo) -> bool {
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        let m = &mut doc.model;
        ui.label(if m.geo.located {
            "Geo-located (set manually)"
        } else {
            "Not Geo-located"
        });
        ui.add_space(6.0);
        ui.strong("Set Manual Location");
        let (lat, lon) = info.geo_text.get_or_insert_with(|| {
            (
                format!("{:.4}", m.shadows.latitude),
                format!("{:.4}", m.shadows.longitude),
            )
        });
        let mut commit = false;
        egui::Grid::new("mt_mi_geo").num_columns(2).show(ui, |ui| {
            ui.label("Latitude");
            commit |= ui
                .add(egui::TextEdit::singleline(lat).desired_width(100.0))
                .lost_focus();
            ui.end_row();
            ui.label("Longitude");
            commit |= ui
                .add(egui::TextEdit::singleline(lon).desired_width(100.0))
                .lost_focus();
            ui.end_row();
        });
        let mut north = m.geo.north_angle;
        ui.horizontal(|ui| {
            ui.label("North angle");
            ui.add(
                egui::DragValue::new(&mut north)
                    .range(0.0..=360.0)
                    .speed(0.5)
                    .suffix("°"),
            );
        });
        let mut changed = false;
        if commit {
            let parse = |s: &str, max: f64| {
                s.trim()
                    .replace(',', ".")
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && v.abs() <= max)
            };
            let (la, lo) = (parse(lat, 90.0), parse(lon, 180.0));
            if let (Some(la), Some(lo)) = (la, lo)
                && (la, lo) != (m.shadows.latitude, m.shadows.longitude)
            {
                m.shadows.latitude = la;
                m.shadows.longitude = lo;
                m.geo.located = true;
                changed = true;
            }
            info.geo_text = None;
        }
        if north != m.geo.north_angle {
            m.geo.north_angle = north.rem_euclid(360.0);
            changed = true;
        }
        if ui
            .add_enabled(m.geo.located, egui::Button::new("Clear Location"))
            .clicked()
        {
            let d = mt_kernel::ShadowSettings::default();
            m.shadows.latitude = d.latitude;
            m.shadows.longitude = d.longitude;
            m.geo = Default::default();
            info.geo_text = None;
            changed = true;
        }
        ui.label(
            egui::RichText::new(
                "The location and north angle place the sun for shadows. Kept in the \
                 model, not saved in the .skp file; no map import.",
            )
            .small()
            .weak(),
        );
        if changed {
            self.apply_shadows();
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precision_labels() {
        let d = LengthFormat::Decimal;
        assert_eq!(precision_label(d, LengthUnit::Millimeter, 0), "0mm");
        assert_eq!(precision_label(d, LengthUnit::Meter, 3), "0.000m");
        assert_eq!(
            precision_label(LengthFormat::Engineering, LengthUnit::Meter, 1),
            "0.0'"
        );
        assert_eq!(
            precision_label(LengthFormat::Architectural, LengthUnit::Inch, 4),
            "1/16\""
        );
        assert_eq!(
            precision_label(LengthFormat::Fractional, LengthUnit::Inch, 0),
            "1\""
        );
    }

    #[test]
    fn choosing_a_format_fixes_the_unit() {
        let opts = UnitsOptions::default();
        let (u, o) = with_format(LengthUnit::Millimeter, opts, LengthFormat::Architectural);
        assert_eq!(
            (u, o.format, o.precision),
            (LengthUnit::Inch, LengthFormat::Architectural, 4)
        );
        let (u, o) = with_format(u, o, LengthFormat::Engineering);
        assert_eq!((u, o.precision), (LengthUnit::Foot, 2));
        let (u, _) = with_format(LengthUnit::Meter, opts, LengthFormat::Decimal);
        assert_eq!(u, LengthUnit::Meter);
    }
}
