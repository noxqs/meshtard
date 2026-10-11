//! SketchUp-style main window around the 3D viewport, built from `spec/`.
//!
//! Layout (SketchUp 2026, Windows, default workspace):
//!
//! ```text
//! +-------------------------------------------------------------+
//! | menu bar                                                    |
//! | top toolbars (Getting Started)                              |
//! +----+-------------------------------------------+------------+
//! |left|                                           | Default    |
//! |tool|              3D viewport                  | Tray       |
//! |set |          (drawn by the caller)            | (panels)   |
//! +----+-------------------------------------------+------------+
//! | status hint                          Measurements [______]  |
//! +-------------------------------------------------------------+
//! ```
//!
//! # Use
//!
//! ```no_run
//! # fn demo(ui: &mut egui::Ui, spec: &mt_spec::Spec) {
//! let mut shell = mt_ui::Shell::new(spec);
//! // every frame:
//! shell.capture_typing(ui.ctx()); // before your own shortcut handling
//! let typing = shell.is_typing(); // if true, skip single-key tool shortcuts
//! let info = mt_ui::EntityInfo::no_selection();
//! let frame = mt_ui::Frame {
//!     spec,
//!     active_tool: "tool.select",
//!     status: "Select objects.",
//!     measurements_label: "Measurements",
//!     measurements_value: "",
//!     is_enabled: &|_id| true,
//!     is_checked: &|_id| false,
//!     entity_info: &info,
//! };
//! shell.set_tray_data(mt_ui::TrayData::default()); // tags, materials, scenes, ...
//! for event in shell.show(ui, &frame, |ui| { /* draw the 3D view in ui */ }) {
//!     match event {
//!         mt_ui::Event::Command(id) => { /* run the command */ }
//!         mt_ui::Event::Measurements(text) => { /* parse the typed value */ }
//!         mt_ui::Event::SceneActivate(i) => { /* restore scene i */ }
//!         _ => {} // `Event` is non_exhaustive
//!     }
//! }
//! # }
//! ```
//!
//! # Tray panels and scene tabs
//!
//! Besides Entity Info (from [`Frame::entity_info`]), the tray panels show a
//! [`TrayData`] the app passes to [`Shell::set_tray_data`] before [`Shell::show`]
//! (every frame or whenever the model changes). It uses plain data only: indices,
//! index paths and `[u8; 3]` sRGB colors. What the user does comes back as [`Event`]s,
//! only on user action:
//!
//! | Panel     | Shows                                         | Events |
//! |-----------|-----------------------------------------------|--------|
//! | Tags      | [`TrayData::tags`] (0 = "Untagged"), eye, color swatch, pencil on [`TrayData::active_tag`]; double-click renames | [`Event::TagAdd`], [`Event::TagDelete`], [`Event::TagRename`], [`Event::TagVisible`], [`Event::TagActive`], [`Event::TagColor`] |
//! | Materials | "In Model" grid of [`TrayData::materials`], current material preview, Edit tab (name, color, opacity) | [`Event::MaterialPick`], [`Event::MaterialEdit`] |
//! | Components | Select tab: "In Model" list of [`TrayData::components`] (icon, name, instance count; groups are not definitions and not listed), Details menu with Purge Unused; header and Edit tab (name, description) for [`TrayData::current_component`] | [`Event::ComponentPick`], [`Event::ComponentEdit`], [`Event::ComponentPurge`] |
//! | Outliner  | [`TrayData::outliner`] tree under [`TrayData::outliner_root`], filter field | [`Event::OutlinerSelect`] |
//! | Scenes    | [`TrayData::scenes`], [`TrayData::active_scene`] | [`Event::SceneAdd`], [`Event::SceneDelete`], [`Event::SceneUpdate`], [`Event::SceneActivate`] |
//! | Styles    | [`TrayData::style`]: edges, profiles, section cut width, face style, X-ray | [`Event::StyleChanged`] |
//! | Shadows   | [`TrayData::shadows`]: on/off, time, date, light, dark, on faces/ground | [`Event::ShadowsChanged`] |
//!
//! When [`TrayData::scene_tabs`] is on and there are scenes, [`Shell::show`] draws a
//! strip of scene tabs between the toolbars and the viewport (the viewport closure
//! gets the rect below it); clicking a tab emits [`Event::SceneActivate`], its
//! context menu Update / Add / Delete.
//!
//! The shell applies a picked material and edited style, shadow and material values
//! to its copy of the data at once so controls respond without waiting for the app;
//! the next [`Shell::set_tray_data`] replaces that copy. Expand state, the Outliner
//! filter, the selected tag and the open tabs are shell UI state.
//!
//! Pure helpers are public for tests and reuse: [`outliner_rows`] (flattening and
//! filtering), [`OutlinerNode::label`], [`time_label`], [`date_label`],
//! [`instances_label`], [`component_edit`].
//!
//! # Toolbars
//!
//! Toolbars from `spec/toolbars.toml` start where the spec docks them. Dragging a
//! docked toolbar's grip (the dotted strip at its start) moves it: an outline shows
//! where it would land; dropped over or just beyond a dock's rows, or near a bare
//! window edge (over the menu bar counts as top, over the status bar as bottom), it
//! docks there, joining a row or starting a new one; elsewhere it becomes a floating
//! window with a title bar (drag it to move or dock it again, double-click it to dock
//! it where it was, its close button hides it). Left and right docks stand toolbars
//! upright in `columns` buttons per line; top and bottom docks lay them flat. The
//! viewport gets what the docks leave. View > Toolbars... (`view.toolbars`) and a
//! right-click on a grip or title bar list the toolbars with check boxes.
//!
//! The app can save and restore the arrangement: [`Shell::toolbar_layout`] (a
//! serde-serializable [`ToolbarLayout`]) and [`Shell::set_toolbar_layout`];
//! [`Event::ToolbarLayoutChanged`] tells when the user changed it. Also
//! [`Shell::toolbars`], [`Shell::set_toolbar_visible`], [`Shell::reset_toolbar_layout`].
//! The pure layout logic is in [`dock`].
//!
//! # Measurements box
//!
//! As in SketchUp, typing while the viewport has focus goes into the Measurements box:
//! a digit or one of `. , ; - + [ < ' " / * @ =` starts typing, after which every
//! character (including letters, e.g. `10mm`) is collected until Enter (emits
//! [`Event::Measurements`]) or Escape (cancels). Backspace edits. [`Shell::capture_typing`]
//! does this from the egui input queue and removes the consumed key events so tool
//! shortcuts don't fire; [`Shell::show`] calls it too. Callers that route keys
//! themselves can use [`Shell::type_text`], [`Shell::backspace`], [`Shell::submit`]
//! and [`Shell::cancel_typing`] instead. Clicking the box and typing there works too.
//!
//! # Theme
//!
//! [`Shell::show`] applies `spec/ui_theme.toml` to the context on its first call
//! (see [`apply_theme`]); call it yourself earlier if other windows should use it.

pub mod dock;
pub mod icons;
mod menu;
mod panels;
mod status;
mod theme;
mod toolbar;
mod tray;
mod tray_data;

use std::collections::{BTreeMap, HashMap};

use mt_spec::Spec;

pub use dock::{DockEdge, ToolbarLayout, ToolbarPlace, ToolbarPosition};
pub use theme::apply as apply_theme;
pub use tray_data::{
    ComponentRow, FaceStyle, MaterialRow, OutlinerKind, OutlinerNode, OutlinerRow, SelectMode,
    ShadowSettings, StyleSettings, TagRow, TrayData, component_edit, date_label, day_of_year,
    instances_label, month_day, outliner_node, outliner_rows, time_label,
};

/// What the Entity Info panel shows: a title (e.g. "Edge", "Face", "No Selection")
/// and rows of label and value (e.g. "Length", "1000 mm").
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EntityInfo {
    pub title: String,
    pub rows: Vec<(String, String)>,
}

impl EntityInfo {
    pub fn no_selection() -> Self {
        Self {
            title: "No Selection".to_owned(),
            rows: Vec::new(),
        }
    }
}

/// Application state the shell displays this frame.
pub struct Frame<'a> {
    pub spec: &'a Spec,
    /// Command id of the active tool; its toolbar buttons are drawn pressed.
    pub active_tool: &'a str,
    /// Status bar hint, left side.
    pub status: &'a str,
    /// Label left of the Measurements box, e.g. "Length".
    pub measurements_label: &'a str,
    /// Shown in the Measurements box while the user is not typing.
    pub measurements_value: &'a str,
    /// Whether a command can run now. Disabled menu items and toolbar buttons are greyed.
    pub is_enabled: &'a dyn Fn(&str) -> bool,
    /// Whether a toggle command is on (check mark in menus, pressed toolbar button).
    pub is_checked: &'a dyn Fn(&str) -> bool,
    pub entity_info: &'a EntityInfo,
}

/// Something the user did in the shell.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// A menu item or toolbar button was clicked: a command id from `menus.toml`.
    Command(String),
    /// Text entered in the Measurements box and confirmed with Enter.
    Measurements(String),

    /// Tags: "Add Tag" clicked.
    TagAdd,
    /// Tags: "Delete Tag" clicked with this tag selected (never 0, "Untagged").
    TagDelete(usize),
    /// Tags: tag renamed by double-click editing (never 0). The name is trimmed and
    /// non-empty.
    TagRename(usize, String),
    /// Tags: eye clicked; the new visibility.
    TagVisible(usize, bool),
    /// Tags: pencil column clicked; make this the active tag.
    TagActive(usize),
    /// Tags: color picked in the swatch's color picker.
    TagColor(usize, [u8; 3]),

    /// Materials: swatch clicked in the "In Model" grid; make it the Paint Bucket's
    /// current material. The shell also shows it as current at once.
    MaterialPick(usize),
    /// Materials: the Create Material button. The app adds a material and makes it
    /// current.
    MaterialCreate,
    /// Entity Info: a tag chosen in the Tag dropdown for the selected entities.
    EntityTag(usize),
    /// Materials: the current material edited (name committed, color or opacity
    /// changed). Carries all editable fields; `opacity` is in percent 0..=100.
    MaterialEdit {
        index: usize,
        name: String,
        color: [u8; 3],
        opacity: u8,
    },

    /// Components: definition clicked in the "In Model" list. As in SketchUp, the
    /// app starts placing a new instance of it at the cursor (Component tool). The
    /// shell also shows it as the current definition at once.
    ComponentPick(usize),
    /// Components: name or description of the current definition committed (Enter
    /// or focus loss) in the Edit tab. Carries both fields; `name` is trimmed and
    /// non-empty.
    ComponentEdit {
        index: usize,
        name: String,
        description: String,
    },
    /// Components: Details menu > Purge Unused; remove definitions with no
    /// instances.
    ComponentPurge,

    /// Outliner: row clicked. `path` = child indices from the root down
    /// (see [`OutlinerNode`]); `mode` from the Ctrl/Shift keys.
    OutlinerSelect { path: Vec<usize>, mode: SelectMode },

    /// Scenes: "Add Scene" clicked (panel button or tab context menu).
    SceneAdd,
    /// Scenes: "Delete Scene" clicked for this scene.
    SceneDelete(usize),
    /// Scenes: "Update Scene" clicked for this scene.
    SceneUpdate(usize),
    /// Scenes: scene clicked in the panel or its tab clicked above the viewport.
    SceneActivate(usize),

    /// Styles: any setting changed; the complete new settings.
    StyleChanged(StyleSettings),
    /// Shadows: any setting changed; the complete new settings.
    ShadowsChanged(ShadowSettings),

    /// Toolbars: the user docked, floated, moved, closed or showed a toolbar. Save
    /// [`Shell::toolbar_layout`] if the app keeps the layout between runs.
    ToolbarLayoutChanged,
}

/// A toolbar as listed by [`Shell::toolbars`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolbarInfo {
    /// Toolbar id from `spec/toolbars.toml`.
    pub id: String,
    /// Title shown in View > Toolbars.
    pub name: String,
    pub visible: bool,
    pub floating: bool,
}

/// Height of the scene tab strip above the viewport.
const SCENE_TAB_HEIGHT: f32 = 24.0;

/// Characters that start Measurements typing while the viewport has focus.
const TYPING_STARTERS: &str = ".,;-+[<'\"/*@=";

/// UI state of the main window: tray panels, flyout choices, Measurements text.
pub struct Shell {
    /// The app shows the menus natively (macOS menu bar); don't draw them in the window.
    native_menu: bool,
    theme: theme::Theme32,
    icon_palette: icons::Palette,
    styled: bool,
    tray_visible: bool,
    /// Tray panels in order, with (shown, expanded).
    panels: Vec<PanelState>,
    /// Flyout button key (`toolbar/button`) to the command it currently shows.
    flyouts: HashMap<String, String>,
    /// Command id to menu label, for tooltips of flyout entries.
    labels: BTreeMap<String, String>,
    /// Measurements box text while typing (or editing it directly).
    typed: String,
    typing: bool,
    field_text: String,
    pending: Vec<Event>,
    /// What the tray panels and scene tabs show, from [`Shell::set_tray_data`].
    data: TrayData,
    tray_ui: panels::TrayUi,
    /// Where the toolbars are now, and the spec's default workspace.
    toolbar_layout: ToolbarLayout,
    toolbar_defaults: ToolbarLayout,
    /// Toolbar ids and names in spec order.
    toolbar_names: Vec<(String, String)>,
    /// Docked toolbar lines as drawn this frame, and the window between menu bar and
    /// status bar: where drops dock.
    dock_lines: Vec<dock::DockLine>,
    dock_area: egui::Rect,
    toolbar_drag: Option<toolbar::ToolbarDrag>,
    /// Where floating toolbars were docked before, for double-click on the title.
    last_dock: HashMap<String, ToolbarPosition>,
    toolbars_dialog_open: bool,
}

#[derive(Debug, Clone)]
struct PanelState {
    id: String,
    title: String,
    shown: bool,
    expanded: bool,
}

impl Shell {
    pub fn new(spec: &Spec) -> Self {
        let toolbar_defaults = ToolbarLayout::from_spec(&spec.toolbars.toolbar);
        Self {
            native_menu: false,
            theme: theme::Theme32::from_spec(&spec.ui_theme),
            icon_palette: icons::Palette::from_spec(&spec.ui_theme.icons),
            styled: false,
            tray_visible: true,
            panels: spec
                .tray
                .panel
                .iter()
                .map(|p| PanelState {
                    id: p.id.clone(),
                    title: p.title.clone(),
                    shown: true,
                    expanded: p.expanded,
                })
                .collect(),
            flyouts: HashMap::new(),
            labels: spec.commands(),
            typed: String::new(),
            typing: false,
            field_text: String::new(),
            pending: Vec::new(),
            data: TrayData::default(),
            tray_ui: panels::TrayUi::default(),
            toolbar_layout: toolbar_defaults.clone(),
            toolbar_defaults,
            toolbar_names: spec
                .toolbars
                .toolbar
                .iter()
                .map(|t| (t.id.clone(), t.name.clone()))
                .collect(),
            dock_lines: Vec::new(),
            dock_area: egui::Rect::NOTHING,
            toolbar_drag: None,
            last_dock: HashMap::new(),
            toolbars_dialog_open: false,
        }
    }

    /// Lays out the whole window in `ui` (normally the root ui of `eframe::App::ui`);
    /// `viewport` draws the 3D view in the remaining central rect. Returns what the
    /// user did this frame.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        frame: &Frame,
        viewport: impl FnOnce(&mut egui::Ui),
    ) -> Vec<Event> {
        if !self.styled {
            apply_theme(ui.ctx(), &frame.spec.ui_theme);
            self.styled = true;
        }
        self.capture_typing(ui.ctx());

        let t = self.theme.clone();
        if !self.native_menu {
            egui::Panel::top("mt_ui_menu_bar")
                .frame(
                    egui::Frame::NONE
                        .fill(t.menu_bar)
                        .inner_margin(egui::Margin::symmetric(2, 0)),
                )
                .exact_size(t.sizes.menu_bar_height)
                .show_separator_line(false)
                .show(ui, |ui| self.menu_bar(ui, frame));
        }

        let below_menu = ui.available_rect_before_wrap();
        self.dock_lines.clear();
        self.docked_toolbars(ui, frame, DockEdge::Top);

        let status = egui::Panel::bottom("mt_ui_status_bar")
            .frame(
                egui::Frame::NONE
                    .fill(t.status_bar)
                    .inner_margin(egui::Margin::symmetric(6, 0))
                    .stroke(egui::Stroke::new(1.0, t.separator)),
            )
            .exact_size(t.sizes.status_bar_height)
            .show_separator_line(false)
            .show(ui, |ui| self.status_bar(ui, frame));
        self.dock_area = egui::Rect::from_min_max(
            below_menu.min,
            egui::pos2(below_menu.max.x, status.response.rect.top()),
        );

        // Toolbars dock outside the tray, the status bar outside the toolbars.
        // Unverified for SketchUp 2026.
        self.docked_toolbars(ui, frame, DockEdge::Bottom);
        self.docked_toolbars(ui, frame, DockEdge::Left);
        self.docked_toolbars(ui, frame, DockEdge::Right);

        if self.tray_visible {
            egui::Panel::right("mt_ui_tray")
                .frame(egui::Frame::NONE.fill(t.tray))
                .default_size(frame.spec.tray.width)
                .size_range(180.0..=600.0)
                .resizable(true)
                .show_separator_line(true)
                .show(ui, |ui| self.tray(ui, frame));
        }

        if self.data.scene_tabs && !self.data.scenes.is_empty() {
            egui::Panel::top("mt_ui_scene_tabs")
                .frame(egui::Frame::NONE.fill(t.toolbar))
                .exact_size(SCENE_TAB_HEIGHT)
                .show_separator_line(false)
                .show(ui, |ui| self.scene_tabs(ui));
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, viewport);

        self.floating_toolbars(ui.ctx(), frame);
        self.toolbars_dialog(ui.ctx());
        self.toolbar_drag(ui.ctx(), frame);

        std::mem::take(&mut self.pending)
    }

    /// Forwards typed text from the viewport to the Measurements box. Starts typing
    /// (clearing the box) if the user was not typing yet. `'\n'` or `'\r'` submits.
    pub fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            match ch {
                '\n' | '\r' => self.submit(),
                '\u{8}' => self.backspace(),
                c if c.is_control() => {}
                c => {
                    if !self.typing {
                        self.typing = true;
                        self.typed.clear();
                    }
                    self.typed.push(c);
                }
            }
        }
    }

    /// Removes the last typed character.
    pub fn backspace(&mut self) {
        if self.typing {
            self.typed.pop();
        }
    }

    /// Confirms the typed text: queues [`Event::Measurements`] (returned by the next
    /// [`Shell::show`]) and ends typing.
    pub fn submit(&mut self) {
        if self.typing {
            self.typing = false;
            let text = std::mem::take(&mut self.typed);
            self.pending.push(Event::Measurements(text));
        }
    }

    /// Drops the typed text; the box shows the caller's value again.
    pub fn cancel_typing(&mut self) {
        self.typing = false;
        self.typed.clear();
    }

    /// True while the user is typing into the Measurements box. Single-key tool
    /// shortcuts must not run then (`10mm` must not switch to Move).
    pub fn is_typing(&self) -> bool {
        self.typing
    }

    /// Text currently in the Measurements box while typing.
    pub fn typed_text(&self) -> &str {
        &self.typed
    }

    /// Whether `ch` starts Measurements typing when pressed over the viewport.
    pub fn starts_typing(ch: char) -> bool {
        ch.is_ascii_digit() || TYPING_STARTERS.contains(ch)
    }

    /// Moves viewport typing from egui's input queue into the Measurements box and
    /// removes the consumed events, so shortcut handling after this call doesn't see
    /// them. Does nothing while a text field (e.g. the box itself) has keyboard focus.
    /// Safe to call more than once per frame.
    pub fn capture_typing(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let mut actions = Vec::new();
        let typing = self.typing;
        ctx.input_mut(|i| {
            let starts = typing
                || i.events.iter().any(|e| {
                    matches!(e, egui::Event::Text(t)
                        if t.chars().next().is_some_and(Self::starts_typing))
                });
            if !starts {
                return;
            }
            i.events.retain(|e| match e {
                egui::Event::Text(t) => {
                    actions.push(Typing::Text(t.clone()));
                    false
                }
                egui::Event::Key {
                    key,
                    pressed,
                    modifiers,
                    ..
                } if !modifiers.ctrl && !modifiers.alt && !modifiers.command => {
                    if *pressed {
                        match key {
                            egui::Key::Enter => actions.push(Typing::Enter),
                            egui::Key::Escape => actions.push(Typing::Escape),
                            egui::Key::Backspace => actions.push(Typing::Backspace),
                            _ => {}
                        }
                    }
                    false
                }
                _ => true,
            });
        });
        for action in actions {
            match action {
                Typing::Text(t) => self.type_text(&t),
                Typing::Enter => self.submit(),
                Typing::Escape => self.cancel_typing(),
                Typing::Backspace => self.backspace(),
            }
        }
    }

    /// Sets what the tray panels (Tags, Materials, Components, Outliner, Scenes, Styles, Shadows)
    /// and the scene tabs show. Call before [`Shell::show`] whenever the model changed
    /// (calling it every frame is fine).
    /// Leaves the menu bar to the app (a native macOS menu bar built from the same
    /// spec); the window then starts with the toolbars.
    pub fn set_native_menu(&mut self, on: bool) {
        self.native_menu = on;
    }

    pub fn set_tray_data(&mut self, data: TrayData) {
        self.data = data;
    }

    /// The tray data last set, including changes the user made since (a picked
    /// material, edited style or shadow settings are applied here at once).
    pub fn tray_data(&self) -> &TrayData {
        &self.data
    }

    pub fn tray_visible(&self) -> bool {
        self.tray_visible
    }

    pub fn set_tray_visible(&mut self, visible: bool) {
        self.tray_visible = visible;
    }

    /// Shows and expands the tray panel opened by Window menu command `id`.
    /// Returns false if `id` is not a tray panel.
    pub fn open_panel(&mut self, id: &str) -> bool {
        match self.panels.iter_mut().find(|p| p.id == id) {
            Some(p) => {
                p.shown = true;
                p.expanded = true;
                self.tray_visible = true;
                true
            }
            None => false,
        }
    }

    /// Whether tray panel `id` is shown and expanded.
    pub fn panel_expanded(&self, id: &str) -> bool {
        self.panels
            .iter()
            .any(|p| p.id == id && p.shown && p.expanded)
    }

    /// Where the toolbars are: docked rows and floating positions, and which are
    /// shown. Save it (it is serde-serializable) and restore it with
    /// [`Shell::set_toolbar_layout`]; [`Event::ToolbarLayoutChanged`] tells when it
    /// changed. (eframe's persistence feature is off in this workspace, so the shell
    /// doesn't store it itself.)
    pub fn toolbar_layout(&self) -> &ToolbarLayout {
        &self.toolbar_layout
    }

    /// Restores a layout from [`Shell::toolbar_layout`]. Toolbars it doesn't mention
    /// keep their default place; ids no longer in the spec are ignored. Floating
    /// positions outside the window are pulled inside when drawn.
    pub fn set_toolbar_layout(&mut self, layout: ToolbarLayout) {
        self.toolbar_layout = ToolbarLayout::merged(&self.toolbar_defaults, &layout);
        self.toolbar_drag = None;
    }

    /// Puts every toolbar back in its default place (`spec/toolbars.toml`).
    pub fn reset_toolbar_layout(&mut self) {
        self.toolbar_layout = self.toolbar_defaults.clone();
        self.toolbar_drag = None;
    }

    /// All toolbars in spec order, with whether they are shown and floating; what
    /// View > Toolbars lists.
    pub fn toolbars(&self) -> Vec<ToolbarInfo> {
        self.toolbar_names
            .iter()
            .map(|(id, name)| {
                let place = self.toolbar_layout.get(id);
                ToolbarInfo {
                    id: id.clone(),
                    name: name.clone(),
                    visible: place.is_some_and(|p| p.visible),
                    floating: place
                        .is_some_and(|p| matches!(p.position, ToolbarPosition::Floating { .. })),
                }
            })
            .collect()
    }

    /// Shows or hides toolbar `id` in its last place. Returns false if there is no
    /// such toolbar.
    pub fn set_toolbar_visible(&mut self, id: &str, visible: bool) -> bool {
        self.toolbar_layout.set_visible(id, visible)
    }

    /// True while the user drags a toolbar (the viewport should ignore the mouse).
    pub fn is_dragging_toolbar(&self) -> bool {
        self.toolbar_drag.is_some()
    }

    /// Opens the View > Toolbars... dialog (the shell does this itself for the
    /// `view.toolbars` command).
    pub fn open_toolbars_dialog(&mut self) {
        self.toolbars_dialog_open = true;
    }

    /// Records a command chosen in the menu bar or a toolbar, and handles the ones
    /// that only change the shell (tray panels).
    fn command(&mut self, id: &str) {
        if id == "window.tray_show" {
            self.tray_visible = !self.tray_visible;
        } else if id == "view.toolbars" {
            self.toolbars_dialog_open = true;
        } else {
            self.open_panel(id);
        }
        self.pending.push(Event::Command(id.to_owned()));
    }

    fn label<'a>(&'a self, id: &'a str) -> &'a str {
        self.labels.get(id).map_or(id, String::as_str)
    }
}

enum Typing {
    Text(String),
    Enter,
    Escape,
    Backspace,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell() -> Shell {
        Shell::new(&Spec::builtin().unwrap())
    }

    #[test]
    fn typing_collects_until_enter() {
        let mut s = shell();
        s.type_text("10");
        assert!(s.is_typing());
        s.type_text("mmx");
        s.backspace();
        assert_eq!(s.typed_text(), "10mm");
        s.type_text("\n");
        assert!(!s.is_typing());
        assert_eq!(s.pending, [Event::Measurements("10mm".into())]);
    }

    #[test]
    fn escape_cancels() {
        let mut s = shell();
        s.type_text("5");
        s.cancel_typing();
        assert!(!s.is_typing());
        s.submit();
        assert!(s.pending.is_empty());
    }

    #[test]
    fn starters() {
        assert!(Shell::starts_typing('3'));
        assert!(Shell::starts_typing('['));
        assert!(!Shell::starts_typing('L'));
    }

    /// Runs one frame with `events` and calls `capture_typing` in it.
    fn frame(
        ctx: &egui::Context,
        s: &mut Shell,
        events: Vec<egui::Event>,
        check: impl Fn(&egui::Ui),
    ) {
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            s.capture_typing(ui.ctx());
            check(ui);
        });
        out.textures_delta.clear();
    }

    #[test]
    fn capture_takes_digits_but_not_shortcut_letters() {
        let ctx = egui::Context::default();
        let mut s = shell();
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        // "L" alone is a shortcut: left alone.
        frame(
            &ctx,
            &mut s,
            vec![key(egui::Key::L), egui::Event::Text("l".into())],
            |ui| {
                assert!(ui.input(|i| i.key_pressed(egui::Key::L)));
            },
        );
        assert!(!s.is_typing());
        // "2" starts typing; the following "m" is captured, not a shortcut.
        let events = vec![
            key(egui::Key::Num2),
            egui::Event::Text("2".into()),
            key(egui::Key::M),
            egui::Event::Text("m".into()),
        ];
        frame(&ctx, &mut s, events, |ui| {
            assert!(!ui.input(|i| i.key_pressed(egui::Key::M)));
        });
        assert_eq!(s.typed_text(), "2m");
        frame(&ctx, &mut s, vec![key(egui::Key::Enter)], |_| {});
        assert_eq!(s.pending, [Event::Measurements("2m".into())]);
    }

    #[test]
    fn components_panel_draws() {
        let spec = Spec::builtin().unwrap();
        let mut s = Shell::new(&spec);
        assert!(s.open_panel("window.components"));
        s.set_tray_data(TrayData {
            components: vec![ComponentRow {
                name: "Door".into(),
                description: "Single door".into(),
                instances: 2,
            }],
            current_component: Some(0),
            ..TrayData::default()
        });
        let info = EntityInfo::no_selection();
        let frame = Frame {
            spec: &spec,
            active_tool: "tool.select",
            status: "",
            measurements_label: "Measurements",
            measurements_value: "",
            is_enabled: &|_| true,
            is_checked: &|_| false,
            entity_info: &info,
        };
        let ctx = egui::Context::default();
        let mut events = Vec::new();
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                events.extend(s.show(ui, &frame, |_| {}));
            });
            out.textures_delta.clear();
        }
        assert!(events.is_empty(), "no events without user input");
    }

    /// Drags the Large Tool Set by its grip into the viewport (it floats), then by its
    /// title bar to the bottom edge (it docks there).
    #[test]
    fn drag_toolbar_floats_and_docks() {
        let spec = Spec::builtin().unwrap();
        let mut s = Shell::new(&spec);
        let info = EntityInfo::no_selection();
        let frame = Frame {
            spec: &spec,
            active_tool: "tool.select",
            status: "",
            measurements_label: "Measurements",
            measurements_value: "",
            is_enabled: &|_| true,
            is_checked: &|_| false,
            entity_info: &info,
        };
        let ctx = egui::Context::default();
        let events = std::cell::RefCell::new(Vec::new());
        let mut run = |s: &mut Shell, input: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                events: input,
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| {
                events.borrow_mut().extend(s.show(ui, &frame, |_| {}));
            });
            out.textures_delta.clear();
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let drag = |run: &mut dyn FnMut(&mut Shell, Vec<egui::Event>),
                    s: &mut Shell,
                    from: egui::Pos2,
                    to: egui::Pos2| {
            run(s, vec![egui::Event::PointerMoved(from)]);
            run(s, vec![button(from, true)]);
            for i in 1..=5 {
                let p = from + (to - from) * (i as f32 / 5.0);
                run(s, vec![egui::Event::PointerMoved(p)]);
            }
            run(s, vec![button(to, false)]);
            run(s, vec![]);
        };
        run(&mut s, vec![]);
        let lts = s
            .dock_lines
            .iter()
            .flat_map(|l| &l.items)
            .find(|(id, _)| id == "large_tool_set")
            .map(|(_, r)| *r)
            .expect("docked left");
        let grip = egui::pos2(lts.center().x, lts.top() + 5.0);
        drag(&mut run, &mut s, grip, egui::pos2(600.0, 300.0));
        let pos = match s.toolbar_layout().get("large_tool_set").unwrap().position {
            ToolbarPosition::Floating { x, y } => egui::pos2(x, y),
            p => panic!("not floating: {p:?}"),
        };
        assert!(events.borrow().contains(&Event::ToolbarLayoutChanged));
        assert!(s.toolbars()[1].floating);

        drag(
            &mut run,
            &mut s,
            pos + egui::vec2(20.0, 8.0),
            egui::pos2(500.0, 790.0),
        );
        assert_eq!(
            s.toolbar_layout().get("large_tool_set").unwrap().position,
            ToolbarPosition::Docked {
                edge: DockEdge::Bottom,
                row: 0,
                index: 0
            }
        );
        // Restore the default layout through the public API.
        let saved = s.toolbar_layout().clone();
        s.reset_toolbar_layout();
        assert_eq!(s.toolbar_layout(), &s.toolbar_defaults);
        s.set_toolbar_layout(saved.clone());
        assert_eq!(s.toolbar_layout(), &saved);
        assert!(s.set_toolbar_visible("getting_started", false));
        assert!(!s.toolbars()[0].visible);
    }

    #[test]
    fn window_menu_command_opens_tray_panel() {
        let mut s = shell();
        assert!(!s.panel_expanded("window.materials"));
        s.command("window.materials");
        assert!(s.panel_expanded("window.materials"));
        assert_eq!(s.pending, [Event::Command("window.materials".into())]);
    }
}
