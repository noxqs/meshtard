//! Preferences > Drawing: the click style of the drawing and modify tools, "Continue
//! line drawing", Display Crosshairs and Disable Pre-Pick on Push/Pull.
//!
//! SketchUp's help lists Click-drag-release, Auto Detect (the default) and
//! Click-move-click. The help does not describe how Auto Detect tells the two apart;
//! here (from memory, unverified) a press, a drag and a release further than
//! [`AUTO_DRAG_PX`] from the press place the press and release points (click-drag-
//! release), anything shorter counts as a click (click-move-click). Click-drag-release
//! always takes the release as the second point and ends the Line tool's chain after
//! it (unverified: the help only says "Continue line drawing" applies to the other two
//! styles); Click-move-click ignores releases after a drag.

use eframe::egui;
use glam::{DVec2, DVec3};

use crate::App;
use crate::nav::Tool;

/// Preferences > Drawing > Click Style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ClickStyle {
    ClickDragRelease,
    #[default]
    AutoDetect,
    ClickMoveClick,
}

impl ClickStyle {
    pub(crate) const ALL: [(ClickStyle, &'static str, &'static str); 3] = [
        (
            ClickStyle::ClickDragRelease,
            "Click-drag-release",
            "click_drag_release",
        ),
        (ClickStyle::AutoDetect, "Auto detect", "auto_detect"),
        (
            ClickStyle::ClickMoveClick,
            "Click-move-click",
            "click_move_click",
        ),
    ];

    pub(crate) fn key(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(s, _, _)| *s == self)
            .map_or("auto_detect", |(_, _, k)| k)
    }

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(_, _, k)| *k == key)
            .map(|(s, _, _)| *s)
    }

    /// Whether the Line tool goes on from the end of a segment it just drew.
    pub(crate) fn line_continues(self, continue_line: bool) -> bool {
        self != ClickStyle::ClickDragRelease && continue_line
    }
}

/// Preferences > Drawing.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Drawing {
    pub click_style: ClickStyle,
    /// Continue line drawing (on by default; help).
    pub continue_line: bool,
    /// Display Crosshairs (off by default, unverified).
    pub crosshairs: bool,
    /// Disable Pre-Pick on Push/Pull (off by default, unverified).
    pub no_prepick: bool,
}

impl Default for Drawing {
    fn default() -> Self {
        Self {
            click_style: ClickStyle::AutoDetect,
            continue_line: true,
            crosshairs: false,
            no_prepick: false,
        }
    }
}

/// A drag shorter than this (points, between press and release) is a click when the
/// style is Auto Detect. Unverified: SketchUp's threshold is unknown.
pub(crate) const AUTO_DRAG_PX: f32 = 5.0;

/// What the primary button did this frame, as the tools take it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Gesture {
    /// A point placed by a click.
    Click,
    /// The button went down and the pointer moved: the tool's drag start.
    DragStart,
    /// The button came up after a drag: the tool's drag end (second point).
    DragEnd,
    /// Nothing for the tool.
    None,
}

/// The pointer's state between frames.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Pointer {
    /// Where the current drag's press was (screen points).
    press: Option<egui::Pos2>,
    /// The drag start changed the tool (placed its first point).
    start_placed: bool,
}

impl Pointer {
    /// Turns this frame's primary-button events into a gesture for the tools.
    /// `anchor_before` and the result of the tool's `drag_start` are reported back
    /// through [`Pointer::started`].
    pub(crate) fn gesture(
        &mut self,
        style: ClickStyle,
        response: &egui::Response,
        press_origin: Option<egui::Pos2>,
    ) -> Gesture {
        let primary = egui::PointerButton::Primary;
        if response.clicked_by(primary) {
            self.press = None;
            return Gesture::Click;
        }
        if response.drag_started_by(primary) {
            self.press = press_origin.or(response.interact_pointer_pos());
            self.start_placed = false;
            return match style {
                ClickStyle::ClickMoveClick => Gesture::Click,
                _ => Gesture::DragStart,
            };
        }
        if response.drag_stopped_by(primary) {
            let travel = match (self.press.take(), response.interact_pointer_pos()) {
                (Some(a), Some(b)) => a.distance(b),
                _ => f32::INFINITY,
            };
            return release(style, travel, self.start_placed);
        }
        Gesture::None
    }

    /// The tool's anchor changed when it took the drag start (its first point).
    pub(crate) fn started(&mut self, placed: bool) {
        self.start_placed = placed;
    }
}

/// The gesture of a release after a drag of `travel` points; `start_placed`: the drag
/// start placed a point (else the press did nothing and a short drag is a click).
pub(crate) fn release(style: ClickStyle, travel: f32, start_placed: bool) -> Gesture {
    match style {
        ClickStyle::ClickDragRelease => Gesture::DragEnd,
        ClickStyle::ClickMoveClick => Gesture::None,
        ClickStyle::AutoDetect if travel > AUTO_DRAG_PX => Gesture::DragEnd,
        ClickStyle::AutoDetect if start_placed => Gesture::None,
        ClickStyle::AutoDetect => Gesture::Click,
    }
}

impl App {
    /// Display Crosshairs, in the drawing and modify tools while the pointer is over
    /// the viewport: through the inferred point (unverified: SketchUp may center them
    /// on the cursor itself, and may show them in other tools).
    pub(crate) fn crosshairs(
        &self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        viewport_px: DVec2,
    ) {
        use crate::prefs_pages::Swatch;
        let on = self.prefs.drawing.crosshairs
            && matches!(self.tool, Tool::Draw(_) | Tool::Modify(_))
            && (response.hovered() || response.dragged());
        let (true, Some(inf)) = (on, &self.inference) else {
            return;
        };
        let ppp = ui.ctx().pixels_per_point();
        let camera = &self.camera;
        let to_screen = |p: DVec3| {
            camera
                .project(p, viewport_px)
                .map(|q| rect.min + egui::vec2(q.x as f32, q.y as f32) / ppp)
        };
        let step = (camera.eye.distance(inf.point) * 0.01).max(1e-6);
        let palette = &self.prefs.more.palette;
        paint_crosshairs(
            &ui.painter().with_clip_rect(rect),
            rect,
            to_screen,
            inf.point,
            step,
            [Swatch::X, Swatch::Y, Swatch::Z].map(|s| palette.color(s)),
        );
    }
}

/// Display Crosshairs: lines along the three axes through `p`, across the viewport, in
/// the axis colors. `to_screen` maps world points to screen points.
pub(crate) fn paint_crosshairs(
    painter: &egui::Painter,
    rect: egui::Rect,
    to_screen: impl Fn(DVec3) -> Option<egui::Pos2>,
    p: DVec3,
    step: f64,
    colors: [[u8; 3]; 3],
) {
    let Some(c) = to_screen(p) else {
        return;
    };
    let reach = rect.size().length();
    for (axis, color) in [DVec3::X, DVec3::Y, DVec3::Z].into_iter().zip(colors) {
        let Some(q) = to_screen(p + axis * step) else {
            continue;
        };
        let Some((a, b)) = screen_line(c, q, reach) else {
            continue;
        };
        let stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(color[0], color[1], color[2]));
        painter.line_segment([a, b], stroke);
    }
}

/// The segment through `c` towards `q`, `reach` long each way; `None` when the two
/// points are (almost) the same (an axis pointing at the viewer).
pub(crate) fn screen_line(
    c: egui::Pos2,
    q: egui::Pos2,
    reach: f32,
) -> Option<(egui::Pos2, egui::Pos2)> {
    let d = DVec2::new(f64::from(q.x - c.x), f64::from(q.y - c.y));
    if d.length() < 1e-3 {
        return None;
    }
    let d = d.normalize();
    let v = egui::vec2(d.x as f32, d.y as f32) * reach;
    Some((c - v, c + v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_detect_tells_drags_from_clicks() {
        let s = ClickStyle::AutoDetect;
        assert_eq!(release(s, 40.0, true), Gesture::DragEnd);
        // A press held still is a click: the press placed the point already.
        assert_eq!(release(s, 2.0, true), Gesture::None);
        // The press did nothing (Line with a start point): the release is the click.
        assert_eq!(release(s, 2.0, false), Gesture::Click);
        assert_eq!(
            release(ClickStyle::ClickDragRelease, 1.0, true),
            Gesture::DragEnd
        );
        assert_eq!(
            release(ClickStyle::ClickMoveClick, 80.0, true),
            Gesture::None
        );
    }

    #[test]
    fn styles_round_trip_and_line_chain() {
        for (s, _, k) in ClickStyle::ALL {
            assert_eq!(ClickStyle::from_key(k), Some(s));
            assert_eq!(s.key(), k);
        }
        assert_eq!(ClickStyle::from_key("nope"), None);
        assert!(ClickStyle::AutoDetect.line_continues(true));
        assert!(!ClickStyle::AutoDetect.line_continues(false));
        assert!(ClickStyle::ClickMoveClick.line_continues(true));
        assert!(!ClickStyle::ClickDragRelease.line_continues(true));
    }

    #[test]
    fn crosshair_lines_cross_the_point() {
        let c = egui::pos2(100.0, 50.0);
        let (a, b) = screen_line(c, egui::pos2(110.0, 50.0), 300.0).unwrap();
        assert_eq!(a, egui::pos2(-200.0, 50.0));
        assert_eq!(b, egui::pos2(400.0, 50.0));
        assert!(screen_line(c, c, 300.0).is_none());
    }
}
