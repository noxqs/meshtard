//! Tray panel parts of LEFTOVERS.md 10, app side: several tags at once (delete,
//! show / hide, move, Select All Tagged), Apply Tags to Scenes, Show New Tags,
//! section plane rows of the Outliner, the Components Statistics tab, library
//! collections (Materials, Components, Styles), `.style` files, the Styles Mix tab
//! and scene thumbnails.
//!
//! Collections are folders: the Preferences > Files folder of the kind, `Materials`,
//! `Components` or `Styles` under the data folder, and folders added with Details >
//! Open or Create a Collection (remembered in `collections.txt` in the config
//! folder). A folder with files is a collection, and so is each subfolder with
//! files. Materials collections hold PNG / JPEG images and SketchUp `.skm` files
//! (read by [`crate::skm`]), Components collections `.skp` files, Styles collections
//! `.style` files. Built in: a "Colors" materials collection and a "Default Styles"
//! styles collection (names and contents our own drafts, unverified). Favorite
//! collections: see [`crate::tray_files`].
//!
//! `.style` files are told apart by their content: SketchUp's own are zip archives
//! (read by [`crate::style_zip`]: its items, watermarks and thumbnail), ours are a
//! text file of `key=value` lines with the style items of
//! `mt_kernel::export::style_items` plus fog, a few model-only settings and the
//! watermarks (image in base64). Save As writes either kind
//! ([`crate::tray_files`]).

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mt_kernel::model_data::NamedStyle;
use mt_kernel::{DefinitionId, Entities, Material, Model, StyleSettings};
use mt_ui::{Collection, CollectionItem, CollectionKind, Event, StyleSource};

use crate::App;
use crate::document::Owner;
use crate::selection::Mode;

/// What a tag index becomes when `deleted` (sorted, unique) tags are removed:
/// `None` for a deleted tag.
pub(crate) fn remap_tag(l: usize, deleted: &[usize]) -> Option<usize> {
    if deleted.binary_search(&l).is_ok() {
        return None;
    }
    Some(l - deleted.iter().take_while(|&&d| d < l).count())
}

/// Removes tags `tags` (Untagged and unknown indices are skipped); entities on them
/// go to Untagged, indices above them move down, scenes forget them. `ents` is the
/// open context's entities (taken out of the model by `Document::edit_model`).
pub(crate) fn delete_tags(m: &mut Model, ents: &mut Entities, tags: &[usize]) -> bool {
    let deleted: Vec<usize> = tags
        .iter()
        .copied()
        .filter(|&t| t > 0 && t < m.layers.len())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if deleted.is_empty() {
        return false;
    }
    for &t in deleted.iter().rev() {
        m.layers.remove(t);
    }
    let fix = |l: &mut Option<usize>| *l = l.and_then(|x| remap_tag(x, &deleted));
    for e in std::iter::once(ents)
        .chain(m.definitions.values_mut().map(|d| &mut d.entities))
        .chain(std::iter::once(&mut m.root))
    {
        for x in e.edges.values_mut() {
            fix(&mut x.attrs.layer);
        }
        for x in e.faces.values_mut() {
            fix(&mut x.attrs.layer);
        }
        for x in &mut e.instances {
            fix(&mut x.layer);
        }
        for x in &mut e.dimensions {
            fix(&mut x.layer);
        }
        for x in &mut e.texts {
            fix(&mut x.layer);
        }
    }
    for p in &mut m.pages {
        p.hidden_layers = p
            .hidden_layers
            .iter()
            .filter_map(|&l| remap_tag(l, &deleted))
            .collect();
    }
    true
}

/// A tag just added: hidden in every existing scene unless Show New Tags is on.
pub(crate) fn new_tag_in_scenes(m: &mut Model, tag: usize) {
    if m.show_new_tags {
        return;
    }
    for p in &mut m.pages {
        if !p.hidden_layers.contains(&tag) {
            p.hidden_layers.push(tag);
        }
    }
}

/// Apply Tags to Scenes: scenes `scenes` take the current visibility of `tags`.
/// Returns whether a scene changed.
pub(crate) fn apply_tags_to_scenes(m: &mut Model, tags: &[usize], scenes: &[usize]) -> bool {
    let mut changed = false;
    for &s in scenes {
        let Some(p) = m.pages.get_mut(s) else {
            continue;
        };
        for &t in tags {
            let Some(l) = m.layers.get(t) else { continue };
            let hidden = p.hidden_layers.contains(&t);
            if l.visible && hidden {
                p.hidden_layers.retain(|&x| x != t);
                changed = true;
            } else if !l.visible && !hidden {
                p.hidden_layers.push(t);
                changed = true;
            }
        }
        p.hidden_layers.sort_unstable();
    }
    changed
}

/// What the Components Statistics tab counts.
pub(crate) const STAT_ROWS: [&str; 12] = [
    "Edges",
    "Faces",
    "Component Instances",
    "Groups",
    "Images",
    "Guides",
    "Guide Points",
    "Section Planes",
    "Dimensions",
    "Texts",
    "Materials",
    "Tags",
];

#[derive(Default)]
struct Counts {
    n: [usize; 10],
    materials: BTreeSet<usize>,
    tags: BTreeSet<usize>,
}

fn count(m: &Model, ents: &Entities, nested: bool, depth: usize, out: &mut Counts) {
    let c = &mut out.n;
    c[0] += ents.edges.len();
    c[1] += ents.faces.len();
    c[5] += ents.guides.iter().filter(|g| g.dir.is_some()).count();
    c[6] += ents.guides.iter().filter(|g| g.dir.is_none()).count();
    c[7] += ents.section_planes.len();
    c[8] += ents.dimensions.len();
    c[9] += ents.texts.len();
    for f in ents.faces.values() {
        out.materials.extend(f.attrs.material);
        out.materials.extend(f.attrs.back_material);
        out.tags.extend(f.attrs.layer);
    }
    for e in ents.edges.values() {
        out.tags.extend(e.attrs.layer);
    }
    for i in &ents.instances {
        out.materials.extend(i.material);
        out.tags.extend(i.layer);
        let Some(d) = m.definitions.get(i.definition) else {
            continue;
        };
        let k = if d.is_image {
            4
        } else if d.is_group {
            3
        } else {
            2
        };
        out.n[k] += 1;
        if nested && depth < 32 {
            count(m, &d.entities, true, depth + 1, out);
        }
    }
}

/// The Statistics tab's rows for definition `d`: (label, count in the definition,
/// count with nested groups and components expanded, each instance counted).
pub(crate) fn component_stats(m: &Model, d: DefinitionId) -> Vec<(String, usize, usize)> {
    let Some(def) = m.definitions.get(d) else {
        return Vec::new();
    };
    let (mut own, mut all) = (Counts::default(), Counts::default());
    count(m, &def.entities, false, 0, &mut own);
    count(m, &def.entities, true, 0, &mut all);
    STAT_ROWS
        .iter()
        .enumerate()
        .map(|(k, label)| {
            let (a, b) = match k {
                10 => (own.materials.len(), all.materials.len()),
                11 => (own.tags.len(), all.tags.len()),
                k => (own.n[k], all.n[k]),
            };
            ((*label).to_owned(), a, b)
        })
        .collect()
}

/// Styles Mix: takes setting group `part` (index into `mt_ui::STYLE_PARTS`) of
/// `src` into `dst`.
pub(crate) fn mix_style(dst: &mut StyleSettings, src: &StyleSettings, part: usize) {
    match part {
        0 => {
            dst.edges = src.edges;
            dst.profiles = src.profiles;
            dst.profile_width = src.profile_width;
            dst.back_edges = src.back_edges;
            dst.depth_cue = src.depth_cue;
            dst.depth_cue_width = src.depth_cue_width;
            dst.extension = src.extension;
            dst.extension_length = src.extension_length;
            dst.edge_look = src.edge_look;
        }
        1 => {
            dst.face_style = src.face_style;
            dst.xray = src.xray;
            dst.face_look = src.face_look;
            dst.ambient_occlusion = src.ambient_occlusion;
        }
        2 => dst.background = src.background,
        3 => dst.watermarks.clone_from(&src.watermarks),
        4 => {
            dst.modeling_colors = src.modeling_colors;
            dst.modeling_extras = src.modeling_extras;
            dst.section_cut_width = src.section_cut_width;
            dst.section_fill = src.section_fill;
            dst.section_fill_color = src.section_fill_color;
            dst.section_planes = src.section_planes;
            dst.section_cuts = src.section_cuts;
            dst.guides = src.guides;
            dst.hidden_geometry = src.hidden_geometry;
            dst.hidden_objects = src.hidden_objects;
            dst.axes = src.axes;
        }
        _ => {}
    }
}

const STYLE_HEADER: &str = "# meshtard style 1";

/// Face styles by their `face_style=` number (Monochrome has no style item).
const FACE_STYLES: [mt_kernel::FaceStyle; 5] = [
    mt_kernel::FaceStyle::Wireframe,
    mt_kernel::FaceStyle::HiddenLine,
    mt_kernel::FaceStyle::Shaded,
    mt_kernel::FaceStyle::ShadedWithTextures,
    mt_kernel::FaceStyle::Monochrome,
];

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n")
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        match (c, c == '\\') {
            (_, true) => match it.next() {
                Some('n') => out.push('\n'),
                Some(x) => out.push(x),
                None => out.push('\\'),
            },
            (c, false) => out.push(c),
        }
    }
    out
}

/// A `.style` file's text for `style` (see the module docs).
pub(crate) fn style_file_text(style: &NamedStyle) -> String {
    use mt_skp::OptionValue;
    let s = &style.settings;
    let mut out = format!(
        "{STYLE_HEADER}\nname={}\ndescription={}\n",
        escape(&style.name),
        escape(&style.description)
    );
    for (id, v) in mt_kernel::export::style_items(s) {
        let v = match v {
            OptionValue::Bool(b) => i32::from(b).to_string(),
            OptionValue::Int(i) => i.to_string(),
            OptionValue::Double(d) => d.to_string(),
        };
        out.push_str(&format!("{id}={v}\n"));
    }
    let f = &s.fog;
    let [r, g, b] = f.color;
    let [ar, ag, ab] = s.ambient_occlusion.color;
    let me = &s.modeling_extras;
    let rgb = |c: [u8; 3]| format!("{},{},{}", c[0], c[1], c[2]);
    out.push_str(&format!(
        "modeling.hidden={}\nmodeling.endpoint={}\nmodeling.midpoint={}\n\
         modeling.on_edge={}\nmodeling.on_face={}\n",
        rgb(me.hidden_geometry),
        rgb(me.inference.endpoint),
        rgb(me.inference.midpoint),
        rgb(me.inference.on_edge),
        rgb(me.inference.on_face),
    ));
    out.push_str(&format!(
        "fog.enabled={}\nfog.start={}\nfog.end={}\nfog.use_background={}\nfog.color={r},{g},{b}\n\
         hidden_objects={}\nao.use_color={}\nao.color={ar},{ag},{ab}\nface_style={}\n\
         horizon_blend={}\n",
        u8::from(f.enabled),
        f.start,
        f.end,
        u8::from(f.use_background),
        u8::from(s.hidden_objects),
        u8::from(s.ambient_occlusion.use_color),
        FACE_STYLES
            .iter()
            .position(|f| *f == s.face_style)
            .unwrap_or(0),
        s.background.horizon_blend,
    ));
    // Watermarks, bottom first: overlay;blend;mask;placement;image (base64);name.
    for w in &s.watermarks {
        out.push_str(&format!(
            "watermark={};{};{};{};{};{}\n",
            u8::from(w.overlay),
            w.opacity,
            u8::from(w.mask),
            crate::style_zip::mode_text(&w.mode),
            mt_kernel::gltf_io::base64_encode(&w.image),
            escape(&w.name),
        ));
    }
    out
}

/// A `watermark=` line's value (see [`style_file_text`]).
fn parse_watermark(v: &str) -> Option<mt_kernel::style_extras::Watermark> {
    let mut it = v.splitn(6, ';');
    let overlay = it.next()?.trim() == "1";
    let opacity = it.next()?.trim().parse::<f32>().ok()?;
    let mask = it.next()?.trim() == "1";
    let mode = crate::style_zip::parse_mode(it.next()?)?;
    let image = mt_kernel::gltf_io::base64_decode(it.next()?)?;
    let name = unescape(it.next().unwrap_or(""));
    (!image.is_empty()).then(|| mt_kernel::style_extras::Watermark {
        name: if name.is_empty() {
            "Watermark".into()
        } else {
            name
        },
        image: Arc::from(image),
        overlay,
        opacity: if opacity.is_finite() {
            opacity.clamp(0.0, 1.0)
        } else {
            1.0
        },
        mode,
        mask,
    })
}

fn parse_rgb(v: &str) -> Option<[u8; 3]> {
    let mut it = v.split(',').map(|x| x.trim().parse::<u8>().ok());
    Some([it.next()??, it.next()??, it.next()??])
}

/// Reads a `.style` file of either kind (see the module docs), with its thumbnail
/// image when it has one.
pub(crate) fn read_style_bytes(data: &[u8]) -> Result<(NamedStyle, Option<Vec<u8>>), String> {
    if crate::style_zip::is_zip(data) {
        return crate::errors::guard(|| crate::style_zip::read(data))
            .map(|z| (z.style, z.thumbnail));
    }
    let text = std::str::from_utf8(data).map_err(|_| "Not a style file.".to_owned())?;
    parse_style_file(text).map(|s| (s, None))
}

/// Reads a `.style` file's text; settings it lacks keep the defaults.
pub(crate) fn parse_style_file(text: &str) -> Result<NamedStyle, String> {
    if !text.starts_with(STYLE_HEADER) {
        return Err("Not a meshtard style file.".into());
    }
    let mut items: HashMap<i32, String> = HashMap::new();
    let mut named = NamedStyle {
        name: "Style".into(),
        description: String::new(),
        settings: StyleSettings::default(),
    };
    let mut rest: Vec<(String, String)> = Vec::new();
    for line in text.lines().skip(1) {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match (k.trim(), k.trim().parse::<i32>()) {
            ("name", _) => named.name = unescape(v),
            ("description", _) => named.description = unescape(v),
            (_, Ok(id)) => {
                items.insert(id, v.trim().to_owned());
            }
            (k, _) => rest.push((k.to_owned(), v.trim().to_owned())),
        }
    }
    mt_kernel::import::read_style_items(
        &|id| items.get(&id).map(String::as_str),
        &mut named.settings,
    );
    let s = &mut named.settings;
    for (k, v) in rest {
        let flag = v == "1";
        match k.as_str() {
            "fog.enabled" => s.fog.enabled = flag,
            "fog.use_background" => s.fog.use_background = flag,
            "fog.start" => s.fog.start = v.parse().unwrap_or(s.fog.start),
            "fog.end" => s.fog.end = v.parse().unwrap_or(s.fog.end),
            "fog.color" => s.fog.color = parse_rgb(&v).unwrap_or(s.fog.color),
            "hidden_objects" => s.hidden_objects = flag,
            "face_style" => {
                if let Some(f) = v.parse::<usize>().ok().and_then(|i| FACE_STYLES.get(i)) {
                    s.face_style = *f;
                }
            }
            "horizon_blend" => {
                s.background.horizon_blend = v.parse().unwrap_or(s.background.horizon_blend);
            }
            "ao.use_color" => s.ambient_occlusion.use_color = flag,
            "watermark" => s.watermarks.extend(parse_watermark(&v)),
            "modeling.hidden" | "modeling.endpoint" | "modeling.midpoint" | "modeling.on_edge"
            | "modeling.on_face" => {
                let me = &mut s.modeling_extras;
                let c = match k.as_str() {
                    "modeling.hidden" => &mut me.hidden_geometry,
                    "modeling.endpoint" => &mut me.inference.endpoint,
                    "modeling.midpoint" => &mut me.inference.midpoint,
                    "modeling.on_edge" => &mut me.inference.on_edge,
                    _ => &mut me.inference.on_face,
                };
                *c = parse_rgb(&v).unwrap_or(*c);
            }
            "ao.color" => {
                s.ambient_occlusion.color = parse_rgb(&v).unwrap_or(s.ambient_occlusion.color);
            }
            _ => {}
        }
    }
    if named.name.trim().is_empty() {
        named.name = "Style".into();
    }
    Ok(named)
}

/// The built-in "Default Styles" collection (drafts of our own).
pub(crate) fn builtin_styles() -> Vec<NamedStyle> {
    use mt_kernel::FaceStyle;
    let base = StyleSettings::default;
    let with = |name: &str, desc: &str, f: &dyn Fn(&mut StyleSettings)| {
        let mut settings = base();
        f(&mut settings);
        NamedStyle {
            name: name.into(),
            description: desc.into(),
            settings,
        }
    };
    vec![
        with("Default", "Shaded with textures, profiles.", &|_| {}),
        with("Shaded", "Shaded without textures.", &|s| {
            s.face_style = FaceStyle::Shaded;
        }),
        with("Hidden Line", "Hidden line, no profiles.", &|s| {
            s.face_style = FaceStyle::HiddenLine;
            s.profiles = false;
        }),
        with("Wireframe", "Edges only.", &|s| {
            s.face_style = FaceStyle::Wireframe;
            s.profiles = false;
        }),
        with("Monochrome", "Faces in the front and back colors.", &|s| {
            s.face_style = FaceStyle::Monochrome;
        }),
        with("X-ray", "Translucent faces.", &|s| s.xray = true),
        with(
            "Sketchy",
            "Jittered edges with extensions and endpoints.",
            &|s| {
                s.extension = true;
                s.edge_look.endpoints = true;
                s.edge_look.jitter = true;
                s.depth_cue = true;
            },
        ),
        with("Blueprint", "White edges on blue, hidden line.", &|s| {
            s.face_style = FaceStyle::HiddenLine;
            s.edge_look.color = [255, 255, 255];
            s.background.color = [24, 60, 120];
            s.background.sky = false;
            s.background.ground = false;
        }),
    ]
}

/// The built-in "Colors" materials collection (names our own).
pub(crate) fn builtin_colors() -> Vec<(&'static str, [u8; 3])> {
    vec![
        ("White", [255, 255, 255]),
        ("Light Gray", [200, 200, 200]),
        ("Gray", [128, 128, 128]),
        ("Charcoal", [64, 64, 64]),
        ("Black", [20, 20, 20]),
        ("Red", [200, 40, 40]),
        ("Brick Red", [156, 70, 50]),
        ("Orange", [235, 130, 40]),
        ("Sand", [220, 196, 150]),
        ("Yellow", [240, 210, 60]),
        ("Olive", [128, 128, 50]),
        ("Green", [70, 150, 70]),
        ("Grass", [110, 160, 60]),
        ("Teal", [40, 140, 140]),
        ("Sky Blue", [120, 180, 230]),
        ("Blue", [50, 90, 190]),
        ("Navy", [30, 40, 100]),
        ("Purple", [120, 70, 160]),
        ("Pink", [230, 150, 180]),
        ("Brown", [120, 80, 50]),
        ("Wood", [176, 128, 80]),
        ("Concrete", [170, 168, 160]),
        ("Glass", [190, 215, 230]),
        ("Cream", [245, 238, 215]),
    ]
}

/// An entry of a scanned collection.
#[derive(Clone)]
pub(crate) enum LibItem {
    Color(String, [u8; 3]),
    File(String, PathBuf),
    Style(Box<LibStyle>),
}

/// A style of a collection: the style, its thumbnail key (file path, or the name of
/// a built-in one) and its own thumbnail image (SketchUp `.style` files).
#[derive(Clone)]
pub(crate) struct LibStyle {
    pub style: NamedStyle,
    key: String,
    image: Option<Arc<[u8]>>,
}

impl LibItem {
    fn name(&self) -> &str {
        match self {
            Self::Color(n, _) | Self::File(n, _) => n,
            Self::Style(s) => &s.style.name,
        }
    }
}

#[derive(Clone)]
pub(crate) struct LibCollection {
    pub(crate) name: String,
    pub(crate) items: Vec<LibItem>,
    /// What favorites remember it by: its folder, or `builtin:<name>`.
    pub(crate) key: String,
}

fn kind_index(k: CollectionKind) -> usize {
    match k {
        CollectionKind::Materials => 0,
        CollectionKind::Components => 1,
        CollectionKind::Styles => 2,
    }
}

fn kind_name(k: usize) -> &'static str {
    ["materials", "components", "styles"][k]
}

fn folder_kind(k: usize) -> crate::filedialog::Folder {
    use crate::filedialog::Folder;
    [Folder::Materials, Folder::Components, Folder::Styles][k]
}

fn extensions(k: usize) -> &'static [&'static str] {
    match k {
        0 => &["png", "jpg", "jpeg", "skm"],
        1 => &["skp"],
        _ => &["style"],
    }
}

fn file_stem(p: &Path) -> String {
    p.file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned())
}

/// Files of kind `k` directly in `dir`, sorted by name.
fn files_in(dir: &Path, k: usize) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| extensions(k).iter().any(|x| x.eq_ignore_ascii_case(e)))
        })
        .collect();
    files.sort_by(|a, b| mt_ui::natural_cmp(&file_stem(a), &file_stem(b)));
    files
}

/// Collections found under `roots`: each root and each of its subfolders holding
/// files of kind `k`.
pub(crate) fn scan_folders(roots: &[PathBuf], k: usize) -> Vec<LibCollection> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let mut add = |dir: &Path, out: &mut Vec<LibCollection>| {
        let key = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        if !seen.insert(key) {
            return;
        }
        let files = files_in(dir, k);
        if files.is_empty() {
            return;
        }
        let items = files
            .into_iter()
            .filter_map(|p| match k {
                2 => std::fs::read(&p)
                    .ok()
                    .and_then(|t| read_style_bytes(&t).ok())
                    .map(|(style, image)| {
                        LibItem::Style(Box::new(LibStyle {
                            style,
                            key: p.display().to_string(),
                            image: image.map(Arc::from),
                        }))
                    }),
                _ => Some(LibItem::File(file_stem(&p), p)),
            })
            .collect();
        let name = dir.file_name().map_or_else(
            || dir.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let key = dir.display().to_string();
        out.push(LibCollection { name, items, key });
    };
    for root in roots {
        add(root, &mut out);
        let Ok(rd) = std::fs::read_dir(root) else {
            continue;
        };
        let mut subs: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        subs.sort();
        for s in subs {
            add(&s, &mut out);
        }
    }
    out
}

fn extra_path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("collections.txt"))
}

/// The collection folders added with Open or Create a Collection, per kind.
fn load_extra() -> [Vec<PathBuf>; 3] {
    let mut out: [Vec<PathBuf>; 3] = Default::default();
    let text = extra_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    for line in text.lines() {
        if let Some((k, p)) = line.split_once('\t')
            && let Some(i) = (0..3).find(|&i| kind_name(i) == k)
        {
            out[i].push(PathBuf::from(p));
        }
    }
    out
}

fn save_extra(extra: &[Vec<PathBuf>; 3]) {
    let Some(path) = extra_path() else { return };
    let mut text = String::new();
    for (k, dirs) in extra.iter().enumerate() {
        for d in dirs {
            text.push_str(&format!("{}\t{}\n", kind_name(k), d.display()));
        }
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, text) {
        log::warn!("could not save {}: {e}", path.display());
    }
}

/// A collection material's swatch: its image (none for a plain `.skm` color) and
/// color.
type MaterialThumb = (Option<egui::TextureHandle>, [u8; 3]);

/// Library collections and thumbnails of the panels.
#[derive(Default)]
pub(crate) struct TrayLib {
    /// Scanned collections per kind (`None`: to scan).
    pub(crate) collections: [Option<Vec<LibCollection>>; 3],
    extra: Option<[Vec<PathBuf>; 3]>,
    /// Favorite collections' keys per kind ([`LibCollection::key`]).
    pub(crate) favorites: Option<[Vec<String>; 3]>,
    /// Material thumbnails (image, or none for a plain `.skm` color, and the swatch
    /// color), and the files waiting to be loaded.
    thumbs: HashMap<PathBuf, Option<MaterialThumb>>,
    want_thumbs: Vec<PathBuf>,
    /// Scene thumbnails by [`scene_key`].
    scene_thumbs: HashMap<u64, egui::TextureHandle>,
    /// Rendered thumbnails of materials, components and styles.
    pub(crate) render: crate::thumbs::Thumbs,
}

impl TrayLib {
    fn extra(&mut self) -> &mut [Vec<PathBuf>; 3] {
        self.extra.get_or_insert_with(load_extra)
    }

    /// Collections of kind `k`, scanned on first use.
    pub(crate) fn collections(&mut self, k: usize) -> &[LibCollection] {
        if self.collections[k].is_none() {
            let mut roots: Vec<PathBuf> = Vec::new();
            roots.extend(crate::filedialog::folder(folder_kind(k)));
            let sub = ["Materials", "Components", "Styles"][k];
            roots.extend(crate::settings::data_dir().map(|d| d.join(sub)));
            roots.extend(self.extra()[k].iter().cloned());
            roots.extend(
                (self.favorites()[k].iter())
                    .filter(|f| !f.starts_with("builtin:"))
                    .map(PathBuf::from),
            );
            let mut list = match k {
                0 => vec![LibCollection {
                    key: "builtin:Colors".into(),
                    name: "Colors".into(),
                    items: builtin_colors()
                        .into_iter()
                        .map(|(n, c)| LibItem::Color(n.into(), c))
                        .collect(),
                }],
                2 => vec![LibCollection {
                    key: "builtin:Default Styles".into(),
                    name: "Default Styles".into(),
                    items: builtin_styles()
                        .into_iter()
                        .map(|style| {
                            LibItem::Style(Box::new(LibStyle {
                                key: format!("builtin:{}", style.name),
                                style,
                                image: None,
                            }))
                        })
                        .collect(),
                }],
                _ => Vec::new(),
            };
            list.extend(scan_folders(&roots, k));
            self.collections[k] = Some(list);
        }
        self.collections[k].as_deref().unwrap_or_default()
    }

    /// Scanned collection styles without an image of their own: their thumbnail
    /// keys and settings (to render).
    pub(crate) fn styles_to_render(&self) -> Vec<(String, StyleSettings)> {
        self.lib_styles()
            .filter(|s| s.image.is_none())
            .map(|s| (s.key.clone(), s.style.settings.clone()))
            .collect()
    }

    /// Scanned collection styles with an image of their own (keys and bytes).
    pub(crate) fn style_images(&self) -> Vec<(String, Arc<[u8]>)> {
        self.lib_styles()
            .filter_map(|s| Some((s.key.clone(), s.image.clone()?)))
            .collect()
    }

    fn lib_styles(&self) -> impl Iterator<Item = &LibStyle> {
        (self.collections[2].iter().flatten())
            .flat_map(|c| &c.items)
            .filter_map(|it| match it {
                LibItem::Style(s) => Some(&**s),
                _ => None,
            })
    }

    pub(crate) fn item(&mut self, k: usize, c: usize, i: usize) -> Option<LibItem> {
        self.collections(k).get(c)?.items.get(i).cloned()
    }

    fn ui_collections(&mut self, k: usize) -> Vec<Collection> {
        let cols: Vec<LibCollection> = self.collections(k).to_vec();
        let favorites = self.favorites()[k].clone();
        cols.iter()
            .map(|c| Collection {
                name: c.name.clone(),
                favorite: favorites.contains(&c.key),
                items: c
                    .items
                    .iter()
                    .map(|it| {
                        let (color, thumbnail) = match it {
                            LibItem::Color(_, c) => (Some(*c), None),
                            LibItem::File(_, p) if k == 1 => (None, self.render.file(p)),
                            LibItem::File(_, p) => match self.thumbs.get(p) {
                                Some(Some((t, c))) => (Some(*c), t.as_ref().map(|t| t.id())),
                                _ => (None, None),
                            },
                            LibItem::Style(s) => {
                                (None, self.render.style(&s.key, s.image.is_some()))
                            }
                        };
                        CollectionItem {
                            name: it.name().to_owned(),
                            color,
                            thumbnail,
                        }
                    })
                    .collect(),
            })
            .collect()
    }
}

/// Cache key of a scene's thumbnail: its camera and the model state.
fn scene_key(p: &mt_kernel::Page, state: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for v in [p.eye.to_array(), p.target.to_array()] {
        for x in v {
            x.to_bits().hash(&mut h);
        }
    }
    p.fov_deg.to_bits().hash(&mut h);
    p.perspective.hash(&mut h);
    state.hash(&mut h);
    h.finish()
}

/// Thumbnail size in pixels (16:9).
const SCENE_THUMB: [u32; 2] = [160, 90];

impl App {
    /// The parts of the tray data from this module.
    pub(crate) fn tray_data_extra(&mut self, data: &mut mt_ui::TrayData) {
        data.material_collections = self.tray_lib.ui_collections(0);
        data.component_collections = self.tray_lib.ui_collections(1);
        data.style_collections = self.tray_lib.ui_collections(2);
        data.material_thumbnails = self.material_thumbnails();
        (data.component_thumbnails, data.style_thumbnails) = self.rendered_thumbnails();
        let Some(doc) = &self.document else {
            return;
        };
        let m = &doc.model;
        data.show_new_tags = m.show_new_tags;
        if let Some(&d) = self
            .current_component
            .and_then(|i| crate::tray::component_definitions(m).get(i).copied())
            .as_ref()
        {
            data.component_stats = component_stats(m, d);
        }
        let state = doc.state_id();
        data.scene_thumbnails = m
            .pages
            .iter()
            .map(|p| {
                self.tray_lib
                    .scene_thumbs
                    .get(&scene_key(p, state))
                    .map(egui::TextureHandle::id)
            })
            .collect();
    }

    /// Loads waiting material thumbnails and renders one missing scene thumbnail per
    /// frame while the Scenes panel shows thumbnails.
    pub(crate) fn tray_lib_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        self.thumbnails_ui(ctx, frame);
        for _ in 0..4 {
            let Some(path) = self.tray_lib.want_thumbs.pop() else {
                break;
            };
            if self.tray_lib.thumbs.contains_key(&path) {
                continue;
            }
            let to_texture = |img: &image::DynamicImage| {
                let small = img.thumbnail(64, 64).to_rgba8();
                let size = [small.width() as usize, small.height() as usize];
                let pixels = egui::ColorImage::from_rgba_unmultiplied(size, small.as_raw());
                let name = format!("mt_material_thumb_{}", path.display());
                ctx.load_texture(name, pixels, egui::TextureOptions::LINEAR)
            };
            let thumb = if crate::skm::is_skm(&path) {
                // A `.skm` file: its texture image (shown flat or on a cube like
                // model materials), or its color.
                (std::fs::read(&path).ok())
                    .and_then(|d| crate::errors::guard(|| crate::skm::read(&d)).ok())
                    .map(|s| {
                        let m = s.material;
                        // The file's small thumbnail, else the texture itself.
                        let img = (m.texture.as_ref()).and_then(|t| {
                            (s.thumbnail.as_deref())
                                .and_then(|b| mt_kernel::images::decode(b).ok())
                                .or_else(|| mt_kernel::images::decode(&t.data).ok())
                        });
                        (img.as_ref().map(to_texture), m.color)
                    })
            } else {
                mt_kernel::images::open(&path).ok().map(|img| {
                    let color = crate::image_entity::average_color(&img.to_rgb8());
                    (Some(to_texture(&img)), color)
                })
            };
            self.tray_lib.thumbs.insert(path, thumb);
            ctx.request_repaint();
        }
        let shown = self
            .shell
            .as_ref()
            .is_some_and(mt_ui::Shell::scene_thumbnails_shown);
        let Some(doc) = &self.document else {
            return;
        };
        if !shown {
            return;
        }
        let state = doc.state_id();
        let keys: Vec<u64> = doc
            .model
            .pages
            .iter()
            .map(|p| scene_key(p, state))
            .collect();
        self.tray_lib.scene_thumbs.retain(|k, _| keys.contains(k));
        let Some(k) = keys
            .iter()
            .position(|k| !self.tray_lib.scene_thumbs.contains_key(k))
        else {
            return;
        };
        let p = &doc.model.pages[k];
        let camera = mt_render::Camera {
            eye: p.eye,
            target: p.target,
            fov_deg: p.fov_deg,
            perspective: p.perspective,
            two_point: false,
        };
        let Some(rs) = frame.wgpu_render_state() else {
            return;
        };
        // The current model as drawn now, seen from the scene's camera (the
        // scene's own style and hidden tags are not applied; unverified whether
        // SketchUp's thumbnails show them).
        let img =
            self.viewport
                .render_image(&rs.device, &rs.queue, &camera, SCENE_THUMB, 1.0, false);
        let size = [img.width() as usize, img.height() as usize];
        let pixels = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
        let tex = ctx.load_texture(
            format!("mt_scene_thumb_{k}"),
            pixels,
            egui::TextureOptions::LINEAR,
        );
        self.tray_lib.scene_thumbs.insert(keys[k], tex);
        ctx.request_repaint();
    }

    /// Outliner events on section plane rows (`Err`: not a section row, handled
    /// elsewhere).
    pub(crate) fn outliner_section_event(&mut self, e: Event) -> Result<bool, Event> {
        let path = match &e {
            Event::OutlinerSelect { path, .. }
            | Event::OutlinerRename { path, .. }
            | Event::OutlinerVisible { path, .. } => path.clone(),
            _ => return Err(e),
        };
        let Some(doc) = self.document.as_mut() else {
            return Err(e);
        };
        let Some((&last, up)) = path.split_last() else {
            return Err(e);
        };
        let parent = |m: &Model| -> Option<usize> {
            let ents = if up.is_empty() {
                &m.root
            } else {
                let i = crate::tray_more::instance_at(m, up)?;
                &m.definitions.get(i.definition)?.entities
            };
            let n = ents.instances.len();
            (last >= n && last - n < ents.section_planes.len()).then(|| last - n)
        };
        let Some(k) = parent(&doc.model) else {
            return Err(e);
        };
        let up = up.to_vec();
        let with = |m: &mut Model, f: &mut dyn FnMut(&mut mt_kernel::SectionPlane) -> bool| {
            let ents = if up.is_empty() {
                &mut m.root
            } else {
                let Some(d) = crate::tray_more::instance_at(m, &up).map(|i| i.definition) else {
                    return false;
                };
                match m.definitions.get_mut(d) {
                    Some(d) => &mut d.entities,
                    None => return false,
                }
            };
            ents.section_planes.get_mut(k).is_some_and(f)
        };
        Ok(match e {
            Event::OutlinerSelect { mode, .. } => {
                if doc.context != up {
                    doc.context.clone_from(&up);
                    self.selection.clear();
                }
                let mode = match mode {
                    mt_ui::SelectMode::Replace => Mode::Replace,
                    mt_ui::SelectMode::Add => Mode::Add,
                    mt_ui::SelectMode::Toggle => Mode::Toggle,
                    mt_ui::SelectMode::Remove => Mode::Remove,
                };
                self.selection.apply(mode, [Owner::Section(k)]);
                true
            }
            Event::OutlinerRename { name, .. } => doc.edit_whole(|m| {
                with(m, &mut |s| {
                    let changed = s.name != name;
                    s.name.clone_from(&name);
                    changed
                })
            }),
            Event::OutlinerVisible { visible, .. } => doc.edit_whole(|m| {
                with(m, &mut |s| {
                    let changed = s.hidden == visible;
                    s.hidden = !visible;
                    changed
                })
            }),
            _ => false,
        })
    }

    /// The selection becomes the open context's entities on `tags` (0: Untagged).
    fn select_tagged(&mut self, tags: &[usize]) -> bool {
        let Some(doc) = self.document.as_ref() else {
            return false;
        };
        let on = |l: Option<usize>| tags.contains(&l.unwrap_or(0));
        let ents = doc.active();
        let mut items: Vec<Owner> = Vec::new();
        items.extend(
            (ents.faces.iter())
                .filter(|(_, f)| on(f.attrs.layer))
                .map(|(k, _)| Owner::Face(k)),
        );
        items.extend(
            (ents.edges.iter())
                .filter(|(_, e)| on(e.attrs.layer))
                .map(|(k, _)| Owner::Edge(k)),
        );
        items.extend(
            (ents.instances.iter().enumerate())
                .filter(|(_, i)| on(i.layer))
                .map(|(k, _)| Owner::Instance(k)),
        );
        items.extend(
            (ents.dimensions.iter().enumerate())
                .filter(|(_, d)| on(d.layer))
                .map(|(k, _)| Owner::Dimension(k)),
        );
        items.extend(
            (ents.texts.iter().enumerate())
                .filter(|(_, t)| on(t.layer))
                .map(|(k, _)| Owner::Text(k)),
        );
        self.selection.apply(Mode::Replace, items);
        true
    }

    /// Makes material `i` current (Paint Bucket and the panel).
    pub(crate) fn make_current_material(&mut self, i: usize) {
        self.current_material = Some(i);
        if let Some(t) = self.modify.as_mut() {
            t.set_material(Some(i));
        }
    }

    /// Adds `style` to "In Model" (or updates the one of that name) and applies it.
    fn add_and_apply_style(&mut self, style: NamedStyle) -> bool {
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        let mut index = 0;
        doc.edit_model(|m, _| {
            index = match m.styles.iter().position(|s| s.name == style.name) {
                Some(i) => {
                    m.styles[i] = style.clone();
                    i
                }
                None => {
                    m.styles.push(style.clone());
                    m.styles.len() - 1
                }
            };
            true
        });
        // Applying is a display change (as the Select tab's click), not an undo step.
        doc.model.style = style.settings;
        doc.model.active_style = Some(index);
        true
    }

    /// The events of this module (`Err`: not one of them).
    pub(crate) fn tray_extra_event(&mut self, event: Event) -> Result<bool, Event> {
        let k = match &event {
            Event::CollectionOpened { kind, collection } => {
                // Material images: load their thumbnails.
                if *kind == CollectionKind::Materials {
                    let c = *collection;
                    let paths: Vec<PathBuf> = (self.tray_lib.collections(0).get(c))
                        .map(|c| {
                            (c.items.iter())
                                .filter_map(|it| match it {
                                    LibItem::File(_, p) => Some(p.clone()),
                                    _ => None,
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    for p in paths.into_iter().rev() {
                        if !self.tray_lib.thumbs.contains_key(&p) {
                            self.tray_lib.want_thumbs.push(p);
                        }
                    }
                }
                // Components: read the files for their rendered thumbnails.
                if *kind == CollectionKind::Components {
                    let paths: Vec<PathBuf> = (self.tray_lib.collections(1).get(*collection))
                        .map(|c| {
                            (c.items.iter())
                                .filter_map(|it| match it {
                                    LibItem::File(_, p) => Some(p.clone()),
                                    _ => None,
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    self.tray_lib.render.want_files(paths);
                }
                return Ok(false);
            }
            Event::CollectionAdd(kind) => kind_index(*kind),
            _ => usize::MAX,
        };
        if k < 3 {
            let start = crate::filedialog::folder(folder_kind(k)).or_else(|| {
                crate::settings::data_dir()
                    .map(|d| d.join(["Materials", "Components", "Styles"][k]))
            });
            if let Some(dir) = crate::filedialog::pick_folder(start.as_deref()) {
                let extra = &mut self.tray_lib.extra()[k];
                if !extra.contains(&dir) {
                    extra.push(dir.clone());
                }
                let all = self.tray_lib.extra().clone();
                save_extra(&all);
                self.tray_lib.collections[k] = None;
                if k == 2 {
                    self.tray_lib.render.forget_collections();
                }
                self.message = Some(format!("Collection folder added: {}", dir.display()));
            }
            return Ok(false);
        }
        match event {
            Event::TagSelectAll(t) => Ok(self.select_tagged(&[t])),
            Event::TagsSelectTagged(tags) => Ok(self.select_tagged(&tags)),
            Event::TagsDelete(tags) => {
                let Some(doc) = self.document.as_mut() else {
                    return Ok(false);
                };
                let mut sorted = tags.clone();
                sorted.sort_unstable();
                sorted.dedup();
                let changed = doc.edit_model(|m, ents| delete_tags(m, ents, &sorted));
                if changed {
                    self.active_tag = remap_tag(self.active_tag, &sorted).unwrap_or(0);
                }
                Ok(changed)
            }
            Event::TagsVisible(tags, v) => {
                let Some(doc) = self.document.as_mut() else {
                    return Ok(false);
                };
                Ok(doc.edit_model(|m, _| {
                    let mut changed = false;
                    for t in tags {
                        if let Some(l) = m.layers.get_mut(t)
                            && l.visible != v
                        {
                            l.visible = v;
                            changed = true;
                        }
                    }
                    changed
                }))
            }
            Event::TagsMove { tags, folder } => {
                let Some(doc) = self.document.as_mut() else {
                    return Ok(false);
                };
                Ok(doc.edit_model(|m, _| {
                    let folder = folder.filter(|&f| f < m.tag_folders.len());
                    let mut changed = false;
                    for t in tags.into_iter().filter(|&t| t > 0) {
                        if let Some(l) = m.layers.get_mut(t)
                            && l.folder != folder
                        {
                            l.folder = folder;
                            changed = true;
                        }
                    }
                    changed
                }))
            }
            Event::TagsApplyToScenes { tags, scenes } => {
                let Some(doc) = self.document.as_mut() else {
                    return Ok(false);
                };
                let changed = doc.edit_model(|m, _| apply_tags_to_scenes(m, &tags, &scenes));
                self.message = Some(if changed {
                    format!("Tag visibility applied to {} scenes.", scenes.len())
                } else {
                    "The scenes already show these tags so.".into()
                });
                Ok(false)
            }
            Event::ShowNewTags(on) => {
                if let Some(doc) = self.document.as_mut() {
                    doc.model.show_new_tags = on;
                }
                Ok(false)
            }
            Event::MaterialSample => {
                if let Some(t) = self.modify.as_mut() {
                    t.start_sample();
                }
                self.message = Some("Click a face or object to sample its material.".into());
                Ok(false)
            }
            Event::MaterialFromCollection { collection, item } => {
                let Some(it) = self.tray_lib.item(0, collection, item) else {
                    return Ok(false);
                };
                if self.document.is_none() {
                    return Ok(false);
                }
                // A `.skm` file: its own material (and name).
                let skm = match &it {
                    LibItem::File(_, path) if crate::skm::is_skm(path) => {
                        match std::fs::read(path)
                            .map_err(|e| e.to_string())
                            .and_then(|d| crate::errors::guard(|| crate::skm::read(&d)))
                        {
                            Ok(s) => Some(s.material),
                            Err(e) => {
                                self.file_error("open", path, e);
                                return Ok(false);
                            }
                        }
                    }
                    _ => None,
                };
                let Some(doc) = self.document.as_mut() else {
                    return Ok(false);
                };
                let name = skm
                    .as_ref()
                    .map_or(it.name(), |m| m.name.as_str())
                    .to_owned();
                // A material of that name in the model is used again (unverified).
                if let Some(i) = doc.model.materials.iter().position(|m| m.name == name) {
                    self.make_current_material(i);
                    return Ok(false);
                }
                let mut mat = Material {
                    name,
                    color: [200, 200, 200],
                    opacity: 1.0,
                    texture: None,
                };
                match it {
                    _ if skm.is_some() => mat = skm.unwrap_or(mat),
                    LibItem::Color(_, c) => mat.color = c,
                    LibItem::File(_, path) => {
                        let file = path
                            .file_name()
                            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                        let loaded = std::fs::read(&path)
                            .map_err(|e| e.to_string())
                            .and_then(|data| crate::material_edit::set_image(&mut mat, file, data));
                        if let Err(e) = loaded {
                            self.file_error("open", &path, e);
                            return Ok(false);
                        }
                    }
                    LibItem::Style(_) => return Ok(false),
                }
                let Some(doc) = self.document.as_mut() else {
                    return Ok(false);
                };
                doc.edit_model(|m, _| {
                    m.materials.push(mat);
                    true
                });
                let i = doc.model.materials.len() - 1;
                self.make_current_material(i);
                Ok(true)
            }
            Event::ComponentFromCollection { collection, item } => {
                let Some(LibItem::File(name, path)) = self.tray_lib.item(1, collection, item)
                else {
                    return Ok(false);
                };
                let model = match crate::errors::read_skp_model(&path) {
                    Ok(m) => m,
                    Err(e) => {
                        self.file_error("open", &path, e);
                        return Ok(false);
                    }
                };
                let Some(doc) = self.document.as_mut() else {
                    return Ok(false);
                };
                let mut def = None;
                doc.edit_model(|m, _| {
                    def = Some(mt_kernel::import::merge_as_definition(m, model, &name));
                    true
                });
                let Some(definition) = def else {
                    return Ok(false);
                };
                self.current_component = crate::tray::component_definitions(&doc.model)
                    .iter()
                    .position(|&d| d == definition);
                if let Some(d) = self.draw.as_mut() {
                    d.cancel();
                }
                self.tool = crate::nav::Tool::Modify("tool.place_component");
                self.draw = None;
                let glue = doc.model.definitions[definition].glue;
                self.modify = Some(Box::new(crate::modify::PlaceComponent {
                    definition,
                    glue,
                    ..Default::default()
                }));
                self.selection.clear();
                self.message = Some(format!("{name}: click to place it."));
                Ok(true)
            }
            Event::StyleFromCollection { collection, item } => {
                match self.tray_lib.item(2, collection, item) {
                    Some(LibItem::Style(s)) => Ok(self.add_and_apply_style(s.style)),
                    _ => Ok(false),
                }
            }
            Event::StyleOpenFile => {
                let filters: [(&str, &[&str]); 1] = [("Styles (*.style)", &["style"])];
                let Some(path) = crate::filedialog::open(&filters) else {
                    return Ok(false);
                };
                match std::fs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|t| read_style_bytes(&t))
                {
                    Ok((s, _)) => Ok(self.add_and_apply_style(s)),
                    Err(e) => {
                        self.file_error("open", &path, e);
                        Ok(false)
                    }
                }
            }
            Event::StyleMix { source, part } => {
                let src = match source {
                    StyleSource::InModel(i) => self
                        .document
                        .as_ref()
                        .and_then(|d| d.model.styles.get(i))
                        .map(|s| s.settings.clone()),
                    StyleSource::Collection { collection, item } => {
                        match self.tray_lib.item(2, collection, item) {
                            Some(LibItem::Style(s)) => Some(s.style.settings),
                            _ => None,
                        }
                    }
                };
                let (Some(src), Some(doc)) = (src, self.document.as_mut()) else {
                    return Ok(false);
                };
                // A display change like the Edit tab's: not an undo step.
                mix_style(&mut doc.model.style, &src, part);
                Ok(true)
            }
            e => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;
    use mt_kernel::{Layer, Page, add_line};

    fn model_with_tags(n: usize) -> Model {
        let mut m = Model::default();
        for k in 1..=n {
            m.layers.push(Layer {
                name: format!("T{k}"),
                ..Layer::default()
            });
        }
        m
    }

    #[test]
    fn remap_skips_deleted_tags() {
        let del = [2, 4];
        assert_eq!(remap_tag(1, &del), Some(1));
        assert_eq!(remap_tag(2, &del), None);
        assert_eq!(remap_tag(3, &del), Some(2));
        assert_eq!(remap_tag(5, &del), Some(3));
    }

    #[test]
    fn deleting_several_tags_moves_entities_and_scenes() {
        let mut m = model_with_tags(4);
        let mut ents = Entities::new();
        let e = add_line(&mut ents, DVec3::ZERO, DVec3::X)[0];
        ents.edges[e].attrs.layer = Some(4);
        let mut t = mt_kernel::Text::leader(DVec3::ZERO, DVec3::X, "a".into());
        t.layer = Some(2);
        m.root.texts.push(t);
        m.pages.push(Page {
            hidden_layers: vec![1, 2, 3, 4],
            ..Page::default()
        });
        assert!(delete_tags(&mut m, &mut ents, &[2, 3, 0]));
        assert_eq!(m.layers.len(), 3);
        assert_eq!(m.layers[2].name, "T4");
        assert_eq!(ents.edges[e].attrs.layer, Some(2));
        assert_eq!(m.root.texts[0].layer, None);
        assert_eq!(m.pages[0].hidden_layers, vec![1, 2]);
        assert!(!delete_tags(&mut m, &mut ents, &[0, 9]));
    }

    #[test]
    fn new_tags_hidden_in_scenes_unless_shown() {
        let mut m = model_with_tags(1);
        m.pages.push(Page::default());
        new_tag_in_scenes(&mut m, 1);
        assert!(m.pages[0].hidden_layers.is_empty());
        m.show_new_tags = false;
        new_tag_in_scenes(&mut m, 1);
        new_tag_in_scenes(&mut m, 1);
        assert_eq!(m.pages[0].hidden_layers, vec![1]);
    }

    #[test]
    fn apply_tags_writes_visibility_into_chosen_scenes() {
        let mut m = model_with_tags(3);
        m.layers[2].visible = false;
        m.pages.push(Page {
            hidden_layers: vec![1],
            ..Page::default()
        });
        m.pages.push(Page::default());
        assert!(apply_tags_to_scenes(&mut m, &[1, 2], &[0]));
        assert_eq!(m.pages[0].hidden_layers, vec![2]);
        assert!(m.pages[1].hidden_layers.is_empty(), "scene 1 not chosen");
        assert!(!apply_tags_to_scenes(&mut m, &[1, 2], &[0]), "nothing left");
    }

    #[test]
    fn statistics_count_own_and_nested() {
        let mut m = Model::default();
        let mut inner = mt_kernel::Definition {
            name: "Inner".into(),
            ..Default::default()
        };
        add_line(&mut inner.entities, DVec3::ZERO, DVec3::X);
        let inner = m.definitions.insert(inner);
        let mut outer = mt_kernel::Definition {
            name: "Outer".into(),
            ..Default::default()
        };
        add_line(&mut outer.entities, DVec3::ZERO, DVec3::Y);
        for x in [0.0, 5.0] {
            outer.entities.instances.push(mt_kernel::Instance {
                definition: inner,
                transform: glam::DMat4::from_translation(DVec3::new(x, 0.0, 0.0)),
                name: String::new(),
                hidden: false,
                locked: false,
                layer: None,
                material: None,
                attrs: Default::default(),
            });
        }
        outer.entities.guides.push(mt_kernel::Guide {
            point: DVec3::ZERO,
            dir: None,
        });
        let outer = m.definitions.insert(outer);
        let rows = component_stats(&m, outer);
        let row = |l: &str| rows.iter().find(|r| r.0 == l).map(|r| (r.1, r.2)).unwrap();
        assert_eq!(row("Edges"), (1, 3));
        assert_eq!(row("Component Instances"), (2, 2));
        assert_eq!(row("Guide Points"), (1, 1));
        assert_eq!(row("Guides"), (0, 0));
        assert_eq!(rows.len(), STAT_ROWS.len());
    }

    #[test]
    fn style_file_round_trip() {
        let mut s = builtin_styles()
            .into_iter()
            .find(|s| s.name == "Sketchy")
            .unwrap();
        s.description = "two\nlines".into();
        s.settings.fog.enabled = true;
        s.settings.fog.color = [1, 2, 3];
        s.settings.fog.start = 0.25;
        s.settings.background.color = [10, 20, 30];
        s.settings.face_style = mt_kernel::FaceStyle::HiddenLine;
        s.settings.modeling_extras.hidden_geometry = [4, 5, 6];
        s.settings.modeling_extras.inference.on_face = [7, 8, 9];
        s.settings.watermarks = test_watermarks();
        let text = style_file_text(&s);
        let back = parse_style_file(&text).unwrap();
        assert_eq!(back.name, s.name);
        assert_eq!(back.description, s.description);
        assert_eq!(back.settings, s.settings);
        assert!(parse_style_file("hello").is_err());
    }

    /// A background and an overlay of every kind of placement.
    pub(crate) fn test_watermarks() -> Vec<mt_kernel::style_extras::Watermark> {
        use mt_kernel::style_extras::{Watermark, WatermarkMode};
        vec![
            Watermark {
                name: "Paper; old\nworn".into(),
                image: Arc::from(&b"\x89PNG paper"[..]),
                overlay: false,
                opacity: 0.25,
                mode: WatermarkMode::Tiled { scale: 1.5 },
                mask: true,
            },
            Watermark {
                name: "Logo <&>".into(),
                image: Arc::from(&b"\xFF\xD8 logo"[..]),
                overlay: true,
                opacity: 1.0,
                mode: WatermarkMode::Positioned {
                    anchor: [1.0, 0.5],
                    scale: 0.75,
                },
                mask: false,
            },
            Watermark {
                name: "Frame".into(),
                image: Arc::from(&b"\x89PNG frame"[..]),
                overlay: true,
                opacity: 0.5,
                mode: WatermarkMode::Stretched { lock_aspect: false },
                mask: false,
            },
        ]
    }

    #[test]
    fn sketchup_style_round_trips_through_the_reader() {
        let mut s = builtin_styles()
            .into_iter()
            .find(|s| s.name == "Blueprint")
            .unwrap();
        s.name = "Mine & \"yours\"".into();
        s.description = "A <test>".into();
        s.settings.fog.enabled = true;
        s.settings.face_style = mt_kernel::FaceStyle::Monochrome;
        s.settings.watermarks = test_watermarks();
        let data = crate::style_zip::write(&s, Some(b"thumb")).unwrap();
        let (back, thumb) = read_style_bytes(&data).unwrap();
        assert_eq!(back.name, s.name);
        assert_eq!(back.description, s.description);
        // Backgrounds come first when read; the order within each kind is kept.
        assert_eq!(back.settings, s.settings);
        assert_eq!(thumb.as_deref(), Some(&b"thumb"[..]));
        // Without our text file the items alone carry most of it.
        let files: Vec<(String, Vec<u8>)> = crate::style_zip::entries(&data)
            .unwrap()
            .into_iter()
            .filter(|(n, _)| n != "meshtard.style")
            .collect();
        let bare = crate::style_zip::zip_files(&files).unwrap();
        let (b2, _) = read_style_bytes(&bare).unwrap();
        assert_eq!(b2.settings.background.color, s.settings.background.color);
        assert_eq!(b2.settings.edge_look.color, s.settings.edge_look.color);
        assert_eq!(b2.settings.watermarks, s.settings.watermarks);
        assert!(!b2.settings.fog.enabled);
    }

    #[test]
    fn every_builtin_style_round_trips() {
        for s in builtin_styles() {
            let back = parse_style_file(&style_file_text(&s)).unwrap();
            assert_eq!(back.settings, s.settings, "{}", s.name);
        }
    }

    #[test]
    fn style_files_of_either_kind_by_content() {
        use std::io::Write;
        let ours = style_file_text(&builtin_styles()[3]);
        let (s, image) = read_style_bytes(ours.as_bytes()).unwrap();
        assert_eq!(s.name, "Wireframe");
        assert!(image.is_none());
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        zip.start_file("style.xml", o).unwrap();
        zip.write_all(br#"<sty:style name="[Z]"><sty:item id="2001"><t:variant type="4">0</t:variant></sty:item></sty:style>"#)
            .unwrap();
        zip.start_file("doc_thumbnail.png", o).unwrap();
        zip.write_all(b"png").unwrap();
        let data = zip.finish().unwrap().into_inner();
        let (s, image) = read_style_bytes(&data).unwrap();
        assert_eq!(s.name, "Z");
        assert_eq!(s.settings.face_style, mt_kernel::FaceStyle::Wireframe);
        assert_eq!(image.as_deref(), Some(&b"png"[..]));
        assert!(read_style_bytes(&[0xff, 0xfe, 0]).is_err());
    }

    #[test]
    fn mix_takes_only_the_chosen_group() {
        let mut dst = StyleSettings::default();
        let src = builtin_styles()
            .into_iter()
            .find(|s| s.name == "Blueprint")
            .unwrap()
            .settings;
        mix_style(&mut dst, &src, 2);
        assert_eq!(dst.background, src.background);
        assert_eq!(dst.face_style, StyleSettings::default().face_style);
        mix_style(&mut dst, &src, 1);
        assert_eq!(dst.face_style, src.face_style);
        assert_ne!(dst.edge_look, src.edge_look);
        mix_style(&mut dst, &src, 0);
        assert_eq!(dst, src);
    }

    #[test]
    fn folders_and_subfolders_with_files_are_collections() {
        let dir = std::env::temp_dir().join(format!("mt_collections_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Wood")).unwrap();
        std::fs::create_dir_all(dir.join("Empty")).unwrap();
        std::fs::write(dir.join("Wood/oak 10.png"), b"x").unwrap();
        std::fs::write(dir.join("Wood/oak 9.jpg"), b"x").unwrap();
        std::fs::write(dir.join("Wood/notes.txt"), b"x").unwrap();
        let style = style_file_text(&builtin_styles()[2]);
        std::fs::write(dir.join("Mine.style"), style).unwrap();
        let mats = scan_folders(std::slice::from_ref(&dir), 0);
        assert_eq!(mats.len(), 1);
        assert_eq!(mats[0].name, "Wood");
        let names: Vec<&str> = mats[0].items.iter().map(LibItem::name).collect();
        assert_eq!(names, ["oak 9", "oak 10"]);
        let styles = scan_folders(&[dir.clone(), dir.clone()], 2);
        assert_eq!(styles.len(), 1, "a folder listed twice is one collection");
        assert_eq!(styles[0].items[0].name(), "Hidden Line");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Color by Tag colors texts and dimensions (labels and lines); a hidden tag
    /// hides them.
    #[test]
    fn annotations_follow_their_tag() {
        let mut doc = crate::document::Document::untitled();
        let m = &mut doc.model;
        m.layers.push(Layer {
            name: "Notes".into(),
            color: [10, 200, 30],
            ..Layer::default()
        });
        let mut t = mt_kernel::Text::leader(DVec3::ZERO, DVec3::new(10.0, 0.0, 0.0), "a".into());
        t.layer = Some(1);
        m.root.texts.push(t);
        let mut d = mt_kernel::Dimension::linear(DVec3::ZERO, DVec3::X * 10.0, DVec3::Y * 5.0);
        d.layer = Some(1);
        m.root.dimensions.push(d);
        let collect = |doc: &crate::document::Document| {
            let mut scene = crate::document::Scene::default();
            crate::annot::collect(doc, &mut scene);
            scene
        };
        // Not on geometry: the dimension would be highlighted.
        doc.model.dimension_expert.highlight_non_associated = false;
        let plain = collect(&doc);
        assert_eq!(plain.annotations.len(), 2);
        assert_eq!(plain.annotations[0].color, [0, 0, 0]);
        doc.model.color_by_tag = true;
        let colored = collect(&doc);
        assert!(colored.annotations.iter().all(|a| a.color == [10, 200, 30]));
        let green = [10.0 / 255.0, 200.0 / 255.0, 30.0 / 255.0, 1.0];
        assert!(colored.edges().iter().all(|e| e.color == green));
        doc.model.layers[1].visible = false;
        let hidden = collect(&doc);
        assert!(hidden.annotations.is_empty() && hidden.edges().is_empty());
    }
}
