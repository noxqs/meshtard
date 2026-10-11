//! Drawing-tool overlay: preview geometry, inference marker and tooltip.

use glam::DVec3;
use mt_infer::{Inference, Kind};

fn color32(c: [u8; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(c[0], c[1], c[2])
}

/// Draws the tool's preview polylines, the guide line, the inference marker and its
/// tooltip. A single preview segment from `anchor` is the rubber band and takes the
/// inference's color when it follows a direction.
pub fn paint(
    painter: &egui::Painter,
    to_screen: impl Fn(DVec3) -> Option<egui::Pos2>,
    anchor: Option<DVec3>,
    previews: &[Vec<DVec3>],
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
    let directional = matches!(
        inf.kind,
        Kind::Along(_) | Kind::Parallel | Kind::Perpendicular | Kind::FromPoint(_)
    );
    for line in previews {
        let rubber_band = previews.len() == 1 && line.len() == 2 && Some(line[0]) == anchor;
        let (color, width) = if rubber_band && directional {
            (
                color32(inf.kind.color()),
                if inf.locked { 3.0 } else { 2.0 },
            )
        } else if inf.kind.is_hint() {
            // Shape hints (Square, Half Circle, Tangent at Vertex...) tint the shape.
            (color32(inf.kind.color()), 2.0)
        } else {
            (egui::Color32::BLACK, 1.5)
        };
        let pts: Vec<egui::Pos2> = line.iter().filter_map(|p| to_screen(*p)).collect();
        if pts.len() == line.len() {
            painter.add(egui::Shape::line(pts, egui::Stroke::new(width, color)));
        }
    }

    // Marker: dot for points, square for "on" inferences, diamond on faces.
    let fill = color32(inf.kind.color());
    let outline = egui::Stroke::new(1.0, egui::Color32::BLACK);
    match inf.kind {
        Kind::Endpoint | Kind::Midpoint | Kind::Origin | Kind::GuidePoint | Kind::Center => {
            painter.circle(p, 5.0, fill, outline);
        }
        Kind::Intersection => {
            // A cross (SketchUp's intersection marker; unverified).
            let d = 4.5;
            let stroke = egui::Stroke::new(2.0, fill);
            painter.line_segment([p + egui::vec2(-d, -d), p + egui::vec2(d, d)], stroke);
            painter.line_segment([p + egui::vec2(-d, d), p + egui::vec2(d, -d)], stroke);
        }
        Kind::OnEdge | Kind::OnAxis(_) | Kind::OnGuide => {
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
        Kind::Free
        | Kind::Along(_)
        | Kind::Parallel
        | Kind::Perpendicular
        | Kind::FromPoint(_)
        | Kind::Square
        | Kind::GoldenSection
        | Kind::HalfCircle
        | Kind::Tangent => {
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

/// Scale grips: small green squares, red under the cursor or while held (SketchUp's
/// colors from memory; unverified).
pub fn markers(
    painter: &egui::Painter,
    to_screen: impl Fn(DVec3) -> Option<egui::Pos2>,
    markers: &[(DVec3, bool)],
) {
    for (p, hot) in markers {
        let Some(q) = to_screen(*p) else { continue };
        let (size, fill) = if *hot {
            (9.0, egui::Color32::from_rgb(230, 30, 30))
        } else {
            (7.0, egui::Color32::from_rgb(40, 180, 40))
        };
        painter.rect(
            egui::Rect::from_center_size(q, egui::vec2(size, size)),
            0.0,
            fill,
            egui::Stroke::new(1.0, egui::Color32::BLACK),
            egui::StrokeKind::Middle,
        );
    }
}
