//! The Line tool: click a start point, click end points; each segment continues from the
//! last end, until a face closes or Esc. Typing a length in the Measurements box makes
//! the next segment that long in the current direction.

use glam::DVec3;
use mt_infer::{Inference, Kind};

#[derive(Debug, Default)]
pub struct LineTool {
    /// Start of the segment being drawn.
    pub anchor: Option<DVec3>,
}

impl LineTool {
    pub fn status(&self) -> &'static str {
        if self.anchor.is_some() {
            "Select end point or enter value."
        } else {
            "Select start point."
        }
    }

    /// Length of the segment being drawn.
    pub fn length(&self, current: Option<&Inference>) -> Option<f64> {
        Some(current?.point.distance(self.anchor?))
    }

    /// End point for a typed length along the current direction.
    pub fn typed_end(&self, current: Option<&Inference>, length: f64) -> Option<DVec3> {
        let a = self.anchor?;
        let d = (current?.point - a).normalize_or_zero();
        (d != DVec3::ZERO && length != 0.0).then(|| a + d * length)
    }
}

fn color32(c: [u8; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(c[0], c[1], c[2])
}

/// Draws the rubber band, guide line, inference marker and tooltip.
pub fn paint(
    painter: &egui::Painter,
    to_screen: impl Fn(DVec3) -> Option<egui::Pos2>,
    anchor: Option<DVec3>,
    inf: &Inference,
) {
    let Some(p) = to_screen(inf.point) else {
        return;
    };
    if let Some((from, to)) = inf.guide
        && let (Some(a), Some(b)) = (to_screen(from), to_screen(to))
    {
        let stroke = egui::Stroke::new(1.0, color32(inf.kind.color()));
        painter.extend(egui::Shape::dashed_line(&[a, b], stroke, 3.0, 3.0));
    }
    if let Some(a) = anchor.and_then(&to_screen) {
        let directional = matches!(
            inf.kind,
            Kind::Along(_) | Kind::Parallel | Kind::Perpendicular | Kind::FromPoint(_)
        );
        let (color, width) = if directional {
            (
                color32(inf.kind.color()),
                if inf.locked { 3.0 } else { 2.0 },
            )
        } else {
            (egui::Color32::BLACK, 1.5)
        };
        painter.line_segment([a, p], egui::Stroke::new(width, color));
    }

    // Marker: dot for points, square for "on" inferences, diamond on faces.
    let fill = color32(inf.kind.color());
    let outline = egui::Stroke::new(1.0, egui::Color32::BLACK);
    match inf.kind {
        Kind::Endpoint | Kind::Midpoint | Kind::Origin => {
            painter.circle(p, 5.0, fill, outline);
        }
        Kind::OnEdge | Kind::OnAxis(_) => {
            painter.rect(
                egui::Rect::from_center_size(p, egui::vec2(8.0, 8.0)),
                0.0,
                fill,
                outline,
                egui::StrokeKind::Middle,
            );
        }
        Kind::OnFace => {
            let d = 6.0;
            let pts = vec![
                p + egui::vec2(0.0, -d),
                p + egui::vec2(d, 0.0),
                p + egui::vec2(0.0, d),
                p + egui::vec2(-d, 0.0),
            ];
            painter.add(egui::Shape::convex_polygon(pts, fill, outline));
        }
        Kind::Free | Kind::Along(_) | Kind::Parallel | Kind::Perpendicular | Kind::FromPoint(_) => {
            painter.circle_stroke(p, 2.5, egui::Stroke::new(1.0, egui::Color32::BLACK));
        }
    }

    // Tooltip, like SketchUp's yellow inference tooltip.
    let label = inf.kind.label();
    if !label.is_empty() {
        let text = if inf.locked {
            format!("{label} (locked)")
        } else {
            label
        };
        let galley =
            painter.layout_no_wrap(text, egui::FontId::proportional(12.0), egui::Color32::BLACK);
        let pos = p + egui::vec2(12.0, 12.0);
        let rect = egui::Rect::from_min_size(pos, galley.size() + egui::vec2(8.0, 4.0));
        painter.rect(
            rect,
            2.0,
            egui::Color32::from_rgb(255, 255, 225),
            outline,
            egui::StrokeKind::Inside,
        );
        painter.galley(pos + egui::vec2(4.0, 2.0), galley, egui::Color32::BLACK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_length_follows_current_direction() {
        let tool = LineTool {
            anchor: Some(DVec3::ZERO),
        };
        let cur = Inference {
            point: DVec3::new(3.0, 4.0, 0.0),
            kind: Kind::Free,
            edge: None,
            guide: None,
            locked: false,
        };
        assert_eq!(tool.length(Some(&cur)), Some(5.0));
        let end = tool.typed_end(Some(&cur), 10.0).unwrap();
        assert!((end - DVec3::new(6.0, 8.0, 0.0)).length() < 1e-12);
    }
}
