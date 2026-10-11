//! Window > Model Library (SketchUp's Window > 3D Warehouse; FEATURES.md 68): search
//! model sources, preview, download and place a model with Place Component.
//!
//! Trimble's 3D Warehouse has no public API and its terms forbid scraping it
//! (spec/parity/LIBRARY.md), so the sources are:
//! - Local folders ([`local`]): the Components folder of Preferences > Files and
//!   folders added here.
//! - Poly Haven ([`polyhaven`]): CC0 models, public API, no account.
//! - Sketchfab ([`sketchfab`]): only when the user has entered their own API token;
//!   downloadable models, license per model.
//!
//! Network and file work runs on background threads ([`Worker`], [`Download`]); the
//! UI thread only polls their channels. Downloads and thumbnails are cached under
//! the user data folder (`settings::data_dir()/library`). A placed model's
//! definition gets the source page (URL), license and credit in its Advanced
//! Attributes (Entity Info) and description.

pub(crate) mod local;
pub(crate) mod net;
pub(crate) mod polyhaven;
pub(crate) mod sketchfab;
mod ui;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use eframe::egui;

/// Where a model comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) enum Source {
    #[default]
    Local,
    PolyHaven,
    Sketchfab,
}

impl Source {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Source::Local => "Local folders",
            Source::PolyHaven => "Poly Haven",
            Source::Sketchfab => "Sketchfab",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Source::Local => "local",
            Source::PolyHaven => "polyhaven",
            Source::Sketchfab => "sketchfab",
        }
    }
}

/// One model of a source.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Asset {
    pub source: Source,
    /// Unique within the source (a file path for local files).
    pub id: String,
    pub name: String,
    pub author: String,
    pub license: String,
    pub license_url: String,
    /// The model's page on the source's site.
    pub page_url: String,
    pub thumbnail_url: Option<String>,
    pub description: String,
    pub tags: Vec<String>,
    /// Download (or file) size in bytes, when known.
    pub size: Option<u64>,
    pub polycount: Option<u64>,
    pub local_path: Option<PathBuf>,
}

impl Asset {
    /// Key of the thumbnail and cache folder (file-name safe).
    pub(crate) fn key(&self) -> String {
        match &self.local_path {
            Some(p) => {
                let (len, mtime) = std::fs::metadata(p)
                    .map(|m| {
                        let t = m
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map_or(0, |d| d.as_secs());
                        (m.len(), t)
                    })
                    .unwrap_or_default();
                let h = fnv1a(format!("{}|{len}|{mtime}", p.display()).as_bytes());
                format!("local-{h:016x}")
            }
            None => format!("{}-{}", self.source.key(), file_safe(&self.id)),
        }
    }

    /// The credit line kept with a placed model.
    pub(crate) fn credit(&self) -> String {
        let mut s = self.name.clone();
        if !self.author.is_empty() {
            s += &format!(" by {}", self.author);
        }
        if self.source != Source::Local {
            s += &format!(", from {}", self.source.label());
        }
        s
    }
}

/// FNV-1a: a hash that stays the same between runs and Rust versions.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// `s` with characters other than letters, digits, `-`, `_` and `.` replaced.
fn file_safe(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    t.trim_start_matches('.').chars().take(100).collect()
}

/// A relative path that stays inside its folder (no `..`, not absolute).
pub(crate) fn safe_relative(path: &str) -> bool {
    let p = Path::new(path);
    !path.is_empty()
        && !path.contains('\\')
        && !p.is_absolute()
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// Indices of the assets whose name, tags or author contain every word of `query`.
pub(crate) fn filter(assets: &[Asset], query: &str) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    (0..assets.len())
        .filter(|&i| {
            let a = &assets[i];
            let hay =
                format!("{} {} {} {}", a.name, a.id, a.author, a.tags.join(" ")).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

/// Human-readable byte size.
pub(crate) fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{:.1} MB", b / KB / KB)
    }
}

/// The library's folder in the user data folder.
fn cache_dir() -> Option<PathBuf> {
    crate::settings::data_dir().map(|d| d.join("library"))
}

/// Settings kept in `library.toml` in the config folder.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Config {
    /// Folders added in the panel (the Components folder is always looked in too).
    pub folders: Vec<PathBuf>,
    /// The user's own Sketchfab API token (kept in plain text, as SketchUp keeps its
    /// login tokens in its own settings; noted in the panel).
    pub sketchfab_token: String,
}

impl Config {
    fn path() -> Option<PathBuf> {
        crate::settings::config_dir().map(|d| d.join("library.toml"))
    }

    pub(crate) fn from_toml(text: &str) -> Self {
        let Ok(t) = text.parse::<toml::Table>() else {
            return Self::default();
        };
        Self {
            folders: t
                .get("folders")
                .and_then(toml::Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(toml::Value::as_str)
                        .map(PathBuf::from)
                        .collect()
                })
                .unwrap_or_default(),
            sketchfab_token: t
                .get("sketchfab_token")
                .and_then(toml::Value::as_str)
                .unwrap_or("")
                .to_owned(),
        }
    }

    pub(crate) fn to_toml(&self) -> String {
        let mut t = toml::Table::new();
        t.insert(
            "folders".into(),
            toml::Value::Array(
                self.folders
                    .iter()
                    .map(|f| toml::Value::String(f.display().to_string()))
                    .collect(),
            ),
        );
        if !self.sketchfab_token.is_empty() {
            t.insert(
                "sketchfab_token".into(),
                toml::Value::String(self.sketchfab_token.clone()),
            );
        }
        format!("# meshtard Model Library (Window > Model Library).\n{t}")
    }

    fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| Self::from_toml(&t))
            .unwrap_or_default()
    }

    fn save(&self) {
        let Some(path) = Self::path() else { return };
        let r = std::fs::create_dir_all(path.parent().expect("has a parent"))
            .and_then(|()| std::fs::write(&path, self.to_toml()));
        if let Err(e) = r {
            log::warn!("could not save {}: {e}", path.display());
        }
    }
}

/// Background work other than downloads.
enum Job {
    /// List or search a source. `generation` tells stale answers apart.
    List {
        generation: u64,
        source: Source,
        query: String,
        folders: Vec<PathBuf>,
        token: String,
        /// Sketchfab: the next page's URL (results are added to the list).
        next: Option<String>,
    },
    Thumb {
        generation: u64,
        asset: Box<Asset>,
    },
    /// Poly Haven: the download's size for the details.
    Details {
        asset: Box<Asset>,
    },
}

enum Reply {
    List {
        generation: u64,
        append: bool,
        result: Result<(Vec<Asset>, Option<String>), String>,
    },
    /// A thumbnail image (or an error, then a placeholder is drawn).
    Thumb {
        key: String,
        image: Result<image::RgbaImage, String>,
    },
    /// A local file read for its thumbnail, to render on the UI thread.
    ThumbModel {
        key: String,
        model: Box<mt_kernel::Model>,
    },
    Details {
        id: String,
        size: Option<u64>,
    },
}

/// Thumbnails are this many pixels square.
pub(crate) const THUMB_PX: u32 = 128;
/// Local files larger than this get a placeholder instead of a rendered thumbnail.
const MAX_THUMB_FILE: u64 = 40 << 20;
/// Poly Haven's model list is fetched again after this long.
const LIST_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

/// Threads for listing and thumbnails: one for lists and details, two for
/// thumbnails (their own queue, so a long thumbnail queue never holds up a search).
struct Worker {
    jobs: Sender<Job>,
    thumbs: Sender<Job>,
    replies: Receiver<Reply>,
    /// The current search generation; thumbnail jobs of older ones are skipped.
    generation: Arc<AtomicU64>,
}

impl Worker {
    fn start(ctx: &egui::Context) -> Self {
        let (jobs, job_rx) = channel::<Job>();
        let (thumbs, thumb_rx) = channel::<Job>();
        let (reply_tx, replies) = channel();
        let generation = Arc::new(AtomicU64::new(0));
        let spawn = |rx: Arc<Mutex<Receiver<Job>>>, name: &str| {
            let tx = reply_tx.clone();
            let ctx = ctx.clone();
            let generation = generation.clone();
            let _ = std::thread::Builder::new()
                .name(name.into())
                .spawn(move || {
                    loop {
                        let job = match rx.lock() {
                            Ok(rx) => rx.recv(),
                            Err(_) => return,
                        };
                        let Ok(job) = job else { return };
                        if let Some(reply) = run_job(job, &generation)
                            && tx.send(reply).is_err()
                        {
                            return;
                        }
                        ctx.request_repaint();
                    }
                });
        };
        spawn(Arc::new(Mutex::new(job_rx)), "library-list");
        let thumb_rx = Arc::new(Mutex::new(thumb_rx));
        spawn(thumb_rx.clone(), "library-thumbs-1");
        spawn(thumb_rx, "library-thumbs-2");
        Self {
            jobs,
            thumbs,
            replies,
            generation,
        }
    }
}

fn run_job(job: Job, current: &AtomicU64) -> Option<Reply> {
    match job {
        Job::List {
            generation,
            source,
            query,
            folders,
            token,
            next,
        } => {
            let append = next.is_some();
            let result = match source {
                Source::Local => Ok((local::scan(&folders), None)),
                Source::PolyHaven => polyhaven_list().map(|a| (a, None)),
                Source::Sketchfab => {
                    let url = next.unwrap_or_else(|| sketchfab::search_url(&query));
                    let tok = (!token.is_empty()).then_some(token.as_str());
                    net::get_json(&url, tok).and_then(|j| sketchfab::parse_search(&j))
                }
            };
            Some(Reply::List {
                generation,
                append,
                result,
            })
        }
        Job::Thumb { generation, asset } => {
            if generation < current.load(Ordering::Relaxed) {
                return None;
            }
            let key = asset.key();
            let file = cache_dir().map(|d| d.join("thumbs").join(format!("{key}.png")));
            if let Some(f) = &file
                && let Ok(img) = image::open(f)
            {
                return Some(Reply::Thumb {
                    key,
                    image: Ok(img.to_rgba8()),
                });
            }
            if let Some(p) = &asset.local_path {
                if asset.size.unwrap_or(0) > MAX_THUMB_FILE {
                    return Some(Reply::Thumb {
                        key,
                        image: Err("too large".into()),
                    });
                }
                return Some(match local::load_model(p) {
                    Ok(m) => Reply::ThumbModel {
                        key,
                        model: Box::new(m),
                    },
                    Err(e) => Reply::Thumb { key, image: Err(e) },
                });
            }
            let image = asset
                .thumbnail_url
                .as_deref()
                .ok_or_else(|| "no thumbnail".to_owned())
                .and_then(|u| net::get_bytes(u, None))
                .and_then(|b| image::load_from_memory(&b).map_err(|e| e.to_string()))
                .map(|img| {
                    img.resize(THUMB_PX, THUMB_PX, image::imageops::FilterType::Triangle)
                        .to_rgba8()
                });
            if let (Ok(img), Some(f)) = (&image, &file) {
                save_png(f, img);
            }
            Some(Reply::Thumb { key, image })
        }
        Job::Details { asset } => {
            let size = net::get_json(&polyhaven::files_url(&asset.id), None)
                .and_then(|j| polyhaven::parse_files(&j, "1k"))
                .ok()
                .map(|d| d.size());
            Some(Reply::Details { id: asset.id, size })
        }
    }
}

pub(crate) fn save_png(path: &Path, img: &image::RgbaImage) {
    let r = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .map_err(|e| e.to_string())
        .and_then(|()| {
            img.save_with_format(path, image::ImageFormat::Png)
                .map_err(|e| e.to_string())
        });
    if let Err(e) = r {
        log::warn!("could not save thumbnail {}: {e}", path.display());
    }
}

/// Poly Haven's model list, from the cache when less than a day old.
fn polyhaven_list() -> Result<Vec<Asset>, String> {
    let file = cache_dir().map(|d| d.join("polyhaven").join("assets.json"));
    if let Some(f) = &file
        && let Ok(meta) = std::fs::metadata(f)
        && meta
            .modified()
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age < LIST_MAX_AGE)
        && let Ok(text) = std::fs::read_to_string(f)
        && let Ok(list) = polyhaven::parse_assets(&text)
    {
        return Ok(list);
    }
    match net::get_json(&polyhaven::list_url(), None) {
        Ok(text) => {
            let list = polyhaven::parse_assets(&text)?;
            if let Some(f) = &file {
                let _ = std::fs::create_dir_all(f.parent().expect("has a parent"));
                let _ = std::fs::write(f, &text);
            }
            Ok(list)
        }
        // Offline: an old list is better than none.
        Err(e) => file
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|t| polyhaven::parse_assets(&t).ok())
            .ok_or(e),
    }
}

/// A model downloading (and then being read) on its own thread.
pub(crate) struct Download {
    pub asset: Asset,
    pub progress: Arc<net::Progress>,
    rx: Receiver<Result<mt_kernel::Model, String>>,
}

impl Download {
    fn start(asset: Asset, token: String, ctx: &egui::Context) -> Self {
        let progress = Arc::new(net::Progress::default());
        let (tx, rx) = channel();
        let (a, p, ctx) = (asset.clone(), progress.clone(), ctx.clone());
        let _ = std::thread::Builder::new()
            .name("library-download".into())
            .spawn(move || {
                let result = fetch_and_read(&a, &token, &p, cache_dir().as_deref());
                let _ = tx.send(result);
                ctx.request_repaint();
            });
        Self {
            asset,
            progress,
            rx,
        }
    }

    fn poll(&self) -> Option<Result<mt_kernel::Model, String>> {
        self.rx.try_recv().ok()
    }
}

/// Downloads the model's files into the cache (files already there with the right
/// size are kept) and reads the main one.
fn fetch_and_read(
    asset: &Asset,
    token: &str,
    progress: &net::Progress,
    cache: Option<&Path>,
) -> Result<mt_kernel::Model, String> {
    if let Some(p) = &asset.local_path {
        return local::load_model(p);
    }
    let plan = match asset.source {
        Source::PolyHaven => net::get_json(&polyhaven::files_url(&asset.id), None)
            .and_then(|j| polyhaven::parse_files(&j, "1k"))?,
        Source::Sketchfab => {
            if token.is_empty() {
                return Err("Enter your Sketchfab API token to download.".into());
            }
            net::get_json(&sketchfab::download_url(&asset.id), Some(token))
                .and_then(|j| sketchfab::parse_download(&j, &asset.id))?
        }
        Source::Local => return Err("No file.".into()),
    };
    let dir = cache
        .ok_or("No user data folder for the download cache.")?
        .join("models")
        .join(asset.key());
    progress.total.store(plan.size(), Ordering::Relaxed);
    for f in &plan.files {
        if !safe_relative(&f.path) {
            return Err(format!("Unsafe file path {}.", f.path));
        }
        let path = dir.join(&f.path);
        if f.size > 0 && std::fs::metadata(&path).is_ok_and(|m| m.len() == f.size) {
            progress.done.fetch_add(f.size, Ordering::Relaxed);
            continue;
        }
        let bytes = net::get_bytes(&f.url, Some(progress))?;
        std::fs::create_dir_all(path.parent().expect("has a parent")).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("part");
        std::fs::write(&tmp, &bytes)
            .and_then(|()| std::fs::rename(&tmp, &path))
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let main = dir.join(&plan.files.first().ok_or("Nothing to download.")?.path);
    local::load_model(&main)
}

/// The Model Library window's state.
#[derive(Default)]
pub(crate) struct Library {
    pub open: bool,
    pub config: Config,
    config_loaded: bool,
    pub source: Source,
    pub query: String,
    /// Everything the current source listed (Poly Haven and local: all of it,
    /// filtered by `query` when shown; Sketchfab: the search results).
    pub assets: Vec<Asset>,
    pub next_page: Option<String>,
    pub selected: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub download: Option<Download>,
    worker: Option<Worker>,
    generation: u64,
    /// Thumbnails by asset key; `None` while loading or after an error.
    thumbs: HashMap<String, Option<egui::TextureHandle>>,
    requested: HashSet<String>,
    /// Local files read for thumbnails, rendered on the UI thread one per frame.
    to_render: Vec<(String, Box<mt_kernel::Model>)>,
    /// Sketchfab token being typed.
    pub token_edit: String,
    /// Offscreen view for local files' thumbnails (made on first use).
    viewport: Option<mt_render::Viewport>,
}

impl Library {
    pub(crate) fn open(&mut self, ctx: &egui::Context) {
        self.open = true;
        if !self.config_loaded {
            self.config = Config::load();
            self.token_edit = self.config.sketchfab_token.clone();
            self.config_loaded = true;
        }
        if self.worker.is_none() {
            self.worker = Some(Worker::start(ctx));
            self.search();
        }
    }

    /// Folders scanned for local files: the Components folder, then added ones.
    pub(crate) fn folders(&self) -> Vec<PathBuf> {
        let mut f: Vec<PathBuf> = crate::filedialog::folder(crate::filedialog::Folder::Components)
            .into_iter()
            .collect();
        for x in &self.config.folders {
            if !f.contains(x) {
                f.push(x.clone());
            }
        }
        f
    }

    pub(crate) fn sources(&self) -> Vec<Source> {
        let mut s = vec![Source::Local, Source::PolyHaven];
        if !self.config.sketchfab_token.is_empty() {
            s.push(Source::Sketchfab);
        }
        s
    }

    /// Lists the current source again (Sketchfab: searches for `query`).
    pub(crate) fn search(&mut self) {
        self.list(None);
    }

    pub(crate) fn more(&mut self) {
        if let Some(next) = self.next_page.take() {
            self.list(Some(next));
        }
    }

    fn list(&mut self, next: Option<String>) {
        let Some(w) = &self.worker else { return };
        if next.is_none() {
            self.generation += 1;
            w.generation.store(self.generation, Ordering::Relaxed);
            self.assets.clear();
            self.selected = None;
            self.requested.retain(|k| self.thumbs.contains_key(k));
        }
        self.loading = true;
        self.error = None;
        let _ = w.jobs.send(Job::List {
            generation: self.generation,
            source: self.source,
            query: self.query.clone(),
            folders: self.folders(),
            token: self.config.sketchfab_token.clone(),
            next,
        });
    }

    /// Indices into `assets` of those shown: filtered here for sources listed whole.
    pub(crate) fn shown(&self) -> Vec<usize> {
        match self.source {
            Source::Sketchfab => (0..self.assets.len()).collect(),
            _ => filter(&self.assets, &self.query),
        }
    }

    pub(crate) fn selected_asset(&self) -> Option<&Asset> {
        let id = self.selected.as_ref()?;
        self.assets.iter().find(|a| &a.id == id)
    }

    pub(crate) fn select(&mut self, id: String) {
        if self.selected.as_ref() == Some(&id) {
            return;
        }
        self.selected = Some(id);
        if let (Some(a), Some(w)) = (self.selected_asset(), &self.worker)
            && a.source == Source::PolyHaven
            && a.size.is_none()
        {
            let _ = w.jobs.send(Job::Details {
                asset: Box::new(a.clone()),
            });
        }
    }

    /// The thumbnail of `a`, asking for it the first time.
    pub(crate) fn thumbnail(&mut self, a: &Asset) -> Option<egui::TextureHandle> {
        let key = a.key();
        if let Some(t) = self.thumbs.get(&key) {
            return t.clone();
        }
        if self.requested.insert(key)
            && let Some(w) = &self.worker
        {
            let _ = w.thumbs.send(Job::Thumb {
                generation: self.generation,
                asset: Box::new(a.clone()),
            });
        }
        None
    }

    /// The thumbnail of `a` was asked for and has not come yet.
    pub(crate) fn thumb_pending(&self, a: &Asset) -> bool {
        let key = a.key();
        self.requested.contains(&key) && !self.thumbs.contains_key(&key)
    }

    pub(crate) fn add_folder(&mut self, dir: PathBuf) {
        if !self.config.folders.contains(&dir) {
            self.config.folders.push(dir);
            self.config.save();
        }
        if self.source == Source::Local {
            self.search();
        }
    }

    pub(crate) fn remove_folder(&mut self, i: usize) {
        if i < self.config.folders.len() {
            self.config.folders.remove(i);
            self.config.save();
            if self.source == Source::Local {
                self.search();
            }
        }
    }

    pub(crate) fn save_token(&mut self) {
        self.config.sketchfab_token = self.token_edit.trim().to_owned();
        self.config.save();
        if self.config.sketchfab_token.is_empty() && self.source == Source::Sketchfab {
            self.source = Source::PolyHaven;
            self.search();
        }
    }

    pub(crate) fn start_download(&mut self, ctx: &egui::Context) {
        if self.download.is_some() {
            return;
        }
        if let Some(a) = self.selected_asset().cloned() {
            self.error = None;
            self.download = Some(Download::start(a, self.config.sketchfab_token.clone(), ctx));
        }
    }

    /// Takes the worker's answers. Returns a model to render a thumbnail of.
    fn poll(&mut self, ctx: &egui::Context) {
        let Some(w) = &self.worker else { return };
        while let Ok(reply) = w.replies.try_recv() {
            match reply {
                Reply::List {
                    generation,
                    append,
                    result,
                } => {
                    if generation != self.generation {
                        continue;
                    }
                    self.loading = false;
                    match result {
                        Ok((assets, next)) => {
                            if !append {
                                self.assets.clear();
                            }
                            self.assets.extend(assets);
                            self.next_page = next;
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                Reply::Thumb { key, image } => {
                    let tex = image.ok().map(|img| texture(ctx, &key, &img));
                    self.thumbs.insert(key, tex);
                }
                Reply::ThumbModel { key, model } => self.to_render.push((key, model)),
                Reply::Details { id, size } => {
                    if let Some(a) = self.assets.iter_mut().find(|a| a.id == id) {
                        a.size = size;
                    }
                }
            }
        }
    }

    fn set_rendered_thumb(&mut self, ctx: &egui::Context, key: String, img: &image::RgbaImage) {
        if let Some(d) = cache_dir() {
            save_png(&d.join("thumbs").join(format!("{key}.png")), img);
        }
        let tex = texture(ctx, &key, img);
        self.thumbs.insert(key, Some(tex));
    }
}

fn texture(ctx: &egui::Context, key: &str, img: &image::RgbaImage) -> egui::TextureHandle {
    let size = [img.width() as usize, img.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    ctx.load_texture(
        format!("mt_library_{key}"),
        color,
        egui::TextureOptions::LINEAR,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str, tags: &[&str]) -> Asset {
        Asset {
            name: name.into(),
            id: name.to_lowercase(),
            tags: tags.iter().map(|t| (*t).to_owned()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn filter_matches_every_word_in_name_or_tags() {
        let a = [
            asset("Wooden Chair", &["furniture"]),
            asset("Table", &["wood", "furniture"]),
            asset("Lamp", &["light"]),
        ];
        let names =
            |q| -> Vec<String> { filter(&a, q).iter().map(|&i| a[i].name.clone()).collect() };
        assert_eq!(names(""), ["Wooden Chair", "Table", "Lamp"]);
        assert_eq!(names("wood"), ["Wooden Chair", "Table"]);
        assert_eq!(names("WOOD furniture chair"), ["Wooden Chair"]);
        assert!(names("sofa").is_empty());
    }

    #[test]
    fn safe_relative_paths() {
        assert!(safe_relative("m.bin"));
        assert!(safe_relative("textures/a.jpg"));
        for bad in ["", "../a", "a/../../b", "/etc/passwd", "a\\..\\b", "./a"] {
            assert!(!safe_relative(bad), "{bad}");
        }
    }

    #[test]
    fn keys_are_file_safe_and_stable() {
        let a = Asset {
            source: Source::Sketchfab,
            id: "a/b:c".into(),
            ..Default::default()
        };
        assert_eq!(a.key(), "sketchfab-a_b_c");
        assert_eq!(fnv1a(b"abc"), 0xe71f_a219_0541_574b);
        assert_eq!(file_safe("..x"), "x");
    }

    #[test]
    fn config_round_trips() {
        let c = Config {
            folders: vec![PathBuf::from("/models/a"), PathBuf::from("/b")],
            sketchfab_token: "tok".into(),
        };
        assert_eq!(Config::from_toml(&c.to_toml()), c);
        assert_eq!(Config::from_toml("not = [toml"), Config::default());
        assert!(!Config::default().to_toml().contains("token"));
    }

    /// Live check against Poly Haven (network): list, search, thumbnail, download
    /// and read a small CC0 model. `cargo test -p mt-app live_ -- --ignored`.
    #[test]
    #[ignore = "uses the network"]
    fn live_polyhaven_search_download_and_read() {
        let list = net::get_json(&polyhaven::list_url(), None).unwrap();
        let assets = polyhaven::parse_assets(&list).unwrap();
        assert!(assets.len() > 50, "{} models", assets.len());
        let found = filter(&assets, "rubber duck");
        let a = &assets[*found.first().expect("the rubber duck model")];
        let thumb = net::get_bytes(a.thumbnail_url.as_deref().unwrap(), None).unwrap();
        assert!(image::load_from_memory(&thumb).is_ok());
        let cache = std::env::temp_dir().join(format!("mt_library_live_{}", std::process::id()));
        let p = net::Progress::default();
        let m = fetch_and_read(a, "", &p, Some(&cache)).unwrap();
        let faces: usize = m.definitions.values().map(|d| d.entities.faces.len()).sum();
        assert!(faces + m.root.faces.len() > 100, "{faces} faces");
        assert!(p.fraction() > 0.99);
        assert!(m.materials.iter().any(|x| x.texture.is_some()));
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn credit_and_sizes() {
        let a = Asset {
            source: Source::PolyHaven,
            name: "Stool".into(),
            author: "A".into(),
            ..Default::default()
        };
        assert_eq!(a.credit(), "Stool by A, from Poly Haven");
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(2048), "2 KB");
        assert_eq!(format_size(3 << 20), "3.0 MB");
    }
}
