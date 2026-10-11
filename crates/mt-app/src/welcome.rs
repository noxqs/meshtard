//! Auto-save recovery files, the recent files list and the Welcome window
//! (Preferences > General: Auto-save, Show Welcome Window; Help > Welcome to
//! Meshtard).
//!
//! Auto-save writes the model with unsaved changes to `AutoSave_<name>~<pid>.skp` in
//! `recovery/` under the user data folder (`settings::data_dir`), with the model's own
//! path in a `.origin` file next to it; the process id keeps two running meshtards
//! apart. The file is removed when the model is saved, closed without saving or the
//! app exits normally, so what is left at start comes from a crash: the Welcome window
//! lists it under Recovered with Open and Discard. An opened recovered model is a new
//! unsaved model named after the original file (Save asks where to save it).
//! SketchUp (from its help) also writes a temporary recovery file and offers it at the
//! next start; its names, folder and dialog are unverified.
//!
//! The recent files (up to [`RECENT_MAX`], newest first) are kept in `recent.toml` in
//! the config folder. The Welcome window's layout (recent files, templates, recovered
//! files, Open...) is from memory of SketchUp's Welcome screen and unverified.

use std::path::{Path, PathBuf};

use eframe::egui;

use crate::App;
use crate::confirm::Pending;
use crate::document::Document;

/// How many recent files are kept (unverified for SketchUp).
pub(crate) const RECENT_MAX: usize = 10;

const PREFIX: &str = "AutoSave_";

pub(crate) fn recovery_dir() -> Option<PathBuf> {
    crate::settings::data_dir().map(|d| d.join("recovery"))
}

/// The recovery file name of the model at `model` (its file name, or Untitled) for
/// process `pid`.
pub(crate) fn recovery_name(model: &Path, pid: u32) -> String {
    let stem = model
        .file_stem()
        .map_or_else(|| "Untitled".into(), |s| s.to_string_lossy().into_owned());
    format!("{PREFIX}{stem}~{pid}.skp")
}

/// The model name shown for a recovery file name (`AutoSave_house~12.skp`: house).
pub(crate) fn display_name(file_name: &str) -> String {
    let s = file_name.strip_suffix(".skp").unwrap_or(file_name);
    let s = s.strip_prefix(PREFIX).unwrap_or(s);
    match s.rsplit_once('~') {
        Some((name, pid)) if !pid.is_empty() && pid.chars().all(|c| c.is_ascii_digit()) => {
            name.to_owned()
        }
        _ => s.to_owned(),
    }
}

fn origin_path(recovery: &Path) -> PathBuf {
    let mut s = recovery.as_os_str().to_owned();
    s.push(".origin");
    PathBuf::from(s)
}

/// A recovery file found at start.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Recovered {
    pub path: PathBuf,
    /// The model's own file, when it had one.
    pub origin: Option<PathBuf>,
    pub name: String,
}

/// The recovery files in `dir`, newest first.
pub(crate) fn list_recovered(dir: &Path) -> Vec<Recovered> {
    let mut out: Vec<(std::time::SystemTime, Recovered)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = path.file_name()?.to_str()?.to_owned();
            if !(name.starts_with(PREFIX) && name.ends_with(".skp") && path.is_file()) {
                return None;
            }
            let origin = std::fs::read_to_string(origin_path(&path))
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from);
            let time = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            Some((
                time,
                Recovered {
                    name: display_name(&name),
                    path,
                    origin,
                },
            ))
        })
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.0));
    out.into_iter().map(|(_, r)| r).collect()
}

/// Removes a recovery file and its `.origin`.
pub(crate) fn remove_recovery(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(origin_path(path));
}

/// Writes `bytes` as the recovery file `path` of the model at `origin` (`None`:
/// untitled).
pub(crate) fn write_recovery(
    path: &Path,
    bytes: &[u8],
    origin: Option<&Path>,
) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, bytes)?;
    let origin = origin.map_or_else(String::new, |p| p.to_string_lossy().into_owned());
    std::fs::write(origin_path(path), origin)
}

/// Moves a recovery file (and its `.origin`) to `to`.
fn rename_recovery(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)?;
    let _ = std::fs::rename(origin_path(from), origin_path(to));
    Ok(())
}

/// `list` with `path` moved (or added) to the front, at most `max` long.
pub(crate) fn push_recent(list: &mut Vec<PathBuf>, path: &Path, max: usize) {
    list.retain(|p| p != path);
    list.insert(0, path.to_owned());
    list.truncate(max);
}

pub(crate) fn parse_recent(text: &str) -> Vec<PathBuf> {
    let Ok(t) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    t.get("files")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .map(PathBuf::from)
        .take(RECENT_MAX)
        .collect()
}

pub(crate) fn recent_text(list: &[PathBuf]) -> String {
    let files: Vec<toml::Value> = list
        .iter()
        .map(|p| toml::Value::String(p.to_string_lossy().into_owned()))
        .collect();
    let mut t = toml::Table::new();
    t.insert("files".into(), toml::Value::Array(files));
    format!("# meshtard recent files, newest first.\n{t}")
}

fn recent_file() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("recent.toml"))
}

pub(crate) fn load_recent() -> Vec<PathBuf> {
    recent_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| parse_recent(&t))
        .unwrap_or_default()
}

/// Adds `path` to the saved recent files.
pub(crate) fn add_recent(path: &Path) {
    let Some(file) = recent_file() else { return };
    let mut list = load_recent();
    push_recent(&mut list, path, RECENT_MAX);
    let result = std::fs::create_dir_all(file.parent().expect("has a parent"))
        .and_then(|()| std::fs::write(&file, recent_text(&list)));
    if let Err(e) = result {
        log::warn!("could not save the recent files: {e}");
    }
}

/// The Welcome window's lists.
#[derive(Debug, Clone, Default)]
pub(crate) struct Welcome {
    recent: Vec<PathBuf>,
    templates: Vec<(String, PathBuf)>,
    recovered: Vec<Recovered>,
    /// The chosen template (`None`: the built-in one).
    template: Option<usize>,
}

impl Welcome {
    pub(crate) fn load() -> Self {
        let templates = crate::templates::templates_dir()
            .map_or_else(Vec::new, |d| crate::templates::list_templates(&d));
        let default = crate::templates::load_default();
        Self {
            recent: load_recent(),
            template: default.and_then(|n| templates.iter().position(|(m, _)| *m == n)),
            templates,
            recovered: recovery_dir().map_or_else(Vec::new, |d| list_recovered(&d)),
        }
    }

    pub(crate) fn has_recovered(&self) -> bool {
        !self.recovered.is_empty()
    }
}

enum Action {
    Close,
    Open(Option<PathBuf>),
    New(Option<PathBuf>),
    Recover(usize),
    Discard(usize),
}

impl App {
    /// Help > Welcome to Meshtard.
    pub(crate) fn open_welcome(&mut self) {
        self.prefs.welcome = Some(Welcome::load());
    }

    /// The Welcome window, while open.
    pub(crate) fn welcome_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(mut w) = self.prefs.welcome.take() else {
            return;
        };
        let mut show = self.prefs.general.welcome;
        let mut action = None;
        let r = mt_ui::Dialog::new("welcome", "Welcome to Meshtard")
            .width(520.0)
            .show(ctx, |ui| {
                crate::export::dialog_visuals(ui);
                if !w.recovered.is_empty() {
                    ui.strong("Recovered");
                    ui.label(
                        egui::RichText::new(
                            "These models were auto-saved before meshtard closed unexpectedly.",
                        )
                        .small()
                        .weak(),
                    );
                    for (i, r) in w.recovered.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.button("Open").clicked() {
                                action = Some(Action::Recover(i));
                            }
                            if ui.button("Discard").clicked() {
                                action = Some(Action::Discard(i));
                            }
                            match &r.origin {
                                Some(p) => ui.label(format!("{} ({})", r.name, p.display())),
                                None => ui.label(&r.name),
                            };
                        });
                    }
                    ui.separator();
                }
                ui.columns(2, |cols| {
                    let ui = &mut cols[0];
                    ui.strong("Recent files");
                    if w.recent.is_empty() {
                        ui.weak("No recent files.");
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("mt_welcome_recent")
                        .max_height(200.0)
                        .show(ui, |ui| {
                            for p in &w.recent {
                                let name = p
                                    .file_name()
                                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                                let r = ui
                                    .add_enabled(p.is_file(), egui::Button::selectable(false, name))
                                    .on_hover_text(p.display().to_string());
                                if r.clicked() {
                                    action = Some(Action::Open(Some(p.clone())));
                                }
                            }
                        });
                    let ui = &mut cols[1];
                    ui.strong("Templates");
                    egui::ScrollArea::vertical()
                        .id_salt("mt_welcome_templates")
                        .max_height(200.0)
                        .show(ui, |ui| {
                            let r = ui
                                .selectable_label(w.template.is_none(), crate::templates::BUILT_IN);
                            if r.clicked() {
                                w.template = None;
                            }
                            if r.double_clicked() {
                                action = Some(Action::New(None));
                            }
                            for (i, (name, path)) in w.templates.iter().enumerate() {
                                let r = ui.selectable_label(w.template == Some(i), name);
                                if r.clicked() {
                                    w.template = Some(i);
                                }
                                if r.double_clicked() {
                                    action = Some(Action::New(Some(path.clone())));
                                }
                            }
                        });
                });
                ui.add_space(8.0);
                ui.checkbox(&mut show, "Show the Welcome window at startup");
                match mt_ui::dialog::buttons(ui, &["Open...", "New Model", "Close"], Some(1)) {
                    Some(0) => action = Some(Action::Open(None)),
                    Some(1) => {
                        let t = w
                            .template
                            .and_then(|i| w.templates.get(i))
                            .map(|(_, p)| p.clone());
                        action = Some(Action::New(t));
                    }
                    Some(_) => action = Some(Action::Close),
                    None => {}
                }
            });
        if show != self.prefs.general.welcome {
            self.prefs.general.welcome = show;
            self.save_preferences();
        }
        match action {
            _ if r.closed => {}
            None => self.prefs.welcome = Some(w),
            Some(Action::Close) => {}
            Some(Action::Open(p)) => self.request(ctx, frame, Pending::Open(p)),
            Some(Action::New(t)) => self.request(ctx, frame, Pending::NewFrom(t)),
            Some(Action::Discard(i)) => {
                remove_recovery(&w.recovered.remove(i).path);
                self.prefs.welcome = Some(w);
            }
            Some(Action::Recover(i)) => {
                let r = w.recovered[i].clone();
                if self.document.as_ref().is_some_and(Document::is_modified) {
                    self.message = Some(
                        "Save or undo the current model's changes before opening a recovered one."
                            .into(),
                    );
                    self.prefs.welcome = Some(w);
                } else {
                    self.open_recovered(ctx, frame, &r);
                }
            }
        }
    }

    /// Opens a recovery file as an unsaved model named after its original file, and
    /// makes the file this session's recovery file of that model.
    fn open_recovered(&mut self, ctx: &egui::Context, frame: &eframe::Frame, r: &Recovered) {
        let mut doc = match Document::open(&r.path) {
            Ok(doc) => doc,
            Err(e) => {
                self.file_error("open", &r.path, e);
                return;
            }
        };
        doc.path = r
            .origin
            .clone()
            .unwrap_or_else(|| PathBuf::from("Untitled.skp"));
        doc.has_file = false;
        doc.mark_unsaved();
        if let Some(dir) = recovery_dir() {
            let to = dir.join(recovery_name(&doc.path, std::process::id()));
            if rename_recovery(&r.path, &to).is_ok() {
                self.prefs.recovery_written = Some(to);
                self.prefs.autosaved = Some(doc.state_id());
            }
        }
        if let Some(rs) = frame.wgpu_render_state() {
            let device = rs.device.clone();
            let path = doc.path.clone();
            self.install(ctx, &device, &path, doc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_names() {
        let n = recovery_name(Path::new("/m/house.skp"), 42);
        assert_eq!(n, "AutoSave_house~42.skp");
        assert_eq!(display_name(&n), "house");
        assert_eq!(
            recovery_name(Path::new("Untitled.skp"), 7),
            "AutoSave_Untitled~7.skp"
        );
        assert_eq!(display_name("AutoSave_a~b.skp"), "a~b");
    }

    #[test]
    fn recovery_files_are_listed_with_their_origin() {
        let dir = std::env::temp_dir().join(format!("mt_recovery_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = dir.join(recovery_name(Path::new("/m/house.skp"), 1));
        let b = dir.join(recovery_name(Path::new("Untitled.skp"), 2));
        write_recovery(&a, b"x", Some(Path::new("/m/house.skp"))).unwrap();
        write_recovery(&b, b"y", None).unwrap();
        std::fs::write(dir.join("notes.txt"), "z").unwrap();
        let mut found = list_recovered(&dir);
        found.sort_by(|x, y| x.name.cmp(&y.name));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "Untitled");
        assert_eq!(found[0].origin, None);
        assert_eq!(found[1].name, "house");
        assert_eq!(found[1].origin.as_deref(), Some(Path::new("/m/house.skp")));
        let c = dir.join(recovery_name(Path::new("/m/house.skp"), 3));
        rename_recovery(&a, &c).unwrap();
        assert!(!a.exists() && c.exists() && origin_path(&c).exists());
        remove_recovery(&c);
        remove_recovery(&b);
        assert!(!c.exists() && !origin_path(&c).exists());
        assert!(list_recovered(&dir).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn recent_list_keeps_newest_first() {
        let mut l = Vec::new();
        for p in ["/a.skp", "/b.skp", "/a.skp", "/c.skp"] {
            push_recent(&mut l, Path::new(p), 2);
        }
        assert_eq!(l, [PathBuf::from("/c.skp"), PathBuf::from("/a.skp")]);
        assert_eq!(parse_recent(&recent_text(&l)), l);
        assert!(parse_recent("garbage [").is_empty());
    }
}
