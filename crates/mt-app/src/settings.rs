//! Settings kept between runs, in the user's config directory: `meshtard/` under
//! `$XDG_CONFIG_HOME` (or `~/.config`) on Linux, `%APPDATA%` on Windows and
//! `~/Library/Application Support` on macOS.

use std::path::PathBuf;

pub(crate) fn config_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.map(|b| b.join("meshtard"))
}

fn toolbars_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("toolbars.toml"))
}

/// The saved toolbar layout, if there is a readable one.
pub fn load_toolbar_layout() -> Option<mt_ui::ToolbarLayout> {
    let text = std::fs::read_to_string(toolbars_path()?).ok()?;
    match toml::from_str(&text) {
        Ok(layout) => Some(layout),
        Err(e) => {
            log::warn!("ignoring saved toolbar layout: {e}");
            None
        }
    }
}

pub fn save_toolbar_layout(layout: &mt_ui::ToolbarLayout) {
    let Some(path) = toolbars_path() else { return };
    let result = toml::to_string(layout)
        .map_err(std::io::Error::other)
        .and_then(|text| {
            std::fs::create_dir_all(path.parent().expect("has a parent"))?;
            std::fs::write(&path, text)
        });
    if let Err(e) = result {
        log::warn!(
            "could not save the toolbar layout to {}: {e}",
            path.display()
        );
    }
}

fn trays_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("trays.toml"))
}

/// The saved trays (Window > Manage Trays), if there is a readable file.
pub fn load_tray_layout() -> Option<mt_ui::TrayLayout> {
    let text = std::fs::read_to_string(trays_path()?).ok()?;
    match toml::from_str(&text) {
        Ok(layout) => Some(layout),
        Err(e) => {
            log::warn!("ignoring saved trays: {e}");
            None
        }
    }
}

pub fn save_tray_layout(layout: &mt_ui::TrayLayout) {
    let Some(path) = trays_path() else { return };
    let result = toml::to_string(layout)
        .map_err(std::io::Error::other)
        .and_then(|text| {
            std::fs::create_dir_all(path.parent().expect("has a parent"))?;
            std::fs::write(&path, text)
        });
    if let Err(e) = result {
        log::warn!("could not save the trays to {}: {e}", path.display());
    }
}

/// The user data folder for files that are not settings (auto-save recovery files):
/// `meshtard/` under `$XDG_DATA_HOME` (or `~/.local/share`) on Linux, `%LOCALAPPDATA%`
/// on Windows and `~/Library/Application Support` on macOS.
pub(crate) fn data_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("APPDATA"))
            .map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    };
    base.map(|b| b.join("meshtard"))
}
