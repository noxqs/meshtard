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
//! for event in shell.show(ui, &frame, |ui| { /* draw the 3D view in ui */ }) {
//!     match event {
//!         mt_ui::Event::Command(id) => { /* run the command */ }
//!         mt_ui::Event::Measurements(text) => { /* parse the typed value */ }
//!     }
//! }
//! # }
//! ```
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

pub mod icons;
mod menu;
mod status;
mod theme;
mod toolbar;
mod tray;

use std::collections::{BTreeMap, HashMap};

use mt_spec::Spec;

pub use theme::apply as apply_theme;

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
pub enum Event {
    /// A menu item or toolbar button was clicked: a command id from `menus.toml`.
    Command(String),
    /// Text entered in the Measurements box and confirmed with Enter.
    Measurements(String),
}

/// Characters that start Measurements typing while the viewport has focus.
const TYPING_STARTERS: &str = ".,;-+[<'\"/*@=";

/// UI state of the main window: tray panels, flyout choices, Measurements text.
pub struct Shell {
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
        Self {
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
        egui::Panel::top("mt_ui_menu_bar")
            .frame(
                egui::Frame::NONE
                    .fill(t.menu_bar)
                    .inner_margin(egui::Margin::symmetric(2, 0)),
            )
            .exact_size(t.sizes.menu_bar_height)
            .show_separator_line(false)
            .show(ui, |ui| self.menu_bar(ui, frame));

        self.top_toolbars(ui, frame);

        egui::Panel::bottom("mt_ui_status_bar")
            .frame(
                egui::Frame::NONE
                    .fill(t.status_bar)
                    .inner_margin(egui::Margin::symmetric(6, 0))
                    .stroke(egui::Stroke::new(1.0, t.separator)),
            )
            .exact_size(t.sizes.status_bar_height)
            .show_separator_line(false)
            .show(ui, |ui| self.status_bar(ui, frame));

        self.left_toolbars(ui, frame);

        if self.tray_visible {
            egui::Panel::right("mt_ui_tray")
                .frame(egui::Frame::NONE.fill(t.tray))
                .default_size(frame.spec.tray.width)
                .size_range(180.0..=600.0)
                .resizable(true)
                .show_separator_line(true)
                .show(ui, |ui| self.tray(ui, frame));
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, viewport);

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

    /// Records a command chosen in the menu bar or a toolbar, and handles the ones
    /// that only change the shell (tray panels).
    fn command(&mut self, id: &str) {
        if id == "window.tray_show" {
            self.tray_visible = !self.tray_visible;
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
    fn window_menu_command_opens_tray_panel() {
        let mut s = shell();
        assert!(!s.panel_expanded("window.materials"));
        s.command("window.materials");
        assert!(s.panel_expanded("window.materials"));
        assert_eq!(s.pending, [Event::Command("window.materials".into())]);
    }
}
