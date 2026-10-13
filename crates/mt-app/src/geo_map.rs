//! Model Info > Geo-location > Add Location (LEFTOVERS item 11): imports a map
//! snapshot of a location from OpenStreetMap's standard tile layer.
//!
//! SketchUp's Add Location searches an address on a map, lets one pick a region and
//! an imagery provider (satellite imagery under a Trimble account), and imports a
//! "Location Snapshot" (a textured face on its own tag; terrain optionally). Here a
//! dialog takes the latitude and longitude, a zoom level and the snapshot size in
//! tiles; Get Map downloads the OpenStreetMap tiles around the point, Import adds a
//! group "Location Snapshot" lying on the ground with the stitched map as its texture,
//! centered on the model origin and turned by the model's north angle, on the tag
//! "Location Snapshot", and sets the model's location (one undo step). No satellite
//! imagery and no terrain: those need a paid key. Look, names and defaults are ours
//! (unverified against SketchUp).
//!
//! OpenStreetMap's tile usage policy (operations.osmfoundation.org/policies/tiles):
//! the map is credited ("© OpenStreetMap contributors") in the dialog and in the
//! imported material's name and Model Info > Credits; requests carry the app's User
//! Agent; tiles are downloaded one at a time at most 4 per second, only when the user
//! asks, at most 16 per snapshot; they are cached for 7 days under the data folder
//! (`map_tiles/`) and reused; no bulk or prefetching.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, SystemTime};

use eframe::egui;
use glam::{DMat4, DVec3};

use crate::App;

/// Credit line required by the OpenStreetMap licence.
pub(crate) const ATTRIBUTION: &str = "© OpenStreetMap contributors";
/// Tile size in pixels.
const TILE: u32 = 256;
/// How long a cached tile is used (the policy asks for at least 7 days of caching).
const CACHE_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
/// Zoom levels offered (the standard layer goes to 19).
const ZOOMS: std::ops::RangeInclusive<u8> = 10..=19;
/// Snapshot sizes offered, in tiles across.
const SPANS: std::ops::RangeInclusive<u8> = 1..=3;
/// Name of the imported group, its tag and its material's start.
pub(crate) const SNAPSHOT: &str = "Location Snapshot";
const INCHES_PER_METER: f64 = 1.0 / 0.0254;

/// Fractional Web Mercator tile coordinates of a location at `zoom` (x east, y south).
pub(crate) fn tile_xy(lat: f64, lon: f64, zoom: u8) -> (f64, f64) {
    let n = f64::from(1u32 << zoom);
    let lat = lat.clamp(-85.051_128_78, 85.051_128_78).to_radians();
    let x = (lon + 180.0) / 360.0 * n;
    let y = (1.0 - lat.tan().asinh() / std::f64::consts::PI) / 2.0 * n;
    (x, y)
}

/// Ground meters per map pixel at `lat` and `zoom`.
pub(crate) fn meters_per_pixel(lat: f64, zoom: u8) -> f64 {
    // The equator's length over 256 pixels at zoom 0 (WGS 84 radius).
    let equator = 2.0 * std::f64::consts::PI * 6_378_137.0;
    equator * lat.to_radians().cos() / f64::from(TILE) / f64::from(1u32 << zoom)
}

/// The pixels of a snapshot: a square of `size` pixels centered on a location, in
/// world pixels at its zoom (top left corner), and the tiles it covers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Window {
    pub zoom: u8,
    pub left: i64,
    pub top: i64,
    pub size: u32,
}

impl Window {
    pub(crate) fn new(lat: f64, lon: f64, zoom: u8, span: u8) -> Self {
        let (x, y) = tile_xy(lat, lon, zoom);
        let size = TILE * u32::from(span.max(1));
        let half = f64::from(size) / 2.0;
        Self {
            zoom,
            left: (x * f64::from(TILE) - half).round() as i64,
            top: (y * f64::from(TILE) - half).round() as i64,
            size,
        }
    }

    /// The tiles under the window as (x, y) in tile rows; x may run past the
    /// antimeridian (wrapped by [`Window::tile_url`]), rows outside the map are left
    /// out.
    pub(crate) fn tiles(&self) -> Vec<(i64, i64)> {
        let t = i64::from(TILE);
        let n = 1i64 << self.zoom;
        let end = |a: i64| (a + i64::from(self.size) - 1).div_euclid(t);
        let mut out = Vec::new();
        for ty in self.top.div_euclid(t)..=end(self.top) {
            if !(0..n).contains(&ty) {
                continue;
            }
            for tx in self.left.div_euclid(t)..=end(self.left) {
                out.push((tx, ty));
            }
        }
        out
    }

    /// The tile's x wrapped into the map.
    fn wrap(&self, tx: i64) -> i64 {
        tx.rem_euclid(1i64 << self.zoom)
    }

    pub(crate) fn tile_url(&self, tx: i64, ty: i64) -> String {
        format!(
            "https://tile.openstreetmap.org/{}/{}/{ty}.png",
            self.zoom,
            self.wrap(tx)
        )
    }

    /// Copies the tiles into one image of the window (gray where a tile is missing).
    pub(crate) fn stitch(&self, tiles: &[((i64, i64), image::RgbImage)]) -> image::RgbImage {
        let mut out =
            image::RgbImage::from_pixel(self.size, self.size, image::Rgb([200, 200, 200]));
        let t = i64::from(TILE);
        for ((tx, ty), img) in tiles {
            let (ox, oy) = (tx * t - self.left, ty * t - self.top);
            for (px, py, p) in img.enumerate_pixels() {
                let (x, y) = (ox + i64::from(px), oy + i64::from(py));
                if (0..i64::from(self.size)).contains(&x) && (0..i64::from(self.size)).contains(&y)
                {
                    out.put_pixel(x as u32, y as u32, *p);
                }
            }
        }
        out
    }
}

/// Where a tile is cached.
fn cache_path(dir: &Path, zoom: u8, x: i64, y: i64) -> PathBuf {
    dir.join(zoom.to_string())
        .join(x.to_string())
        .join(format!("{y}.png"))
}

/// A tile: from the cache when younger than [`CACHE_AGE`], else downloaded (and
/// cached).
fn tile(
    window: &Window,
    tx: i64,
    ty: i64,
    cache: Option<&Path>,
) -> Result<image::RgbImage, String> {
    let x = window.wrap(tx);
    let path = cache.map(|d| cache_path(d, window.zoom, x, ty));
    if let Some(p) = &path
        && let Ok(meta) = std::fs::metadata(p)
        && meta
            .modified()
            .ok()
            .and_then(|m| SystemTime::now().duration_since(m).ok())
            .is_some_and(|age| age < CACHE_AGE)
        && let Ok(img) = mt_kernel::images::open(p)
    {
        return Ok(img.to_rgb8());
    }
    crate::library::net::API_THROTTLE.wait();
    let bytes = crate::library::net::get_bytes(&window.tile_url(tx, ty), None)?;
    let img = mt_kernel::images::decode(&bytes).map_err(|e| format!("map tile: {e}"))?;
    if let Some(p) = &path {
        // A cache that cannot be written only costs a download next time.
        let _ = p.parent().map(std::fs::create_dir_all);
        let _ = std::fs::write(p, &bytes);
    }
    Ok(img.to_rgb8())
}

/// A downloaded map ready to import.
pub(crate) struct Snapshot {
    lat: f64,
    lon: f64,
    zoom: u8,
    image: image::RgbImage,
}

impl Snapshot {
    /// Side length of the square on the ground, in inches.
    fn size_inches(&self) -> f64 {
        f64::from(self.image.width()) * meters_per_pixel(self.lat, self.zoom) * INCHES_PER_METER
    }
}

enum State {
    Idle,
    Loading(Receiver<Result<Snapshot, String>>),
    Ready(Snapshot, Option<egui::TextureHandle>),
    Failed(String),
}

/// The Add Location dialog.
pub(crate) struct GeoMap {
    lat: String,
    lon: String,
    zoom: u8,
    span: u8,
    state: State,
}

impl GeoMap {
    pub(crate) fn new(lat: f64, lon: f64) -> Self {
        Self {
            lat: format!("{lat:.5}"),
            lon: format!("{lon:.5}"),
            zoom: 17,
            span: 2,
            state: State::Idle,
        }
    }

    fn location(&self) -> Option<(f64, f64)> {
        let parse = |s: &str, max: f64| {
            s.trim()
                .replace(',', ".")
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && v.abs() <= max)
        };
        Some((parse(&self.lat, 85.0)?, parse(&self.lon, 180.0)?))
    }

    /// Starts downloading the map on a background thread.
    fn fetch(&mut self, lat: f64, lon: f64) {
        let (tx, rx) = channel();
        let (zoom, span) = (self.zoom, self.span);
        let cache = crate::settings::data_dir().map(|d| d.join("map_tiles"));
        std::thread::spawn(move || {
            let window = Window::new(lat, lon, zoom, span);
            let result = window
                .tiles()
                .into_iter()
                .map(|(x, y)| tile(&window, x, y, cache.as_deref()).map(|img| ((x, y), img)))
                .collect::<Result<Vec<_>, String>>()
                .map(|tiles| Snapshot {
                    lat,
                    lon,
                    zoom,
                    image: window.stitch(&tiles),
                });
            let _ = tx.send(result);
        });
        self.state = State::Loading(rx);
    }
}

/// The imported group's contents and the material it needs: the map as a texture
/// on a square of `size` inches centered on the origin. Returns the PNG data too.
fn snapshot_material(snap: &Snapshot) -> Result<mt_kernel::Material, String> {
    let mut png = std::io::Cursor::new(Vec::new());
    snap.image
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    let size = snap.size_inches();
    Ok(mt_kernel::Material {
        name: format!("{SNAPSHOT} ({ATTRIBUTION})"),
        color: crate::image_entity::average_color(&snap.image),
        opacity: 1.0,
        texture: Some(mt_kernel::Texture {
            filename: "location_snapshot.png".into(),
            data: png.into_inner().into(),
            width: size,
            height: size,
            colorize: false,
        }),
    })
}

/// Adds the snapshot to `model`'s root: a group on the "Location Snapshot" tag,
/// centered on the origin and turned by the north angle, and sets the location.
pub(crate) fn import(model: &mut mt_kernel::Model, snap: &Snapshot) -> Result<(), String> {
    let material = snapshot_material(snap)?;
    let size = snap.size_inches();
    model.materials.push(material);
    let material = model.materials.len() - 1;
    let def = model.definitions.insert(mt_kernel::Definition {
        name: model.unique_definition_name(SNAPSHOT),
        entities: crate::image_entity::image_face(size, size, material),
        is_group: true,
        ..Default::default()
    });
    let layer = match model.layers.iter().position(|l| l.name == SNAPSHOT) {
        Some(i) => i,
        None => {
            model.layers.push(mt_kernel::Layer {
                name: SNAPSHOT.into(),
                ..Default::default()
            });
            model.layers.len() - 1
        }
    };
    // North is the green axis turned clockwise (seen from above) by the north angle.
    let turn = DMat4::from_rotation_z(-model.geo.north_angle.to_radians());
    model.root.instances.push(mt_kernel::Instance {
        definition: def,
        transform: turn * DMat4::from_translation(DVec3::new(-size / 2.0, -size / 2.0, 0.0)),
        name: SNAPSHOT.into(),
        hidden: false,
        locked: false,
        layer: Some(layer),
        material: None,
        attrs: Default::default(),
    });
    model.shadows.latitude = snap.lat;
    model.shadows.longitude = snap.lon;
    model.geo.located = true;
    Ok(())
}

impl App {
    /// The Add Location dialog, while open.
    pub(crate) fn geo_map_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(mut g) = self.geo_map.take() else {
            return;
        };
        if let State::Loading(rx) = &g.state {
            match rx.try_recv() {
                Ok(Ok(snap)) => g.state = State::Ready(snap, None),
                Ok(Err(e)) => g.state = State::Failed(e),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
                Err(_) => g.state = State::Failed("The download stopped.".into()),
            }
        }
        let mut import_now = false;
        let r = mt_ui::Dialog::new("mt_geo_map", "Add Location")
            .width(360.0)
            .modal(false)
            .show(ctx, |ui| {
                crate::export::dialog_visuals(ui);
                let loading = matches!(g.state, State::Loading(_));
                egui::Grid::new("mt_geo_map_grid")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label("Latitude");
                        ui.add(egui::TextEdit::singleline(&mut g.lat).desired_width(110.0));
                        ui.end_row();
                        ui.label("Longitude");
                        ui.add(egui::TextEdit::singleline(&mut g.lon).desired_width(110.0));
                        ui.end_row();
                        ui.label("Zoom");
                        ui.add(egui::Slider::new(&mut g.zoom, ZOOMS));
                        ui.end_row();
                        ui.label("Size");
                        ui.add(egui::Slider::new(&mut g.span, SPANS).suffix(" tiles"));
                        ui.end_row();
                    });
                let location = g.location();
                if let Some((lat, _)) = location {
                    let w = Window::new(lat, 0.0, g.zoom, g.span);
                    let meters = f64::from(w.size) * meters_per_pixel(lat, g.zoom);
                    ui.weak(format!("About {meters:.0} m across."));
                } else {
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        "Latitude -85..85 and longitude -180..180, in degrees.",
                    );
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(location.is_some() && !loading, egui::Button::new("Get Map"))
                        .clicked()
                        && let Some((lat, lon)) = location
                    {
                        g.fetch(lat, lon);
                    }
                    if loading {
                        ui.spinner();
                    }
                });
                match &mut g.state {
                    State::Ready(snap, tex) => {
                        let tex = tex.get_or_insert_with(|| {
                            let (w, h) = snap.image.dimensions();
                            let img = egui::ColorImage::from_rgb(
                                [w as usize, h as usize],
                                snap.image.as_raw(),
                            );
                            ui.ctx()
                                .load_texture("mt_geo_map", img, egui::TextureOptions::LINEAR)
                        });
                        let side = ui.available_width().min(300.0);
                        ui.image((tex.id(), egui::vec2(side, side)));
                        if ui.button("Import").clicked() {
                            import_now = true;
                        }
                    }
                    State::Failed(e) => {
                        ui.colored_label(ui.visuals().error_fg_color, e.as_str());
                    }
                    State::Idle | State::Loading(_) => {}
                }
                ui.separator();
                ui.hyperlink_to(
                    format!("Map data {ATTRIBUTION}"),
                    "https://www.openstreetmap.org/copyright",
                );
            });
        if import_now
            && let State::Ready(snap, _) = &g.state
            && let Some(doc) = self.document.as_mut()
        {
            let mut error = None;
            doc.operation = "Add Location".into();
            let changed = doc.edit_whole(|m| match import(m, snap) {
                Ok(()) => true,
                Err(e) => {
                    error = Some(e);
                    false
                }
            });
            self.message = Some(match error {
                Some(e) => format!("Could not import the map: {e}"),
                None => format!("Imported the {SNAPSHOT}. Map data {ATTRIBUTION}."),
            });
            if changed {
                self.apply_shadows();
                if let Some(rs) = frame.wgpu_render_state() {
                    let device = rs.device.clone();
                    self.rebuild(&device);
                }
                return;
            }
        }
        if !r.closed {
            self.geo_map = Some(g);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_math_matches_known_tiles() {
        // Zoom 0 is one tile; the origin is the middle of it.
        let (x, y) = tile_xy(0.0, 0.0, 0);
        assert!((x - 0.5).abs() < 1e-12 && (y - 0.5).abs() < 1e-12);
        // Zurich at zoom 10 is in tile 536 / 358 (OpenStreetMap wiki's example).
        let (x, y) = tile_xy(47.3769, 8.5417, 10);
        assert_eq!((x.floor(), y.floor()), (536.0, 358.0));
        // 156 543 m per pixel at the equator at zoom 0, halving with each level.
        assert!((meters_per_pixel(0.0, 0) - 156_543.034).abs() < 0.01);
        assert!((meters_per_pixel(60.0, 1) - 156_543.034 / 4.0).abs() < 0.01);
    }

    #[test]
    fn window_covers_its_tiles_and_wraps() {
        let w = Window::new(47.3769, 8.5417, 10, 2);
        assert_eq!(w.size, 512);
        let tiles = w.tiles();
        assert_eq!(
            tiles.len(),
            9,
            "two tiles' width spans three columns and rows"
        );
        assert!(tiles.contains(&(536, 358)));
        // At the antimeridian the window wraps around.
        let w = Window::new(0.0, 179.99, 2, 1);
        assert!(w.tiles().iter().any(|t| t.0 == 4));
        assert_eq!(w.tile_url(4, 1), "https://tile.openstreetmap.org/2/0/1.png");
        // Near the pole rows past the map are skipped.
        let w = Window::new(85.0, 0.0, 1, 2);
        assert!(w.tiles().iter().all(|t| t.1 >= 0));
    }

    #[test]
    fn stitching_places_tiles_by_world_pixel() {
        let w = Window {
            zoom: 3,
            left: 128,
            top: 0,
            size: 256,
        };
        assert_eq!(w.tiles(), [(0, 0), (1, 0)]);
        let red = image::RgbImage::from_pixel(256, 256, image::Rgb([255, 0, 0]));
        let blue = image::RgbImage::from_pixel(256, 256, image::Rgb([0, 0, 255]));
        let img = w.stitch(&[((0, 0), red), ((1, 0), blue)]);
        assert_eq!(img.get_pixel(0, 10).0, [255, 0, 0]);
        assert_eq!(img.get_pixel(127, 10).0, [255, 0, 0]);
        assert_eq!(img.get_pixel(128, 10).0, [0, 0, 255]);
        assert_eq!(
            cache_path(Path::new("c"), 3, 1, 2),
            Path::new("c/3/1/2.png")
        );
    }

    #[test]
    fn import_adds_a_textured_group_centered_on_the_origin() {
        let snap = Snapshot {
            lat: 60.0,
            lon: 10.0,
            zoom: 18,
            image: image::RgbImage::from_pixel(256, 256, image::Rgb([10, 200, 10])),
        };
        let mut m = mt_kernel::Model::default();
        m.geo.north_angle = 90.0;
        import(&mut m, &snap).unwrap();
        assert!(m.geo.located);
        assert_eq!((m.shadows.latitude, m.shadows.longitude), (60.0, 10.0));
        let inst = &m.root.instances[0];
        assert_eq!(m.layers[inst.layer.unwrap()].name, SNAPSHOT);
        let def = &m.definitions[inst.definition];
        assert!(def.is_group && def.entities.faces.len() == 1);
        let mat = &m.materials[0];
        assert!(mat.name.contains("OpenStreetMap"));
        // 256 pixels of 0.2986 m at 60 degrees north, zoom 18: about 76.4 m.
        let size = mat.texture.as_ref().unwrap().width;
        assert!((size * 0.0254 - 76.44).abs() < 0.05, "{size}");
        // Centered on the origin (any turn keeps the center there).
        let center = inst
            .transform
            .transform_point3(DVec3::new(size / 2.0, size / 2.0, 0.0));
        assert!(center.length() < 1e-6);
        // A second import reuses the tag.
        import(&mut m, &snap).unwrap();
        assert_eq!(m.layers.iter().filter(|l| l.name == SNAPSHOT).count(), 1);
    }
}
