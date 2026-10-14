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

/// Where a linear dimension's label sits along its dimension line (context menu Text
/// Position).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextPosition {
    /// Outside the start (`a`) end of the dimension line.
    OutsideStart,
    #[default]
    Center,
    /// Outside the end (`b`) end of the dimension line.
    OutsideEnd,
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
    /// Label position along a linear dimension's line. Not saved in .skp.
    pub text_position: TextPosition,
    /// Font, alignment and endpoints set by Model Info > Dimensions > Update Selected;
    /// `None` follows the model's `dimension_style` (not saved in .skp).
    pub style: Option<crate::model_data::DimensionStyle>,
    /// Its tag (index into `Model::layers`; `None` = Untagged). Hidden tags hide it
    /// and Color by Tag colors it. Saved in .skp.
    pub layer: Option<usize>,
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
            text_position: TextPosition::Center,
            style: None,
            layer: None,
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
            DimensionKind::Linear => {
                // Outside an end: a fifth of the length beyond it (unverified).
                let out = (self.b - self.a) * 0.2;
                let at = match self.text_position {
                    TextPosition::Center => (self.a + self.b) / 2.0,
                    TextPosition::OutsideStart => self.a - out,
                    TextPosition::OutsideEnd => self.b + out,
                };
                at + self.offset
            }
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
                // A label outside an end sits on the dimension line extended to it.
                match self.text_position {
                    TextPosition::Center => {}
                    TextPosition::OutsideStart => out.push([p, self.label_point()]),
                    TextPosition::OutsideEnd => out.push([q, self.label_point()]),
                }
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

/// How much of segment `a`-`b`'s length shows when seen along the unit direction
/// `view`: 1 seen square on, 0 end-on (the sine of the angle between them).
pub fn foreshortening(a: DVec3, b: DVec3, view: DVec3) -> f64 {
    let d = (b - a).normalize_or_zero();
    let v = view.normalize_or_zero();
    if d == DVec3::ZERO || v == DVec3::ZERO {
        return 1.0;
    }
    d.cross(v).length().min(1.0)
}

/// Whether `d` is associated with geometry of `ents` (Model Info > Dimensions >
/// Expert Settings): the points it measures lie on vertices or edges there or in the
/// groups and components placed in it (through their transforms). A radius
/// dimension's center counts when its point on the arc does. Meshtard does not
/// store which entities a dimension is attached to, so this is a geometric test.
pub fn dimension_associated(model: &crate::Model, ents: &crate::Entities, d: &Dimension) -> bool {
    let on = |p: DVec3| on_geometry(model, ents, glam::DMat4::IDENTITY, p, 0);
    match d.kind {
        DimensionKind::Radius => on(d.b),
        DimensionKind::Linear | DimensionKind::Diameter => on(d.a) && on(d.b),
    }
}

/// `p` (in the coordinates `xf` maps `ents` to) lies on a vertex or edge of `ents`
/// or of what is nested in it (a few levels deep).
fn on_geometry(
    model: &crate::Model,
    ents: &crate::Entities,
    xf: glam::DMat4,
    p: DVec3,
    depth: usize,
) -> bool {
    let on_edge = ents.edges.values().any(|e| {
        let (a, b) = (
            xf.transform_point3(ents.pos(e.start)),
            xf.transform_point3(ents.pos(e.end)),
        );
        let ab = b - a;
        let t = ((p - a).dot(ab) / ab.length_squared().max(1e-24)).clamp(0.0, 1.0);
        (a + ab * t).distance(p) <= crate::TOL
    });
    on_edge
        || (depth < 8
            && ents.instances.iter().any(|inst| {
                model.definitions.get(inst.definition).is_some_and(|def| {
                    on_geometry(model, &def.entities, xf * inst.transform, p, depth + 1)
                })
            }))
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
    /// Font, color and leader set by Model Info > Text > Update Selected; `None`
    /// follows the model's `text_style` (not saved in .skp).
    pub style: Option<crate::model_data::TextStyle>,
    /// Its tag (see [`Dimension::layer`]). Saved in .skp.
    pub layer: Option<usize>,
}

impl Text {
    pub fn leader(point: DVec3, leader: DVec3, text: String) -> Self {
        Self {
            point,
            leader,
            text,
            screen: false,
            hidden: false,
            style: None,
            layer: None,
        }
    }

    pub fn screen(x: f64, y: f64, text: String) -> Self {
        Self {
            point: DVec3::new(x, y, 0.0),
            leader: DVec3::ZERO,
            text,
            screen: true,
            hidden: false,
            style: None,
            layer: None,
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

/// The signed offset a dimension is saved with, or `None` when its endpoints
/// coincide (not saved). The writer's dimension plane is horizontal (normal +Z) and
/// the reader takes the offset direction as normal x (b - a): the offset is written
/// signed along that direction so it reopens on the same side. An offset out of the
/// horizontal plane keeps only its in-plane part. The writer has no text override
/// for dimensions and no radial dimensions (they are written as linear ones between
/// their points).
pub fn saved_offset(d: &Dimension) -> Option<f64> {
    if d.a.distance(d.b) <= crate::TOL {
        return None;
    }
    let side = DVec3::Z.cross(d.b - d.a).normalize_or_zero();
    Some(if side == DVec3::ZERO {
        d.offset.length()
    } else {
        d.offset.dot(side)
    })
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
    fn foreshortening_is_the_sine_to_the_view() {
        let (a, b) = (DVec3::ZERO, DVec3::X * 10.0);
        assert!((foreshortening(a, b, DVec3::Z) - 1.0).abs() < 1e-12);
        assert!(foreshortening(a, b, DVec3::X) < 1e-12);
        let k = foreshortening(a, b, DVec3::new(1.0, 1.0, 0.0).normalize());
        assert!((k - 0.5f64.sqrt()).abs() < 1e-12);
        assert_eq!(foreshortening(a, a, DVec3::Z), 1.0);
        let ex = crate::model_data::DimensionExpert::default();
        assert!(ex.hidden(a, b, DVec3::X, Some(100.0)), "end-on");
        assert!(ex.hidden(a, b, DVec3::Z, Some(3.0)), "too small");
        assert!(!ex.hidden(a, b, DVec3::Z, Some(100.0)));
        assert!(!ex.hidden(a, b, DVec3::Z, None));
        let off = crate::model_data::DimensionExpert {
            hide_foreshortened: false,
            hide_small: false,
            ..ex
        };
        assert!(!off.hidden(a, b, DVec3::X, Some(1.0)));
    }

    #[test]
    fn association_with_edges_and_nested_geometry() {
        let mut m = crate::Model::default();
        crate::add_line(&mut m.root, DVec3::ZERO, DVec3::X * 10.0);
        let d = Dimension::linear(DVec3::ZERO, DVec3::X * 10.0, DVec3::Y * 3.0);
        assert!(dimension_associated(&m, &m.root, &d));
        // A point along an edge counts; one off it does not.
        let mid = Dimension::linear(DVec3::X * 4.0, DVec3::X * 10.0, DVec3::Y);
        assert!(dimension_associated(&m, &m.root, &mid));
        let free = Dimension::linear(DVec3::ZERO, DVec3::new(10.0, 5.0, 0.0), DVec3::Y);
        assert!(!dimension_associated(&m, &m.root, &free));
        // Geometry inside a group, moved by its transform.
        let mut inner = crate::Entities::new();
        crate::add_line(&mut inner, DVec3::ZERO, DVec3::Y * 5.0);
        let def = m.definitions.insert(crate::Definition {
            name: "g".into(),
            entities: inner,
            is_group: true,
            ..Default::default()
        });
        m.root.instances.push(crate::Instance {
            definition: def,
            transform: glam::DMat4::from_translation(DVec3::X * 10.0),
            name: String::new(),
            hidden: false,
            locked: false,
            layer: None,
            material: None,
            attrs: Default::default(),
        });
        assert!(dimension_associated(&m, &m.root, &free));
        // A radius dimension needs only its point on the arc.
        let r = Dimension {
            kind: DimensionKind::Radius,
            ..Dimension::linear(DVec3::new(3.0, 3.0, 3.0), DVec3::X * 2.0, DVec3::Y)
        };
        assert!(dimension_associated(&m, &m.root, &r));
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
        assert_eq!(texts, ["Edge", "Title"]);
        assert_eq!(back.root.texts[1].screen_position, Some([0.1, 0.1]));
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
