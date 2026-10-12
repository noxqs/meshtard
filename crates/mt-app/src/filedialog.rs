//! Native file dialogs (through `rfd`), with a test hook: when the environment
//! variable `MESHTARD_TEST_DIALOG` names a file, every dialog answers with the path in
//! that file's first line instead (empty: cancelled). Used for GUI checks where no
//! desktop portal is running.
//!
//! Dialogs start in the Preferences > Files folder for what they open or save
//! ([`Folder`], chosen from the file types: models, styles, texture images, exports;
//! or named by the caller with [`open_in`] / [`save_in`]: components, watermark
//! images).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Preferences > Files: the default folders (SketchUp's list, from its help).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Folder {
    Models,
    Components,
    Materials,
    Styles,
    TextureImages,
    WatermarkImages,
    Export,
    Classifications,
    Templates,
}

impl Folder {
    pub(crate) const ALL: [Folder; 9] = [
        Folder::Models,
        Folder::Components,
        Folder::Materials,
        Folder::Styles,
        Folder::TextureImages,
        Folder::WatermarkImages,
        Folder::Export,
        Folder::Classifications,
        Folder::Templates,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Folder::Models => "Models",
            Folder::Components => "Components",
            Folder::Materials => "Materials",
            Folder::Styles => "Styles",
            Folder::TextureImages => "Texture Images",
            Folder::WatermarkImages => "Watermark Images",
            Folder::Export => "Export",
            Folder::Classifications => "Classifications",
            Folder::Templates => "Templates",
        }
    }

    /// Key in `preferences.toml`.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Folder::Models => "models",
            Folder::Components => "components",
            Folder::Materials => "materials",
            Folder::Styles => "styles",
            Folder::TextureImages => "texture_images",
            Folder::WatermarkImages => "watermark_images",
            Folder::Export => "export",
            Folder::Classifications => "classifications",
            Folder::Templates => "templates",
        }
    }

    fn index(self) -> usize {
        Folder::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }
}

/// The default folders set in Preferences > Files (`None`: the system's choice).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Folders(pub [Option<PathBuf>; 9]);

impl Folders {
    pub(crate) fn get(&self, f: Folder) -> Option<&Path> {
        self.0[f.index()].as_deref()
    }

    pub(crate) fn set(&mut self, f: Folder, path: Option<PathBuf>) {
        self.0[f.index()] = path;
    }
}

static FOLDERS: Mutex<Folders> = Mutex::new(Folders([
    None, None, None, None, None, None, None, None, None,
]));

/// Makes `folders` the dialogs' start folders.
pub(crate) fn set_folders(folders: &Folders) {
    if let Ok(mut f) = FOLDERS.lock() {
        *f = folders.clone();
    }
}

/// A set default folder.
pub(crate) fn folder(f: Folder) -> Option<PathBuf> {
    FOLDERS.lock().ok()?.get(f).map(Path::to_path_buf)
}

const IMAGES: [&str; 8] = ["png", "jpg", "jpeg", "bmp", "tif", "tiff", "tga", "gif"];

/// Which default folder a dialog for `filters` starts in: models for `.skp`, styles
/// for `.style`, texture images when opening images, Export for other saved files,
/// Models for other opened (imported) files. Unverified: which folder SketchUp's
/// Import dialog starts in.
pub(crate) fn folder_for(saving: bool, filters: &[(&str, &[&str])]) -> Folder {
    let has = |ext: &str| {
        filters
            .iter()
            .any(|(_, exts)| exts.iter().any(|e| e.eq_ignore_ascii_case(ext)))
    };
    if has("skp") {
        Folder::Models
    } else if has("style") {
        Folder::Styles
    } else if saving {
        Folder::Export
    } else if IMAGES.iter().any(|e| has(e)) {
        Folder::TextureImages
    } else {
        Folder::Models
    }
}

fn start_dir(f: Folder) -> Option<PathBuf> {
    folder(f).filter(|d| d.is_dir())
}

fn test_answer() -> Option<Option<PathBuf>> {
    let hook = std::env::var_os("MESHTARD_TEST_DIALOG")?;
    let text = std::fs::read_to_string(hook).unwrap_or_default();
    let line = text.lines().next().unwrap_or("").trim();
    Some((!line.is_empty()).then(|| PathBuf::from(line)))
}

/// A "Save" dialog. `filters` are (name, extensions); a chosen name without an
/// extension gets the first filter's first extension.
pub(crate) fn save(filters: &[(&str, &[&str])], file_name: &str) -> Option<PathBuf> {
    save_in(folder_for(true, filters), filters, file_name)
}

/// A "Save" dialog starting in the default folder `f`.
pub(crate) fn save_in(f: Folder, filters: &[(&str, &[&str])], file_name: &str) -> Option<PathBuf> {
    let path = match test_answer() {
        Some(answer) => answer,
        None => {
            let mut d = rfd::FileDialog::new().set_file_name(file_name);
            if let Some(dir) = start_dir(f) {
                d = d.set_directory(dir);
            }
            for (name, exts) in filters {
                d = d.add_filter(*name, exts);
            }
            d.save_file()
        }
    }?;
    Some(match (path.extension(), filters.first()) {
        (None, Some((_, [ext, ..]))) => path.with_extension(ext),
        _ => path,
    })
}

/// An "Open" dialog.
pub(crate) fn open(filters: &[(&str, &[&str])]) -> Option<PathBuf> {
    open_in(folder_for(false, filters), filters)
}

/// An "Open" dialog starting in the default folder `f`.
pub(crate) fn open_in(f: Folder, filters: &[(&str, &[&str])]) -> Option<PathBuf> {
    if let Some(answer) = test_answer() {
        return answer;
    }
    let mut d = rfd::FileDialog::new();
    if let Some(dir) = start_dir(f) {
        d = d.set_directory(dir);
    }
    for (name, exts) in filters {
        d = d.add_filter(*name, exts);
    }
    d.pick_file()
}

/// A folder picker (Preferences > Files > Change).
pub(crate) fn pick_folder(start: Option<&Path>) -> Option<PathBuf> {
    if let Some(answer) = test_answer() {
        return answer;
    }
    let mut d = rfd::FileDialog::new();
    if let Some(dir) = start.filter(|d| d.is_dir()) {
        d = d.set_directory(dir);
    }
    d.pick_folder()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialogs_pick_their_folder_by_file_type() {
        let skp: &[(&str, &[&str])] = &[("Models", &["skp"])];
        let png: &[(&str, &[&str])] = &[("PNG", &["png"]), ("JPEG", &["jpg"])];
        let obj: &[(&str, &[&str])] = &[("OBJ", &["obj"])];
        assert_eq!(folder_for(false, skp), Folder::Models);
        assert_eq!(folder_for(true, skp), Folder::Models);
        assert_eq!(folder_for(true, png), Folder::Export);
        assert_eq!(folder_for(false, png), Folder::TextureImages);
        assert_eq!(folder_for(false, obj), Folder::Models);
        assert_eq!(folder_for(true, obj), Folder::Export);
        let style: &[(&str, &[&str])] = &[("Styles", &["style"])];
        assert_eq!(folder_for(false, style), Folder::Styles);
        assert_eq!(folder_for(true, style), Folder::Styles);
        let mut f = Folders::default();
        f.set(Folder::Export, Some(PathBuf::from("/x")));
        assert_eq!(f.get(Folder::Export), Some(Path::new("/x")));
        assert_eq!(f.get(Folder::Models), None);
    }
}
