//! Plain data the tray panels show ([`TrayData`]) and the pure logic behind them
//! (outliner flattening and filtering, time and date labels).
//!
//! Everything here uses indices, index paths and `[u8; 3]` sRGB colors, never kernel
//! ids, so the app maps them to its model.

use std::collections::HashSet;

/// Everything the tray panels and the scene tabs show. The app builds it from its
/// model and passes it to [`crate::Shell::set_tray_data`] before [`crate::Shell::show`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayData {
    /// Tags in list order. Index 0 is "Untagged": always listed first, cannot be
    /// renamed or deleted.
    pub tags: Vec<TagRow>,
    /// Index into `tags` of the active tag (new geometry goes there; pencil icon).
    pub active_tag: usize,
    /// Materials in the model ("In Model" collection of the Materials panel).
    pub materials: Vec<MaterialRow>,
    /// Index into `materials` of the current Paint Bucket material; `None` is the
    /// default material.
    pub current_material: Option<usize>,
    /// Label of the Outliner root row.
    pub outliner_root: String,
    /// Top-level groups and component instances, in model order.
    pub outliner: Vec<OutlinerNode>,
    /// Scenes in order.
    pub scenes: Vec<String>,
    /// Index into `scenes` of the active (last activated) scene.
    pub active_scene: Option<usize>,
    /// View > Scene Tabs: draw the tab strip above the viewport when there are scenes.
    pub scene_tabs: bool,
    pub style: StyleSettings,
    pub shadows: ShadowSettings,
    pub fog: FogSettings,
    pub soften: SoftenSettings,
    /// Tag of the selected entities, shown as a Tag dropdown in Entity Info; `None`
    /// hides the row (nothing selected).
    pub entity_tag: Option<usize>,
    /// Component definitions in the model ("In Model" collection of the Components
    /// panel), groups excluded.
    pub components: Vec<ComponentRow>,
    /// Index into `components` of the definition the panel shows and edits (last
    /// picked, or the definition of the selected instance); `None` = none.
    pub current_component: Option<usize>,
    /// One face with a textured material is selected: the Materials panel's Position
    /// Texture button (command `tool.texture_position`) is enabled.
    pub can_position_texture: bool,
    /// Tag folders in model order ([`TagRow::folder`] indexes them).
    pub tag_folders: Vec<TagFolderRow>,
    /// Tags panel "Color by tag".
    pub color_by_tag: bool,
    /// Scenes panel details, one per entry of `scenes`.
    pub scene_details: Vec<SceneDetails>,
    /// Styles panel "In Model" style names and the one last applied.
    pub styles: Vec<String>,
    pub active_style: Option<usize>,
    /// The selection holds groups or components (Components > Replace Selected).
    pub can_replace_component: bool,
    /// Styles panel Edit tab extras and watermarks (bottom first).
    pub style_extras: crate::StyleExtras,
    pub watermarks: Vec<crate::WatermarkRow>,
}

impl Default for TrayData {
    fn default() -> Self {
        Self {
            tags: vec![TagRow {
                name: "Untagged".to_owned(),
                visible: true,
                color: [255, 255, 255],
                ..TagRow::default()
            }],
            active_tag: 0,
            materials: Vec::new(),
            current_material: None,
            outliner_root: "Model".to_owned(),
            outliner: Vec::new(),
            scenes: Vec::new(),
            active_scene: None,
            scene_tabs: true,
            style: StyleSettings::default(),
            shadows: ShadowSettings::default(),
            fog: FogSettings::default(),
            soften: SoftenSettings::default(),
            entity_tag: None,
            components: Vec::new(),
            current_component: None,
            can_position_texture: false,
            tag_folders: Vec::new(),
            color_by_tag: false,
            scene_details: Vec::new(),
            styles: Vec::new(),
            active_style: None,
            can_replace_component: false,
            style_extras: Default::default(),
            watermarks: Vec::new(),
        }
    }
}

/// One row of the Tags panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRow {
    pub name: String,
    pub visible: bool,
    /// Tag color (used by "Color by tag").
    pub color: [u8; 3],
    /// The folder it is in (index into [`TrayData::tag_folders`]).
    pub folder: Option<usize>,
    /// Dash pattern, an index into [`DASH_NAMES`].
    pub dashes: usize,
}

impl Default for TagRow {
    fn default() -> Self {
        Self {
            name: String::new(),
            visible: true,
            color: [255, 255, 255],
            folder: None,
            dashes: 0,
        }
    }
}

/// The Tags panel's dash patterns, in the order the app maps them (unverified
/// against SketchUp's list).
pub const DASH_NAMES: [&str; 6] = [
    "Default",
    "Dash",
    "Short Dash",
    "Long Dash",
    "Dot",
    "Dash Dot",
];

/// A tag folder of the Tags panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagFolderRow {
    pub name: String,
    /// The folder it is in.
    pub parent: Option<usize>,
    pub visible: bool,
}

/// A row of the Tags panel's tree: a folder or a tag, indented by `depth`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagTreeItem {
    Folder(usize),
    Tag(usize),
}

/// The Tags panel's rows: "Untagged" first, then the folders (with their subfolders
/// and tags under them, unless collapsed) and the tags outside folders, each in model
/// order. With a filter (case-insensitive substring of the name) only matching tags
/// and the folders leading to them are listed, opened.
pub fn tag_tree_rows(
    tags: &[TagRow],
    folders: &[TagFolderRow],
    collapsed: &HashSet<usize>,
    filter: &str,
) -> Vec<(TagTreeItem, usize)> {
    let filter = filter.trim().to_lowercase();
    let tag_matches = |i: usize| filter.is_empty() || tags[i].name.to_lowercase().contains(&filter);
    // Whether a folder holds a matching tag anywhere below it.
    fn holds(
        f: usize,
        tags: &[TagRow],
        folders: &[TagFolderRow],
        m: &dyn Fn(usize) -> bool,
        depth: usize,
    ) -> bool {
        depth <= folders.len()
            && ((1..tags.len()).any(|t| tags[t].folder == Some(f) && m(t))
                || (0..folders.len())
                    .any(|g| folders[g].parent == Some(f) && holds(g, tags, folders, m, depth + 1)))
    }
    #[allow(clippy::too_many_arguments)]
    fn walk(
        parent: Option<usize>,
        depth: usize,
        tags: &[TagRow],
        folders: &[TagFolderRow],
        collapsed: &HashSet<usize>,
        filtering: bool,
        m: &dyn Fn(usize) -> bool,
        out: &mut Vec<(TagTreeItem, usize)>,
    ) {
        if depth > folders.len() {
            return;
        }
        for f in 0..folders.len() {
            let parent_ok = match parent {
                Some(p) => folders[f].parent == Some(p),
                // Folders whose parent is missing are listed at the top.
                None => folders[f].parent.is_none_or(|p| p >= folders.len()),
            };
            if !parent_ok || (filtering && !holds(f, tags, folders, m, 0)) {
                continue;
            }
            out.push((TagTreeItem::Folder(f), depth));
            if filtering || !collapsed.contains(&f) {
                walk(
                    Some(f),
                    depth + 1,
                    tags,
                    folders,
                    collapsed,
                    filtering,
                    m,
                    out,
                );
            }
        }
        for (t, tag) in tags.iter().enumerate().skip(1) {
            let here = match parent {
                Some(p) => tag.folder == Some(p),
                None => tag.folder.is_none_or(|p| p >= folders.len()),
            };
            if here && m(t) {
                out.push((TagTreeItem::Tag(t), depth));
            }
        }
    }
    let mut out = Vec::new();
    if !tags.is_empty() && tag_matches(0) {
        out.push((TagTreeItem::Tag(0), 0));
    }
    walk(
        None,
        0,
        tags,
        folders,
        collapsed,
        !filter.is_empty(),
        &tag_matches,
        &mut out,
    );
    out
}

/// Names of the scene properties a scene can save, in [`SceneDetails::props`] order.
pub const SCENE_PROPS: [&str; 7] = [
    "Camera Location",
    "Hidden Geometry",
    "Visible Tags",
    "Active Section Planes",
    "Style and Fog",
    "Shadow Settings",
    "Axes Location",
];

/// A scene's details in the Scenes panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneDetails {
    pub description: String,
    pub include_in_animation: bool,
    /// Properties to save, in [`SCENE_PROPS`] order.
    pub props: [bool; 7],
}

impl Default for SceneDetails {
    fn default() -> Self {
        Self {
            description: String::new(),
            include_in_animation: true,
            props: [true; 7],
        }
    }
}

/// Component behaviors of the Components panel's Edit tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ComponentBehavior {
    /// "Glue to", an index into [`GLUE_NAMES`].
    pub glue: usize,
    pub cut_opening: bool,
    pub always_face_camera: bool,
    pub shadows_face_sun: bool,
}

/// The "Glue to" choices.
pub const GLUE_NAMES: [&str; 5] = ["None", "Any", "Horizontal", "Vertical", "Sloped"];

/// One material of the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialRow {
    pub name: String,
    pub color: [u8; 3],
    /// Opacity in percent, 0..=100 (SketchUp's Opacity slider works in whole percent).
    pub opacity: u8,
    /// The texture image, `None` for a plain color.
    pub texture: Option<TextureRow>,
}

/// A material's texture as the Materials panel's Edit tab shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureRow {
    /// Image file name (as stored in the model).
    pub file: String,
    /// Tile width and height in model units, formatted by the app.
    pub width: String,
    pub height: String,
    /// Colorize: the image is tinted towards the material color.
    pub colorize: bool,
}

/// A texture control of the Materials panel's Edit tab (see
/// [`crate::Event::MaterialTexture`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextureAction {
    /// "Use texture image" checked, or the Browse button: pick an image file to load
    /// (or replace the current one).
    Load,
    /// "Use texture image" unchecked: back to a plain color.
    Remove,
    /// Width or height typed (model units, parsed by the app). With `lock` the other
    /// size follows to keep the image's aspect ratio.
    Size {
        width: Option<String>,
        height: Option<String>,
        lock: bool,
    },
    /// The Colorize check box.
    Colorize(bool),
    /// Reset Color: back to the image's own colors.
    ResetColor,
    /// Edit the image in the system's default image editor; the app reloads it when
    /// the file changes.
    EditExternally,
}

/// HSL of an sRGB color: hue in degrees 0..360, saturation and lightness in percent
/// 0..=100 (the HSL picker's scales; SketchUp's own scales are unverified).
pub fn rgb_to_hsl(c: [u8; 3]) -> [f64; 3] {
    let [r, g, b] = c.map(|x| f64::from(x) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d < 1e-12 {
        return [0.0, 0.0, l * 100.0];
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } * 60.0;
    [h, s * 100.0, l * 100.0]
}

/// sRGB of a hue (degrees, any value), saturation and lightness (percent, clamped).
pub fn hsl_to_rgb(hsl: [f64; 3]) -> [u8; 3] {
    let h = hsl[0].rem_euclid(360.0) / 60.0;
    let s = (hsl[1] / 100.0).clamp(0.0, 1.0);
    let l = (hsl[2] / 100.0).clamp(0.0, 1.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r, g, b].map(|v| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8)
}

/// `#RRGGBB` (upper case) of an sRGB color.
pub fn hex_color(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

/// Parses `#RRGGBB`, `RRGGBB`, `#RGB` or `RGB` (any case, surrounding spaces
/// ignored).
pub fn parse_hex_color(text: &str) -> Option<[u8; 3]> {
    let t = text.trim();
    let t = t.strip_prefix('#').unwrap_or(t);
    if !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |i: usize, n: usize| u8::from_str_radix(&t[i..i + n], 16).ok();
    match t.len() {
        6 => Some([digit(0, 2)?, digit(2, 2)?, digit(4, 2)?]),
        3 => Some([digit(0, 1)? * 17, digit(1, 1)? * 17, digit(2, 1)? * 17]),
        _ => None,
    }
}

/// One component definition of the model.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ComponentRow {
    pub name: String,
    pub description: String,
    /// Number of instances placed in the model (0 = unused; Purge Unused removes it).
    pub instances: usize,
    pub behavior: ComponentBehavior,
}

/// "1 instance" / "3 instances": the instance count of a component definition.
pub fn instances_label(count: usize) -> String {
    match count {
        1 => "1 instance".to_owned(),
        n => format!("{n} instances"),
    }
}

/// What committing the Components panel's Edit fields changes, if anything: the
/// new `(name, description)`. The name is trimmed and an empty name keeps the old
/// one (SketchUp refuses empty definition names as far as remembered, unverified);
/// the description is kept as typed except for trailing whitespace.
pub fn component_edit(
    row: &ComponentRow,
    name: &str,
    description: &str,
) -> Option<(String, String)> {
    let name = match name.trim() {
        "" => row.name.clone(),
        n => n.to_owned(),
    };
    let description = description.trim_end().to_owned();
    (name != row.name || description != row.description).then_some((name, description))
}

/// Kind of an Outliner entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutlinerKind {
    Group,
    /// Component instance; `definition` is the component definition name.
    Component {
        definition: String,
    },
}

/// A group or component instance in the Outliner tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlinerNode {
    /// Instance name; may be empty.
    pub name: String,
    pub kind: OutlinerKind,
    /// Hidden (or on a hidden tag): drawn greyed.
    pub hidden: bool,
    /// Part of the current selection: drawn highlighted.
    pub selected: bool,
    /// Nested groups and components, in model order.
    pub children: Vec<OutlinerNode>,
    /// Its own Hidden flag (the eye; `hidden` also counts hidden tags).
    pub instance_hidden: bool,
    /// Locked (lock icon).
    pub locked: bool,
    /// Open for editing, or containing the open group or component.
    pub open: bool,
}

impl Default for OutlinerNode {
    fn default() -> Self {
        Self {
            name: String::new(),
            kind: OutlinerKind::Group,
            hidden: false,
            selected: false,
            children: Vec::new(),
            instance_hidden: false,
            locked: false,
            open: false,
        }
    }
}

/// Whether the Outliner may move the node at `from` into the node at `to` (empty: the
/// model root): not into itself or its own subtree, and not where it already is.
pub fn outliner_can_move(from: &[usize], to: &[usize]) -> bool {
    !from.is_empty() && !to.starts_with(from) && from[..from.len() - 1] != *to
}

impl OutlinerNode {
    /// Text of the row as SketchUp's Outliner shows it: a group shows its name or
    /// "Group"; a component shows `<Definition>`, prefixed by the instance name if
    /// it has one (`Left door <Door>`).
    pub fn label(&self) -> String {
        match &self.kind {
            OutlinerKind::Group if self.name.is_empty() => "Group".to_owned(),
            OutlinerKind::Group => self.name.clone(),
            OutlinerKind::Component { definition } if self.name.is_empty() => {
                format!("<{definition}>")
            }
            OutlinerKind::Component { definition } => format!("{} <{definition}>", self.name),
        }
    }
}

/// How a click changes the selection (Outliner).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectMode {
    /// Plain click: select only this.
    Replace,
    /// Ctrl+click: add to the selection.
    Add,
    /// Shift+click: toggle in the selection.
    Toggle,
    /// Ctrl+Shift+click: remove from the selection.
    Remove,
}

impl SelectMode {
    /// Mode for a click with these modifier keys. Follows the Select tool's keys;
    /// whether SketchUp's Outliner uses the same keys is unverified.
    pub fn from_modifiers(ctrl: bool, shift: bool) -> Self {
        match (ctrl, shift) {
            (false, false) => Self::Replace,
            (true, false) => Self::Add,
            (false, true) => Self::Toggle,
            (true, true) => Self::Remove,
        }
    }
}

/// Face style buttons of Styles > Edit > Face Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FaceStyle {
    Wireframe,
    HiddenLine,
    Shaded,
    #[default]
    ShadedWithTextures,
    /// "Shaded using same as all" / Monochrome.
    Monochrome,
}

impl FaceStyle {
    pub const ALL: [FaceStyle; 5] = [
        Self::Wireframe,
        Self::HiddenLine,
        Self::Shaded,
        Self::ShadedWithTextures,
        Self::Monochrome,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Wireframe => "Wireframe",
            Self::HiddenLine => "Hidden Line",
            Self::Shaded => "Shaded",
            Self::ShadedWithTextures => "Shaded With Textures",
            Self::Monochrome => "Monochrome",
        }
    }
}

/// The style settings the Styles panel edits. Defaults are believed to match the
/// default template's style but are unverified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleSettings {
    /// Edge Settings > Edges.
    pub edges: bool,
    /// Edge Settings > Profiles.
    pub profiles: bool,
    /// Profile width in pixels.
    pub profile_width: u8,
    /// Edge Settings > Back Edges.
    pub back_edges: bool,
    /// Edge Settings > Depth Cue and its width in pixels.
    pub depth_cue: bool,
    pub depth_cue_width: u8,
    /// Edge Settings > Extension and its length in pixels.
    pub extension: bool,
    pub extension_length: u8,
    /// Modeling Settings > Section Cut Width, pixels.
    pub section_cut_width: u8,
    /// Modeling Settings > Section Fill and its color (sRGB).
    pub section_fill: bool,
    pub section_fill_color: [u8; 3],
    pub face_style: FaceStyle,
    /// Face Settings > X-ray toggle; combines with `face_style`.
    pub x_ray: bool,
}

impl Default for StyleSettings {
    fn default() -> Self {
        Self {
            edges: true,
            profiles: true,
            profile_width: 3,
            back_edges: false,
            depth_cue: false,
            depth_cue_width: 4,
            extension: false,
            extension_length: 2,
            section_cut_width: 3,
            section_fill: true,
            section_fill_color: [64, 64, 64],
            face_style: FaceStyle::default(),
            x_ray: false,
        }
    }
}

/// Settings of the Soften Edges panel. The defaults (20 degrees, Smooth normals on,
/// Soften coplanar off) are from memory, unverified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoftenSettings {
    /// "Angle between normals" in degrees, 0..=180: edges between faces meeting at a
    /// smaller angle are softened.
    pub angle: u8,
    /// Smooth normals: softened edges are smoothed too.
    pub smooth: bool,
    /// Soften coplanar: edges between coplanar faces are softened.
    pub coplanar: bool,
}

impl Default for SoftenSettings {
    fn default() -> Self {
        Self {
            angle: 20,
            smooth: true,
            coplanar: false,
        }
    }
}

/// Settings of the Fog panel. Distances are percentages of the model's depth seen
/// from the camera (0 its nearest point, 100 its farthest); SketchUp's scale and the
/// defaults are unverified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FogSettings {
    /// Display Fog.
    pub enabled: bool,
    /// Distance: where the fog starts and where it is complete, 0..=100.
    pub start: u8,
    pub end: u8,
    /// Use Background Color; otherwise `color`.
    pub use_background: bool,
    pub color: [u8; 3],
}

impl Default for FogSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            start: 0,
            end: 100,
            use_background: true,
            color: [255, 255, 255],
        }
    }
}

/// Settings of the Shadows panel. Defaults (13:30 on 8 November, light 80, dark 45)
/// are from memory, unverified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowSettings {
    pub enabled: bool,
    /// Local time, minutes after midnight (0..1440).
    pub minute_of_day: u16,
    /// Day of a non-leap year, 1..=365.
    pub day_of_year: u16,
    /// Light slider, 0..=100.
    pub light: u8,
    /// Dark slider, 0..=100.
    pub dark: u8,
    /// Display: On faces.
    pub on_faces: bool,
    /// Display: On ground.
    pub on_ground: bool,
    /// UTC offset in minutes (east positive).
    pub utc_offset_minutes: i16,
    /// "Use sun for shading" (lit by the sun with shadows off).
    pub use_sun_for_shading: bool,
    /// Display: From edges.
    pub from_edges: bool,
}

impl Default for ShadowSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            minute_of_day: 13 * 60 + 30,
            day_of_year: day_of_year(11, 8),
            light: 80,
            dark: 45,
            on_faces: true,
            on_ground: true,
            utc_offset_minutes: -7 * 60,
            use_sun_for_shading: false,
            from_edges: false,
        }
    }
}

/// "UTC-07:00" style label of a UTC offset in minutes.
pub fn utc_label(minutes: i16) -> String {
    let sign = if minutes < 0 { '-' } else { '+' };
    let m = minutes.unsigned_abs();
    format!("UTC{sign}{:02}:{:02}", m / 60, m % 60)
}

const MONTH_DAYS: [u16; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// Day of a non-leap year (1..=365) for `month` 1..=12 and `day` 1..=31.
pub fn day_of_year(month: u16, day: u16) -> u16 {
    MONTH_DAYS[..(month.clamp(1, 12) - 1) as usize]
        .iter()
        .sum::<u16>()
        + day
}

/// Month and day (both 1-based) of `day_of_year` in a non-leap year; clamped to 1..=365.
pub fn month_day(day_of_year: u16) -> (u16, u16) {
    let mut d = day_of_year.clamp(1, 365);
    for (m, &len) in MONTH_DAYS.iter().enumerate() {
        if d <= len {
            return (m as u16 + 1, d);
        }
        d -= len;
    }
    (12, 31)
}

/// "01:30 PM" style label of a time of day, as SketchUp's Shadows panel shows it
/// (12-hour clock as on US-English Windows; other locales unverified).
pub fn time_label(minute_of_day: u16) -> String {
    let m = minute_of_day.min(24 * 60 - 1);
    let (h, min) = (m / 60, m % 60);
    let (h12, ampm) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    format!("{h12:02}:{min:02} {ampm}")
}

/// "11/08" style label (MM/DD) of a day of the year.
pub fn date_label(day_of_year: u16) -> String {
    let (m, d) = month_day(day_of_year);
    format!("{m:02}/{d:02}")
}

/// One visible row of the Outliner below the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlinerRow {
    /// Child indices from the root down to this node.
    pub path: Vec<usize>,
    /// 0 for top-level nodes.
    pub depth: usize,
    pub has_children: bool,
    /// Whether the children are listed (expanded, or forced open by a filter match).
    pub open: bool,
}

/// Flattens the Outliner tree into the rows to draw. Without a filter, children are
/// listed only below nodes whose path is in `expanded`. With a filter (case-insensitive
/// substring of [`OutlinerNode::label`]), a node is listed if it or any descendant
/// matches; nodes with matching descendants are shown open.
pub fn outliner_rows(
    nodes: &[OutlinerNode],
    expanded: &HashSet<Vec<usize>>,
    filter: &str,
) -> Vec<OutlinerRow> {
    let filter = filter.trim().to_lowercase();
    let mut rows = Vec::new();
    let mut path = Vec::new();
    walk(nodes, expanded, &filter, &mut path, &mut rows);
    rows
}

fn matches(node: &OutlinerNode, filter: &str) -> bool {
    node.label().to_lowercase().contains(filter)
}

fn subtree_matches(node: &OutlinerNode, filter: &str) -> bool {
    matches(node, filter) || node.children.iter().any(|c| subtree_matches(c, filter))
}

fn walk(
    nodes: &[OutlinerNode],
    expanded: &HashSet<Vec<usize>>,
    filter: &str,
    path: &mut Vec<usize>,
    rows: &mut Vec<OutlinerRow>,
) {
    for (i, node) in nodes.iter().enumerate() {
        if !filter.is_empty() && !subtree_matches(node, filter) {
            continue;
        }
        path.push(i);
        let has_children = !node.children.is_empty();
        let open = has_children
            && if filter.is_empty() {
                expanded.contains(path)
            } else {
                node.children.iter().any(|c| subtree_matches(c, filter)) || expanded.contains(path)
            };
        rows.push(OutlinerRow {
            path: path.clone(),
            depth: path.len() - 1,
            has_children,
            open,
        });
        if open {
            walk(&node.children, expanded, filter, path, rows);
        }
        path.pop();
    }
}

/// The node at `path`, if any.
pub fn outliner_node<'a>(nodes: &'a [OutlinerNode], path: &[usize]) -> Option<&'a OutlinerNode> {
    let (&first, rest) = path.split_first()?;
    let node = nodes.get(first)?;
    if rest.is_empty() {
        Some(node)
    } else {
        outliner_node(&node.children, rest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(name: &str, children: Vec<OutlinerNode>) -> OutlinerNode {
        OutlinerNode {
            name: name.to_owned(),
            kind: OutlinerKind::Group,
            children,
            ..OutlinerNode::default()
        }
    }

    fn comp(name: &str, def: &str) -> OutlinerNode {
        OutlinerNode {
            name: name.to_owned(),
            kind: OutlinerKind::Component {
                definition: def.to_owned(),
            },
            ..OutlinerNode::default()
        }
    }

    fn tree() -> Vec<OutlinerNode> {
        vec![
            group(
                "House",
                vec![comp("Front", "Door"), comp("", "Window"), group("", vec![])],
            ),
            comp("", "Chair"),
        ]
    }

    fn paths(rows: &[OutlinerRow]) -> Vec<Vec<usize>> {
        rows.iter().map(|r| r.path.clone()).collect()
    }

    #[test]
    fn labels() {
        assert_eq!(comp("Front", "Door").label(), "Front <Door>");
        assert_eq!(comp("", "Door").label(), "<Door>");
        assert_eq!(group("", vec![]).label(), "Group");
        assert_eq!(group("House", vec![]).label(), "House");
    }

    #[test]
    fn collapsed_tree_lists_top_level_only() {
        let rows = outliner_rows(&tree(), &HashSet::new(), "");
        assert_eq!(paths(&rows), [vec![0], vec![1]]);
        assert!(rows[0].has_children && !rows[0].open);
        assert!(!rows[1].has_children);
    }

    #[test]
    fn expanded_node_lists_children() {
        let expanded = HashSet::from([vec![0]]);
        let rows = outliner_rows(&tree(), &expanded, "");
        assert_eq!(
            paths(&rows),
            [vec![0], vec![0, 0], vec![0, 1], vec![0, 2], vec![1]]
        );
        assert_eq!(rows[1].depth, 1);
    }

    #[test]
    fn filter_keeps_ancestors_and_opens_them() {
        let rows = outliner_rows(&tree(), &HashSet::new(), "  DOOR ");
        assert_eq!(paths(&rows), [vec![0], vec![0, 0]]);
        assert!(rows[0].open);
        let rows = outliner_rows(&tree(), &HashSet::new(), "chair");
        assert_eq!(paths(&rows), [vec![1]]);
        assert!(outliner_rows(&tree(), &HashSet::new(), "nothing").is_empty());
    }

    #[test]
    fn filter_match_on_parent_lists_it_closed() {
        let rows = outliner_rows(&tree(), &HashSet::new(), "house");
        assert_eq!(paths(&rows), [vec![0]]);
        assert!(!rows[0].open);
    }

    #[test]
    fn node_lookup() {
        let t = tree();
        assert_eq!(outliner_node(&t, &[0, 1]).unwrap().label(), "<Window>");
        assert!(outliner_node(&t, &[0, 5]).is_none());
        assert!(outliner_node(&t, &[]).is_none());
    }

    #[test]
    fn dates_and_times() {
        assert_eq!(day_of_year(1, 1), 1);
        assert_eq!(day_of_year(11, 8), 312);
        assert_eq!(month_day(312), (11, 8));
        assert_eq!(month_day(365), (12, 31));
        assert_eq!(month_day(0), (1, 1));
        for d in 1..=365 {
            let (m, dd) = month_day(d);
            assert_eq!(day_of_year(m, dd), d);
        }
        assert_eq!(date_label(312), "11/08");
        assert_eq!(time_label(13 * 60 + 30), "01:30 PM");
        assert_eq!(time_label(0), "12:00 AM");
        assert_eq!(time_label(12 * 60 + 5), "12:05 PM");
        assert_eq!(time_label(9 * 60), "09:00 AM");
    }

    #[test]
    fn component_labels_and_edits() {
        assert_eq!(instances_label(0), "0 instances");
        assert_eq!(instances_label(1), "1 instance");
        assert_eq!(instances_label(12), "12 instances");
        let row = ComponentRow {
            name: "Door".to_owned(),
            description: "Single door".to_owned(),
            instances: 2,
            ..ComponentRow::default()
        };
        assert_eq!(component_edit(&row, " Door ", "Single door "), None);
        assert_eq!(component_edit(&row, "  ", "Single door"), None);
        assert_eq!(
            component_edit(&row, " Front door", "Single door"),
            Some(("Front door".to_owned(), "Single door".to_owned()))
        );
        assert_eq!(
            component_edit(&row, "", ""),
            Some(("Door".to_owned(), String::new()))
        );
    }

    #[test]
    fn color_conversions() {
        assert_eq!(rgb_to_hsl([255, 0, 0]), [0.0, 100.0, 50.0]);
        assert_eq!(rgb_to_hsl([0, 0, 0]), [0.0, 0.0, 0.0]);
        assert_eq!(rgb_to_hsl([255, 255, 255]), [0.0, 0.0, 100.0]);
        let [h, s, l] = rgb_to_hsl([0, 128, 255]);
        assert!((h - 209.88).abs() < 0.01, "{h}");
        assert!((s - 100.0).abs() < 1e-9 && (l - 50.0).abs() < 1e-9);
        assert_eq!(hsl_to_rgb([120.0, 100.0, 25.0]), [0, 128, 0]);
        assert_eq!(hsl_to_rgb([480.0, 100.0, 25.0]), [0, 128, 0], "hue wraps");
        // Every color survives a round trip through HSL.
        for r in (0..=255).step_by(15) {
            for g in (0..=255).step_by(17) {
                for b in (0..=255).step_by(51) {
                    let c = [r as u8, g as u8, b as u8];
                    assert_eq!(hsl_to_rgb(rgb_to_hsl(c)), c, "{c:?}");
                }
            }
        }
        assert_eq!(hex_color([10, 171, 255]), "#0AABFF");
        assert_eq!(parse_hex_color(" #0aabff "), Some([10, 171, 255]));
        assert_eq!(parse_hex_color("0AABFF"), Some([10, 171, 255]));
        assert_eq!(parse_hex_color("#f0a"), Some([255, 0, 170]));
        assert_eq!(parse_hex_color("#12345"), None);
        assert_eq!(parse_hex_color("#12345g"), None);
        assert_eq!(parse_hex_color("+1+2+3"), None);
    }

    fn tag(name: &str, folder: Option<usize>) -> TagRow {
        TagRow {
            name: name.into(),
            folder,
            ..TagRow::default()
        }
    }

    fn folder(name: &str, parent: Option<usize>) -> TagFolderRow {
        TagFolderRow {
            name: name.into(),
            parent,
            visible: true,
        }
    }

    #[test]
    fn tag_tree_lists_folders_then_loose_tags() {
        use TagTreeItem::{Folder, Tag};
        let tags = vec![
            tag("Untagged", None),
            tag("Walls", Some(0)),
            tag("Roof", None),
            tag("Doors", Some(1)),
        ];
        let folders = vec![folder("Building", None), folder("Openings", Some(0))];
        let rows = tag_tree_rows(&tags, &folders, &HashSet::new(), "");
        assert_eq!(
            rows,
            [
                (Tag(0), 0),
                (Folder(0), 0),
                (Folder(1), 1),
                (Tag(3), 2),
                (Tag(1), 1),
                (Tag(2), 0)
            ]
        );
        let collapsed = HashSet::from([0]);
        let rows = tag_tree_rows(&tags, &folders, &collapsed, "");
        assert_eq!(rows, [(Tag(0), 0), (Folder(0), 0), (Tag(2), 0)]);
        // A filter opens the folders leading to matches and drops the rest.
        let rows = tag_tree_rows(&tags, &folders, &collapsed, " door");
        assert_eq!(rows, [(Folder(0), 0), (Folder(1), 1), (Tag(3), 2)]);
    }

    #[test]
    fn outliner_moves_refuse_loops() {
        assert!(outliner_can_move(&[0, 1], &[]));
        assert!(outliner_can_move(&[0, 1], &[2]));
        assert!(!outliner_can_move(&[0], &[]), "already at the root");
        assert!(!outliner_can_move(&[0], &[0, 3]), "into its own child");
        assert!(!outliner_can_move(&[0], &[0]), "into itself");
        assert!(!outliner_can_move(&[0, 1], &[0]), "already there");
        assert_eq!(utc_label(-420), "UTC-07:00");
        assert_eq!(utc_label(330), "UTC+05:30");
    }

    #[test]
    fn select_modes() {
        assert_eq!(
            SelectMode::from_modifiers(false, false),
            SelectMode::Replace
        );
        assert_eq!(SelectMode::from_modifiers(true, false), SelectMode::Add);
        assert_eq!(SelectMode::from_modifiers(false, true), SelectMode::Toggle);
        assert_eq!(SelectMode::from_modifiers(true, true), SelectMode::Remove);
    }
}
