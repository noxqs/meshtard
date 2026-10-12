//! Tools > Lasso (`tool.lasso`, Shift+Space; FEATURES.md 56).
//!
//! As SketchUp's help describes it: a lasso drawn clockwise selects everything it
//! touches (like a crossing box), drawn counterclockwise only what lies fully inside
//! (like a window box). Drag for a freehand lasso (released: selected), or click
//! points for a lasso of straight segments and double-click to finish. Shift adds and
//! subtracts, Alt adds and Shift+Alt subtracts (2026 Quick Reference Card; the
//! Select tool's Ctrl adds and Ctrl+Shift removes work too). Escape drops the lasso.
//!
//! Unverified against SketchUp 2026: the outline's look (solid for inside, dashed for
//! touching, like the selection box), that the outline closes itself with a straight
//! line, and that a single click does not select the entity under it.

use eframe::egui;
use egui_wgpu::wgpu;
use glam::DVec2;

use crate::App;
use crate::nav::Tool;
use crate::selection::{self, Mode, Polygon};

/// Freehand points closer than this to the previous one are skipped (pixels).
const MIN_STEP_PX: f64 = 3.0;

/// The lasso being drawn.
#[derive(Debug, Default)]
pub(crate) struct Lasso {
    /// Outline points so far, viewport pixels.
    points: Vec<DVec2>,
    /// Drawn by dragging (finished on release); otherwise by clicks.
    freehand: bool,
}

impl Lasso {
    pub(crate) fn is_active(&self) -> bool {
        !self.points.is_empty()
    }

    fn push(&mut self, p: DVec2, min_step: f64) {
        if self.points.last().is_none_or(|q| q.distance(p) >= min_step) {
            self.points.push(p);
        }
    }

    /// The finished outline; empty when it is too small to select anything.
    fn take(&mut self) -> Polygon {
        let mut points = std::mem::take(&mut self.points);
        self.freehand = false;
        points.dedup_by(|a, b| a.distance(*b) < 0.5);
        Polygon::new(points)
    }
}

impl App {
    /// Mouse and keys of the Lasso tool.
    pub(crate) fn lasso_tool(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        viewport_px: DVec2,
        device: &wgpu::Device,
    ) {
        self.inference = None;
        let ppp = ui.ctx().pixels_per_point();
        let to_px = |p: egui::Pos2| {
            DVec2::new(
                f64::from((p.x - rect.min.x) * ppp),
                f64::from((p.y - rect.min.y) * ppp),
            )
        };
        let mods = ui.input(|i| i.modifiers);
        let mode = Mode::for_lasso(mods.command, mods.shift, mods.alt);
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.lasso = Lasso::default();
            return;
        }
        let primary = egui::PointerButton::Primary;
        if response.drag_started_by(primary)
            && !self.lasso.is_active()
            && let Some(p) = ui.input(|i| i.pointer.press_origin())
        {
            self.lasso.freehand = true;
            self.lasso.push(to_px(p), 0.0);
        }
        if self.lasso.freehand {
            if let Some(p) = response.interact_pointer_pos() {
                self.lasso.push(to_px(p), MIN_STEP_PX);
            }
            if response.drag_stopped_by(primary) {
                self.finish_lasso(device, mode, viewport_px);
            }
            return;
        }
        // Segments: each click adds a corner, a double click finishes.
        if response.clicked_by(primary)
            && let Some(p) = response.interact_pointer_pos()
        {
            self.lasso.push(to_px(p), 1.0);
        }
        if response.double_clicked_by(primary) {
            self.finish_lasso(device, mode, viewport_px);
        }
    }

    fn finish_lasso(&mut self, device: &wgpu::Device, mode: Mode, viewport_px: DVec2) {
        let polygon = self.lasso.take();
        if polygon.points.len() < 3 {
            return;
        }
        let items = selection::lasso_select(
            &self.scene,
            &self.camera,
            viewport_px,
            &polygon,
            self.prefs.more.lasso_clockwise_crossing,
        );
        self.select(device, mode, items);
    }

    /// The lasso outline, closed back to its start by a straight line through the
    /// cursor (segment mode). Solid when counterclockwise (inside only), dashed when
    /// clockwise (touching).
    pub(crate) fn draw_lasso(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
    ) {
        if self.tool != Tool::Lasso {
            self.lasso = Lasso::default();
            return;
        }
        if !self.lasso.is_active() {
            return;
        }
        let ppp = ui.ctx().pixels_per_point();
        let to_pos = |p: DVec2| rect.min + egui::vec2(p.x as f32, p.y as f32) / ppp;
        let mut pts: Vec<DVec2> = self.lasso.points.clone();
        if !self.lasso.freehand
            && let Some(c) = response.hover_pos()
        {
            let ppp = f64::from(ppp);
            pts.push(DVec2::new(
                f64::from(c.x - rect.min.x) * ppp,
                f64::from(c.y - rect.min.y) * ppp,
            ));
        }
        let crossing = selection::lasso_crossing(
            Polygon::new(pts.clone()).is_clockwise(),
            self.prefs.more.lasso_clockwise_crossing,
        );
        let mut line: Vec<egui::Pos2> = pts.iter().map(|p| to_pos(*p)).collect();
        if let Some(first) = line.first().copied() {
            line.push(first);
        }
        let stroke = egui::Stroke::new(1.0, egui::Color32::BLACK);
        let painter = ui.painter().with_clip_rect(rect);
        if crossing {
            painter.extend(egui::Shape::dashed_line(&line, stroke, 4.0, 3.0));
        } else {
            painter.add(egui::Shape::line(line, stroke));
        }
        for p in &self.lasso.points {
            if !self.lasso.freehand {
                painter.circle_filled(to_pos(*p), 2.0, egui::Color32::BLACK);
            }
        }
    }
}
