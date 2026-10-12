//! Window > Preferences pages Template, Workspace, Accessibility, Graphics,
//! Compatibility and Files (FEATURES.md 43; option lists from SketchUp's help pages,
//! layouts and wording unverified), their settings in `preferences.toml`, and the
//! Files page's Import / Export of all preferences in one file.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use eframe::egui;

use crate::App;
use crate::filedialog::{Folder, Folders};

/// Preferences > Accessibility: the colors that can be changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Swatch {
    X,
    Y,
    Z,
    Parallel,
    Tangent,
}

impl Swatch {
    pub(crate) const ALL: [Swatch; 5] = [
        Swatch::X,
        Swatch::Y,
        Swatch::Z,
        Swatch::Parallel,
        Swatch::Tangent,
    ];

    fn label(self) -> &'static str {
        match self {
            Swatch::X => "X axis",
            Swatch::Y => "Y axis",
            Swatch::Z => "Z axis",
            Swatch::Parallel => "Parallel / Perpendicular",
            Swatch::Tangent => "Tangent",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Swatch::X => "x",
            Swatch::Y => "y",
            Swatch::Z => "z",
            Swatch::Parallel => "parallel",
            Swatch::Tangent => "tangent",
        }
    }

    /// The color the inference engine gives this kind (`mt_infer`), which the overlay
    /// replaces.
    pub(crate) fn inference_default(self) -> [u8; 3] {
        use mt_infer::{Axis, Kind};
        match self {
            Swatch::X => Axis::Red.color(),
            Swatch::Y => Axis::Green.color(),
            Swatch::Z => Axis::Blue.color(),
            Swatch::Parallel => Kind::Parallel.color(),
            Swatch::Tangent => Kind::Tangent.color(),
        }
    }

    fn index(self) -> usize {
        Swatch::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
}

/// User colors per [`Swatch`] (`None`: the default).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Palette(pub [Option<[u8; 3]>; 5]);

impl Palette {
    pub(crate) fn get(&self, s: Swatch) -> Option<[u8; 3]> {
        self.0[s.index()]
    }

    /// The color drawn for `s`.
    pub(crate) fn color(&self, s: Swatch) -> [u8; 3] {
        self.get(s).unwrap_or_else(|| s.inference_default())
    }

    /// An inference overlay color with the user's colors: the defaults of changed
    /// swatches are replaced (the midpoint marker shares the tangent's cyan).
    pub(crate) fn remap(&self, c: [u8; 3]) -> [u8; 3] {
        Swatch::ALL
            .into_iter()
            .find(|s| s.inference_default() == c)
            .and_then(|s| self.get(s))
            .unwrap_or(c)
    }
}

static PALETTE: RwLock<Palette> = RwLock::new(Palette([None; 5]));

/// The overlay's color for inference color `c` (see [`Palette::remap`]).
pub(crate) fn remap(c: [u8; 3]) -> [u8; 3] {
    PALETTE.read().map_or(c, |p| p.remap(c))
}

fn set_palette(p: Palette) {
    if let Ok(mut g) = PALETTE.write() {
        *g = p;
    }
}

/// Settings of the pages here.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct More {
    /// Workspace: large tool icons (on by default; unverified).
    pub large_icons: bool,
    /// Accessibility.
    pub palette: Palette,
    /// Graphics: multisample anti-aliasing samples (1 = off; 4 is meshtard's own
    /// default, SketchUp's is unverified).
    pub msaa: u32,
    /// Compatibility (help: both off by default).
    pub bbox_highlight: bool,
    pub invert_wheel: bool,
    /// Files.
    pub folders: Folders,
    /// Drawing > Lasso Direction: Clockwise (help: the default) makes clockwise
    /// lassos select what they touch; Counterclockwise swaps the two.
    pub lasso_clockwise_crossing: bool,
    /// Drawing > Show Move Tool Rotation Grips (on by default; unverified).
    pub rotation_grips: bool,
    /// Graphics: the GPU to draw with (adapter name), `None` = the system's choice.
    /// Takes effect after a restart.
    pub gpu: Option<String>,
    /// Applications: the image editor for Materials > Edit Texture Image; `None` =
    /// the system's default application.
    pub image_editor: Option<PathBuf>,
}

impl Default for More {
    fn default() -> Self {
        Self {
            large_icons: true,
            palette: Palette::default(),
            msaa: mt_render::viewport::DEFAULT_SAMPLES,
            bbox_highlight: false,
            invert_wheel: false,
            folders: Folders::default(),
            lasso_clockwise_crossing: true,
            rotation_grips: true,
            gpu: None,
            image_editor: None,
        }
    }
}

fn rgb_value(c: [u8; 3]) -> toml::Value {
    toml::Value::Array(
        c.iter()
            .map(|&v| toml::Value::Integer(i64::from(v)))
            .collect(),
    )
}

fn rgb_from(v: &toml::Value) -> Option<[u8; 3]> {
    let a = v.as_array()?;
    let mut out = [0u8; 3];
    if a.len() != 3 {
        return None;
    }
    for (o, x) in out.iter_mut().zip(a) {
        *o = u8::try_from(x.as_integer()?).ok()?;
    }
    Some(out)
}

impl More {
    /// Writes the pages' sections into `t`.
    pub(crate) fn write(&self, t: &mut toml::Table) {
        let mut ws = toml::Table::new();
        ws.insert("large_icons".into(), self.large_icons.into());
        t.insert("workspace".into(), ws.into());
        let mut acc = toml::Table::new();
        for s in Swatch::ALL {
            if let Some(c) = self.palette.get(s) {
                acc.insert(s.key().into(), rgb_value(c));
            }
        }
        t.insert("accessibility".into(), acc.into());
        let mut g = toml::Table::new();
        g.insert("msaa".into(), i64::from(self.msaa).into());
        if let Some(gpu) = &self.gpu {
            g.insert("gpu".into(), gpu.clone().into());
        }
        t.insert("graphics".into(), g.into());
        let mut c = toml::Table::new();
        c.insert("bbox_highlight".into(), self.bbox_highlight.into());
        c.insert("invert_wheel".into(), self.invert_wheel.into());
        t.insert("compatibility".into(), c.into());
        let mut f = toml::Table::new();
        for k in Folder::ALL {
            if let Some(p) = self.folders.get(k) {
                f.insert(k.key().into(), p.to_string_lossy().into_owned().into());
            }
        }
        t.insert("files".into(), f.into());
        let mut d = toml::Table::new();
        d.insert(
            "lasso_clockwise_crossing".into(),
            self.lasso_clockwise_crossing.into(),
        );
        d.insert("rotation_grips".into(), self.rotation_grips.into());
        t.insert("drawing_more".into(), d.into());
        let mut a = toml::Table::new();
        if let Some(p) = &self.image_editor {
            a.insert(
                "image_editor".into(),
                p.to_string_lossy().into_owned().into(),
            );
        }
        t.insert("applications".into(), a.into());
    }

    /// Reads the pages' sections of `t`; missing or bad values keep their default.
    pub(crate) fn read(t: &toml::Table) -> Self {
        let mut m = More::default();
        let section = |name: &str| t.get(name).and_then(toml::Value::as_table);
        let flag = |s: Option<&toml::Table>, k: &str| s.and_then(|s| s.get(k)?.as_bool());
        if let Some(v) = flag(section("workspace"), "large_icons") {
            m.large_icons = v;
        }
        if let Some(acc) = section("accessibility") {
            for s in Swatch::ALL {
                m.palette.0[s.index()] = acc.get(s.key()).and_then(rgb_from);
            }
        }
        if let Some(n) = section("graphics").and_then(|g| g.get("msaa")?.as_integer()) {
            m.msaa = msaa_choice(n);
        }
        m.gpu = section("graphics")
            .and_then(|g| g.get("gpu")?.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let d = section("drawing_more");
        if let Some(v) = flag(d, "lasso_clockwise_crossing") {
            m.lasso_clockwise_crossing = v;
        }
        if let Some(v) = flag(d, "rotation_grips") {
            m.rotation_grips = v;
        }
        m.image_editor = section("applications")
            .and_then(|a| a.get("image_editor")?.as_str())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        let c = section("compatibility");
        if let Some(v) = flag(c, "bbox_highlight") {
            m.bbox_highlight = v;
        }
        if let Some(v) = flag(c, "invert_wheel") {
            m.invert_wheel = v;
        }
        if let Some(f) = section("files") {
            for k in Folder::ALL {
                let p = f.get(k.key()).and_then(toml::Value::as_str);
                m.folders
                    .set(k, p.filter(|p| !p.is_empty()).map(PathBuf::from));
            }
        }
        m
    }

    /// Makes the settings that live outside the app's state take effect (overlay
    /// colors, file dialog folders).
    pub(crate) fn apply_globals(&self) {
        set_palette(self.palette);
        crate::filedialog::set_folders(&self.folders);
    }
}

/// A saved sample count as one of 1, 2, 4, 8 (0 means off, as the page shows it).
pub(crate) fn msaa_choice(n: i64) -> u32 {
    match n {
        ..=1 => 1,
        2 => 2,
        3..=4 => 4,
        _ => 8,
    }
}

/// The sample count to draw with: the saved one if the adapter supports it, else the
/// largest supported one below it.
pub(crate) fn usable_samples(wanted: u32, supported: &[u32]) -> u32 {
    supported
        .iter()
        .copied()
        .filter(|&n| n <= wanted)
        .max()
        .unwrap_or(1)
}

/// Graphics > Graphics Engine Details text.
pub(crate) fn engine_details(info: &egui_wgpu::wgpu::AdapterInfo) -> String {
    format!(
        "Adapter: {}\nBackend: {:?}\nDevice type: {:?}\nDriver: {} {}\nVendor / device id: {:#06x} / {:#06x}",
        info.name,
        info.backend,
        info.device_type,
        info.driver,
        info.driver_info,
        info.vendor,
        info.device
    )
}

/// A preferences file holding all pages (and, optionally, the shortcuts): the
/// `preferences.toml` text with a `[shortcuts]` table of shortcut differences.
pub(crate) fn export_text(prefs_text: &str, shortcuts: Option<&toml::Table>) -> String {
    let mut t: toml::Table = prefs_text.parse().unwrap_or_default();
    if let Some(s) = shortcuts {
        t.insert("shortcuts".into(), toml::Value::Table(s.clone()));
    }
    format!("# meshtard preferences (Window > Preferences > Files > Export).\n{t}")
}

/// Splits an exported preferences file into the preferences and the shortcut
/// differences (`None` when it has none).
pub(crate) fn import_text(text: &str) -> Result<(toml::Table, Option<toml::Table>), String> {
    let mut t: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let shortcuts = match t.remove("shortcuts") {
        Some(toml::Value::Table(s)) => Some(s),
        Some(_) => return Err("`shortcuts` is not a table".into()),
        None => None,
    };
    Ok((t, shortcuts))
}

impl App {
    /// The Template page.
    pub(crate) fn template_page(&mut self, ui: &mut egui::Ui) {
        ui.strong("Default Drawing Template");
        ui.label(
            egui::RichText::new("File > New starts from this template.")
                .small()
                .weak(),
        );
        ui.add_space(4.0);
        let list = crate::templates::templates_dir()
            .map_or_else(Vec::new, |d| crate::templates::list_templates(&d));
        let default = crate::templates::load_default();
        let mut set = None;
        egui::ScrollArea::vertical()
            .id_salt("mt_pref_templates")
            .max_height(260.0)
            .show(ui, |ui| {
                if ui
                    .radio(default.is_none(), crate::templates::BUILT_IN)
                    .clicked()
                {
                    set = Some(None);
                }
                for (name, _) in &list {
                    if ui.radio(default.as_deref() == Some(name), name).clicked() {
                        set = Some(Some(name.clone()));
                    }
                }
            });
        if list.is_empty() {
            ui.weak("No saved templates yet: use File > Save As Template.");
        }
        if let Some(name) = set {
            crate::templates::save_default(name.as_deref());
        }
        if let Some(dir) = crate::templates::templates_dir() {
            ui.add_space(6.0);
            ui.weak(format!("Templates folder: {}", dir.display()));
        }
    }

    /// The Workspace page.
    pub(crate) fn workspace_page(&mut self, ui: &mut egui::Ui, m: &mut More) {
        ui.strong("Tool Palette");
        ui.checkbox(&mut m.large_icons, "Use large tool icons");
        ui.add_space(8.0);
        ui.strong("Workspace");
        if ui
            .button("Reset Workspace")
            .on_hover_text("Toolbars and trays back to their default places")
            .clicked()
            && let Some(shell) = &mut self.shell
        {
            shell.reset_workspace();
        }
    }

    /// The Graphics page.
    pub(crate) fn graphics_page(&mut self, ui: &mut egui::Ui, m: &mut More) {
        ui.strong("Multisample Anti-aliasing");
        let supported = self.prefs.samples_supported.clone();
        ui.horizontal(|ui| {
            for n in [1u32, 2, 4, 8] {
                let label = if n == 1 {
                    "0x".to_owned()
                } else {
                    format!("{n}x")
                };
                ui.add_enabled_ui(supported.contains(&n), |ui| {
                    if ui.radio(m.msaa == n, label).clicked() {
                        m.msaa = n;
                    }
                });
            }
        });
        if usable_samples(m.msaa, &supported) != self.prefs.samples_in_use {
            ui.label(
                egui::RichText::new("Restart meshtard to apply the change.")
                    .small()
                    .color(egui::Color32::from_rgb(180, 30, 30)),
            );
        }
        ui.add_space(8.0);
        ui.strong("Graphics Engine Details");
        ui.add(
            egui::TextEdit::multiline(&mut self.prefs.adapter_details.as_str())
                .desired_rows(5)
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace),
        );
        ui.horizontal(|ui| {
            if ui.button("Copy").clicked() {
                ui.ctx().copy_text(self.prefs.adapter_details.clone());
            }
            if ui
                .button("Show Log File")
                .on_hover_text("The graphics log written when meshtard started")
                .clicked()
            {
                match crate::graphics::log_path().filter(|p| p.exists()) {
                    Some(p) => {
                        if let Err(e) = crate::presentation::open_with_system(&p.to_string_lossy())
                        {
                            self.message = Some(format!("Could not open {}: {e}", p.display()));
                        }
                    }
                    None => self.message = Some("No graphics log was written.".into()),
                }
            }
        });
        ui.add_space(8.0);
        ui.strong("GPU");
        let adapters = crate::graphics::adapters();
        let current = m.gpu.clone().unwrap_or_else(|| "Automatic".to_owned());
        egui::ComboBox::from_id_salt("mt_pref_gpu")
            .selected_text(current)
            .width(320.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut m.gpu, None, "Automatic");
                for a in adapters {
                    ui.selectable_value(&mut m.gpu, Some(a.clone()), a);
                }
            });
        if m.gpu != self.prefs.more.gpu {
            ui.label(
                egui::RichText::new("Restart meshtard to use the chosen GPU.")
                    .small()
                    .color(egui::Color32::from_rgb(180, 30, 30)),
            );
        }
    }
}

/// The Drawing page's Lasso Direction and Move tool options.
pub(crate) fn drawing_more(ui: &mut egui::Ui, m: &mut More) {
    ui.add_space(8.0);
    ui.strong("Lasso Direction");
    if ui
        .radio(m.lasso_clockwise_crossing, "Clockwise")
        .on_hover_text(
            "Clockwise selects what the lasso touches; counterclockwise only what is inside",
        )
        .clicked()
    {
        m.lasso_clockwise_crossing = true;
    }
    if ui
        .radio(!m.lasso_clockwise_crossing, "Counterclockwise")
        .on_hover_text(
            "Counterclockwise selects what the lasso touches; clockwise only what is inside",
        )
        .clicked()
    {
        m.lasso_clockwise_crossing = false;
    }
    ui.add_space(8.0);
    ui.strong("Move Tool");
    ui.checkbox(&mut m.rotation_grips, "Show Move tool rotation grips");
}

/// The Applications page: the image editor for Edit Texture Image.
pub(crate) fn applications_page(ui: &mut egui::Ui, m: &mut More) {
    ui.strong("Default Image Editor");
    ui.label(
        egui::RichText::new("Opens texture images from the Materials panel's Edit tab.")
            .small()
            .weak(),
    );
    ui.add_space(4.0);
    let shown = m
        .image_editor
        .as_ref()
        .map_or_else(|| "System default".to_owned(), |p| p.display().to_string());
    ui.add(egui::Label::new(egui::RichText::new(shown).monospace()).truncate());
    ui.horizontal(|ui| {
        if ui.button("Choose...").clicked() {
            // No filter: programs often have no extension.
            if let Some(p) = crate::filedialog::open(&[]) {
                m.image_editor = Some(p);
            }
        }
        if ui
            .add_enabled(m.image_editor.is_some(), egui::Button::new("Default"))
            .clicked()
        {
            m.image_editor = None;
        }
    });
}

/// The Accessibility page.
pub(crate) fn accessibility_page(ui: &mut egui::Ui, m: &mut More) {
    ui.strong("Color Options");
    ui.label(
        egui::RichText::new("Axes and inference colors, for color vision deficiencies.")
            .small()
            .weak(),
    );
    ui.add_space(4.0);
    egui::Grid::new("mt_pref_access")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            for s in Swatch::ALL {
                ui.label(s.label());
                let mut c = m.palette.color(s);
                if ui.color_edit_button_srgb(&mut c).changed() {
                    m.palette.0[s.index()] = (c != s.inference_default()).then_some(c);
                }
                ui.end_row();
            }
        });
    ui.add_space(6.0);
    if ui.button("Reset All").clicked() {
        m.palette = Palette::default();
    }
}

/// The Compatibility page.
pub(crate) fn compatibility_page(ui: &mut egui::Ui, m: &mut More) {
    ui.strong("Component/Group Highlighting");
    ui.checkbox(&mut m.bbox_highlight, "Bounding box only");
    ui.add_space(8.0);
    ui.strong("Mouse Wheel Style");
    ui.checkbox(&mut m.invert_wheel, "Invert");
}

/// What the Files page asks the app to do.
pub(crate) enum FilesAction {
    Import { shortcuts: bool },
    Export { shortcuts: bool },
}

/// The Files page. `with_shortcuts` is the Import / Export "Shortcuts" box.
pub(crate) fn files_page(
    ui: &mut egui::Ui,
    m: &mut More,
    with_shortcuts: &mut bool,
) -> Option<FilesAction> {
    ui.strong("Default Folders");
    egui::ScrollArea::vertical()
        .id_salt("mt_pref_files")
        .max_height(240.0)
        .show(ui, |ui| {
            egui::Grid::new("mt_pref_folders")
                .num_columns(3)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    for f in Folder::ALL {
                        ui.label(f.label());
                        let current = m.folders.get(f).map(Path::to_path_buf);
                        let text = current
                            .as_ref()
                            .map_or_else(|| "(default)".to_owned(), |p| p.display().to_string());
                        ui.allocate_ui_with_layout(
                            egui::vec2(200.0, 18.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_width(200.0);
                                ui.add(
                                    egui::Label::new(egui::RichText::new(text).small()).truncate(),
                                )
                                .on_hover_text(f.label());
                            },
                        );
                        ui.horizontal(|ui| {
                            if ui.small_button("Change...").clicked()
                                && let Some(dir) =
                                    crate::filedialog::pick_folder(current.as_deref())
                            {
                                m.folders.set(f, Some(dir));
                            }
                            if let Some(p) = &current {
                                if ui.small_button("Open").clicked() {
                                    let _ =
                                        crate::presentation::open_with_system(&p.to_string_lossy());
                                }
                                if ui.small_button("Default").clicked() {
                                    m.folders.set(f, None);
                                }
                            }
                        });
                        ui.end_row();
                    }
                });
        });
    ui.label(
        egui::RichText::new(
            "File dialogs start here: Models (open, save, import), Texture Images, Export, \
             Templates (the templates list). The other folders are kept for later.",
        )
        .small()
        .weak(),
    );
    ui.add_space(6.0);
    ui.strong("Preferences File");
    let mut action = None;
    ui.horizontal(|ui| {
        ui.checkbox(with_shortcuts, "Shortcuts");
        if ui.button("Import...").clicked() {
            action = Some(FilesAction::Import {
                shortcuts: *with_shortcuts,
            });
        }
        if ui.button("Export...").clicked() {
            action = Some(FilesAction::Export {
                shortcuts: *with_shortcuts,
            });
        }
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_round_trip() {
        let mut m = More {
            large_icons: false,
            msaa: 8,
            bbox_highlight: true,
            invert_wheel: true,
            lasso_clockwise_crossing: false,
            rotation_grips: false,
            gpu: Some("GPU (Vulkan)".into()),
            image_editor: Some(PathBuf::from("/usr/bin/gimp")),
            ..Default::default()
        };
        m.palette.0[Swatch::Y.index()] = Some([1, 2, 3]);
        m.folders
            .set(Folder::Export, Some(PathBuf::from("/tmp/out")));
        let mut t = toml::Table::new();
        m.write(&mut t);
        let text = t.to_string();
        assert_eq!(More::read(&text.parse().unwrap()), m);
        assert_eq!(More::read(&toml::Table::new()), More::default());
    }

    #[test]
    fn palette_remaps_changed_colors_only() {
        let mut p = Palette::default();
        let red = Swatch::X.inference_default();
        assert_eq!(p.remap(red), red);
        p.0[Swatch::X.index()] = Some([10, 20, 30]);
        assert_eq!(p.remap(red), [10, 20, 30]);
        assert_eq!(p.remap([1, 1, 1]), [1, 1, 1]);
        assert_eq!(p.color(Swatch::Z), Swatch::Z.inference_default());
    }

    #[test]
    fn sample_counts() {
        assert_eq!(msaa_choice(0), 1);
        assert_eq!(msaa_choice(3), 4);
        assert_eq!(msaa_choice(16), 8);
        assert_eq!(usable_samples(8, &[1, 4]), 4);
        assert_eq!(usable_samples(2, &[1, 4]), 1);
        assert_eq!(usable_samples(4, &[]), 1);
    }

    #[test]
    fn preferences_file_carries_shortcuts() {
        let mut s = toml::Table::new();
        s.insert("tool.line".into(), "K".into());
        let text = export_text("backup = false\n", Some(&s));
        let (t, sc) = import_text(&text).unwrap();
        assert_eq!(t.get("backup").and_then(toml::Value::as_bool), Some(false));
        assert_eq!(sc, Some(s));
        let (_, none) = import_text(&export_text("backup = true\n", None)).unwrap();
        assert!(none.is_none());
        assert!(import_text("shortcuts = 3").is_err());
    }
}
