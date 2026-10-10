//! Loads the SketchUp behavior spec from `spec/*.toml`.
//!
//! The spec files are embedded at compile time so the binary is self-contained.
//! `Spec::load_dir` reads them from disk instead, for editing without rebuilding.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde::Deserialize;

const MENUS: &str = include_str!("../../../spec/menus.toml");
const KEYMAP: &str = include_str!("../../../spec/keymap.toml");
const DEFAULT_MODEL: &str = include_str!("../../../spec/default_model.toml");

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
}

impl Spec {
    /// The spec embedded in the binary.
    pub fn builtin() -> Result<Self, Error> {
        Self::parse(MENUS, KEYMAP, DEFAULT_MODEL)
    }

    /// Reads the spec from a `spec/` directory on disk.
    pub fn load_dir(dir: &Path) -> Result<Self, Error> {
        let read = |file: &'static str| {
            std::fs::read_to_string(dir.join(file)).map_err(|e| Error {
                file,
                message: e.to_string(),
            })
        };
        Self::parse(
            &read("menus.toml")?,
            &read("keymap.toml")?,
            &read("default_model.toml")?,
        )
    }

    fn parse(menus: &str, keymap: &str, default_model: &str) -> Result<Self, Error> {
        let spec = Self {
            menus: parse_toml("menus.toml", menus)?,
            keymap: parse_toml("keymap.toml", keymap)?,
            default_model: parse_toml("default_model.toml", default_model)?,
        };
        spec.validate()?;
        Ok(spec)
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
        let err = Spec::parse(MENUS, keymap, DEFAULT_MODEL).unwrap_err();
        assert!(err.to_string().contains("bound to both"), "{err}");
    }
}
