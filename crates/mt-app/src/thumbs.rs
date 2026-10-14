//! Rendered thumbnails of the Materials, Components and Styles panels.
//!
//! - Materials: a textured material's image, decoded off the UI thread; the panel
//!   draws it flat or on a cube by the Auto / Cube / Flat thumbnail style.
//! - Components: a small offscreen render of each definition (the model's, and the
//!   `.skp` files of an opened collection, which are read off the UI thread), from
//!   the default view direction, fitted. Rendered again after a model change.
//! - Styles: a fixed sample scene drawn with each style (In Model styles and those
//!   of the collections); a SketchUp `.style` file's own thumbnail image is used
//!   when it has one.
//!
//! Rendering happens on the UI thread (it needs the GPU), one thumbnail per frame
//! and only while its panel is open; results are cached. What SketchUp's panels show
//! exactly (sphere or cube swatches, the sample scene of its style thumbnails) is
//! from memory and unverified.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

use glam::{DMat4, DVec3};
use mt_kernel::{DefinitionId, Model, StyleSettings};

use crate::App;
use crate::document::{Document, Scene};

/// Component thumbnail size in pixels.
const COMPONENT_PX: [u32; 2] = [96, 96];
/// Style thumbnail size in pixels (width / height = `mt_ui::STYLE_THUMB_ASPECT`).
const STYLE_PX: [u32; 2] = [108, 60];

enum Job {
    /// Decode image bytes into a small thumbnail.
    Image(String, Arc<[u8]>),
    /// Read a `.skp` file into a model.
    Skp(PathBuf),
}

enum Done {
    Image(String, Option<image::RgbaImage>),
    Skp(PathBuf, Option<Box<Model>>),
}

/// The background worker: decodes images and reads `.skp` files.
struct Worker {
    jobs: Sender<Job>,
    done: Receiver<Done>,
}

impl Worker {
    fn spawn(ctx: &egui::Context) -> Option<Self> {
        let (jobs, job_rx) = std::sync::mpsc::channel::<Job>();
        let (done_tx, done) = std::sync::mpsc::channel::<Done>();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("mt-thumbnails".into())
            .spawn(move || {
                for job in job_rx {
                    let out = match job {
                        Job::Image(key, data) => Done::Image(
                            key,
                            mt_kernel::images::decode(&data)
                                .ok()
                                .map(|i| i.thumbnail(128, 128).to_rgba8()),
                        ),
                        Job::Skp(path) => {
                            let model = crate::errors::read_skp_model(&path).ok().map(Box::new);
                            Done::Skp(path, model)
                        }
                    };
                    if done_tx.send(out).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .ok()?;
        Some(Self { jobs, done })
    }
}

/// Thumbnail caches (see the module docs).
#[derive(Default)]
pub(crate) struct Thumbs {
    viewport: Option<mt_render::Viewport>,
    worker: Option<Worker>,
    /// Decoded images by key (`None`: failed).
    images: HashMap<String, Option<egui::TextureHandle>>,
    /// Image keys sent to the worker and not back yet.
    pending: HashSet<String>,
    /// Material texture bytes behind each `tex:` key (kept so the address stays
    /// unique while cached).
    textures: HashMap<String, Arc<[u8]>>,
    /// Model definitions: the model state rendered and the image.
    components: HashMap<DefinitionId, (u64, egui::TextureHandle)>,
    /// Collection `.skp` files: read (waiting to be rendered), and rendered.
    files_read: Vec<(PathBuf, Option<Box<Model>>)>,
    files_pending: HashSet<PathBuf>,
    files_wanted: Vec<PathBuf>,
    files: HashMap<PathBuf, Option<egui::TextureHandle>>,
    /// Rendered style thumbnails by key, with the settings they show.
    styles: HashMap<String, (StyleSettings, egui::TextureHandle)>,
    /// The In Model styles' thumbnails as PNG (for `.style` files written).
    style_pngs: HashMap<String, (StyleSettings, Vec<u8>)>,
    sample: Option<Model>,
}

/// The cache key of a material texture image.
fn texture_key(data: &Arc<[u8]>) -> String {
    format!("tex:{:p}:{}", Arc::as_ptr(data).cast::<u8>(), data.len())
}

fn to_texture(ctx: &egui::Context, name: String, img: &image::RgbaImage) -> egui::TextureHandle {
    let size = [img.width() as usize, img.height() as usize];
    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    ctx.load_texture(name, pixels, egui::TextureOptions::LINEAR)
}

impl Thumbs {
    fn send(&mut self, ctx: &egui::Context, job: Job) {
        if self.worker.is_none() {
            self.worker = Worker::spawn(ctx);
        }
        if let Some(w) = &self.worker {
            let _ = w.jobs.send(job);
        }
    }

    /// The image of `key`, decoding `data` in the background on first use.
    fn request_image(&mut self, ctx: &egui::Context, key: &str, data: &Arc<[u8]>) {
        if !self.images.contains_key(key) && self.pending.insert(key.to_owned()) {
            self.send(ctx, Job::Image(key.to_owned(), data.clone()));
        }
    }

    fn lookup_image(&self, key: &str) -> Option<egui::TextureId> {
        self.images.get(key)?.as_ref().map(egui::TextureHandle::id)
    }

    /// A collection component's rendered thumbnail, if done.
    pub(crate) fn file(&self, path: &PathBuf) -> Option<egui::TextureId> {
        self.files.get(path)?.as_ref().map(egui::TextureHandle::id)
    }

    /// Collection components `paths` get thumbnails (read in the background).
    pub(crate) fn want_files(&mut self, paths: Vec<PathBuf>) {
        for p in paths {
            if !self.files.contains_key(&p) && !self.files_pending.contains(&p) {
                self.files_wanted.push(p);
            }
        }
    }

    /// A collection style's thumbnail: its own image (when `has_image`, a `.style`
    /// file's) or the sample scene rendered with it.
    pub(crate) fn style(&self, key: &str, has_image: bool) -> Option<egui::TextureId> {
        if has_image {
            self.lookup_image(&format!("style:{key}"))
        } else {
            self.styles.get(key).map(|s| s.1.id())
        }
    }

    /// The PNG of In Model style thumbnail `key` when it shows `settings`.
    pub(crate) fn style_png(&self, key: &str, settings: &StyleSettings) -> Option<Vec<u8>> {
        let (s, png) = self.style_pngs.get(key)?;
        (s == settings).then(|| png.clone())
    }

    /// Forgets collection thumbnails (the collections are scanned again).
    pub(crate) fn forget_collections(&mut self) {
        self.styles.retain(|k, _| k.starts_with("model:"));
        self.images.retain(|k, _| !k.starts_with("style:"));
    }

    fn take_done(&mut self, ctx: &egui::Context) {
        for p in std::mem::take(&mut self.files_wanted) {
            if self.files_pending.insert(p.clone()) {
                self.send(ctx, Job::Skp(p));
            }
        }
        let Some(w) = &self.worker else { return };
        let done: Vec<Done> = w.done.try_iter().collect();
        for d in done {
            match d {
                Done::Image(key, img) => {
                    self.pending.remove(&key);
                    let tex = img.map(|i| to_texture(ctx, format!("mt_thumb_{key}"), &i));
                    self.images.insert(key, tex);
                }
                Done::Skp(path, model) => {
                    self.files_pending.remove(&path);
                    self.files_read.push((path, model));
                }
            }
        }
    }

    /// Material texture keys of models no longer holding them are dropped.
    fn prune_textures(&mut self) {
        let gone: Vec<String> = (self.textures.iter())
            .filter(|(_, a)| Arc::strong_count(a) == 1)
            .map(|(k, _)| k.clone())
            .collect();
        for k in gone {
            self.textures.remove(&k);
            self.images.remove(&k);
        }
    }

    fn render(
        &mut self,
        spec: &mt_spec::Spec,
        rs: &egui_wgpu::RenderState,
        scene: &Scene,
        styled: Option<&Model>,
        size: [u32; 2],
    ) -> image::RgbaImage {
        render_scene(&mut self.viewport, spec, rs, scene, styled, size)
    }
}

/// The renderer's style for `m`'s style settings (as the viewport draws them).
fn render_style(spec: &mt_spec::Spec, m: &Model) -> mt_render::Style {
    let st = &m.style;
    let rgb = |c: [u8; 3]| c.map(|x| f32::from(x) / 255.0);
    let mut style = mt_render::Style::from_spec(&spec.default_model);
    style.profiles = st.profiles.then_some(st.profile_width);
    style.edges = st.edges;
    style.back_edges = st.back_edges;
    style.depth_cue = st.depth_cue.then_some(st.depth_cue_width);
    style.extension = st.extension.then_some(st.extension_length);
    style.fog = st.fog.enabled.then(|| mt_render::Fog {
        start: st.fog.start,
        end: st.fog.end,
        color: (!st.fog.use_background).then(|| rgb(st.fog.color)),
    });
    crate::style_view::apply_render(m, &mut style);
    style
}

fn face_style(f: mt_kernel::FaceStyle) -> mt_render::FaceStyle {
    match f {
        mt_kernel::FaceStyle::Wireframe => mt_render::FaceStyle::Wireframe,
        mt_kernel::FaceStyle::HiddenLine => mt_render::FaceStyle::HiddenLine,
        mt_kernel::FaceStyle::Shaded => mt_render::FaceStyle::Shaded,
        mt_kernel::FaceStyle::ShadedWithTextures => mt_render::FaceStyle::ShadedWithTextures,
        mt_kernel::FaceStyle::Monochrome => mt_render::FaceStyle::Monochrome,
    }
}

/// Renders `scene` fitted from the default view direction, without axes: with the
/// style of `styled` (its style settings and face colors), or the default style.
fn render_scene(
    viewport: &mut Option<mt_render::Viewport>,
    spec: &mt_spec::Spec,
    rs: &egui_wgpu::RenderState,
    scene: &Scene,
    styled: Option<&Model>,
    size: [u32; 2],
) -> image::RgbaImage {
    let (device, queue) = (&rs.device, &rs.queue);
    let mut style = match styled {
        Some(m) => render_style(spec, m),
        None => {
            // Component thumbnails: on white, without sky and ground.
            let mut s = mt_render::Style::from_spec(&spec.default_model);
            s.background = [1.0; 3];
            s.sky = None;
            s.ground = None;
            s
        }
    };
    style.axes = None;
    let vp = viewport.get_or_insert_with(|| mt_render::Viewport::new(device, style.clone()));
    match styled {
        Some(m) => {
            vp.set_face_style(face_style(m.style.face_style), m.style.xray);
            let rgb = |c: [u8; 3]| c.map(|x| f32::from(x) / 255.0);
            vp.set_monochrome_colors(rgb(m.front_color), rgb(m.back_color));
        }
        None => vp.set_face_style(mt_render::FaceStyle::ShadedWithTextures, false),
    }
    vp.set_style(device, style);
    crate::instancing::upload_faces(vp, device, scene);
    vp.set_textures(&scene.textures);
    let (edges, instanced) = scene.flat_edges();
    vp.set_edges(device, &edges);
    vp.set_edge_instances(device, &scene.gpu_instances(instanced));
    let pts = scene
        .triangles()
        .iter()
        .flatten()
        .copied()
        .chain(scene.edges().iter().flat_map(|s| {
            [
                DVec3::from(glam::Vec3::from(s.a)),
                DVec3::from(glam::Vec3::from(s.b)),
            ]
        }));
    let (mut min, mut max) = (DVec3::splat(f64::MAX), DVec3::splat(f64::MIN));
    for p in pts {
        min = min.min(p);
        max = max.max(p);
    }
    if min.x > max.x {
        (min, max) = (DVec3::ZERO, DVec3::ONE);
    }
    let mut camera = mt_render::Camera::from_spec(&spec.default_model.camera);
    camera.fit(min, max, f64::from(size[0]) / f64::from(size[1]));
    vp.render_image(device, queue, &camera, size, 1.0, false)
}

/// The scene of definition `def` of `doc`'s model on its own (one instance at the
/// origin; the model's materials and tags). The definitions are lent to a scratch
/// model, not copied.
fn definition_scene(doc: &mut Document, def: DefinitionId) -> Scene {
    let mut tmp = Document::untitled();
    let m = &mut tmp.model;
    m.definitions = std::mem::take(&mut doc.model.definitions);
    m.materials.clone_from(&doc.model.materials);
    m.layers.clone_from(&doc.model.layers);
    m.front_color = doc.model.front_color;
    m.back_color = doc.model.back_color;
    m.root.instances.push(instance(def));
    let scene = tmp.scene();
    doc.model.definitions = std::mem::take(&mut tmp.model.definitions);
    scene
}

fn instance(definition: DefinitionId) -> mt_kernel::Instance {
    mt_kernel::Instance {
        definition,
        transform: DMat4::IDENTITY,
        name: String::new(),
        hidden: false,
        locked: false,
        layer: None,
        material: None,
        attrs: Default::default(),
    }
}

/// The scene a model read from a `.skp` file shows (its root).
fn file_scene(model: Model) -> Scene {
    let mut doc = Document::untitled();
    doc.model = model;
    doc.scene()
}

/// The style thumbnails' sample: an L-shaped block.
fn sample_model() -> Model {
    let mut m = Model::default();
    let pts = [
        (0.0, 0.0),
        (72.0, 0.0),
        (72.0, 30.0),
        (30.0, 30.0),
        (30.0, 60.0),
        (0.0, 60.0),
    ];
    let ents = &mut m.root;
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        mt_kernel::add_line(ents, DVec3::new(a.0, a.1, 0.0), DVec3::new(b.0, b.1, 0.0));
    }
    if let Some((base, n)) = ents.faces.iter().next().map(|(id, f)| (id, f.normal.z)) {
        mt_kernel::push_pull(ents, base, 40.0 * n.signum(), false);
    }
    m
}

impl App {
    /// Material texture images per model material (`None`: plain color or not
    /// decoded yet).
    pub(crate) fn material_thumbnails(&self) -> Vec<Option<egui::TextureId>> {
        let Some(doc) = &self.document else {
            return Vec::new();
        };
        let th = &self.tray_lib.render;
        (doc.model.materials.iter())
            .map(|m| th.lookup_image(&texture_key(&m.texture.as_ref()?.data)))
            .collect()
    }

    /// Rendered thumbnails of the Components panel's definitions and the In Model
    /// styles (whatever is rendered so far).
    pub(crate) fn rendered_thumbnails(
        &self,
    ) -> (Vec<Option<egui::TextureId>>, Vec<Option<egui::TextureId>>) {
        let Some(doc) = &self.document else {
            return Default::default();
        };
        let th = &self.tray_lib.render;
        let comps = crate::tray::component_definitions(&doc.model)
            .iter()
            .map(|d| th.components.get(d).map(|c| c.1.id()))
            .collect();
        let styles = (0..doc.model.styles.len())
            .map(|i| th.styles.get(&format!("model:{i}")).map(|s| s.1.id()))
            .collect();
        (comps, styles)
    }

    /// Takes finished background work and renders one missing thumbnail of an open
    /// panel (Styles first, then Components).
    pub(crate) fn thumbnails_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        self.tray_lib.render.take_done(ctx);
        self.tray_lib.render.prune_textures();
        let Some(shell) = &self.shell else { return };
        let styles_open = shell.panel_expanded("window.styles");
        let components_open = shell.panel_expanded("window.components");
        if shell.panel_expanded("window.materials")
            && let Some(doc) = &self.document
        {
            let th = &mut self.tray_lib.render;
            for m in &doc.model.materials {
                if let Some(t) = &m.texture {
                    let key = texture_key(&t.data);
                    th.textures
                        .entry(key.clone())
                        .or_insert_with(|| t.data.clone());
                    th.request_image(ctx, &key, &t.data);
                }
            }
        }
        if styles_open {
            for (key, data) in self.tray_lib.style_images() {
                self.tray_lib
                    .render
                    .request_image(ctx, &format!("style:{key}"), &data);
            }
        }
        if !styles_open && !components_open {
            return;
        }
        let Some(rs) = frame.wgpu_render_state() else {
            return;
        };
        let rendered = (styles_open && self.render_style_thumbnail(ctx, rs))
            || (components_open && self.render_component_thumbnail(ctx, rs));
        if rendered {
            ctx.request_repaint();
        }
    }

    /// Renders one In Model or collection style whose thumbnail is missing or out of
    /// date. Returns whether it rendered one.
    fn render_style_thumbnail(&mut self, ctx: &egui::Context, rs: &egui_wgpu::RenderState) -> bool {
        let mut todo: Vec<(String, StyleSettings)> = Vec::new();
        if let Some(doc) = &self.document {
            for (i, s) in doc.model.styles.iter().enumerate() {
                todo.push((format!("model:{i}"), s.settings.clone()));
            }
        }
        todo.extend(self.tray_lib.styles_to_render());
        let th = &mut self.tray_lib.render;
        let Some((key, settings)) = todo
            .into_iter()
            .find(|(k, s)| th.styles.get(k).is_none_or(|x| x.0 != *s))
        else {
            return false;
        };
        let mut doc = Document::untitled();
        doc.model = th.sample.get_or_insert_with(sample_model).clone();
        doc.model.style = settings.clone();
        if let Some(d) = &self.document {
            doc.model.front_color = d.model.front_color;
            doc.model.back_color = d.model.back_color;
        }
        let scene = doc.scene();
        let img = th.render(&self.spec, rs, &scene, Some(&doc.model), STYLE_PX);
        let tex = to_texture(ctx, format!("mt_style_thumb_{key}"), &img);
        if key.starts_with("model:") {
            let mut png = Vec::new();
            if img
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .is_ok()
            {
                th.style_pngs.insert(key.clone(), (settings.clone(), png));
            }
        }
        th.styles.insert(key, (settings, tex));
        true
    }

    /// Renders one model definition (after a model change) or one read collection
    /// file. Returns whether it rendered one.
    fn render_component_thumbnail(
        &mut self,
        ctx: &egui::Context,
        rs: &egui_wgpu::RenderState,
    ) -> bool {
        if let Some((path, model)) = self.tray_lib.render.files_read.pop() {
            let tex = model.map(|m| {
                let scene = file_scene(*m);
                let img = self
                    .tray_lib
                    .render
                    .render(&self.spec, rs, &scene, None, COMPONENT_PX);
                to_texture(ctx, format!("mt_file_thumb_{}", path.display()), &img)
            });
            self.tray_lib.render.files.insert(path, tex);
            return true;
        }
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        let state = doc.state_id();
        let defs = crate::tray::component_definitions(&doc.model);
        let th = &mut self.tray_lib.render;
        th.components.retain(|d, _| defs.contains(d));
        let Some(&def) = defs
            .iter()
            .find(|d| th.components.get(d).is_none_or(|c| c.0 != state))
        else {
            return false;
        };
        let scene = definition_scene(doc, def);
        let img = th.render(&self.spec, rs, &scene, None, COMPONENT_PX);
        let tex = to_texture(ctx, format!("mt_component_thumb_{def:?}"), &img);
        th.components.insert(def, (state, tex));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_is_a_closed_block() {
        let m = sample_model();
        assert_eq!(m.root.faces.len(), 8);
        let mut doc = Document::untitled();
        doc.model = m;
        assert!(!doc.scene().triangles().is_empty());
    }

    #[test]
    fn definition_scene_lends_the_definitions_back() {
        let mut doc = Document::untitled();
        let mut d = mt_kernel::Definition::default();
        mt_kernel::add_line(&mut d.entities, DVec3::ZERO, DVec3::X * 10.0);
        let id = doc.model.definitions.insert(d);
        let scene = definition_scene(&mut doc, id);
        assert_eq!(scene.edges().len(), 1);
        assert_eq!(doc.model.definitions.len(), 1);
        assert!(doc.model.root.instances.is_empty());
    }
}
