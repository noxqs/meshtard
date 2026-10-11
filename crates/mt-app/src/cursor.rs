//! The viewport's mouse cursor per tool, modifier and state (FEATURES.md 41).
//!
//! [`cursor_for`] picks the drawing and badge; `mt_ui::cursors` draws it at the
//! pointer in place of the hidden system cursor. Outside the viewport the system
//! cursors stay. All choices are from memory of SketchUp and unverified (the trial
//! records the real cursors, TRIAL_CHECKLIST.md 1c).

use eframe::egui;
use glam::DVec2;
use mt_ui::cursors::{Badge, Cursor, Kind};

use crate::App;
use crate::document::Owner;
use crate::nav::Tool;
use crate::selection::{self, Mode};

/// Modifier keys held (Ctrl is Cmd on macOS, as everywhere in the app).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// A middle-button drag navigating the view in any tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavDrag {
    Orbit,
    Pan,
}

/// The active tool's state that changes the cursor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct State {
    /// Move / Rotate copy or Push/Pull new faces is toggled on (Ctrl).
    pub copy: bool,
    /// The entity under the cursor is not a valid target.
    pub invalid: bool,
    /// Solid Tools: the first solid is picked.
    pub second: bool,
    pub nav: Option<NavDrag>,
}

/// The cursor for `tool` with `mods` held in `state`.
pub fn cursor_for(tool: Tool, mods: Mods, state: State) -> Cursor {
    match state.nav {
        Some(NavDrag::Orbit) => return Cursor::new(Kind::Orbit),
        Some(NavDrag::Pan) => return Cursor::new(Kind::Pan),
        None => {}
    }
    let copy = if state.copy { Badge::Plus } else { Badge::None };
    match tool {
        Tool::Select => Cursor::with(
            Kind::Arrow,
            match Mode::from_modifiers(mods.ctrl, mods.shift) {
                Mode::Replace => Badge::None,
                Mode::Add => Badge::Plus,
                Mode::Toggle => Badge::PlusMinus,
                Mode::Remove => Badge::Minus,
            },
        ),
        Tool::Draw(id) => Cursor::new(match id {
            "tool.rectangle" => Kind::Rectangle,
            "tool.rotated_rectangle" => Kind::RotatedRectangle,
            "tool.circle" => Kind::Circle,
            "tool.polygon" => Kind::Polygon,
            "tool.arc" => Kind::Arc,
            "tool.arc_2pt" => Kind::Arc2Point,
            "tool.arc_3pt" => Kind::Arc3Point,
            "tool.pie" => Kind::Pie,
            "tool.freehand" => Kind::Freehand,
            "tool.tape_measure" => Kind::TapeMeasure,
            "tool.protractor" => Kind::Protractor,
            "tool.dimensions" => Kind::Dimension,
            "tool.text" => Kind::Text,
            // tool.line and any other drawing tool.
            _ => Kind::Pencil,
        }),
        Tool::Modify(id) => match id {
            "tool.push_pull" if state.invalid => Cursor::with(Kind::PushPull, Badge::NotAllowed),
            "tool.push_pull" => Cursor::with(Kind::PushPull, copy),
            "tool.move" => Cursor::with(Kind::Move, copy),
            "tool.rotate" => Cursor::with(Kind::Rotate, copy),
            // Placing pasted entities, components and images: the Move cursor.
            "edit.paste" | "tool.place_component" | "tool.place_image" => Cursor::new(Kind::Move),
            "tool.scale" => Cursor::new(Kind::Scale),
            "tool.offset" => Cursor::new(Kind::Offset),
            "tool.follow_me" => Cursor::new(Kind::FollowMe),
            "tool.paint_bucket" if mods.alt => Cursor::new(Kind::Eyedropper),
            "tool.paint_bucket" => Cursor::new(Kind::PaintBucket),
            "tool.section_plane" => Cursor::new(Kind::SectionPlane),
            "tool.axes" | "tool.component_axes" => Cursor::new(Kind::Axes),
            "tool.sandbox_from_scratch" => Cursor::new(Kind::Sandbox),
            _ => Cursor::new(Kind::Arrow),
        },
        Tool::Eraser => Cursor::with(
            Kind::Eraser,
            match (mods.ctrl, mods.shift) {
                (false, false) => Badge::None,
                (false, true) => Badge::Hide,
                (true, false) => Badge::Soften,
                (true, true) => Badge::Unsoften,
            },
        ),
        Tool::Pick("tool.tag") if mods.alt => Cursor::new(Kind::Eyedropper),
        Tool::Pick("tool.tag") => Cursor::new(Kind::Tag),
        Tool::Pick(id) => Cursor::with(
            match id {
                "tool.outer_shell" => Kind::OuterShell,
                "tool.solid_intersect" => Kind::Intersect,
                "tool.solid_union" => Kind::Union,
                "tool.solid_subtract" => Kind::Subtract,
                "tool.solid_trim" => Kind::Trim,
                "tool.solid_split" => Kind::Split,
                _ => Kind::Arrow,
            },
            if state.invalid {
                Badge::NotAllowed
            } else if state.second {
                Badge::Two
            } else {
                Badge::One
            },
        ),
        // Orbit with Shift pans (see the status hint).
        Tool::Orbit if mods.shift => Cursor::new(Kind::Pan),
        Tool::Orbit => Cursor::new(Kind::Orbit),
        Tool::Pan => Cursor::new(Kind::Pan),
        Tool::Zoom => Cursor::new(Kind::Zoom),
        Tool::ZoomWindow => Cursor::new(Kind::ZoomWindow),
        Tool::PositionCamera => Cursor::new(Kind::PositionCamera),
        Tool::Walk => Cursor::new(Kind::Walk),
        Tool::LookAround => Cursor::new(Kind::LookAround),
        Tool::FieldOfView => Cursor::new(Kind::FieldOfView),
    }
}

impl App {
    /// Paints the tool cursor while the pointer is over the viewport or a drag that
    /// started there goes on (call after the tool handled the frame).
    pub(crate) fn tool_cursor(
        &self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        viewport_px: DVec2,
    ) {
        let pointer = if response.dragged() {
            response
                .interact_pointer_pos()
                .or(ui.ctx().pointer_latest_pos())
        } else if response.hovered() {
            response.hover_pos()
        } else {
            None
        };
        let Some(pos) = pointer else {
            return;
        };
        let m = ui.input(|i| i.modifiers);
        let mods = Mods {
            ctrl: m.command,
            shift: m.shift,
            alt: m.alt,
        };
        let nav = response
            .dragged_by(egui::PointerButton::Middle)
            .then_some(if m.shift {
                NavDrag::Pan
            } else {
                NavDrag::Orbit
            });
        let modify = match self.tool {
            Tool::Modify(_) => self.modify.as_deref(),
            _ => None,
        };
        let pick = match self.tool {
            Tool::Pick(_) => self.pick.as_ref(),
            _ => None,
        };
        let mut invalid = pick.is_some_and(|p| p.hover_invalid());
        if modify.is_some_and(|t| t.needs_face()) && response.hovered() {
            let ppp = ui.ctx().pixels_per_point();
            let c = DVec2::new(
                f64::from((pos.x - rect.min.x) * ppp),
                f64::from((pos.y - rect.min.y) * ppp),
            );
            let hit = selection::pick(&self.scene, self.bvh.as_ref(), &self.camera, c, viewport_px);
            invalid = !matches!(hit, Some(Owner::Face(_)));
        }
        let state = State {
            copy: modify.is_some_and(|t| t.copy_mode()),
            invalid,
            second: pick.is_some_and(|p| p.first.is_some()),
            nav,
        };
        mt_ui::cursors::show(ui.ctx(), pos, cursor_for(self.tool, mods, state));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every tool the app has: every command `Tool::from_command` knows plus the
    /// modify tools started without a tool command.
    fn all_tools() -> Vec<Tool> {
        let ids = [
            "tool.select",
            "tool.eraser",
            "tool.line",
            "tool.rectangle",
            "tool.rotated_rectangle",
            "tool.circle",
            "tool.polygon",
            "tool.arc",
            "tool.arc_2pt",
            "tool.arc_3pt",
            "tool.pie",
            "tool.freehand",
            "tool.tape_measure",
            "tool.protractor",
            "tool.dimensions",
            "tool.text",
            "tool.push_pull",
            "tool.move",
            "tool.rotate",
            "tool.scale",
            "tool.offset",
            "tool.follow_me",
            "tool.paint_bucket",
            "tool.section_plane",
            "tool.axes",
            "tool.sandbox_from_scratch",
            "tool.tag",
            "tool.outer_shell",
            "tool.solid_intersect",
            "tool.solid_union",
            "tool.solid_subtract",
            "tool.solid_trim",
            "tool.solid_split",
            "camera.orbit",
            "camera.pan",
            "camera.zoom",
            "camera.zoom_window",
            "camera.position",
            "camera.walk",
            "camera.look_around",
            "camera.field_of_view",
        ];
        let mut tools: Vec<Tool> = ids
            .iter()
            .map(|id| Tool::from_command(id).unwrap_or_else(|| panic!("{id}")))
            .collect();
        for id in [
            "edit.paste",
            "tool.place_component",
            "tool.place_image",
            "tool.component_axes",
        ] {
            tools.push(Tool::Modify(id));
        }
        tools
    }

    /// Fails to compile when `Tool` gets a variant, so the list above is revisited.
    fn variant_name(t: Tool) -> &'static str {
        match t {
            Tool::Select => "Select",
            Tool::Draw(_) => "Draw",
            Tool::Modify(_) => "Modify",
            Tool::Eraser => "Eraser",
            Tool::Pick(_) => "Pick",
            Tool::Orbit => "Orbit",
            Tool::Pan => "Pan",
            Tool::Zoom => "Zoom",
            Tool::ZoomWindow => "ZoomWindow",
            Tool::PositionCamera => "PositionCamera",
            Tool::Walk => "Walk",
            Tool::LookAround => "LookAround",
            Tool::FieldOfView => "FieldOfView",
        }
    }

    const NONE: Mods = Mods {
        ctrl: false,
        shift: false,
        alt: false,
    };
    const CTRL: Mods = Mods { ctrl: true, ..NONE };
    const SHIFT: Mods = Mods {
        shift: true,
        ..NONE
    };
    const CTRL_SHIFT: Mods = Mods {
        ctrl: true,
        shift: true,
        alt: false,
    };
    const ALT: Mods = Mods { alt: true, ..NONE };

    fn kind(tool: Tool) -> Kind {
        cursor_for(tool, NONE, State::default()).kind
    }

    #[test]
    fn every_tool_has_its_own_cursor() {
        let tools = all_tools();
        let variants: std::collections::HashSet<_> =
            tools.iter().map(|&t| variant_name(t)).collect();
        assert_eq!(
            variants.len(),
            13,
            "a Tool variant is missing from all_tools"
        );
        for &t in &tools {
            let k = kind(t);
            assert!(
                t == Tool::Select || k != Kind::Arrow,
                "{t:?} falls back to the arrow"
            );
            // The cursor stays a cursor under every modifier and state.
            for mods in [NONE, CTRL, SHIFT, CTRL_SHIFT, ALT] {
                for copy in [false, true] {
                    for invalid in [false, true] {
                        for second in [false, true] {
                            let s = State {
                                copy,
                                invalid,
                                second,
                                nav: None,
                            };
                            let c = cursor_for(t, mods, s);
                            assert!(t == Tool::Select || c.kind != Kind::Arrow, "{t:?}");
                        }
                    }
                }
            }
        }
        // Each drawing tool has a different cursor from the others (Line and Freehand
        // included).
        let mut seen = std::collections::HashMap::new();
        for &t in &tools {
            if let Some(prev) = seen.insert(kind(t), t) {
                let shared = matches!(
                    t.command(),
                    "edit.paste"
                        | "tool.place_component"
                        | "tool.place_image"
                        | "tool.component_axes"
                );
                assert!(shared, "{t:?} has the cursor of {prev:?}");
            }
        }
    }

    #[test]
    fn expected_cursors() {
        let t = |id| Tool::from_command(id).unwrap();
        let k = |id| kind(t(id));
        assert_eq!(k("tool.select"), Kind::Arrow);
        assert_eq!(k("tool.line"), Kind::Pencil);
        assert_eq!(k("tool.rectangle"), Kind::Rectangle);
        assert_eq!(k("tool.eraser"), Kind::Eraser);
        assert_eq!(k("tool.paint_bucket"), Kind::PaintBucket);
        assert_eq!(k("tool.move"), Kind::Move);
        assert_eq!(k("tool.rotate"), Kind::Rotate);
        assert_eq!(k("tool.push_pull"), Kind::PushPull);
        assert_eq!(k("tool.tape_measure"), Kind::TapeMeasure);
        assert_eq!(k("camera.orbit"), Kind::Orbit);
        assert_eq!(k("camera.pan"), Kind::Pan);
        assert_eq!(k("camera.zoom"), Kind::Zoom);
        assert_eq!(k("camera.walk"), Kind::Walk);
        assert_eq!(k("camera.look_around"), Kind::LookAround);
        assert_eq!(k("camera.position"), Kind::PositionCamera);
        assert_eq!(k("tool.solid_union"), Kind::Union);
        assert_eq!(kind(Tool::Modify("edit.paste")), Kind::Move);
    }

    #[test]
    fn select_badges_follow_the_selection_modes() {
        let s = |m| cursor_for(Tool::Select, m, State::default()).badge;
        assert_eq!(s(NONE), Badge::None);
        // Mode::from_modifiers: Ctrl adds, Shift toggles, Ctrl+Shift removes.
        assert_eq!(s(CTRL), Badge::Plus);
        assert_eq!(s(SHIFT), Badge::PlusMinus);
        assert_eq!(s(CTRL_SHIFT), Badge::Minus);
        assert_eq!(s(ALT), Badge::None);
    }

    #[test]
    fn modify_badges_follow_the_ctrl_toggle() {
        let on = State {
            copy: true,
            ..State::default()
        };
        for id in ["tool.move", "tool.rotate", "tool.push_pull"] {
            let t = Tool::from_command(id).unwrap();
            assert_eq!(cursor_for(t, NONE, State::default()).badge, Badge::None);
            assert_eq!(cursor_for(t, NONE, on).badge, Badge::Plus, "{id}");
        }
        // The toggle, not the key held, decides.
        let mv = Tool::from_command("tool.move").unwrap();
        assert_eq!(cursor_for(mv, CTRL, State::default()).badge, Badge::None);
        let invalid = State {
            invalid: true,
            copy: true,
            ..State::default()
        };
        let pp = Tool::from_command("tool.push_pull").unwrap();
        assert_eq!(cursor_for(pp, NONE, invalid).badge, Badge::NotAllowed);
    }

    #[test]
    fn eraser_paint_and_tag_modifiers() {
        let e = |m| cursor_for(Tool::Eraser, m, State::default()).badge;
        assert_eq!(e(NONE), Badge::None);
        assert_eq!(e(SHIFT), Badge::Hide);
        assert_eq!(e(CTRL), Badge::Soften);
        assert_eq!(e(CTRL_SHIFT), Badge::Unsoften);
        let pb = Tool::from_command("tool.paint_bucket").unwrap();
        assert_eq!(cursor_for(pb, ALT, State::default()).kind, Kind::Eyedropper);
        let tag = Tool::from_command("tool.tag").unwrap();
        assert_eq!(
            cursor_for(tag, ALT, State::default()).kind,
            Kind::Eyedropper
        );
        assert_eq!(cursor_for(tag, NONE, State::default()).kind, Kind::Tag);
    }

    #[test]
    fn solid_tools_badges() {
        let t = Tool::from_command("tool.solid_subtract").unwrap();
        let c = |s| cursor_for(t, NONE, s);
        assert_eq!(
            c(State::default()),
            Cursor::with(Kind::Subtract, Badge::One)
        );
        let second = State {
            second: true,
            ..State::default()
        };
        assert_eq!(c(second).badge, Badge::Two);
        let bad = State {
            invalid: true,
            second: true,
            ..State::default()
        };
        assert_eq!(c(bad).badge, Badge::NotAllowed);
    }

    #[test]
    fn middle_drag_shows_orbit_or_pan_in_any_tool() {
        for t in all_tools() {
            let orbit = State {
                nav: Some(NavDrag::Orbit),
                ..State::default()
            };
            let pan = State {
                nav: Some(NavDrag::Pan),
                ..State::default()
            };
            assert_eq!(cursor_for(t, NONE, orbit).kind, Kind::Orbit);
            assert_eq!(cursor_for(t, SHIFT, pan).kind, Kind::Pan);
        }
        let orbit = Tool::from_command("camera.orbit").unwrap();
        assert_eq!(cursor_for(orbit, SHIFT, State::default()).kind, Kind::Pan);
    }

    #[test]
    fn every_cursor_kind_is_used_by_some_tool() {
        let mut used = std::collections::HashSet::new();
        for t in all_tools() {
            for mods in [NONE, ALT] {
                used.insert(cursor_for(t, mods, State::default()).kind);
            }
        }
        for k in Kind::ALL {
            assert!(used.contains(k), "{k:?} is never shown");
        }
    }
}
