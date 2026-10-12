//! Window > Environments (FEATURES.md 66): the environment library, the panel's data
//! and events, and what the viewport gets.
//!
//! The library holds the bundled procedural skies ([`mt_render::ProceduralSky`],
//! generated at first use) and panoramas the user added (HDR, EXR, PNG, JPEG), whose
//! paths are kept in `<config>/meshtard/environments.toml`. The model's environment
//! (`Model::environment`, model-only: not written to .skp) names a library entry by
//! key: `builtin:<name>` or the file path. A model naming a file not in the library
//! loads it (without adding it to the saved list).
//!
//! Edits are display settings, not undo steps (as the Shadows panel here).
//! Set Sun Location replaces the Shadows panel's sun direction (help page [doc]).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::DVec3;
use mt_kernel::model_data::EnvironmentSettings;
use mt_render::{EnvImage, EnvView, ProceduralSky};
use mt_ui::{EnvSettings, EnvironmentData, EnvironmentRow, Event};
use serde::{Deserialize, Serialize};

use crate::App;

/// Width of the generated skies, in pixels.
const PROCEDURAL_WIDTH: u32 = 1024;
const THUMB_WIDTH: u32 = 128;
const PREVIEW_WIDTH: u32 = 512;

struct Entry {
    key: String,
    name: String,
    image: Arc<EnvImage>,
    /// Added by the user and saved in the list.
    removable: bool,
    thumb: Option<egui::TextureHandle>,
}

/// The library and its textures.
#[derive(Default)]
pub(crate) struct Environments {
    entries: Vec<Entry>,
    loaded: bool,
    /// Preview texture of the entry it was made for.
    preview: Option<(String, egui::TextureHandle)>,
    /// What the viewport was last given (key and view), to skip redundant updates.
    applied: Option<(String, [i64; 5])>,
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
struct SavedList {
    #[serde(default)]
    files: Vec<PathBuf>,
}

fn list_path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("environments.toml"))
}

pub(crate) fn builtin_key(sky: ProceduralSky) -> String {
    format!("builtin:{}", sky.name())
}

fn file_name(path: &Path) -> String {
    path.file_stem().map_or_else(
        || path.display().to_string(),
        |s| s.to_string_lossy().into_owned(),
    )
}

/// The panel's settings from the model's.
pub(crate) fn to_ui(s: &EnvironmentSettings) -> EnvSettings {
    EnvSettings {
        skydome: s.skydome,
        reflections: s.reflections,
        rotation_deg: s.rotation_deg.round().rem_euclid(360.0) as i32,
        skydome_exposure: (s.skydome_exposure * 100.0).round() as i32,
        reflection_exposure: (s.reflection_exposure * 100.0).round() as i32,
        sun: s
            .sun_uv
            .map(|[u, v]| [(u * 1000.0).round() as u16, (v * 1000.0).round() as u16]),
    }
}

/// The panel's settings written into the model's (the source is kept).
pub(crate) fn from_ui(e: &EnvSettings, s: &mut EnvironmentSettings) {
    s.skydome = e.skydome;
    s.reflections = e.reflections;
    s.rotation_deg = f64::from(e.rotation_deg);
    s.skydome_exposure = f64::from(e.skydome_exposure.max(0)) / 100.0;
    s.reflection_exposure = f64::from(e.reflection_exposure.max(0)) / 100.0;
    s.sun_uv = e
        .sun
        .map(|[u, v]| [f64::from(u) / 1000.0, f64::from(v) / 1000.0]);
}

/// Set Sun Location: the direction towards the sun for the model's environment, if
/// one is set (the panorama's rotation applies).
pub(crate) fn sun_override(s: &EnvironmentSettings) -> Option<DVec3> {
    s.source.as_ref()?;
    let [u, v] = s.sun_uv?;
    let d = mt_render::environment::uv_to_dir(u as f32, v as f32, s.rotation_deg as f32);
    Some(DVec3::new(d[0].into(), d[1].into(), d[2].into()))
}

impl Environments {
    fn load(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        for sky in ProceduralSky::ALL {
            self.entries.push(Entry {
                key: builtin_key(sky),
                name: sky.name().to_owned(),
                image: Arc::new(EnvImage::procedural(sky, PROCEDURAL_WIDTH)),
                removable: false,
                thumb: None,
            });
        }
        let list: SavedList = list_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default();
        for path in list.files {
            match EnvImage::open(&path) {
                Ok(img) => {
                    self.push_file(&path, img, true);
                }
                Err(e) => log::warn!("environment {}: {e}", path.display()),
            }
        }
    }

    fn push_file(&mut self, path: &Path, img: EnvImage, removable: bool) -> usize {
        let key = path.display().to_string();
        if let Some(i) = self.index(&key) {
            return i;
        }
        self.entries.push(Entry {
            key,
            name: file_name(path),
            image: Arc::new(img),
            removable,
            thumb: None,
        });
        self.entries.len() - 1
    }

    fn save_list(&self) {
        let list = SavedList {
            files: self
                .entries
                .iter()
                .filter(|e| e.removable)
                .map(|e| PathBuf::from(&e.key))
                .collect(),
        };
        if let Some(p) = list_path() {
            let _ = p.parent().map(std::fs::create_dir_all);
            if let Ok(text) = toml::to_string(&list)
                && let Err(e) = std::fs::write(&p, text)
            {
                log::warn!("saving {}: {e}", p.display());
            }
        }
    }

    fn index(&self, key: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.key == key)
    }

    /// The entry for `key`, loading a file the model names if needed.
    fn find(&mut self, key: &str) -> Option<usize> {
        self.load();
        if let Some(i) = self.index(key) {
            return Some(i);
        }
        if key.starts_with("builtin:") {
            return None;
        }
        let img = EnvImage::open(Path::new(key)).ok()?;
        Some(self.push_file(Path::new(key), img, false))
    }

    /// What the panel shows. Loads the library only when the panel is open or the
    /// model has an environment.
    pub(crate) fn data(
        &mut self,
        ctx: &egui::Context,
        settings: Option<&EnvironmentSettings>,
        panel_open: bool,
    ) -> EnvironmentData {
        let source = settings.and_then(|s| s.source.clone());
        if !panel_open && source.is_none() {
            return EnvironmentData {
                settings: settings.map(to_ui).unwrap_or_default(),
                ..Default::default()
            };
        }
        self.load();
        let current = source.as_deref().and_then(|k| self.find(k));
        let mut rows = Vec::with_capacity(self.entries.len());
        for e in &mut self.entries {
            if e.thumb.is_none() && panel_open {
                e.thumb = Some(texture(ctx, &e.key, &e.image, THUMB_WIDTH));
            }
            rows.push(EnvironmentRow {
                name: e.name.clone(),
                thumbnail: e.thumb.as_ref().map(egui::TextureHandle::id),
                removable: e.removable,
            });
        }
        let preview = match current {
            Some(i) if panel_open => {
                let e = &self.entries[i];
                if self.preview.as_ref().is_none_or(|(k, _)| *k != e.key) {
                    self.preview =
                        Some((e.key.clone(), texture(ctx, &e.key, &e.image, PREVIEW_WIDTH)));
                }
                self.preview
                    .as_ref()
                    .map(|(_, t)| (t.id(), [t.size()[0] as u32, t.size()[1] as u32]))
            }
            _ => None,
        };
        EnvironmentData {
            rows,
            current,
            settings: settings.map(to_ui).unwrap_or_default(),
            preview,
        }
    }

    /// The image and view for the viewport, or `None`.
    fn view(&mut self, s: &EnvironmentSettings) -> Option<(Arc<EnvImage>, EnvView)> {
        let i = self.find(s.source.as_deref()?)?;
        Some((
            self.entries[i].image.clone(),
            EnvView {
                skydome: s.skydome,
                reflections: s.reflections,
                rotation_deg: s.rotation_deg as f32,
                skydome_exposure: s.skydome_exposure as f32,
                reflection_exposure: s.reflection_exposure as f32,
            },
        ))
    }
}

fn texture(ctx: &egui::Context, key: &str, img: &EnvImage, width: u32) -> egui::TextureHandle {
    let (w, h, rgba) = img.preview_rgba(width, 1.0);
    ctx.load_texture(
        format!("mt_env_{key}_{width}"),
        egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba),
        egui::TextureOptions::LINEAR,
    )
}

impl App {
    /// Passes the model's environment to the viewport (from `apply_shadows`).
    pub(crate) fn apply_environment(&mut self) {
        let settings = self.document.as_ref().map(|d| d.model.environment.clone());
        let view = settings.as_ref().and_then(|s| self.envs.view(s));
        let key = view.as_ref().map(|(img, v)| {
            (
                format!("{:p}", Arc::as_ptr(img)),
                [
                    i64::from(v.skydome),
                    i64::from(v.reflections),
                    (v.rotation_deg * 100.0) as i64,
                    (v.skydome_exposure * 1000.0) as i64,
                    (v.reflection_exposure * 1000.0) as i64,
                ],
            )
        });
        if key != self.envs.applied {
            self.envs.applied = key;
            self.viewport.set_environment(view);
        }
    }

    /// The panel's data for this frame.
    pub(crate) fn environment_data(
        &mut self,
        ctx: &egui::Context,
        panel_open: bool,
    ) -> EnvironmentData {
        let settings = self.document.as_ref().map(|d| d.model.environment.clone());
        self.envs.data(ctx, settings.as_ref(), panel_open)
    }

    /// Handles an Environments panel event; `None` if `event` is not one. The result
    /// tells whether the view must be rebuilt.
    pub(crate) fn environment_event(&mut self, event: &Event) -> Option<bool> {
        match event {
            Event::EnvironmentSelect(i) => {
                self.envs.load();
                let key = i
                    .and_then(|i| self.envs.entries.get(i))
                    .map(|e| e.key.clone());
                if let Some(doc) = &mut self.document {
                    let env = &mut doc.model.environment;
                    env.source = key;
                    // The sun point belongs to the panorama it was set on.
                    env.sun_uv = None;
                }
            }
            Event::EnvironmentAdd => {
                let filters: &[(&str, &[&str])] =
                    &[("Environment images", &["hdr", "exr", "png", "jpg", "jpeg"])];
                if let Some(path) = crate::filedialog::open(filters) {
                    self.add_environment(&path);
                }
            }
            Event::EnvironmentRemove(i) => {
                if self.envs.entries.get(*i).is_some_and(|e| e.removable) {
                    let e = self.envs.entries.remove(*i);
                    self.envs.save_list();
                    if let Some(doc) = &mut self.document
                        && doc.model.environment.source.as_deref() == Some(&e.key)
                    {
                        doc.model.environment.source = None;
                    }
                }
            }
            Event::EnvironmentChanged(s) => {
                if let Some(doc) = &mut self.document {
                    from_ui(s, &mut doc.model.environment);
                }
            }
            _ => return None,
        }
        self.apply_shadows();
        Some(false)
    }

    /// Adds a panorama file to the library and makes it the model's environment.
    pub(crate) fn add_environment(&mut self, path: &Path) {
        self.envs.load();
        match EnvImage::open(path) {
            Ok(img) => {
                if img.width < 2 * img.height.saturating_sub(img.height / 10) {
                    log::warn!(
                        "{}: {}x{} is not a 2:1 panorama; it is used anyway",
                        path.display(),
                        img.width,
                        img.height
                    );
                }
                let i = self.envs.push_file(path, img, true);
                self.envs.entries[i].removable = true;
                self.envs.save_list();
                let key = self.envs.entries[i].key.clone();
                if let Some(doc) = &mut self.document {
                    doc.model.environment.source = Some(key);
                    doc.model.environment.sun_uv = None;
                }
            }
            Err(e) => self.file_error("add the environment", path, e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_through_the_panel() {
        let s = EnvironmentSettings {
            source: Some("builtin:Sunset".into()),
            skydome: false,
            reflections: true,
            rotation_deg: 90.0,
            skydome_exposure: 1.5,
            reflection_exposure: 0.25,
            sun_uv: Some([0.25, 0.4]),
        };
        let ui = to_ui(&s);
        assert_eq!(ui.rotation_deg, 90);
        assert_eq!(ui.skydome_exposure, 150);
        assert_eq!(ui.sun, Some([250, 400]));
        let mut back = EnvironmentSettings {
            source: s.source.clone(),
            ..Default::default()
        };
        from_ui(&ui, &mut back);
        assert_eq!(back, s);
    }

    #[test]
    fn sun_location_overrides_only_with_an_environment() {
        let mut s = EnvironmentSettings {
            sun_uv: Some([0.5, 0.25]),
            ..Default::default()
        };
        assert!(sun_override(&s).is_none(), "no environment, no override");
        s.source = Some("builtin:Clear Day".into());
        let d = sun_override(&s).unwrap();
        // u = 0.5 looks along +X; v = 0.25 is 45 degrees up.
        assert!(
            (d.x - d.z).abs() < 1e-5 && d.x > 0.0 && d.y.abs() < 1e-5,
            "{d}"
        );
        // Rotation turns it about Z.
        s.rotation_deg = 90.0;
        let d = sun_override(&s).unwrap();
        assert!(d.y > 0.7 && d.x.abs() < 1e-5, "{d}");
    }

    #[test]
    fn library_finds_builtins_and_lists_saved_files() {
        let mut envs = Environments::default();
        let i = envs.find(&builtin_key(ProceduralSky::Overcast)).unwrap();
        assert_eq!(envs.entries[i].name, "Overcast");
        assert!(!envs.entries[i].removable);
        assert!(envs.find("builtin:Nope").is_none());
        assert!(envs.find("/no/such/file.hdr").is_none());
        // A real file the model names is loaded but not saved in the list.
        let dir = std::env::temp_dir().join(format!("mt_env_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pano.png");
        image::RgbImage::from_pixel(8, 4, image::Rgb([10, 20, 30]))
            .save(&path)
            .unwrap();
        let key = path.display().to_string();
        let j = envs.find(&key).unwrap();
        assert_eq!(envs.entries[j].name, "pano");
        assert!(!envs.entries[j].removable);
        let view = envs
            .view(&EnvironmentSettings {
                source: Some(key),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(view.0.width, 8);
        let _ = std::fs::remove_dir_all(dir);
    }
}
