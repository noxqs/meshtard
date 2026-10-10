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
}

impl Default for TrayData {
    fn default() -> Self {
        Self {
            tags: vec![TagRow {
                name: "Untagged".to_owned(),
                visible: true,
                color: [255, 255, 255],
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
}

/// One material of the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialRow {
    pub name: String,
    pub color: [u8; 3],
    /// Opacity in percent, 0..=100 (SketchUp's Opacity slider works in whole percent).
    pub opacity: u8,
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
    /// Modeling Settings > Section Cut Width, pixels.
    pub section_cut_width: u8,
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
            section_cut_width: 3,
            face_style: FaceStyle::default(),
            x_ray: false,
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
        }
    }
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
            hidden: false,
            selected: false,
            children,
        }
    }

    fn comp(name: &str, def: &str) -> OutlinerNode {
        OutlinerNode {
            name: name.to_owned(),
            kind: OutlinerKind::Component {
                definition: def.to_owned(),
            },
            hidden: false,
            selected: false,
            children: Vec::new(),
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
