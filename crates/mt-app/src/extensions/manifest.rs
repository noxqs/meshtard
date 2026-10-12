//! `extension.toml`: what an extension is, which scripts it runs and what it may do.
//! See `spec/EXTENSIONS.md`.

use serde::Deserialize;

/// What a script may do. Anything not listed is refused at run time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Permission {
    /// Read the active context's entities, the selection, materials and tags; observe
    /// selection and model changes.
    ModelRead,
    /// Create, change and erase entities, materials and tags (one undo step per
    /// callback).
    ModelWrite,
    /// Add menu items (commands and tools).
    UiMenu,
    /// Add toolbar buttons.
    UiToolbar,
    /// HTTP GET requests (asked at install).
    Net,
    /// Read and write files in the extension's own data folder (asked at install).
    File,
}

impl Permission {
    pub(crate) const ALL: [Self; 6] = [
        Self::ModelRead,
        Self::ModelWrite,
        Self::UiMenu,
        Self::UiToolbar,
        Self::Net,
        Self::File,
    ];

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::ModelRead => "model.read",
            Self::ModelWrite => "model.write",
            Self::UiMenu => "ui.menu",
            Self::UiToolbar => "ui.toolbar",
            Self::Net => "net",
            Self::File => "file",
        }
    }

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == key)
    }

    /// Shown in the install dialog and the Extension Manager.
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::ModelRead => "Read the model and the selection",
            Self::ModelWrite => "Change the model (each change is one undo step)",
            Self::UiMenu => "Add menu items",
            Self::UiToolbar => "Add toolbar buttons",
            Self::Net => "Access the internet (HTTP GET)",
            Self::File => "Read and write files in its own data folder",
        }
    }

    /// Reaches outside the app: granted only when the user approves it at install.
    pub(crate) fn sensitive(self) -> bool {
        matches!(self, Self::Net | Self::File)
    }
}

/// `extension.toml` as written.
#[derive(Debug, Clone, Deserialize)]
struct RawManifest {
    id: String,
    name: String,
    version: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    license: String,
    #[serde(default)]
    min_meshtard: Option<String>,
    #[serde(default)]
    permissions: Vec<String>,
    /// Script files, run in order (default: `main.rhai`).
    #[serde(default)]
    scripts: Vec<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    homepage: String,
}

/// A checked `extension.toml`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub license: String,
    pub min_meshtard: Option<String>,
    pub permissions: Vec<Permission>,
    pub scripts: Vec<String>,
    pub icon: Option<String>,
    pub homepage: String,
}

/// The running app's version (`min_meshtard` is compared with it).
pub(crate) const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

impl Manifest {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let raw: RawManifest =
            toml::from_str(text).map_err(|e| format!("extension.toml: {}", e.message()))?;
        if !valid_id(&raw.id) {
            return Err(format!(
                "extension.toml: id `{}` must be 3-64 characters a-z, 0-9, `_`, `-` or `.`",
                raw.id
            ));
        }
        let name = raw.name.trim().to_owned();
        if name.is_empty() || name.chars().count() > 80 {
            return Err("extension.toml: name must be 1-80 characters".into());
        }
        if parse_version(&raw.version).is_none() {
            return Err(format!(
                "extension.toml: version `{}` is not like 1.2.3",
                raw.version
            ));
        }
        if let Some(min) = &raw.min_meshtard {
            let Some(want) = parse_version(min) else {
                return Err(format!(
                    "extension.toml: min_meshtard `{min}` is not like 1.2.3"
                ));
            };
            if want > parse_version(APP_VERSION).unwrap_or_default() {
                return Err(format!(
                    "needs Meshtard {min} or later (this is {APP_VERSION})"
                ));
            }
        }
        let mut permissions = Vec::new();
        for p in &raw.permissions {
            let perm = Permission::from_key(p)
                .ok_or_else(|| format!("extension.toml: unknown permission `{p}`"))?;
            if !permissions.contains(&perm) {
                permissions.push(perm);
            }
        }
        permissions.sort();
        let scripts = if raw.scripts.is_empty() {
            vec!["main.rhai".to_owned()]
        } else {
            raw.scripts
        };
        for s in &scripts {
            if !crate::library::safe_relative(s) || !s.ends_with(".rhai") {
                return Err(format!(
                    "extension.toml: script `{s}` must be a .rhai file inside the extension"
                ));
            }
        }
        if let Some(icon) = &raw.icon
            && (!crate::library::safe_relative(icon) || !icon.ends_with(".png"))
        {
            return Err(format!(
                "extension.toml: icon `{icon}` must be a .png file inside the extension"
            ));
        }
        Ok(Self {
            id: raw.id,
            name,
            version: raw.version,
            author: raw.author,
            description: raw.description,
            license: raw.license,
            min_meshtard: raw.min_meshtard,
            permissions,
            scripts,
            icon: raw.icon,
            homepage: raw.homepage,
        })
    }

    /// The permissions the user must approve (internet, files).
    pub(crate) fn sensitive(&self) -> Vec<Permission> {
        (self.permissions.iter().copied())
            .filter(|p| p.sensitive())
            .collect()
    }
}

/// Extension ids: `[a-z0-9_.-]{3,64}`, not starting with `.` or `-`.
pub(crate) fn valid_id(id: &str) -> bool {
    (3..=64).contains(&id.len())
        && !id.starts_with(['.', '-'])
        && !id.contains("..")
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.'))
}

/// `1.2.3` (missing parts are 0; a `-suffix` is ignored).
pub(crate) fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let v = v.trim();
    let v = v.split(['-', '+']).next()?;
    let mut parts = v.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |p| p.parse().ok())?;
    let patch = parts.next().map_or(Some(0), |p| p.parse().ok())?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
id = "meshtard.stair_maker"
name = "Stair Maker"
version = "1.0.0"
author = "Meshtard"
description = "Makes stairs."
license = "MIT"
min_meshtard = "0.1.0"
permissions = ["model.write", "model.read", "ui.menu", "ui.menu"]
"#;

    #[test]
    fn parses_a_manifest() {
        let m = Manifest::parse(GOOD).unwrap();
        assert_eq!(m.id, "meshtard.stair_maker");
        assert_eq!(m.scripts, ["main.rhai"]);
        assert_eq!(
            m.permissions,
            [
                Permission::ModelRead,
                Permission::ModelWrite,
                Permission::UiMenu
            ]
        );
        assert!(m.sensitive().is_empty());
        assert!(!m.permissions.contains(&Permission::Net));
    }

    #[test]
    fn rejects_bad_manifests() {
        let bad = |from: &str, to: &str| Manifest::parse(&GOOD.replace(from, to)).unwrap_err();
        assert!(bad("meshtard.stair_maker", "Bad Id").contains("id"));
        assert!(bad("meshtard.stair_maker", "../x").contains("id"));
        assert!(bad("\"1.0.0\"", "\"one\"").contains("version"));
        assert!(bad("\"0.1.0\"", "\"999.0\"").contains("needs Meshtard 999.0"));
        assert!(bad("\"ui.menu\", \"ui.menu\"", "\"root\"").contains("unknown permission"));
        assert!(bad("license", "scripts = [\"../evil.rhai\"]\nlicense").contains("script"));
        assert!(bad("license", "icon = \"/etc/x.png\"\nlicense").contains("icon"));
        assert!(Manifest::parse("name = \"x\"").is_err());
    }

    #[test]
    fn sensitive_permissions_need_approval() {
        let m = Manifest::parse(&GOOD.replace("\"ui.menu\", \"ui.menu\"", "\"net\", \"file\""))
            .unwrap();
        assert_eq!(m.sensitive(), [Permission::Net, Permission::File]);
    }

    #[test]
    fn versions() {
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("2"), Some((2, 0, 0)));
        assert_eq!(parse_version("1.2.3-beta"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.x"), None);
        assert!(parse_version("0.10.0") > parse_version("0.9.9"));
    }
}
