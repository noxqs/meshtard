//! The texture controls of the Materials panel's Edit tab: load or replace the image,
//! remove it, size with aspect lock, Colorize, Reset Color, and editing the image in
//! the system's default editor (the file is watched and reloaded when it changes).
//! Each change is one undo step.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use mt_infer::units::{LengthUnit, parse_length};
use mt_kernel::{Material, Texture};
use mt_ui::{TextureAction, TextureRow};

use crate::App;

/// Width of a newly loaded texture when the material had none, in inches (1 m;
/// SketchUp's default is unverified). The height follows the image's aspect.
const NEW_TEXTURE_WIDTH: f64 = 1000.0 / 25.4;

/// How often the externally edited file is checked.
const WATCH_INTERVAL: Duration = Duration::from_millis(700);

/// Image formats offered when loading a texture (those the app can decode).
const IMAGE_FILTERS: &[(&str, &[&str])] = &[("Images", &["png", "jpg", "jpeg"])];

/// A texture image being edited in another program.
pub(crate) struct ExternalEdit {
    material: usize,
    /// The material's name, to notice it was deleted or replaced meanwhile.
    name: String,
    path: PathBuf,
    modified: Option<SystemTime>,
    checked: Instant,
}

/// The Edit tab's view of a texture, sizes in `units`.
pub(crate) fn texture_row(t: &Texture, units: LengthUnit) -> TextureRow {
    TextureRow {
        file: t.filename.clone(),
        width: units.format(tile(t.width)),
        height: units.format(tile(t.height)),
        colorize: t.colorize,
    }
}

/// Tile sizes of 0 count as 1 inch, as when drawing.
fn tile(s: f64) -> f64 {
    if s > 1e-9 { s } else { 1.0 }
}

/// Puts the image `data` (file `name`) on `m`, keeping its width when it had a
/// texture; the height follows the image's aspect. The color becomes the image's
/// average and Colorize goes off.
pub(crate) fn set_image(m: &mut Material, name: String, data: Vec<u8>) -> Result<(), String> {
    let img = image::load_from_memory(&data)
        .map_err(|e| format!("Could not load {name}: {e}"))?
        .to_rgb8();
    let (w, h) = img.dimensions();
    let width = m
        .texture
        .as_ref()
        .map_or(NEW_TEXTURE_WIDTH, |t| tile(t.width));
    m.color = crate::image_entity::average_color(&img);
    m.texture = Some(Texture {
        filename: name,
        data: Arc::from(data),
        width,
        height: width * f64::from(h.max(1)) / f64::from(w.max(1)),
        colorize: false,
    });
    Ok(())
}

/// Sets the texture's tile size (inches). With `lock`, a given width or height scales
/// the other to keep the current aspect. Returns whether anything changed.
pub(crate) fn resize(t: &mut Texture, width: Option<f64>, height: Option<f64>, lock: bool) -> bool {
    let (w0, h0) = (tile(t.width), tile(t.height));
    let (w, h) = match (width, height, lock) {
        (Some(w), None, true) => (w, w * h0 / w0),
        (None, Some(h), true) => (h * w0 / h0, h),
        (w, h, _) => (w.unwrap_or(w0), h.unwrap_or(h0)),
    };
    if !(w > 0.0 && h > 0.0) || ((w - t.width).abs() < 1e-12 && (h - t.height).abs() < 1e-12) {
        return false;
    }
    t.width = w;
    t.height = h;
    true
}

/// Reset Color: the image's average color, not colorized. Returns whether anything
/// changed.
pub(crate) fn reset_color(m: &mut Material) -> bool {
    let Some(t) = &mut m.texture else {
        return false;
    };
    let Ok(img) = image::load_from_memory(&t.data) else {
        return false;
    };
    let color = crate::image_entity::average_color(&img.to_rgb8());
    let changed = color != m.color || t.colorize;
    m.color = color;
    t.colorize = false;
    changed
}

/// The file extension for texture bytes (from the stored name, else the content).
fn extension(t: &Texture) -> &'static str {
    let lower = t.filename.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") || t.data.starts_with(&[0xFF, 0xD8]) {
        "jpg"
    } else if lower.ends_with(".bmp") || t.data.starts_with(b"BM") {
        "bmp"
    } else {
        "png"
    }
}

impl App {
    /// Applies a texture control of material `index`. Returns whether the scene must be
    /// rebuilt.
    pub(crate) fn material_texture(&mut self, index: usize, action: TextureAction) -> bool {
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        if index >= doc.model.materials.len() {
            return false;
        }
        let units = doc.units;
        let parse = |t: &Option<String>| -> Result<Option<f64>, String> {
            match t {
                None => Ok(None),
                Some(t) => parse_length(t, units)
                    .filter(|v| *v > 0.0)
                    .map(Some)
                    .ok_or_else(|| format!("Invalid size: {t}")),
            }
        };
        let result: Result<bool, String> = match action {
            TextureAction::Load => {
                let Some(path) = crate::filedialog::open(IMAGE_FILTERS) else {
                    return false;
                };
                match std::fs::read(&path) {
                    Ok(data) => {
                        let name = path.to_string_lossy().into_owned();
                        let mut m = doc.model.materials[index].clone();
                        set_image(&mut m, name, data).map(|()| {
                            doc.edit_model(|model, _| {
                                model.materials[index] = m;
                                true
                            })
                        })
                    }
                    Err(e) => Err(format!("Could not read {}: {e}", path.display())),
                }
            }
            TextureAction::Remove => {
                Ok(doc.edit_model(|m, _| m.materials[index].texture.take().is_some()))
            }
            TextureAction::Size {
                width,
                height,
                lock,
            } => match (parse(&width), parse(&height)) {
                (Ok(w), Ok(h)) => Ok(doc.edit_model(|m, _| {
                    m.materials[index]
                        .texture
                        .as_mut()
                        .is_some_and(|t| resize(t, w, h, lock))
                })),
                (Err(e), _) | (_, Err(e)) => Err(e),
            },
            TextureAction::Colorize(on) => Ok(doc.edit_model(|m, _| {
                m.materials[index].texture.as_mut().is_some_and(|t| {
                    let changed = t.colorize != on;
                    t.colorize = on;
                    changed
                })
            })),
            TextureAction::ResetColor => {
                Ok(doc.edit_model(|m, _| reset_color(&mut m.materials[index])))
            }
            TextureAction::EditExternally => {
                let m = &doc.model.materials[index];
                let Some(t) = &m.texture else {
                    return false;
                };
                let path = std::env::temp_dir().join(format!(
                    "meshtard-texture-{}-{index}.{}",
                    std::process::id(),
                    extension(t)
                ));
                match std::fs::write(&path, &t.data) {
                    Ok(()) => {
                        // Not opened during GUI checks (the file-dialog test hook).
                        if std::env::var_os("MESHTARD_TEST_DIALOG").is_none()
                            && let Err(e) =
                                crate::presentation::open_with_system(&path.to_string_lossy())
                        {
                            self.message = Some(format!("Could not open an image editor: {e}"));
                        } else {
                            self.message = Some(format!(
                                "Editing {} externally: saved changes are loaded here.",
                                path.display()
                            ));
                        }
                        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                        self.textures.external = Some(ExternalEdit {
                            material: index,
                            name: m.name.clone(),
                            path,
                            modified,
                            checked: Instant::now(),
                        });
                        Ok(false)
                    }
                    Err(e) => Err(format!("Could not write {}: {e}", path.display())),
                }
            }
        };
        match result {
            Ok(changed) => changed,
            Err(message) => {
                self.message = Some(message);
                false
            }
        }
    }

    /// Reloads an externally edited texture when its file changed (checked a few times
    /// a second). Returns whether the scene must be rebuilt.
    pub(crate) fn poll_external_texture(&mut self, ctx: &eframe::egui::Context) -> bool {
        let Some(ext) = self.textures.external.as_mut() else {
            return false;
        };
        // Keep checking while the user works in the other program.
        ctx.request_repaint_after(WATCH_INTERVAL);
        if ext.checked.elapsed() < WATCH_INTERVAL {
            return false;
        }
        ext.checked = Instant::now();
        let modified = std::fs::metadata(&ext.path).and_then(|m| m.modified()).ok();
        if modified.is_none() || modified == ext.modified {
            return false;
        }
        ext.modified = modified;
        let (index, name, path) = (ext.material, ext.name.clone(), ext.path.clone());
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        if doc
            .model
            .materials
            .get(index)
            .is_none_or(|m| m.name != name)
        {
            self.textures.external = None;
            return false;
        }
        let Ok(data) = std::fs::read(&path) else {
            return false;
        };
        let mut m = doc.model.materials[index].clone();
        let filename = m.texture.as_ref().map_or_else(
            || path.to_string_lossy().into_owned(),
            |t| t.filename.clone(),
        );
        let colorize = m.texture.as_ref().is_some_and(|t| t.colorize);
        let color = m.color;
        // A half-written file fails to decode; the next check tries again.
        if set_image(&mut m, filename, data).is_err() {
            ext_retry(&mut self.textures.external);
            return false;
        }
        // Keep the user's color choice on a colorized texture.
        if let Some(t) = m.texture.as_mut()
            && colorize
        {
            t.colorize = true;
            m.color = color;
        }
        doc.edit_model(|model, _| {
            model.materials[index] = m;
            true
        })
    }
}

/// Forgets the last seen modification time so the next check reloads.
fn ext_retry(ext: &mut Option<ExternalEdit>) {
    if let Some(e) = ext {
        e.modified = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbImage::from_pixel(w, h, image::Rgb(rgb))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn plain() -> Material {
        Material {
            name: "M".into(),
            color: [1, 2, 3],
            opacity: 1.0,
            texture: None,
        }
    }

    #[test]
    fn load_replace_and_reset() {
        let mut m = plain();
        set_image(&mut m, "a.png".into(), png(4, 2, [200, 10, 10])).unwrap();
        let t = m.texture.clone().unwrap();
        assert!((t.width - NEW_TEXTURE_WIDTH).abs() < 1e-12);
        assert!((t.height - NEW_TEXTURE_WIDTH / 2.0).abs() < 1e-12);
        assert_eq!(m.color, [200, 10, 10]);
        // Replacing keeps the width, takes the new aspect.
        m.texture.as_mut().unwrap().width = 10.0;
        set_image(&mut m, "b.png".into(), png(2, 4, [0, 0, 255])).unwrap();
        let t = m.texture.clone().unwrap();
        assert_eq!((t.width, t.height), (10.0, 20.0));
        assert_eq!(t.filename, "b.png");
        assert!(set_image(&mut m, "c.png".into(), b"not an image".to_vec()).is_err());
        // A recolored, colorized texture goes back to its own colors.
        m.color = [9, 9, 9];
        m.texture.as_mut().unwrap().colorize = true;
        assert!(reset_color(&mut m));
        assert_eq!(m.color, [0, 0, 255]);
        assert!(!m.texture.as_ref().unwrap().colorize);
        assert!(!reset_color(&mut m));
        assert!(!reset_color(&mut plain()));
    }

    #[test]
    fn size_with_and_without_lock() {
        let mut m = plain();
        set_image(&mut m, "a.png".into(), png(4, 2, [1, 1, 1])).unwrap();
        let t = m.texture.as_mut().unwrap();
        assert!(resize(t, Some(8.0), None, true));
        assert_eq!((t.width, t.height), (8.0, 4.0));
        assert!(resize(t, None, Some(1.0), true));
        assert_eq!((t.width, t.height), (2.0, 1.0));
        assert!(resize(t, Some(5.0), None, false));
        assert_eq!((t.width, t.height), (5.0, 1.0));
        assert!(!resize(t, Some(5.0), None, false));
        assert!(!resize(t, Some(-1.0), None, false));
        let row = texture_row(t, LengthUnit::Inch);
        assert_eq!(row.width, LengthUnit::Inch.format(5.0));
        assert_eq!(row.file, "a.png");
    }

    #[test]
    fn extensions() {
        let mut m = plain();
        set_image(&mut m, "noext".into(), png(1, 1, [0, 0, 0])).unwrap();
        assert_eq!(extension(m.texture.as_ref().unwrap()), "png");
        let t = Texture {
            filename: "x.JPG".into(),
            ..m.texture.clone().unwrap()
        };
        assert_eq!(extension(&t), "jpg");
    }
}
