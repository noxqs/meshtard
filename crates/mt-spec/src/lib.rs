//! Loads the SketchUp behavior spec from `spec/*.toml`: menus, keymap, default model,
//! toolbars, the Default Tray and the window theme.
//!
//! The spec files are embedded at compile time so the binary is self-contained.
//! `Spec::load_dir` reads them from disk instead, for editing without rebuilding.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use serde::Deserialize;

mod instructor;
pub use instructor::{Instructor, ModifierHelp, ToolHelp};

const MENUS: &str = include_str!("../../../spec/menus.toml");
const KEYMAP: &str = include_str!("../../../spec/keymap.toml");
const DEFAULT_MODEL: &str = include_str!("../../../spec/default_model.toml");
const TOOLBARS: &str = include_str!("../../../spec/toolbars.toml");
const TRAY: &str = include_str!("../../../spec/tray.toml");
const UI_THEME: &str = include_str!("../../../spec/ui_theme.toml");

/// The text of every spec file, before parsing.
struct Sources<'a> {
    menus: &'a str,
    keymap: &'a str,
    default_model: &'a str,
    toolbars: &'a str,
    tray: &'a str,
    ui_theme: &'a str,
}

const BUILTIN: Sources<'static> = Sources {
    menus: MENUS,
    keymap: KEYMAP,
    default_model: DEFAULT_MODEL,
    toolbars: TOOLBARS,
    tray: TRAY,
    ui_theme: UI_THEME,
};

#[derive(Debug)]
pub struct Error {
    file: &'static str,
    message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.file, self.message)
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone)]
pub struct Spec {
    pub menus: Menus,
    pub keymap: Keymap,
    pub default_model: DefaultModel,
    pub toolbars: Toolbars,
    pub tray: Tray,
    pub ui_theme: UiTheme,
}

impl Spec {
    /// The spec embedded in the binary.
    pub fn builtin() -> Result<Self, Error> {
        Self::parse(&BUILTIN)
    }

    /// Reads the spec from a `spec/` directory on disk.
    pub fn load_dir(dir: &Path) -> Result<Self, Error> {
        let read = |file: &'static str| {
            std::fs::read_to_string(dir.join(file)).map_err(|e| Error {
                file,
                message: e.to_string(),
            })
        };
        Self::parse(&Sources {
            menus: &read("menus.toml")?,
            keymap: &read("keymap.toml")?,
            default_model: &read("default_model.toml")?,
            toolbars: &read("toolbars.toml")?,
            tray: &read("tray.toml")?,
            ui_theme: &read("ui_theme.toml")?,
        })
    }

    fn parse(src: &Sources) -> Result<Self, Error> {
        let spec = Self {
            menus: parse_toml("menus.toml", src.menus)?,
            keymap: parse_toml("keymap.toml", src.keymap)?,
            default_model: parse_toml("default_model.toml", src.default_model)?,
            toolbars: parse_toml("toolbars.toml", src.toolbars)?,
            tray: parse_toml("tray.toml", src.tray)?,
            ui_theme: parse_toml("ui_theme.toml", src.ui_theme)?,
        };
        spec.validate()?;
        Ok(spec)
    }

    /// Every command id in the menu bar, with its label.
    pub fn commands(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for menu in &self.menus.menu {
            menu.visit_commands(&mut |id, label| {
                out.insert(id.to_owned(), label.to_owned());
            });
        }
        out
    }

    /// Menu label of a command id, if it is in the menu bar.
    pub fn command_label(&self, id: &str) -> Option<String> {
        let mut found = None;
        for menu in &self.menus.menu {
            menu.visit_commands(&mut |cid, label| {
                if found.is_none() && cid == id {
                    found = Some(label.to_owned());
                }
            });
        }
        found
    }

    /// Checks cross-file consistency: unique command ids, shortcuts that parse,
    /// no shortcut bound twice.
    fn validate(&self) -> Result<(), Error> {
        let mut ids = BTreeMap::new();
        for menu in &self.menus.menu {
            menu.visit_commands(&mut |id, _| {
                *ids.entry(id.to_owned()).or_insert(0) += 1;
            });
        }
        if let Some((id, _)) = ids.iter().find(|(_, n)| **n > 1) {
            return Err(Error {
                file: "menus.toml",
                message: format!("command id `{id}` appears more than once"),
            });
        }

        self.validate_toolbars(&ids)?;
        self.validate_tray(&ids)?;

        let mut seen: BTreeMap<Shortcut, &str> = BTreeMap::new();
        for (command, key) in &self.keymap.bindings {
            let shortcut = Shortcut::parse(key).map_err(|message| Error {
                file: "keymap.toml",
                message: format!("`{command}`: {message}"),
            })?;
            if let Some(other) = seen.insert(shortcut, command) {
                return Err(Error {
                    file: "keymap.toml",
                    message: format!("`{key}` is bound to both `{other}` and `{command}`"),
                });
            }
        }
        Ok(())
    }
}

impl Spec {
    fn validate_toolbars(&self, commands: &BTreeMap<String, i32>) -> Result<(), Error> {
        let err = |message: String| Error {
            file: "toolbars.toml",
            message,
        };
        let mut toolbar_ids = BTreeSet::new();
        for toolbar in &self.toolbars.toolbar {
            if !toolbar_ids.insert(toolbar.id.as_str()) {
                return Err(err(format!("toolbar id `{}` appears twice", toolbar.id)));
            }
            if toolbar.columns == 0 {
                return Err(err(format!(
                    "toolbar `{}`: columns must be >= 1",
                    toolbar.id
                )));
            }
            for w in toolbar.widgets() {
                if !(w.width.is_finite() && w.width >= 16.0) {
                    return Err(err(format!(
                        "toolbar `{}`: widget {:?} needs a width of at least 16",
                        toolbar.id, w.widget
                    )));
                }
            }
            let mut buttons = BTreeSet::new();
            for button in toolbar.buttons() {
                if !buttons.insert(button.id.as_str()) {
                    return Err(err(format!(
                        "toolbar `{}`: button `{}` appears twice",
                        toolbar.id, button.id
                    )));
                }
                for id in std::iter::once(&button.id).chain(button.flyout.iter()) {
                    if !commands.contains_key(id) {
                        return Err(err(format!(
                            "toolbar `{}`: `{id}` is not a command in menus.toml",
                            toolbar.id
                        )));
                    }
                }
                if !button.flyout.is_empty() && !button.flyout.contains(&button.id) {
                    return Err(err(format!(
                        "toolbar `{}`: flyout of `{}` does not contain it",
                        toolbar.id, button.id
                    )));
                }
            }
        }
        Ok(())
    }

    fn validate_tray(&self, commands: &BTreeMap<String, i32>) -> Result<(), Error> {
        let mut seen = BTreeSet::new();
        for panel in &self.tray.panel {
            let message = if !seen.insert(panel.id.as_str()) {
                format!("panel `{}` appears twice", panel.id)
            } else if !commands.contains_key(&panel.id) {
                format!("panel `{}` is not a Window menu command", panel.id)
            } else {
                continue;
            };
            return Err(Error {
                file: "tray.toml",
                message,
            });
        }
        Ok(())
    }
}

fn parse_toml<T: for<'de> Deserialize<'de>>(file: &'static str, text: &str) -> Result<T, Error> {
    toml::from_str(text).map_err(|e| Error {
        file,
        message: e.to_string(),
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Menus {
    pub verified: bool,
    pub menu: Vec<Menu>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Menu {
    pub label: String,
    pub items: Vec<MenuItem>,
}

impl Menu {
    pub fn visit_commands(&self, f: &mut impl FnMut(&str, &str)) {
        visit_items(&self.items, f);
    }
}

fn visit_items(items: &[MenuItem], f: &mut impl FnMut(&str, &str)) {
    for item in items {
        match item {
            MenuItem::Command { id, label } => f(id, label),
            MenuItem::Submenu { items, .. } => visit_items(items, f),
            MenuItem::Separator { .. } => {}
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum MenuItem {
    Command { id: String, label: String },
    Submenu { label: String, items: Vec<MenuItem> },
    Separator { separator: bool },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keymap {
    pub verified: bool,
    /// Command id to shortcut text, e.g. `"tool.line" = "L"`.
    pub bindings: BTreeMap<String, String>,
}

impl Keymap {
    pub fn shortcut(&self, command: &str) -> Option<Shortcut> {
        self.bindings
            .get(command)
            .and_then(|s| Shortcut::parse(s).ok())
    }
}

/// A key with modifiers. `key` is an egui key name such as `L`, `Space`, `Delete`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Shortcut {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: String,
}

impl Shortcut {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let key = parts.pop().filter(|k| !k.is_empty()).ok_or("missing key")?;
        let mut shortcut = Shortcut {
            ctrl: false,
            shift: false,
            alt: false,
            key: key.to_owned(),
        };
        for modifier in parts {
            let flag = match modifier {
                "Ctrl" => &mut shortcut.ctrl,
                "Shift" => &mut shortcut.shift,
                "Alt" => &mut shortcut.alt,
                other => return Err(format!("unknown modifier `{other}`")),
            };
            if std::mem::replace(flag, true) {
                return Err(format!("modifier `{modifier}` repeated"));
            }
        }
        Ok(shortcut)
    }
}

impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [
            (self.ctrl, "Ctrl"),
            (self.shift, "Shift"),
            (self.alt, "Alt"),
        ] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        f.write_str(&self.key)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefaultModel {
    pub verified: bool,
    pub camera: CameraSpec,
    pub sky: SkySpec,
    pub ground: GroundSpec,
    pub axes: AxesSpec,
    pub shading: ShadingSpec,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraSpec {
    pub eye: [f64; 3],
    pub target: [f64; 3],
    pub fov_deg: f64,
    pub perspective: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkySpec {
    pub enabled: bool,
    pub zenith: [f32; 3],
    pub horizon: [f32; 3],
    pub background: [f32; 3],
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundSpec {
    pub enabled: bool,
    pub color: [f32; 3],
    pub show_from_below: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AxesSpec {
    pub x: [f32; 3],
    pub y: [f32; 3],
    pub z: [f32; 3],
    pub dotted_negative: bool,
    pub width_px: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShadingSpec {
    /// Brightness of a face turned away from the light, 0..1.
    pub ambient: f32,
    pub light_from_camera: bool,
    /// Yaw (towards camera right) and pitch (up) of the light from the view direction.
    pub light_offset_deg: [f32; 2],
    /// World direction towards the light, used when `light_from_camera` is false.
    pub light_direction: [f32; 3],
}

/// `toolbars.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Toolbars {
    pub verified: bool,
    pub toolbar: Vec<Toolbar>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dock {
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Toolbar {
    pub id: String,
    pub name: String,
    pub dock: Dock,
    /// Order among toolbars on the same dock, 0 nearest the window edge.
    pub row: u32,
    /// Buttons per row when docked left or right.
    pub columns: u32,
    pub visible: bool,
    pub items: Vec<ToolbarItem>,
}

impl Toolbar {
    pub fn buttons(&self) -> impl Iterator<Item = &ToolbarButton> {
        self.items.iter().filter_map(|item| match item {
            ToolbarItem::Button(b) => Some(b),
            ToolbarItem::Widget(_) | ToolbarItem::Separator { .. } => None,
        })
    }

    pub fn widgets(&self) -> impl Iterator<Item = &ToolbarWidget> {
        self.items.iter().filter_map(|item| match item {
            ToolbarItem::Widget(w) => Some(w),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ToolbarItem {
    Button(ToolbarButton),
    Widget(ToolbarWidget),
    Separator { separator: bool },
}

/// A toolbar item that is not a button: `{ widget = "...", width = 120, tooltip = "..." }`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolbarWidget {
    pub widget: WidgetKind,
    /// Width in points when the toolbar lies flat (its own line when upright).
    pub width: f32,
    #[serde(default)]
    pub tooltip: String,
}

/// What a [`ToolbarWidget`] shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidgetKind {
    /// Dropdown of the model's tags; picking one makes it the active tag.
    ActiveTag,
    /// Shadows: the date slider.
    ShadowDate,
    /// Shadows: the time-of-day slider.
    ShadowTime,
    /// The Measurements box (label and value field).
    Measurements,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolbarButton {
    /// Command id from `menus.toml`.
    pub id: String,
    pub tooltip: String,
    /// Commands offered by the button's flyout, including `id`. Empty: no flyout.
    #[serde(default)]
    pub flyout: Vec<String>,
}

/// `tray.toml`: the Default Tray.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tray {
    pub verified: bool,
    pub name: String,
    pub dock: Dock,
    pub width: f32,
    pub panel: Vec<TrayPanel>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrayPanel {
    /// The Window menu command that shows this panel.
    pub id: String,
    pub title: String,
    pub expanded: bool,
    /// Whether the panel is in the tray at start; a hidden panel appears when its
    /// Window menu command is chosen. Default true.
    #[serde(default = "yes")]
    pub shown: bool,
}

fn yes() -> bool {
    true
}

/// sRGB color.
pub type Rgb = [u8; 3];

/// `ui_theme.toml`: colors and sizes of the window chrome.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiTheme {
    pub verified: bool,
    pub colors: UiColors,
    pub icons: IconColors,
    pub sizes: UiSizes,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiColors {
    pub window: Rgb,
    pub menu_bar: Rgb,
    pub toolbar: Rgb,
    pub text: Rgb,
    pub text_disabled: Rgb,
    pub hover: Rgb,
    pub hover_border: Rgb,
    pub pressed: Rgb,
    pub pressed_border: Rgb,
    pub menu_highlight: Rgb,
    pub separator: Rgb,
    pub border: Rgb,
    pub tray: Rgb,
    pub tray_title: Rgb,
    pub panel_header: Rgb,
    pub panel_body: Rgb,
    pub status_bar: Rgb,
    pub field: Rgb,
    pub selection: Rgb,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IconColors {
    pub ink: Rgb,
    pub accent: Rgb,
    pub face: Rgb,
    pub face_dark: Rgb,
    pub red: Rgb,
    pub green: Rgb,
    pub paper: Rgb,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiSizes {
    pub font: f32,
    pub menu_bar_height: f32,
    pub toolbar_icon: f32,
    pub toolbar_button: f32,
    pub toolbar_padding: f32,
    pub separator_gap: f32,
    pub panel_header_height: f32,
    pub status_bar_height: f32,
    pub measurements_width: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_spec_loads() {
        let spec = Spec::builtin().unwrap();
        let labels: Vec<_> = spec.menus.menu.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "File",
                "Edit",
                "View",
                "Camera",
                "Draw",
                "Tools",
                "Window",
                "Extensions",
                "Help"
            ]
        );
    }

    #[test]
    fn disk_spec_matches_builtin() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec");
        let disk = Spec::load_dir(&dir).unwrap();
        assert_eq!(
            disk.keymap.bindings,
            Spec::builtin().unwrap().keymap.bindings
        );
    }

    #[test]
    fn every_bound_command_exists_in_a_menu() {
        let spec = Spec::builtin().unwrap();
        let mut ids = Vec::new();
        for menu in &spec.menus.menu {
            menu.visit_commands(&mut |id, _| ids.push(id.to_owned()));
        }
        for command in spec.keymap.bindings.keys() {
            assert!(
                ids.contains(command),
                "`{command}` has a shortcut but no menu item"
            );
        }
    }

    #[test]
    fn shortcut_round_trip() {
        for text in ["L", "Shift+Z", "Ctrl+Shift+I", "Space"] {
            assert_eq!(Shortcut::parse(text).unwrap().to_string(), text);
        }
        assert!(Shortcut::parse("Ctrl+").is_err());
        assert!(Shortcut::parse("Hyper+L").is_err());
        assert!(Shortcut::parse("Ctrl+Ctrl+L").is_err());
    }

    #[test]
    fn duplicate_shortcut_is_rejected() {
        let keymap = "verified = false\n[bindings]\n\"a\" = \"L\"\n\"b\" = \"L\"\n";
        let err = Spec::parse(&Sources { keymap, ..BUILTIN }).unwrap_err();
        assert!(err.to_string().contains("bound to both"), "{err}");
    }

    #[test]
    fn toolbars_load() {
        let spec = Spec::builtin().unwrap();
        let ids: Vec<_> = spec
            .toolbars
            .toolbar
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "getting_started",
                "large_tool_set",
                "standard",
                "views",
                "camera",
                "styles",
                "edit",
                "drawing",
                "principal",
                "construction",
                "section",
                "solid_tools",
                "tags",
                "shadows",
                "sandbox",
                "measurements",
            ]
        );
        let gs = &spec.toolbars.toolbar[0];
        assert_eq!(gs.dock, Dock::Top);
        assert_eq!(gs.buttons().next().unwrap().id, "tool.select");
        assert_eq!(spec.toolbars.toolbar[1].columns, 2);
        // Only the two default toolbars are shown on first start.
        for t in &spec.toolbars.toolbar[2..] {
            assert!(!t.visible, "{}", t.id);
        }
        let widgets = |id: &str| -> Vec<WidgetKind> {
            let t = spec.toolbars.toolbar.iter().find(|t| t.id == id).unwrap();
            t.widgets().map(|w| w.widget).collect()
        };
        assert_eq!(widgets("tags"), [WidgetKind::ActiveTag]);
        assert_eq!(
            widgets("shadows"),
            [WidgetKind::ShadowDate, WidgetKind::ShadowTime]
        );
        assert_eq!(widgets("measurements"), [WidgetKind::Measurements]);
    }

    #[test]
    fn bad_widgets_are_rejected() {
        let narrow = TOOLBARS.replacen("width = 150.0", "width = 2.0", 1);
        let err = Spec::parse(&Sources {
            toolbars: &narrow,
            ..BUILTIN
        })
        .unwrap_err();
        assert!(err.to_string().contains("width"), "{err}");
        let unknown = TOOLBARS.replacen("\"active_tag\", width", "\"nope\", width", 1);
        assert!(
            Spec::parse(&Sources {
                toolbars: &unknown,
                ..BUILTIN
            })
            .is_err()
        );
    }

    #[test]
    fn every_toolbar_command_is_a_menu_command() {
        let spec = Spec::builtin().unwrap();
        let commands = spec.commands();
        for toolbar in &spec.toolbars.toolbar {
            for button in toolbar.buttons() {
                for id in std::iter::once(&button.id).chain(&button.flyout) {
                    assert!(commands.contains_key(id), "{}: `{id}`", toolbar.id);
                }
                assert!(!button.tooltip.is_empty());
            }
        }
    }

    #[test]
    fn unknown_toolbar_command_is_rejected() {
        let toolbars = TOOLBARS.replacen("\"tool.eraser\"", "\"tool.nope\"", 1);
        let err = Spec::parse(&Sources {
            toolbars: &toolbars,
            ..BUILTIN
        })
        .unwrap_err();
        assert!(err.to_string().contains("tool.nope"), "{err}");
    }

    #[test]
    fn duplicate_toolbar_button_is_rejected() {
        let toolbars = TOOLBARS.replacen("\"tool.eraser\"", "\"tool.select\"", 1);
        let err = Spec::parse(&Sources {
            toolbars: &toolbars,
            ..BUILTIN
        })
        .unwrap_err();
        assert!(err.to_string().contains("appears twice"), "{err}");
    }

    #[test]
    fn tray_loads_with_unique_window_commands() {
        let spec = Spec::builtin().unwrap();
        let tray = &spec.tray;
        assert_eq!(tray.dock, Dock::Right);
        assert!(tray.width > 100.0);
        assert_eq!(tray.panel[0].id, "window.entity_info");
        assert!(tray.panel[0].expanded);
        let commands = spec.commands();
        let mut seen = BTreeSet::new();
        for panel in &tray.panel {
            assert!(panel.id.starts_with("window."), "{}", panel.id);
            assert!(commands.contains_key(&panel.id), "{}", panel.id);
            assert!(seen.insert(&panel.id), "{} twice", panel.id);
        }
    }

    #[test]
    fn duplicate_tray_panel_is_rejected() {
        let tray = TRAY.replacen("window.materials", "window.entity_info", 1);
        let err = Spec::parse(&Sources {
            tray: &tray,
            ..BUILTIN
        })
        .unwrap_err();
        assert!(err.to_string().contains("twice"), "{err}");
    }

    #[test]
    fn ui_theme_loads() {
        let spec = Spec::builtin().unwrap();
        let sizes = &spec.ui_theme.sizes;
        assert!(sizes.toolbar_button >= sizes.toolbar_icon);
        assert!(sizes.font > 6.0);
    }

    #[test]
    fn disk_spec_has_same_toolbars_and_tray() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec");
        let disk = Spec::load_dir(&dir).unwrap();
        let builtin = Spec::builtin().unwrap();
        assert_eq!(disk.toolbars.toolbar.len(), builtin.toolbars.toolbar.len());
        assert_eq!(disk.tray.panel.len(), builtin.tray.panel.len());
    }
}
