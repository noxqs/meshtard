//! Annotations: dimensions (Dimension tool) and texts (Text tool). Like guides they are
//! not geometry: they never split edges or bound faces.
//!
//! Only the measured points are stored; a dimension's label is formatted in the
//! model's units when drawn (unless its text is overridden).

use glam::DVec3;
use mt_skp as skp;

/// What a dimension measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DimensionKind {
    /// The distance between `a` and `b`, drawn parallel to `b - a`.
    #[default]
    Linear,
    /// An arc's radius: `a` is the center, `b` a point on the arc.
    Radius,
    /// A circle's diameter: `a` and `b` are opposite points on it.
    Diameter,
}

/// A dimension between two points.
///
/// Linear: extension lines run from `a` and `b` to `a + offset` and `b + offset`, and
/// the dimension line joins their ends. Radial (radius, diameter): the dimension line
/// is `a`-`b` and a leader runs from `b` to the label at `b + offset`.
#[derive(Debug, Clone, PartialEq)]
pub struct Dimension {
    pub a: DVec3,
    pub b: DVec3,
    pub offset: DVec3,
    /// Text shown instead of the measured value.
    pub text: Option<String>,
    pub kind: DimensionKind,
    pub hidden: bool,
}

impl Dimension {
    pub fn linear(a: DVec3, b: DVec3, offset: DVec3) -> Self {
        Self {
            a,
            b,
            offset,
            text: None,
            kind: DimensionKind::Linear,
            hidden: false,
        }
    }

    /// The measured value in inches.
    pub fn value(&self) -> f64 {
        self.a.distance(self.b)
    }

    /// The dimension line's ends.
    pub fn line(&self) -> [DVec3; 2] {
        match self.kind {
            DimensionKind::Linear => [self.a + self.offset, self.b + self.offset],
            DimensionKind::Radius | DimensionKind::Diameter => [self.a, self.b],
        }
    }

    /// Where the label sits: the middle of the dimension line, or the end of the
    /// leader of a radial dimension.
    pub fn label_point(&self) -> DVec3 {
        match self.kind {
            DimensionKind::Linear => (self.a + self.b) / 2.0 + self.offset,
            DimensionKind::Radius | DimensionKind::Diameter => self.b + self.offset,
        }
    }

    /// Line segments to draw: extension lines and dimension line (linear), or the
    /// dimension line and the leader (radial). Extension lines overshoot the
    /// dimension line a little (`overshoot`, inches; SketchUp's amount is unverified).
    pub fn segments(&self, overshoot: f64) -> Vec<[DVec3; 2]> {
        let [p, q] = self.line();
        match self.kind {
            DimensionKind::Linear => {
                let mut out = vec![[p, q]];
                let len = self.offset.length();
                if len > crate::TOL {
                    let over = self.offset / len * overshoot;
                    out.push([self.a, p + over]);
                    out.push([self.b, q + over]);
                }
                out
            }
            DimensionKind::Radius | DimensionKind::Diameter => {
                let mut out = vec![[p, q]];
                if self.offset.length() > crate::TOL {
                    out.push([self.b, self.b + self.offset]);
                }
                out
            }
        }
    }

    /// The offset of a linear dimension from `a`-`b` towards `cursor`: the part of
    /// `cursor - a` perpendicular to the measured segment.
    pub fn offset_towards(a: DVec3, b: DVec3, cursor: DVec3) -> DVec3 {
        let d = (b - a).normalize_or_zero();
        let v = cursor - a;
        v - d * v.dot(d)
    }
}

/// A text: a leader text pointing at `point` with its label at `point + leader`, or a
/// screen text fixed in the view.
#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    /// The point the leader points at. For a screen text, `x` and `y` are the label's
    /// position as fractions of the view's width and height (top left is 0, 0).
    pub point: DVec3,
    /// From `point` to the label (zero for a screen text).
    pub leader: DVec3,
    pub text: String,
    pub screen: bool,
    pub hidden: bool,
}

impl Text {
    pub fn leader(point: DVec3, leader: DVec3, text: String) -> Self {
        Self {
            point,
            leader,
            text,
            screen: false,
            hidden: false,
        }
    }

    pub fn screen(x: f64, y: f64, text: String) -> Self {
        Self {
            point: DVec3::new(x, y, 0.0),
            leader: DVec3::ZERO,
            text,
            screen: true,
            hidden: false,
        }
    }
}

/// Model-level dimensions of a `.skp` with known endpoints. The offset direction
/// (the plane's y axis, `normal x plane_x`) is a reading of the format that is not
/// verified against SketchUp; without a plane the offset goes sideways in the ground
/// plane.
pub fn dimensions_from_skp(dims: &[skp::Dimension]) -> Vec<Dimension> {
    dims.iter()
        .filter(|d| !d.hidden)
        .filter_map(|d| {
            let (a, b) = (DVec3::from(d.a?), DVec3::from(d.b?));
            if a.distance(b) <= crate::TOL {
                return None;
            }
            let along = (b - a).normalize();
            let dir = match (d.plane_x, d.normal) {
                (Some(x), Some(n)) => DVec3::from(n).cross(DVec3::from(x)),
                _ => DVec3::Z.cross(along),
            };
            // Keep the offset perpendicular to the measured segment.
            let dir = (dir - along * dir.dot(along)).normalize_or_zero();
            Some(Dimension {
                text: (!d.text.is_empty()).then(|| d.text.clone()),
                ..Dimension::linear(a, b, dir * d.offset)
            })
        })
        .collect()
}

/// Writes root dimensions and leader texts. The writer has no text override for
/// dimensions, no radial dimensions (they are written as linear ones between their
/// points) and no screen texts (left out); the offset is written as a distance, its
/// direction chosen by SketchUp (unverified which).
pub fn write_root(
    b: &mut skp::write::SkpBuilder,
    ents: &crate::Entities,
) -> Result<(), skp::write::WriteError> {
    for d in &ents.dimensions {
        if d.a.distance(d.b) > crate::TOL {
            b.add_dimension(d.a.to_array(), d.b.to_array(), d.offset.length())?;
        }
    }
    for t in ents.texts.iter().filter(|t| !t.screen) {
        b.add_text(&t.text, t.point.to_array(), t.leader.to_array())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_layout() {
        let d = Dimension::linear(DVec3::ZERO, DVec3::X * 10.0, DVec3::Y * 4.0);
        assert_eq!(d.value(), 10.0);
        assert_eq!(d.line(), [DVec3::Y * 4.0, DVec3::new(10.0, 4.0, 0.0)]);
        assert_eq!(d.label_point(), DVec3::new(5.0, 4.0, 0.0));
        let s = d.segments(1.0);
        assert_eq!(s.len(), 3);
        assert_eq!(s[1], [DVec3::ZERO, DVec3::Y * 5.0], "extension overshoots");
    }

    #[test]
    fn offset_is_perpendicular() {
        let o = Dimension::offset_towards(DVec3::ZERO, DVec3::X * 10.0, DVec3::new(3.0, 2.0, 1.0));
        assert_eq!(o, DVec3::new(0.0, 2.0, 1.0));
    }

    #[test]
    fn radial_layout() {
        let d = Dimension {
            kind: DimensionKind::Radius,
            ..Dimension::linear(DVec3::ZERO, DVec3::X * 5.0, DVec3::new(2.0, 2.0, 0.0))
        };
        assert_eq!(d.line(), [DVec3::ZERO, DVec3::X * 5.0]);
        assert_eq!(d.label_point(), DVec3::new(7.0, 2.0, 0.0));
        assert_eq!(d.segments(1.0).len(), 2);
    }

    #[test]
    fn written_to_skp() {
        let mut m = crate::Model::default();
        crate::add_line(&mut m.root, DVec3::ZERO, DVec3::X * 10.0);
        m.root.dimensions.push(Dimension::linear(
            DVec3::ZERO,
            DVec3::X * 10.0,
            DVec3::Y * 3.0,
        ));
        m.root.texts.push(Text::leader(
            DVec3::X * 5.0,
            DVec3::splat(10.0),
            "Edge".into(),
        ));
        m.root.texts.push(Text::screen(0.1, 0.1, "Title".into()));
        let bytes = crate::export::to_skp(&m).unwrap();
        let back = skp::parse(&bytes).unwrap();
        // Legacy files list root dimensions both in the root and at model level.
        assert_eq!(back.dimensions.len(), 1);
        let texts: Vec<_> = back.root.texts.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, ["Edge"], "screen texts are not written");
    }

    #[test]
    fn skp_dimensions_with_endpoints_only() {
        let dims = [
            skp::Dimension {
                a: Some([0.0; 3]),
                b: Some([10.0, 0.0, 0.0]),
                offset: 3.0,
                plane_x: Some([1.0, 0.0, 0.0]),
                normal: Some([0.0, 0.0, 1.0]),
                ..Default::default()
            },
            skp::Dimension::default(),
        ];
        let out = dimensions_from_skp(&dims);
        assert_eq!(out.len(), 1);
        assert!((out[0].offset - DVec3::Y * 3.0).length() < 1e-12);
    }
}
