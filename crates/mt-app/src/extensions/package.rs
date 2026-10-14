//! Extension packages: a folder or a `.mtx` file (a zip of that folder) with
//! `extension.toml` at its root (or inside one top-level folder), Rhai scripts and
//! PNG icons. Installed extensions live in `<data>/meshtard/extensions/<id>/`; which
//! are enabled and which sensitive permissions the user approved is kept in
//! `<config>/meshtard/extensions.toml`.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use super::manifest::{Manifest, Permission};

/// File types an extension may carry; others in a package are skipped.
const ALLOWED: &[&str] = &["rhai", "toml", "png", "md", "txt", "json"];
/// Largest package (unpacked).
const MAX_TOTAL: u64 = 20 << 20;
const MAX_FILES: usize = 500;

/// A package read into memory, not installed yet.
#[derive(Debug, Clone)]
pub(crate) struct Package {
    pub manifest: Manifest,
    /// Relative path (`/` separated) to contents.
    pub files: BTreeMap<String, Vec<u8>>,
}

impl Package {
    /// Reads a `.mtx` file or an extension folder.
    pub(crate) fn read(path: &Path) -> Result<Self, String> {
        if path.is_dir() {
            Self::from_folder(path)
        } else {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            Self::from_zip(&bytes)
        }
    }

    pub(crate) fn from_folder(dir: &Path) -> Result<Self, String> {
        let mut files = BTreeMap::new();
        let mut total = 0;
        collect(dir, "", &mut files, &mut total)?;
        Self::from_files(files)
    }

    /// A `.mtx` (zip) in memory.
    pub(crate) fn from_zip(bytes: &[u8]) -> Result<Self, String> {
        crate::errors::guard(|| Self::from_zip_unguarded(bytes))
    }

    pub(crate) fn from_zip_unguarded(bytes: &[u8]) -> Result<Self, String> {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
            .map_err(|e| format!("not a valid .mtx package: {e}"))?;
        if zip.len() > MAX_FILES {
            return Err("the package has too many files".into());
        }
        let mut files = BTreeMap::new();
        let mut total = 0u64;
        let mut read = 0u64;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
            if f.is_dir() {
                continue;
            }
            let name = f.name().map_err(|e| e.to_string())?.replace('\\', "/");
            // Zip slip: only plain relative paths.
            if f.enclosed_name().is_none() || !crate::library::safe_relative(&name) {
                return Err(format!("the package has an unsafe path `{name}`"));
            }
            total = total.saturating_add(f.size());
            if total > MAX_TOTAL {
                return Err("the package is too large (over 20 MB)".into());
            }
            if !allowed(&name) {
                log::info!("extension package: skipping {name}");
                continue;
            }
            // Declared sizes can lie: the bytes read count too.
            let mut data = Vec::new();
            (&mut f)
                .take(MAX_TOTAL - read + 1)
                .read_to_end(&mut data)
                .map_err(|e| e.to_string())?;
            read += data.len() as u64;
            if read > MAX_TOTAL {
                return Err("the package is too large (over 20 MB)".into());
            }
            files.insert(name, data);
        }
        Self::from_files(files)
    }

    fn from_files(mut files: BTreeMap<String, Vec<u8>>) -> Result<Self, String> {
        // `extension.toml` at the root, or inside a single top-level folder.
        if !files.contains_key("extension.toml") {
            let tops: BTreeSet<&str> = files
                .keys()
                .filter_map(|k| k.split_once('/'))
                .map(|(a, _)| a)
                .collect();
            let top = match tops.into_iter().collect::<Vec<_>>()[..] {
                [t] if files.contains_key(&format!("{t}/extension.toml")) => t.to_owned(),
                _ => return Err("no extension.toml in the package".into()),
            };
            let prefix = format!("{top}/");
            files = files
                .into_iter()
                .filter_map(|(k, v)| k.strip_prefix(&prefix).map(|k| (k.to_owned(), v)))
                .collect();
        }
        let text = std::str::from_utf8(&files["extension.toml"])
            .map_err(|_| "extension.toml is not UTF-8 text".to_owned())?;
        let manifest = Manifest::parse(text)?;
        for s in &manifest.scripts {
            if !files.contains_key(s) {
                return Err(format!("script `{s}` named in extension.toml is missing"));
            }
        }
        Ok(Self { manifest, files })
    }

    /// Writes the package to `<root>/<id>/`, replacing an installed version.
    pub(crate) fn install(&self, root: &Path) -> Result<PathBuf, String> {
        let dest = root.join(&self.manifest.id);
        let tmp = root.join(format!(".{}.installing", self.manifest.id));
        let err = |e: std::io::Error| format!("could not install into {}: {e}", root.display());
        if tmp.exists() {
            std::fs::remove_dir_all(&tmp).map_err(err)?;
        }
        for (name, data) in &self.files {
            let path = tmp.join(name);
            std::fs::create_dir_all(path.parent().expect("inside tmp")).map_err(err)?;
            std::fs::write(&path, data).map_err(err)?;
        }
        if dest.exists() {
            std::fs::remove_dir_all(&dest).map_err(err)?;
        }
        std::fs::rename(&tmp, &dest).map_err(err)?;
        Ok(dest)
    }
}

fn allowed(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ALLOWED.contains(&e.to_ascii_lowercase().as_str()))
}

fn collect(
    dir: &Path,
    prefix: &str,
    files: &mut BTreeMap<String, Vec<u8>>,
    total: &mut u64,
) -> Result<(), String> {
    let err = |e: std::io::Error| format!("{}: {e}", dir.display());
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(err)?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let rel = format!("{prefix}{name}");
        let ty = e.file_type().map_err(err)?;
        if ty.is_symlink() {
            continue;
        }
        if ty.is_dir() {
            collect(&e.path(), &format!("{rel}/"), files, total)?;
        } else if allowed(&rel) {
            let data = std::fs::read(e.path()).map_err(err)?;
            *total += data.len() as u64;
            if *total > MAX_TOTAL || files.len() >= MAX_FILES {
                return Err("the extension folder is too large".into());
            }
            files.insert(rel, data);
        }
    }
    Ok(())
}

/// Zips an extension folder into `.mtx` bytes (tests and `build_extension_dist`).
/// Deterministic (fixed dates, sorted names), so the same folder always gives the
/// same SHA-256.
#[cfg(test)]
pub(crate) fn pack(dir: &Path) -> Result<Vec<u8>, String> {
    pack_files(&Package::from_folder(dir)?.files)
}

/// Zips package files into `.mtx` bytes, deterministically (`meshtard
/// --extension-sign` rewrites a package with its signature this way).
pub(crate) fn pack_files(files: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, String> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644)
        // The host OS byte would differ on Windows and change the checksum.
        .system(zip::System::Unix);
    for (name, data) in files {
        zip.start_file(name.as_str(), opts)
            .map_err(|e| e.to_string())?;
        std::io::Write::write_all(&mut zip, data).map_err(|e| e.to_string())?;
    }
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}

/// Folder of installed extensions.
pub(crate) fn extensions_dir() -> Option<PathBuf> {
    crate::settings::data_dir().map(|d| d.join("extensions"))
}

/// Folder an extension with the `file` permission may read and write.
pub(crate) fn extension_data_dir(id: &str) -> Option<PathBuf> {
    crate::settings::data_dir().map(|d| d.join("extension_data").join(id))
}

/// Per-extension settings kept between runs.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ExtState {
    pub enabled: bool,
    /// Sensitive permissions the user approved (at install or later in the
    /// Extension Manager).
    pub approved: Vec<Permission>,
    /// Declared permissions the user took back in the Extension Manager.
    pub revoked: Vec<Permission>,
}

/// `<config>/meshtard/extensions.toml`.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Config {
    pub extensions: BTreeMap<String, ExtState>,
    /// Extension Warehouse catalog URL (empty: the default).
    pub catalog_url: String,
    /// Keys whose signatures are trusted, added by the user: name -> hex public key
    /// (the built-in keys are in `signing::BUILTIN_KEYS`).
    pub trusted_keys: BTreeMap<String, String>,
    /// Check the Warehouse catalog for updates of installed extensions at startup
    /// (off by default: no network access unless the user turns it on).
    pub check_updates: bool,
}

impl Config {
    fn path() -> Option<PathBuf> {
        crate::settings::config_dir().map(|d| d.join("extensions.toml"))
    }

    pub(crate) fn from_toml(text: &str) -> Self {
        let mut c = Self::default();
        let Ok(t) = text.parse::<toml::Table>() else {
            log::warn!("ignoring an unreadable extensions.toml");
            return c;
        };
        if let Some(u) = t.get("catalog_url").and_then(|v| v.as_str()) {
            c.catalog_url = u.to_owned();
        }
        c.check_updates = t
            .get("check_updates")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if let Some(keys) = t.get("trusted_keys").and_then(|v| v.as_table()) {
            for (name, v) in keys {
                match v.as_str().map(super::signing::parse_public_key) {
                    Some(Ok(_)) => {
                        c.trusted_keys
                            .insert(name.clone(), v.as_str().unwrap_or_default().to_owned());
                    }
                    _ => log::warn!("extensions.toml: ignoring the bad trusted key `{name}`"),
                }
            }
        }
        if let Some(exts) = t.get("extension").and_then(|v| v.as_table()) {
            for (id, v) in exts {
                let Some(v) = v.as_table() else { continue };
                let perms = |key: &str| -> Vec<Permission> {
                    v.get(key)
                        .and_then(|a| a.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|p| p.as_str().and_then(Permission::from_key))
                                .collect()
                        })
                        .unwrap_or_default()
                };
                c.extensions.insert(
                    id.clone(),
                    ExtState {
                        enabled: v.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true),
                        approved: perms("approved"),
                        revoked: perms("revoked"),
                    },
                );
            }
        }
        c
    }

    pub(crate) fn to_toml(&self) -> String {
        let mut t = toml::Table::new();
        if !self.catalog_url.is_empty() {
            t.insert("catalog_url".into(), self.catalog_url.clone().into());
        }
        if self.check_updates {
            t.insert("check_updates".into(), true.into());
        }
        if !self.trusted_keys.is_empty() {
            let keys: toml::Table = (self.trusted_keys.iter())
                .map(|(k, v)| (k.clone(), toml::Value::from(v.clone())))
                .collect();
            t.insert("trusted_keys".into(), toml::Value::Table(keys));
        }
        let mut exts = toml::Table::new();
        let list = |p: &[Permission]| -> toml::Value {
            toml::Value::Array(p.iter().map(|p| p.key().to_owned().into()).collect())
        };
        for (id, s) in &self.extensions {
            let mut e = toml::Table::new();
            e.insert("enabled".into(), s.enabled.into());
            e.insert("approved".into(), list(&s.approved));
            if !s.revoked.is_empty() {
                e.insert("revoked".into(), list(&s.revoked));
            }
            exts.insert(id.clone(), toml::Value::Table(e));
        }
        t.insert("extension".into(), toml::Value::Table(exts));
        format!("# Meshtard extensions: enabled state and approved permissions.\n{t}")
    }

    pub(crate) fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| Self::from_toml(&t))
            .unwrap_or_default()
    }

    pub(crate) fn save(&self) {
        let Some(path) = Self::path() else { return };
        let r = std::fs::create_dir_all(path.parent().expect("has a parent"))
            .and_then(|()| std::fs::write(&path, self.to_toml()));
        if let Err(e) = r {
            log::warn!("could not save {}: {e}", path.display());
        }
    }
}

/// The installed extensions' folders with their manifests (or why one is broken).
pub(crate) fn installed(root: &Path) -> Vec<(PathBuf, Result<Manifest, String>)> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<_> = rd
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| {
            let p = e.path();
            let m = std::fs::read_to_string(p.join("extension.toml"))
                .map_err(|e| format!("extension.toml: {e}"))
                .and_then(|t| Manifest::parse(&t));
            (p, m)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    pub(crate) fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mt_ext_test_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const MANIFEST: &str = "id = \"test.pkg\"\nname = \"Pkg\"\nversion = \"1.0\"\n";

    #[test]
    fn packs_reads_and_installs() {
        let src = scratch("src");
        std::fs::write(src.join("extension.toml"), MANIFEST).unwrap();
        std::fs::write(src.join("main.rhai"), "let x = 1;").unwrap();
        std::fs::write(src.join("binary.exe"), "MZ").unwrap();
        let bytes = pack(&src).unwrap();
        assert_eq!(bytes, pack(&src).unwrap(), "deterministic");
        let pkg = Package::from_zip(&bytes).unwrap();
        assert_eq!(pkg.manifest.id, "test.pkg");
        assert!(!pkg.files.contains_key("binary.exe"));
        let root = scratch("root");
        let dest = pkg.install(&root).unwrap();
        assert!(dest.join("main.rhai").exists());
        let list = installed(&root);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].1.as_ref().unwrap().name, "Pkg");
        // Reinstall replaces.
        pkg.install(&root).unwrap();
        assert_eq!(installed(&root).len(), 1);
    }

    #[test]
    fn nested_folder_and_unsafe_paths() {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        zip.start_file("pkg/extension.toml", o).unwrap();
        zip.write_all(MANIFEST.as_bytes()).unwrap();
        zip.start_file("pkg/main.rhai", o).unwrap();
        zip.write_all(b"1").unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        let pkg = Package::from_zip(&bytes).unwrap();
        assert!(pkg.files.contains_key("main.rhai"));

        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("../evil.rhai", o).unwrap();
        zip.write_all(b"1").unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        assert!(Package::from_zip(&bytes).unwrap_err().contains("unsafe"));
        assert!(Package::from_zip(b"not a zip").is_err());
    }

    #[test]
    fn missing_script_is_an_error() {
        let src = scratch("noscript");
        std::fs::write(src.join("extension.toml"), MANIFEST).unwrap();
        assert!(
            Package::from_folder(&src)
                .unwrap_err()
                .contains("main.rhai")
        );
    }

    #[test]
    fn config_round_trips() {
        let mut c = Config {
            catalog_url: "file:///tmp/catalog.json".into(),
            ..Default::default()
        };
        c.extensions.insert(
            "a.b".into(),
            ExtState {
                enabled: false,
                approved: vec![Permission::Net],
                revoked: vec![Permission::UiToolbar],
            },
        );
        c.check_updates = true;
        let key = super::super::signing::test_key().verifying_key().to_bytes();
        c.trusted_keys.insert(
            "Someone".into(),
            super::super::signing::public_key_hex(&key),
        );
        assert_eq!(Config::from_toml(&c.to_toml()), c);
        // A bad key is dropped, not trusted.
        let bad = "[trusted_keys]\nx = \"zz\"\n";
        assert!(Config::from_toml(bad).trusted_keys.is_empty());
    }
}
