//! Window > Model Info: a window with SketchUp's pages, alphabetically: Animation,
//! Credits, Dimensions, File, Geo-location, Statistics, Text, Units.
//!
//! Units changes apply at once, each one undo step. They stay in the model while it
//! is open but are not written to the .skp (the writer keeps its template's units),
//! so a saved file reopens in its template unit. Dimensions and Text have no font
//! settings yet (labels use the UI font): those pages show SketchUp's controls
//! disabled. Geo-location is read only (no geo-location data is read).

use eframe::egui;
use mt_infer::units::{LengthFormat, LengthUnit, UnitsOptions, parse_length};
use mt_kernel::purge;

use crate::App;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Page {
    Animation,
    Credits,
    Dimensions,
    File,
    GeoLocation,
    Statistics,
    Text,
    #[default]
    Units,
}

impl Page {
    const ALL: [Self; 8] = [
        Self::Animation,
        Self::Credits,
        Self::Dimensions,
        Self::File,
        Self::GeoLocation,
        Self::Statistics,
        Self::Text,
        Self::Units,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Animation => "Animation",
            Self::Credits => "Credits",
            Self::Dimensions => "Dimensions",
            Self::File => "File",
            Self::GeoLocation => "Geo-location",
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

    pub(crate) fn model_info_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        // The open model's display options for every length shown.
        mt_infer::units::set_display(self.document.as_ref().map(|d| d.unit_options));
        let Some(mut info) = self.model_info.take() else {
            return;
        };
        let mut open = true;
        let mut rebuild = false;
        egui::Window::new("Model Info")
            .id(egui::Id::new("mt_model_info"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([520.0, 380.0])
            .show(ctx, |ui| {
                crate::export::dialog_visuals(ui);
                ui.horizontal_top(|ui| {
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
                        ui.set_min_width(360.0);
                        rebuild |= self.model_info_page(ui, &mut info);
                    });
                });
            });
        if rebuild && let Some(rs) = frame.wgpu_render_state() {
            let device = rs.device.clone();
            self.rebuild(&device);
        }
        if open {
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
                ui.label(
                    egui::RichText::new(
                        "Units are kept while the model is open but not saved in the .skp \
                         file. Snapping values are stored but not yet used by the tools.",
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
            Page::Dimensions | Page::Text => {
                ui.add_enabled_ui(false, |ui| {
                    ui.horizontal(|ui| {
                        let _ = ui.button("Fonts...");
                        ui.label("Default font, 12 pt");
                    });
                    if info.page == Page::Dimensions {
                        ui.label("Leader Lines: Endpoints: Slash");
                        ui.label("Dimension: Align to screen");
                    } else {
                        ui.label("Screen Text / Leader Text: default font");
                        ui.label("Leader Lines: Endpoints: Closed Arrow");
                    }
                });
                ui.label(
                    egui::RichText::new("Font settings are not supported yet.")
                        .small()
                        .weak(),
                );
                false
            }
            Page::GeoLocation => {
                ui.label("Not Geo-located");
                ui.label(
                    egui::RichText::new("Geo-location is read only and not read from files yet.")
                        .small()
                        .weak(),
                );
                false
            }
        }
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
