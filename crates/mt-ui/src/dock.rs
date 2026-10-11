//! Toolbar docking: where each toolbar is (a row on one of the four window edges, or
//! floating), where a dragged toolbar would land, and how toolbars pack into rows.
//! Pure logic and plain data; drawing and input are in `toolbar.rs`.
//!
//! Model (after SketchUp 2026 on Windows, whose toolbars behave like classic Windows
//! dock bars; details marked "unverified" were not checked against the real app):
//!
//! * Each edge (top, bottom, left, right) holds rows of toolbars. Row 0 is nearest the
//!   window edge; on the left and right edges a "row" is a column. Toolbars in a row
//!   are packed one after the other in `index` order. Unverified: SketchUp keeps a
//!   free offset between toolbars in a row; here they are always packed.
//! * A row that does not fit the window wraps onto further lines (unverified; Windows
//!   dock bars may clip or squeeze instead).
//! * A floating toolbar is a small window with a title bar and close button at a
//!   position inside the main window.
//! * Hidden toolbars keep their place, so showing them again restores it.

use egui::{Pos2, Rect, Vec2, pos2, vec2};
use mt_spec::{Dock, Toolbar, ToolbarItem, UiSizes};
use serde::{Deserialize, Serialize};

/// Width of the drag grip at the start of a docked toolbar (height when vertical).
pub(crate) const GRIP: f32 = 7.0;
/// Space after each toolbar in a top or bottom row.
pub(crate) const ROW_GAP: f32 = 6.0;
/// Height of a floating toolbar's title bar.
pub(crate) const TITLE: f32 = 18.0;
/// How far beyond the docked rows (or the bare window edge) a drop still docks.
pub(crate) const SNAP: f32 = 24.0;

/// A window edge toolbars dock to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockEdge {
    Top,
    Bottom,
    Left,
    Right,
}

impl DockEdge {
    pub const ALL: [DockEdge; 4] = [Self::Top, Self::Bottom, Self::Left, Self::Right];

    /// Left and right: toolbars stand upright and rows are columns.
    pub fn is_vertical(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }

    /// Distance of `p` from this edge of `area`, positive inwards.
    fn depth_of(self, area: Rect, p: Pos2) -> f32 {
        match self {
            Self::Top => p.y - area.top(),
            Self::Bottom => area.bottom() - p.y,
            Self::Left => p.x - area.left(),
            Self::Right => area.right() - p.x,
        }
    }

    /// Coordinate of `p` along the rows of this edge.
    fn main(self, p: Pos2) -> f32 {
        if self.is_vertical() { p.y } else { p.x }
    }

    fn main_range(self, r: Rect) -> egui::Rangef {
        if self.is_vertical() {
            r.y_range()
        } else {
            r.x_range()
        }
    }

    fn cross_range(self, r: Rect) -> egui::Rangef {
        if self.is_vertical() {
            r.x_range()
        } else {
            r.y_range()
        }
    }
}

impl From<Dock> for DockEdge {
    fn from(d: Dock) -> Self {
        match d {
            Dock::Top => Self::Top,
            Dock::Bottom => Self::Bottom,
            Dock::Left => Self::Left,
            Dock::Right => Self::Right,
        }
    }
}

/// Where a toolbar is.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ToolbarPosition {
    /// In row `row` of `edge` (0 = nearest the window edge), `index`-th in the row.
    Docked {
        edge: DockEdge,
        row: u32,
        index: u32,
    },
    /// A floating toolbar window; top-left corner in egui points.
    Floating { x: f32, y: f32 },
}

/// One toolbar's place and visibility.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolbarPlace {
    /// Toolbar id from `spec/toolbars.toml`.
    pub id: String,
    pub visible: bool,
    pub position: ToolbarPosition,
}

/// Where all toolbars are: what the app saves and restores
/// ([`crate::Shell::toolbar_layout`], [`crate::Shell::set_toolbar_layout`]).
/// Entries are in `spec/toolbars.toml` order.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ToolbarLayout {
    pub toolbars: Vec<ToolbarPlace>,
}

/// Where a dragged toolbar lands.
#[derive(Debug, Clone, PartialEq)]
pub enum DropTarget {
    /// Into existing row `row` of `edge`, before toolbar `before` (`None`: at the end).
    Row {
        edge: DockEdge,
        row: u32,
        before: Option<String>,
    },
    /// Into a new row inserted at `row` of `edge`; rows from `row` on move one step
    /// further from the window edge.
    NewRow { edge: DockEdge, row: u32 },
    /// A floating toolbar window with its top-left corner at `pos`.
    Float { pos: Pos2 },
}

/// One drawn line of a dock: a row, or part of a wrapped row, with the screen rects
/// of its toolbars. Recorded while drawing, used to find drop targets.
#[derive(Debug, Clone, PartialEq)]
pub struct DockLine {
    pub edge: DockEdge,
    pub row: u32,
    /// The whole line: across the dock along the row, its own thickness across it.
    pub rect: Rect,
    /// Toolbar ids and rects in drawing order.
    pub items: Vec<(String, Rect)>,
}

impl ToolbarLayout {
    /// The default workspace from `spec/toolbars.toml`.
    pub fn from_spec(toolbars: &[Toolbar]) -> Self {
        let mut out = Self::default();
        for t in toolbars {
            let edge = DockEdge::from(t.dock);
            let index = out
                .toolbars
                .iter()
                .filter(|p| {
                    matches!(p.position, ToolbarPosition::Docked { edge: e, row, .. }
                        if e == edge && row == t.row)
                })
                .count() as u32;
            out.toolbars.push(ToolbarPlace {
                id: t.id.clone(),
                visible: t.visible,
                position: ToolbarPosition::Docked {
                    edge,
                    row: t.row,
                    index,
                },
            });
        }
        out.normalize();
        out
    }

    /// `saved` applied over `defaults`: toolbars `saved` doesn't know keep their
    /// default place, ids `defaults` doesn't know are dropped. Order is `defaults`'.
    pub fn merged(defaults: &Self, saved: &Self) -> Self {
        let mut out = defaults.clone();
        for place in &mut out.toolbars {
            if let Some(s) = saved.get(&place.id) {
                place.visible = s.visible;
                place.position = s.position;
            }
        }
        out.normalize();
        out
    }

    pub fn get(&self, id: &str) -> Option<&ToolbarPlace> {
        self.toolbars.iter().find(|p| p.id == id)
    }

    fn get_mut(&mut self, id: &str) -> Option<&mut ToolbarPlace> {
        self.toolbars.iter_mut().find(|p| p.id == id)
    }

    /// Shows or hides toolbar `id`; false if there is no such toolbar.
    pub fn set_visible(&mut self, id: &str, visible: bool) -> bool {
        match self.get_mut(id) {
            Some(p) => {
                p.visible = visible;
                true
            }
            None => false,
        }
    }

    /// Renumbers rows on each edge to 0, 1, 2, ... (dropping empty rows) and
    /// toolbars in each row to 0, 1, 2, ..., keeping their order. Hidden toolbars
    /// count, so they keep their row.
    pub fn normalize(&mut self) {
        for edge in DockEdge::ALL {
            let mut rows: Vec<u32> = self
                .toolbars
                .iter()
                .filter_map(|p| match p.position {
                    ToolbarPosition::Docked { edge: e, row, .. } if e == edge => Some(row),
                    _ => None,
                })
                .collect();
            rows.sort_unstable();
            rows.dedup();
            for (new_row, &old_row) in rows.iter().enumerate() {
                let mut members: Vec<(u32, usize)> = self
                    .toolbars
                    .iter()
                    .enumerate()
                    .filter_map(|(i, p)| match p.position {
                        ToolbarPosition::Docked {
                            edge: e,
                            row,
                            index,
                        } if e == edge && row == old_row => Some((index, i)),
                        _ => None,
                    })
                    .collect();
                members.sort_unstable();
                for (new_index, &(_, i)) in members.iter().enumerate() {
                    self.toolbars[i].position = ToolbarPosition::Docked {
                        edge,
                        row: new_row as u32,
                        index: new_index as u32,
                    };
                }
            }
        }
    }

    /// Toolbar ids on `edge` grouped by row, rows nearest the window edge first,
    /// each row in order. With `visible_only`, hidden toolbars and rows left empty
    /// are skipped.
    pub fn rows(&self, edge: DockEdge, visible_only: bool) -> Vec<(u32, Vec<&str>)> {
        let mut docked: Vec<(u32, u32, &str)> = self
            .toolbars
            .iter()
            .filter(|p| p.visible || !visible_only)
            .filter_map(|p| match p.position {
                ToolbarPosition::Docked {
                    edge: e,
                    row,
                    index,
                } if e == edge => Some((row, index, p.id.as_str())),
                _ => None,
            })
            .collect();
        docked.sort_unstable();
        let mut rows: Vec<(u32, Vec<&str>)> = Vec::new();
        for (row, _, id) in docked {
            match rows.last_mut() {
                Some((r, ids)) if *r == row => ids.push(id),
                _ => rows.push((row, vec![id])),
            }
        }
        rows
    }

    /// Ids of visible floating toolbars with their positions.
    pub fn floating(&self) -> Vec<(&str, Pos2)> {
        self.toolbars
            .iter()
            .filter(|p| p.visible)
            .filter_map(|p| match p.position {
                ToolbarPosition::Floating { x, y } => Some((p.id.as_str(), pos2(x, y))),
                ToolbarPosition::Docked { .. } => None,
            })
            .collect()
    }

    /// Moves toolbar `id` to `target`. Returns false if there is no such toolbar.
    pub fn apply_drop(&mut self, id: &str, target: &DropTarget) -> bool {
        if self.get(id).is_none() {
            return false;
        }
        match target {
            DropTarget::Float { pos } => {
                self.get_mut(id).expect("checked").position =
                    ToolbarPosition::Floating { x: pos.x, y: pos.y };
            }
            DropTarget::Row { edge, row, before } => {
                // Room before `before`; its index and all after it move up by one.
                let index = match before.as_deref().filter(|b| *b != id) {
                    Some(b) => match self.get(b).map(|p| p.position) {
                        Some(ToolbarPosition::Docked {
                            edge: e,
                            row: r,
                            index,
                        }) if e == *edge && r == *row => Some(index),
                        _ => None,
                    },
                    None => None,
                };
                let index = match index {
                    Some(index) => {
                        self.shift(*edge, id, |r, i| r == *row && i >= index, false);
                        index
                    }
                    None => u32::MAX,
                };
                self.get_mut(id).expect("checked").position = ToolbarPosition::Docked {
                    edge: *edge,
                    row: *row,
                    index,
                };
            }
            DropTarget::NewRow { edge, row } => {
                self.shift(*edge, id, |r, _| r >= *row, true);
                self.get_mut(id).expect("checked").position = ToolbarPosition::Docked {
                    edge: *edge,
                    row: *row,
                    index: 0,
                };
            }
        }
        self.normalize();
        true
    }

    /// Adds one to the row (`rows`) or index (not `rows`) of toolbars on `edge`
    /// other than `except` matching `select(row, index)`.
    fn shift(
        &mut self,
        edge: DockEdge,
        except: &str,
        select: impl Fn(u32, u32) -> bool,
        rows: bool,
    ) {
        for p in self.toolbars.iter_mut().filter(|p| p.id != except) {
            if let ToolbarPosition::Docked {
                edge: e,
                row,
                index,
            } = &mut p.position
                && *e == edge
                && select(*row, *index)
            {
                if rows {
                    *row += 1;
                } else {
                    *index += 1;
                }
            }
        }
    }
}

/// Where toolbar `dragged` would land if dropped with the pointer at `pointer`.
/// `area` is the window between menu bar and status bar, `lines` the docks as drawn.
/// `None` means it floats.
///
/// The pointer docks a toolbar when it is over a dock's rows (joining the nearest
/// line, before the first toolbar whose center is past the pointer) or within
/// [`SNAP`] beyond them or beyond a bare window edge (a new outermost row). Over the
/// menu bar counts as the top dock, over the status bar as the bottom dock.
pub fn drop_target(
    pointer: Pos2,
    area: Rect,
    lines: &[DockLine],
    dragged: &str,
) -> Option<DropTarget> {
    // Over the rows of a dock: join the nearest line of that dock.
    for edge in DockEdge::ALL {
        let mine: Vec<&DockLine> = lines.iter().filter(|l| l.edge == edge).collect();
        if mine.is_empty() {
            continue;
        }
        let depth = dock_depth(edge, area, lines);
        let main = mine
            .iter()
            .map(|l| edge.main_range(l.rect))
            .reduce(|a, b| egui::Rangef::new(a.min.min(b.min), a.max.max(b.max)))
            .expect("not empty");
        if edge.depth_of(area, pointer) > depth || !main.contains(edge.main(pointer)) {
            continue;
        }
        let cross = |l: &DockLine| {
            let r = edge.cross_range(l.rect);
            let c = if edge.is_vertical() {
                pointer.x
            } else {
                pointer.y
            };
            if r.contains(c) {
                0.0
            } else {
                (r.min - c).abs().min((r.max - c).abs())
            }
        };
        let at = mine
            .iter()
            .enumerate()
            .min_by(|a, b| cross(a.1).total_cmp(&cross(b.1)))
            .map(|(i, _)| i)
            .expect("not empty");
        let line = mine[at];
        let pm = edge.main(pointer);
        let mut before = line
            .items
            .iter()
            .filter(|(id, _)| id != dragged)
            .find(|(_, r)| edge.main_range(*r).center() > pm)
            .map(|(id, _)| id.clone());
        if before.is_none() {
            // End of a wrapped line: before the first toolbar of the row's next line.
            before = mine[at + 1..]
                .iter()
                .filter(|l| l.row == line.row)
                .flat_map(|l| l.items.iter())
                .find(|(id, _)| id != dragged)
                .map(|(id, _)| id.clone());
        }
        return Some(DropTarget::Row {
            edge,
            row: line.row,
            before,
        });
    }
    // Just beyond a dock's rows or near a bare edge: a new row.
    DockEdge::ALL
        .into_iter()
        .filter_map(|edge| {
            let beyond = edge.depth_of(area, pointer) - dock_depth(edge, area, lines);
            (beyond <= SNAP).then_some((beyond, edge))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, edge)| DropTarget::NewRow {
            edge,
            row: lines
                .iter()
                .filter(|l| l.edge == edge)
                .map(|l| l.row + 1)
                .max()
                .unwrap_or(0),
        })
}

/// How far the docked rows of `edge` reach into `area` (0 without rows).
pub fn dock_depth(edge: DockEdge, area: Rect, lines: &[DockLine]) -> f32 {
    lines
        .iter()
        .filter(|l| l.edge == edge)
        .map(|l| {
            let r = edge.cross_range(l.rect);
            let far = match edge {
                DockEdge::Top | DockEdge::Left => r.max,
                DockEdge::Bottom | DockEdge::Right => r.min,
            };
            let p = if edge.is_vertical() {
                pos2(far, area.center().y)
            } else {
                pos2(area.center().x, far)
            };
            edge.depth_of(area, p)
        })
        .fold(0.0, f32::max)
}

/// Outline to show for a docking `target`: where a toolbar of `size` (its size
/// docked on that edge) would go.
pub fn dock_preview(target: &DropTarget, area: Rect, lines: &[DockLine], size: Vec2) -> Rect {
    match target {
        DropTarget::Float { pos } => Rect::from_min_size(*pos, size),
        DropTarget::Row { edge, row, before } => {
            let mine: Vec<&DockLine> = lines
                .iter()
                .filter(|l| l.edge == *edge && l.row == *row)
                .collect();
            let hit = before.as_ref().and_then(|b| {
                mine.iter()
                    .find_map(|l| l.items.iter().find(|(id, _)| id == b).map(|(_, r)| (l, *r)))
            });
            let (line, start) = match (hit, mine.last()) {
                (Some((l, r)), _) => (*l, r.min),
                (None, Some(l)) => {
                    let end = l.items.last().map_or(l.rect.min, |(_, r)| {
                        if edge.is_vertical() {
                            r.left_bottom()
                        } else {
                            r.right_top()
                        }
                    });
                    (*l, end)
                }
                (None, None) => return Rect::NOTHING,
            };
            let min = if edge.is_vertical() {
                pos2(line.rect.left(), start.y)
            } else {
                pos2(start.x, line.rect.top())
            };
            Rect::from_min_size(min, size)
        }
        DropTarget::NewRow { edge, .. } => {
            let d = dock_depth(*edge, area, lines);
            let top = area.top() + dock_depth(DockEdge::Top, area, lines);
            let min = match edge {
                DockEdge::Top => pos2(area.left(), area.top() + d),
                DockEdge::Bottom => pos2(area.left(), area.bottom() - d - size.y),
                DockEdge::Left => pos2(area.left() + d, top),
                DockEdge::Right => pos2(area.right() - d - size.x, top),
            };
            Rect::from_min_size(min, size)
        }
    }
}

/// Splits toolbars of `lengths` (along the row, gaps included) into lines no longer
/// than `available`. Every line holds at least one toolbar.
pub fn wrap(lengths: &[f32], available: f32) -> Vec<std::ops::Range<usize>> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut used = 0.0;
    for (i, &len) in lengths.iter().enumerate() {
        if i > start && used + len > available {
            lines.push(start..i);
            start = i;
            used = 0.0;
        }
        used += len;
    }
    if start < lengths.len() {
        lines.push(start..lengths.len());
    }
    lines
}

/// Top-left corner for a floating toolbar of `size` at `pos` kept inside `bounds`.
pub fn clamp_float(pos: Pos2, size: Vec2, bounds: Rect) -> Pos2 {
    pos2(
        pos.x.min(bounds.right() - size.x).max(bounds.left()),
        pos.y.min(bounds.bottom() - size.y).max(bounds.top()),
    )
}

/// What a cell of a laid-out toolbar is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cell {
    Grip,
    /// Index into `Toolbar::items`.
    Button(usize),
    /// Index into `Toolbar::items` of a [`mt_spec::ToolbarWidget`].
    Widget(usize),
    Separator,
}

/// A toolbar's grip, buttons and separators, relative to its top-left corner.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Cells {
    pub size: Vec2,
    pub cells: Vec<(Rect, Cell)>,
}

/// Runs of buttons and single widgets between separators.
enum Run {
    Buttons(Vec<usize>),
    Widget(usize, f32),
}

/// Lays out `toolbar` upright (`vertical`: `columns` buttons per line, as docked
/// left or right) or lying (lines of `columns` buttons stacked across, so the
/// two-column Large Tool Set is two buttons high; unverified for SketchUp). `grip`:
/// docked toolbars have one, floating ones are moved by their title bar instead.
/// Widgets (dropdowns, sliders, the Measurements box) are their spec width wide and
/// one button high lying, and get a line of their own upright (the toolbar widens to
/// fit them; unverified for SketchUp).
pub(crate) fn toolbar_cells(
    toolbar: &Toolbar,
    vertical: bool,
    grip: bool,
    sizes: &UiSizes,
) -> Cells {
    let size = sizes.toolbar_button;
    let pad = sizes.toolbar_padding;
    let gap = sizes.separator_gap;
    let n = toolbar.columns.max(1) as usize;
    let mut cells = Vec::new();
    // Runs between separators, then the separator (false at the end).
    let mut groups: Vec<(Vec<Run>, bool)> = vec![(Vec::new(), false)];
    for (i, item) in toolbar.items.iter().enumerate() {
        let group = &mut groups.last_mut().expect("never empty").0;
        match item {
            ToolbarItem::Button(_) => match group.last_mut() {
                Some(Run::Buttons(b)) => b.push(i),
                _ => group.push(Run::Buttons(vec![i])),
            },
            ToolbarItem::Widget(w) => group.push(Run::Widget(i, w.width)),
            ToolbarItem::Separator { .. } => {
                groups.last_mut().expect("never empty").1 = true;
                groups.push((Vec::new(), false));
            }
        }
    }
    if vertical {
        let widest = toolbar
            .widgets()
            .map(|w| w.width)
            .fold(n as f32 * size, f32::max);
        let width = widest + 2.0 * pad;
        // Buttons stay at the left; a wider widget makes the column wider.
        let mut y = pad;
        if grip {
            cells.push((
                Rect::from_min_size(pos2(0.0, y), vec2(width, GRIP)),
                Cell::Grip,
            ));
            y += GRIP;
        }
        for (runs, sep) in groups {
            for run in runs {
                match run {
                    Run::Buttons(buttons) => {
                        for line in buttons.chunks(n) {
                            for (k, &i) in line.iter().enumerate() {
                                let min = pos2(pad + k as f32 * size, y);
                                cells.push((
                                    Rect::from_min_size(min, vec2(size, size)),
                                    Cell::Button(i),
                                ));
                            }
                            y += size;
                        }
                    }
                    Run::Widget(i, w) => {
                        cells.push((
                            Rect::from_min_size(pos2(pad, y), vec2(w, size)),
                            Cell::Widget(i),
                        ));
                        y += size;
                    }
                }
            }
            if sep {
                cells.push((
                    Rect::from_min_size(pos2(0.0, y), vec2(width, gap)),
                    Cell::Separator,
                ));
                y += gap;
            }
        }
        Cells {
            size: vec2(width, y + pad),
            cells,
        }
    } else {
        let height = n as f32 * size;
        let mut x = if grip { 0.0 } else { pad };
        if grip {
            cells.push((
                Rect::from_min_size(pos2(0.0, 0.0), vec2(GRIP, height)),
                Cell::Grip,
            ));
            x += GRIP;
        }
        for (runs, sep) in groups {
            for run in runs {
                match run {
                    Run::Buttons(buttons) => {
                        for column in buttons.chunks(n) {
                            for (k, &i) in column.iter().enumerate() {
                                let min = pos2(x, k as f32 * size);
                                cells.push((
                                    Rect::from_min_size(min, vec2(size, size)),
                                    Cell::Button(i),
                                ));
                            }
                            x += size;
                        }
                    }
                    Run::Widget(i, w) => {
                        let top = ((height - size) / 2.0).floor();
                        cells.push((
                            Rect::from_min_size(pos2(x + 2.0, top), vec2(w, size)),
                            Cell::Widget(i),
                        ));
                        x += w + 4.0;
                    }
                }
            }
            if sep {
                cells.push((
                    Rect::from_min_size(pos2(x, 0.0), vec2(gap, height)),
                    Cell::Separator,
                ));
                x += gap;
            }
        }
        if !grip {
            x += pad;
        }
        Cells {
            size: vec2(x, height),
            cells,
        }
    }
}

/// Floating toolbars stand upright if they have more than one column (the Large Tool
/// Set), else lie flat. Unverified for SketchUp.
pub(crate) fn floats_vertical(toolbar: &Toolbar) -> bool {
    toolbar.columns > 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_spec::Spec;

    fn spec_layout() -> ToolbarLayout {
        ToolbarLayout::from_spec(&Spec::builtin().unwrap().toolbars.toolbar)
    }

    fn docked(edge: DockEdge, row: u32, index: u32) -> ToolbarPosition {
        ToolbarPosition::Docked { edge, row, index }
    }

    fn place(id: &str, position: ToolbarPosition) -> ToolbarPlace {
        ToolbarPlace {
            id: id.into(),
            visible: true,
            position,
        }
    }

    fn layout(places: &[(&str, ToolbarPosition)]) -> ToolbarLayout {
        ToolbarLayout {
            toolbars: places.iter().map(|(id, p)| place(id, *p)).collect(),
        }
    }

    /// Window 1000 x 600 with a top dock of one 30 px line holding "a" (x 0..300) and
    /// "b" (x 306..500), and a left dock column x 0..64 holding "c".
    fn geometry() -> (Rect, Vec<DockLine>) {
        let area = Rect::from_min_max(pos2(0.0, 20.0), pos2(1000.0, 600.0));
        let top = DockLine {
            edge: DockEdge::Top,
            row: 0,
            rect: Rect::from_min_max(pos2(0.0, 20.0), pos2(1000.0, 54.0)),
            items: vec![
                (
                    "a".into(),
                    Rect::from_min_max(pos2(2.0, 22.0), pos2(300.0, 52.0)),
                ),
                (
                    "b".into(),
                    Rect::from_min_max(pos2(306.0, 22.0), pos2(500.0, 52.0)),
                ),
            ],
        };
        let left = DockLine {
            edge: DockEdge::Left,
            row: 0,
            rect: Rect::from_min_max(pos2(0.0, 55.0), pos2(64.0, 600.0)),
            items: vec![(
                "c".into(),
                Rect::from_min_max(pos2(0.0, 55.0), pos2(64.0, 500.0)),
            )],
        };
        (area, vec![top, left])
    }

    #[test]
    fn default_layout_from_spec() {
        let l = spec_layout();
        assert_eq!(
            l.get("getting_started").unwrap().position,
            docked(DockEdge::Top, 0, 0)
        );
        assert_eq!(
            l.get("large_tool_set").unwrap().position,
            docked(DockEdge::Left, 0, 0)
        );
        assert!(l.floating().is_empty());
    }

    #[test]
    fn drop_on_row_inserts_before_next_toolbar() {
        let (area, lines) = geometry();
        // Between a's and b's centers.
        assert_eq!(
            drop_target(pos2(200.0, 30.0), area, &lines, "x"),
            Some(DropTarget::Row {
                edge: DockEdge::Top,
                row: 0,
                before: Some("b".into())
            })
        );
        // Past b: end of the row.
        assert_eq!(
            drop_target(pos2(700.0, 40.0), area, &lines, "x"),
            Some(DropTarget::Row {
                edge: DockEdge::Top,
                row: 0,
                before: None
            })
        );
        // Over the menu bar: still the top dock.
        assert!(matches!(
            drop_target(pos2(700.0, 5.0), area, &lines, "x"),
            Some(DropTarget::Row {
                edge: DockEdge::Top,
                ..
            })
        ));
        // The dragged toolbar itself is skipped.
        assert_eq!(
            drop_target(pos2(100.0, 30.0), area, &lines, "a"),
            Some(DropTarget::Row {
                edge: DockEdge::Top,
                row: 0,
                before: Some("b".into())
            })
        );
    }

    #[test]
    fn drop_near_edges_makes_new_rows() {
        let (area, lines) = geometry();
        // Just below the top row.
        assert_eq!(
            drop_target(pos2(500.0, 70.0), area, &lines, "x"),
            Some(DropTarget::NewRow {
                edge: DockEdge::Top,
                row: 1
            })
        );
        // Right of the left column.
        assert_eq!(
            drop_target(pos2(80.0, 300.0), area, &lines, "x"),
            Some(DropTarget::NewRow {
                edge: DockEdge::Left,
                row: 1
            })
        );
        // Bare right and bottom edges.
        assert_eq!(
            drop_target(pos2(990.0, 300.0), area, &lines, "x"),
            Some(DropTarget::NewRow {
                edge: DockEdge::Right,
                row: 0
            })
        );
        assert_eq!(
            drop_target(pos2(500.0, 590.0), area, &lines, "x"),
            Some(DropTarget::NewRow {
                edge: DockEdge::Bottom,
                row: 0
            })
        );
        // Over the left column.
        assert!(matches!(
            drop_target(pos2(30.0, 300.0), area, &lines, "x"),
            Some(DropTarget::Row {
                edge: DockEdge::Left,
                row: 0,
                ..
            })
        ));
        // Middle of the window: float.
        assert_eq!(drop_target(pos2(500.0, 300.0), area, &lines, "x"), None);
    }

    #[test]
    fn apply_drop_rows_and_float() {
        use DockEdge::*;
        let mut l = layout(&[
            ("a", docked(Top, 0, 0)),
            ("b", docked(Top, 0, 1)),
            ("c", docked(Left, 0, 0)),
        ]);
        // c joins the top row before b.
        l.apply_drop(
            "c",
            &DropTarget::Row {
                edge: Top,
                row: 0,
                before: Some("b".into()),
            },
        );
        assert_eq!(l.rows(Top, true), vec![(0, vec!["a", "c", "b"])]);
        assert!(l.rows(Left, true).is_empty());
        // a moves into a new second row.
        l.apply_drop("a", &DropTarget::NewRow { edge: Top, row: 1 });
        assert_eq!(l.rows(Top, true), vec![(0, vec!["c", "b"]), (1, vec!["a"])]);
        // New row at the window edge pushes the others out.
        l.apply_drop("b", &DropTarget::NewRow { edge: Top, row: 0 });
        assert_eq!(
            l.rows(Top, true),
            vec![(0, vec!["b"]), (1, vec!["c"]), (2, vec!["a"])]
        );
        // Float, and its old row disappears.
        l.apply_drop(
            "c",
            &DropTarget::Float {
                pos: pos2(10.0, 20.0),
            },
        );
        assert_eq!(l.rows(Top, true), vec![(0, vec!["b"]), (1, vec!["a"])]);
        assert_eq!(l.floating(), vec![("c", pos2(10.0, 20.0))]);
        // Re-dock at the end of row 0.
        l.apply_drop(
            "c",
            &DropTarget::Row {
                edge: Top,
                row: 0,
                before: None,
            },
        );
        assert_eq!(l.rows(Top, true), vec![(0, vec!["b", "c"]), (1, vec!["a"])]);
        assert!(!l.apply_drop("nope", &DropTarget::NewRow { edge: Top, row: 0 }));
    }

    #[test]
    fn hidden_toolbars_keep_their_row() {
        use DockEdge::*;
        let mut l = layout(&[("a", docked(Top, 0, 0)), ("b", docked(Top, 1, 0))]);
        l.set_visible("a", false);
        assert_eq!(l.rows(Top, true), vec![(1, vec!["b"])]);
        assert_eq!(l.rows(Top, false).len(), 2);
        l.set_visible("a", true);
        assert_eq!(l.rows(Top, true), vec![(0, vec!["a"]), (1, vec!["b"])]);
    }

    #[test]
    fn merged_keeps_defaults_for_unknown() {
        let defaults = spec_layout();
        let saved = layout(&[
            ("large_tool_set", docked(DockEdge::Right, 5, 3)),
            ("unknown", docked(DockEdge::Top, 0, 0)),
        ]);
        let m = ToolbarLayout::merged(&defaults, &saved);
        assert_eq!(m.toolbars.len(), defaults.toolbars.len());
        assert_eq!(
            m.get("large_tool_set").unwrap().position,
            docked(DockEdge::Right, 0, 0)
        );
        assert_eq!(
            m.get("getting_started").unwrap().position,
            docked(DockEdge::Top, 0, 0)
        );
    }

    /// A `toolbars.toml` saved before the later toolbars existed still loads: its
    /// places are kept and the new toolbars get their default (hidden) places.
    #[test]
    fn old_saved_layout_still_loads() {
        let old = r#"
[[toolbars]]
id = "getting_started"
visible = false

[toolbars.position]
kind = "floating"
x = 300.0
y = 200.0

[[toolbars]]
id = "large_tool_set"
visible = true

[toolbars.position]
kind = "docked"
edge = "right"
row = 0
index = 0
"#;
        let saved: ToolbarLayout = toml::from_str(old).unwrap();
        let defaults = spec_layout();
        let m = ToolbarLayout::merged(&defaults, &saved);
        assert_eq!(m.toolbars.len(), defaults.toolbars.len());
        let gs = m.get("getting_started").unwrap();
        assert!(!gs.visible);
        assert_eq!(
            gs.position,
            ToolbarPosition::Floating { x: 300.0, y: 200.0 }
        );
        assert_eq!(
            m.get("large_tool_set").unwrap().position,
            docked(DockEdge::Right, 0, 0)
        );
        let edge = |p: &ToolbarPlace| match p.position {
            ToolbarPosition::Docked { edge, .. } => Some(edge),
            ToolbarPosition::Floating { .. } => None,
        };
        for place in &m.toolbars[2..] {
            assert!(!place.visible, "{}", place.id);
            assert_eq!(edge(place), edge(defaults.get(&place.id).unwrap()));
        }
        // Showing a new toolbar puts it on its default edge, in one row with the
        // other new toolbars of that edge (row 0 here: Getting Started floats).
        let mut shown = m.clone();
        shown.set_visible("standard", true);
        shown.set_visible("views", true);
        assert_eq!(
            shown.rows(DockEdge::Top, true),
            vec![(0, vec!["standard", "views"])]
        );
        assert_eq!(
            shown.rows(DockEdge::Right, true),
            vec![(0, vec!["large_tool_set"])]
        );
    }

    #[test]
    fn widgets_get_their_width() {
        let spec = Spec::builtin().unwrap();
        let sizes = &spec.ui_theme.sizes;
        let shadows = spec
            .toolbars
            .toolbar
            .iter()
            .find(|t| t.id == "shadows")
            .unwrap();
        let lying = toolbar_cells(shadows, false, true, sizes);
        let widgets: Vec<Rect> = lying
            .cells
            .iter()
            .filter(|c| matches!(c.1, Cell::Widget(_)))
            .map(|c| c.0)
            .collect();
        assert_eq!(widgets.len(), 2);
        assert!(widgets.iter().all(|r| r.width() == 150.0));
        assert!(widgets[0].max.x <= widgets[1].min.x);
        assert!(lying.size.x > 300.0 + 2.0 * sizes.toolbar_button);
        assert_eq!(lying.size.y, sizes.toolbar_button);
        // Upright, each widget has a line of its own and the toolbar is as wide as
        // the widest one.
        let upright = toolbar_cells(shadows, true, true, sizes);
        assert_eq!(upright.size.x, 150.0 + 2.0 * sizes.toolbar_padding);
        let ys: Vec<f32> = upright
            .cells
            .iter()
            .filter(|c| matches!(c.1, Cell::Widget(_)))
            .map(|c| c.0.min.y)
            .collect();
        assert!(ys[0] < ys[1]);
        for (r, _) in &upright.cells {
            assert!(r.max.x <= upright.size.x && r.max.y <= upright.size.y);
        }
    }

    #[test]
    fn layout_serializes() {
        let mut l = spec_layout();
        l.apply_drop(
            "getting_started",
            &DropTarget::Float {
                pos: pos2(300.0, 200.0),
            },
        );
        l.set_visible("large_tool_set", false);
        let text = toml::to_string(&l).unwrap();
        let back: ToolbarLayout = toml::from_str(&text).unwrap();
        assert_eq!(back, l);
    }

    #[test]
    fn wrap_packs_rows() {
        assert_eq!(wrap(&[100.0, 100.0, 100.0], 250.0), vec![0..2, 2..3]);
        assert_eq!(wrap(&[100.0, 100.0], 1000.0), vec![0..2]);
        // A toolbar longer than the line still gets a line of its own.
        assert_eq!(wrap(&[50.0, 400.0, 50.0], 300.0), vec![0..1, 1..2, 2..3]);
        assert!(wrap(&[], 100.0).is_empty());
    }

    #[test]
    fn clamp_keeps_float_inside() {
        let b = Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 600.0));
        let s = vec2(200.0, 50.0);
        assert_eq!(clamp_float(pos2(700.0, -10.0), s, b), pos2(600.0, 0.0));
        assert_eq!(clamp_float(pos2(100.0, 100.0), s, b), pos2(100.0, 100.0));
        assert_eq!(clamp_float(pos2(-50.0, 590.0), s, b), pos2(0.0, 550.0));
    }

    #[test]
    fn cells_match_classic_layout() {
        let spec = Spec::builtin().unwrap();
        let sizes = &spec.ui_theme.sizes;
        let gs = &spec.toolbars.toolbar[0];
        let lts = &spec.toolbars.toolbar[1];
        let buttons = gs.buttons().count() as f32;
        let seps = (gs.items.len() as f32) - buttons;
        let h = toolbar_cells(gs, false, true, sizes);
        assert_eq!(
            h.size,
            vec2(
                GRIP + buttons * sizes.toolbar_button + seps * sizes.separator_gap,
                sizes.toolbar_button
            )
        );
        let v = toolbar_cells(lts, true, true, sizes);
        assert_eq!(
            v.size.x,
            2.0 * sizes.toolbar_button + 2.0 * sizes.toolbar_padding
        );
        // Lying down, the two-column Large Tool Set is two buttons high.
        let lying = toolbar_cells(lts, false, true, sizes);
        assert_eq!(lying.size.y, 2.0 * sizes.toolbar_button);
        assert_eq!(
            lying
                .cells
                .iter()
                .filter(|c| matches!(c.1, Cell::Button(_)))
                .count(),
            lts.buttons().count()
        );
    }

    #[test]
    fn preview_rects() {
        let (area, lines) = geometry();
        let size = vec2(100.0, 30.0);
        let before_b = DropTarget::Row {
            edge: DockEdge::Top,
            row: 0,
            before: Some("b".into()),
        };
        assert_eq!(
            dock_preview(&before_b, area, &lines, size).min,
            pos2(306.0, 20.0)
        );
        let new = DropTarget::NewRow {
            edge: DockEdge::Top,
            row: 1,
        };
        assert_eq!(dock_preview(&new, area, &lines, size).min, pos2(0.0, 54.0));
    }
}
