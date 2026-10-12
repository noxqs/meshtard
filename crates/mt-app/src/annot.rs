//! Dimensions and texts in the viewport. Their lines (extension and dimension lines,
//! leaders) go into the scene's edges with their owner, so they pick, box-select and
//! highlight like edges; labels and arrowheads are painted over the view, facing the
//! screen at a fixed size.
//!
//! Model Info > Dimensions and Text set each label's font size, the dimension text's
//! alignment (to the screen, or along the dimension line above, centered or outside
//! it), the endpoints (none, slash, dot, closed or open arrow), the text color and
//! leader (FEATURES.md 61). Drafts: SketchUp's label font, the endpoint sizes and the
//! leader dot are unverified. Labels use the style's font (a system font loaded by
//! `label_fonts`), else the application font.
//!
//! Model Info > Dimensions > Expert Settings: dimensions whose points are not on
//! geometry are drawn in the highlight color; dimensions seen end-on or too small on
//! screen are hidden. Hiding depends on the view, so the app keeps the hidden set in
//! the document ([`Document::dims_hidden`]) and rebuilds the scene when it changes
//! (`App::update_dimension_view`).

use glam::{DMat4, DVec2, DVec3};
use mt_infer::units::LengthUnit;
use mt_kernel::model_data::{
    DimensionAlign, DimensionExpert, DimensionStyle, Endpoint, LeaderKind, TextStyle,
};
use mt_kernel::{Dimension, DimensionKind, Entities};
use mt_render::Segment;

use crate::document::{Document, Owner, Scene};
use crate::tools::DrawTool;

const LINE_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const SELECTED: egui::Color32 = egui::Color32::from_rgb(0, 0, 255);
/// Label font size and approximate glyph width (for picking), in points.
const FONT_SIZE: f32 = 13.0;
const CHAR_WIDTH: f64 = 7.0;
const LABEL_HEIGHT: f64 = 18.0;
/// Arrowhead length and half width, in points.
const ARROW: (f32, f32) = (10.0, 3.5);

/// Where a label is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Anchor {
    /// World space.
    World(DVec3),
    /// Fractions of the view's width and height.
    Screen(DVec2),
}

/// A dimension's or text's label, arrowheads and leader dot, in world space.
#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    /// `None` outside the open group or component (not selectable).
    pub owner: Option<Owner>,
    pub text: String,
    pub at: Anchor,
    /// Line ends as (tip, a point the line comes from), drawn as `endpoint`.
    pub arrows: Vec<[DVec3; 2]>,
    pub endpoint: Endpoint,
    /// Points.
    pub font_size: f32,
    pub color: [u8; 3],
    /// A dimension's line and how its text is aligned to it.
    pub along: Option<([DVec3; 2], DimensionAlign)>,
    /// The style's font name (`None`: the application font).
    pub font: Option<String>,
    /// A dimension's key and points, to hide it by view (Expert Settings).
    pub dim: Option<DimView>,
    /// Hidden for the current view: not painted or picked (its lines are left out).
    pub view_hidden: bool,
}

/// What hiding a dimension by view needs: its key in [`Document::dims_hidden`] (open
/// context or not, index) and, in world space, the measured points and the
/// dimension line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DimView {
    pub key: (bool, usize),
    pub a: DVec3,
    pub b: DVec3,
    pub line: [DVec3; 2],
}

impl DimView {
    /// Whether `expert` hides this dimension seen from `camera` on a viewport of
    /// `viewport` physical pixels at `ppp` pixels per point.
    pub fn hidden(
        &self,
        expert: &DimensionExpert,
        camera: &mt_render::Camera,
        viewport: DVec2,
        ppp: f64,
    ) -> bool {
        let view = if camera.perspective {
            ((self.a + self.b) / 2.0 - camera.eye).normalize_or_zero()
        } else {
            camera.forward()
        };
        let [p, q] = self.line;
        let screen = camera
            .project(p, viewport)
            .zip(camera.project(q, viewport))
            .map(|(p, q)| p.distance(q) / ppp.max(1e-6));
        expert.hidden(self.a, self.b, view, screen)
    }
}

fn trim_number(v: f64) -> String {
    let text = format!("{v:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// A dimension's label: its text, or the measured value (`R` and `DIA` prefixes for
/// radial ones, unverified).
pub fn dimension_label(d: &Dimension, units: LengthUnit) -> String {
    dimension_label_with(d, units, true)
}

/// [`dimension_label`] with or without the radial prefixes (Expert Settings).
pub fn dimension_label_with(d: &Dimension, units: LengthUnit, prefix: bool) -> String {
    if let Some(t) = &d.text {
        return t.clone();
    }
    let v = units.format(d.value());
    match d.kind {
        DimensionKind::Radius if prefix => format!("R{v}"),
        DimensionKind::Diameter if prefix => format!("DIA {v}"),
        _ => v,
    }
}

/// An area in square inches, e.g. `144 in²` (format unverified).
pub fn area_label(area: f64, units: LengthUnit) -> String {
    // The open model's Model Info > Units area unit and precision.
    if let Some(o) = mt_infer::units::display_options() {
        return mt_infer::units::format_area(area, &o);
    }
    let u = units.inches();
    let name = match units {
        LengthUnit::Inch => "in",
        LengthUnit::Foot => "ft",
        LengthUnit::Millimeter => "mm",
        LengthUnit::Centimeter => "cm",
        LengthUnit::Meter => "m",
    };
    format!("{} {name}²", trim_number(area / (u * u)))
}

/// Point coordinates, e.g. `(10", 0", 5")` (format unverified).
pub fn point_label(p: DVec3, units: LengthUnit) -> String {
    format!(
        "({}, {}, {})",
        units.format(p.x),
        units.format(p.y),
        units.format(p.z)
    )
}

/// Adds the dimensions and texts of the model root and the open context.
pub fn collect(doc: &Document, scene: &mut Scene) {
    let root = doc.context.is_empty();
    let o = Options {
        model: &doc.model,
        units: doc.units,
        // Dimensions and texts are objects: View > Hidden Objects shows them.
        show_hidden: doc.model.style.hidden_objects,
        dimension: &doc.model.dimension_style,
        text: &doc.model.text_style,
        expert: doc.model.dimension_expert,
        view_hidden: &doc.dims_hidden,
        tags: (0..doc.model.layers.len())
            .map(|l| (doc.model.layer_shown(l), doc.model.layers[l].color))
            .collect(),
        color_by_tag: doc.model.color_by_tag,
    };
    add(&doc.model.root, DMat4::IDENTITY, root, &o, scene);
    if !root {
        add(doc.active(), doc.context_transform(), true, &o, scene);
    }
}

/// What labels are drawn with: units, View > Hidden Geometry and the model's
/// default dimension and text styles.
struct Options<'a> {
    model: &'a mt_kernel::Model,
    units: LengthUnit,
    show_hidden: bool,
    dimension: &'a DimensionStyle,
    text: &'a TextStyle,
    expert: DimensionExpert,
    /// Dimensions hidden for the current view (sorted keys).
    view_hidden: &'a [(bool, usize)],
    /// Per tag: shown (tag and folders visible) and color.
    tags: Vec<(bool, [u8; 3])>,
    /// Tags panel Color by Tag: labels and lines in their tag's color.
    color_by_tag: bool,
}

impl Options<'_> {
    /// Whether an annotation on `layer` is drawn (its tag is shown).
    fn shown(&self, layer: Option<usize>) -> bool {
        layer.is_none_or(|l| self.tags.get(l).is_none_or(|t| t.0))
    }

    /// The Color by Tag color for `layer` (Untagged's for `None`), when on.
    fn tag_color(&self, layer: Option<usize>) -> Option<[u8; 3]> {
        self.color_by_tag
            .then(|| self.tags.get(layer.unwrap_or(0)).map(|t| t.1))
            .flatten()
    }
}

fn rgba(c: [u8; 3]) -> [f32; 4] {
    [
        f32::from(c[0]) / 255.0,
        f32::from(c[1]) / 255.0,
        f32::from(c[2]) / 255.0,
        1.0,
    ]
}

fn add(ents: &Entities, xf: DMat4, owned: bool, o: &Options, scene: &mut Scene) {
    let (units, show_hidden) = (o.units, o.show_hidden);
    let w = |p: DVec3| xf.transform_point3(p);
    let f = |p: DVec3| w(p).as_vec3().to_array();
    let mut line = |a: DVec3, b: DVec3, owner: Option<Owner>, color: Option<[u8; 3]>| {
        let c = color.map_or(LINE_COLOR, rgba);
        scene.edges.push(Segment::line(f(a), f(b), c, [1.0, 0.0]));
        scene.edge_owner.push(owner);
    };
    let mut labels = Vec::new();
    for (i, d) in ents.dimensions.iter().enumerate() {
        if (d.hidden && !show_hidden) || !o.shown(d.layer) {
            continue;
        }
        let owner = owned.then_some(Owner::Dimension(i));
        // Expert Settings: dimensions off geometry in the highlight color (over
        // Color by Tag).
        let loose = o.expert.highlight_non_associated
            && !mt_kernel::annotation::dimension_associated(o.model, ents, d);
        let tag_color = if loose {
            Some(o.expert.highlight_color)
        } else {
            o.tag_color(d.layer)
        };
        let key = (owned, i);
        let view_hidden = o.view_hidden.binary_search(&key).is_ok();
        // Extension lines overshoot by a tenth of the offset (unverified).
        if !view_hidden {
            for [a, b] in d.segments(d.offset.length() * 0.1) {
                line(a, b, owner, tag_color);
            }
        }
        let [p, q] = d.line();
        let arrows = match d.kind {
            DimensionKind::Radius => vec![[w(q), w(p)]],
            DimensionKind::Linear | DimensionKind::Diameter => {
                vec![[w(p), w(q)], [w(q), w(p)]]
            }
        };
        let style = d.style.as_ref().unwrap_or(o.dimension);
        labels.push(Annotation {
            owner,
            text: dimension_label_with(d, units, o.expert.radial_prefix),
            at: Anchor::World(w(d.label_point())),
            arrows,
            endpoint: style.endpoint,
            font_size: style.font_size,
            color: tag_color.unwrap_or([0, 0, 0]),
            // Radial dimensions keep their text on the leader, facing the screen.
            along: (d.kind == DimensionKind::Linear).then_some(([w(p), w(q)], style.align)),
            font: style.font.clone(),
            dim: Some(DimView {
                key,
                a: w(d.a),
                b: w(d.b),
                line: [w(p), w(q)],
            }),
            view_hidden,
        });
    }
    for (i, t) in ents.texts.iter().enumerate() {
        if (t.hidden && !show_hidden) || !o.shown(t.layer) {
            continue;
        }
        let tag_color = o.tag_color(t.layer);
        let owner = owned.then_some(Owner::Text(i));
        let style = t.style.as_ref().unwrap_or(o.text);
        let leader = !t.screen && style.leader != LeaderKind::Hidden;
        let at = if t.screen {
            Anchor::Screen(t.point.truncate())
        } else {
            if leader && t.leader.length() > mt_kernel::TOL {
                line(t.point, t.point + t.leader, owner, tag_color);
            }
            Anchor::World(w(t.point + t.leader))
        };
        labels.push(Annotation {
            owner,
            text: live_text(ents, t, units),
            at,
            arrows: if leader && t.leader.length() > mt_kernel::TOL {
                vec![[w(t.point), w(t.point + t.leader)]]
            } else {
                Vec::new()
            },
            endpoint: style.endpoint,
            font_size: style.font_size,
            color: tag_color.unwrap_or(style.color),
            along: None,
            font: style.font.clone(),
            dim: None,
            view_hidden: false,
        });
    }
    scene.annotations.extend(labels);
}

/// The placeholder in a text that shows the measured value of what the leader points
/// at, kept up to date (`Width: <>`). From the SketchUp 2025 release notes as
/// summarised in HELP_COMPARISON.md; the exact behaviour is unverified.
pub const LIVE_VALUE: &str = "<>";

/// A text's label with [`LIVE_VALUE`] replaced by the current value at its point.
pub fn live_text(ents: &Entities, t: &mt_kernel::Text, units: LengthUnit) -> String {
    if t.screen || !t.text.contains(LIVE_VALUE) {
        return t.text.clone();
    }
    t.text
        .replace(LIVE_VALUE, &live_value(ents, t.point, units))
}

/// What a leader at `p` measures now: the length of the edge (or its whole curve)
/// through `p`, else the area of the face under `p`, else `p`'s coordinates.
pub fn live_value(ents: &Entities, p: DVec3, units: LengthUnit) -> String {
    let on_edge = ents.edges.iter().find(|(_, e)| {
        let (a, b) = (ents.pos(e.start), ents.pos(e.end));
        let ab = b - a;
        let t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
        (a + ab * t).distance(p) <= 1e-6
    });
    if let Some((id, e)) = on_edge {
        let len = |e: &mt_kernel::Edge| ents.pos(e.start).distance(ents.pos(e.end));
        let total = match e.attrs.curve {
            Some(c) => ents
                .curve_edges(c)
                .iter()
                .map(|&x| len(&ents.edges[x]))
                .sum(),
            None => len(&ents.edges[id]),
        };
        return units.format(total);
    }
    if let Some(f) = ents.faces.keys().find(|&f| ents.face_contains(f, p)) {
        return area_label(crate::tools::face_area(ents, f), units);
    }
    point_label(p, units)
}

/// Removes the dimensions and texts among `items` from `ents`.
pub fn delete(ents: &mut Entities, items: &[Owner]) {
    let mut dims: Vec<usize> = Vec::new();
    let mut texts: Vec<usize> = Vec::new();
    for o in items {
        match *o {
            Owner::Dimension(i) => dims.push(i),
            Owner::Text(i) => texts.push(i),
            _ => {}
        }
    }
    for (list, len) in [
        (&mut dims, ents.dimensions.len()),
        (&mut texts, ents.texts.len()),
    ] {
        list.retain(|&i| i < len);
        list.sort_unstable();
        list.dedup();
    }
    for i in dims.into_iter().rev() {
        ents.dimensions.remove(i);
    }
    for i in texts.into_iter().rev() {
        ents.texts.remove(i);
    }
}

/// Moves (or copies) the dimensions and texts among `targets` by `xf`. Screen texts
/// stay where they are.
pub fn transform(ents: &mut Entities, targets: &[Owner], xf: DMat4, copy: bool) {
    for t in targets {
        match *t {
            Owner::Dimension(i) if i < ents.dimensions.len() => {
                let d = &ents.dimensions[i];
                let moved = Dimension {
                    a: xf.transform_point3(d.a),
                    b: xf.transform_point3(d.b),
                    offset: xf.transform_vector3(d.offset),
                    ..d.clone()
                };
                if copy {
                    ents.dimensions.push(moved);
                } else {
                    ents.dimensions[i] = moved;
                }
            }
            Owner::Text(i) if i < ents.texts.len() && !ents.texts[i].screen => {
                let t = &ents.texts[i];
                let moved = mt_kernel::Text {
                    point: xf.transform_point3(t.point),
                    leader: xf.transform_vector3(t.leader),
                    ..t.clone()
                };
                if copy {
                    ents.texts.push(moved);
                } else {
                    ents.texts[i] = moved;
                }
            }
            _ => {}
        }
    }
}

/// A label's position in viewport pixels.
fn label_pos(
    a: &Annotation,
    project: impl Fn(DVec3) -> Option<DVec2>,
    viewport: DVec2,
) -> Option<DVec2> {
    match a.at {
        Anchor::World(p) => project(p),
        Anchor::Screen(s) => Some(s * viewport),
    }
}

/// The selectable label under `cursor` (viewport pixels). Label sizes are estimated
/// from the character count.
pub fn pick(
    scene: &Scene,
    project: impl Fn(DVec3) -> Option<DVec2>,
    cursor: DVec2,
    viewport: DVec2,
) -> Option<Owner> {
    scene.annotations.iter().rev().find_map(|a| {
        let owner = a.owner.filter(|_| !a.view_hidden)?;
        let c = label_pos(a, &project, viewport)?;
        let k = f64::from(a.font_size / FONT_SIZE);
        let half = DVec2::new(
            (a.text.chars().count() as f64 * CHAR_WIDTH * k + 8.0) / 2.0,
            LABEL_HEIGHT * k / 2.0,
        );
        let d = (cursor - c).abs();
        (d.x <= half.x && d.y <= half.y).then_some(owner)
    })
}

/// Label positions of selectable annotations (for box selection).
pub fn label_points(
    scene: &Scene,
    project: impl Fn(DVec3) -> Option<DVec2>,
    viewport: DVec2,
) -> Vec<(Owner, Option<DVec2>)> {
    scene
        .annotations
        .iter()
        .filter(|a| !a.view_hidden)
        .filter_map(|a| Some((a.owner?, label_pos(a, &project, viewport))))
        .collect()
}

/// Paints labels, arrowheads and leader dots. `family` gives the egui font family of
/// a style's font (the proportional one while it is not loaded).
pub fn paint(
    painter: &egui::Painter,
    to_screen: &impl Fn(DVec3) -> Option<egui::Pos2>,
    rect: egui::Rect,
    annotations: &[Annotation],
    selected: &impl Fn(&Owner) -> bool,
    family: &mut dyn FnMut(Option<&str>) -> egui::FontFamily,
) {
    for a in annotations.iter().filter(|a| !a.view_hidden) {
        let color = if a.owner.as_ref().is_some_and(selected) {
            SELECTED
        } else {
            egui::Color32::from_rgb(a.color[0], a.color[1], a.color[2])
        };
        for [tip, from] in &a.arrows {
            let (Some(t), Some(f)) = (to_screen(*tip), to_screen(*from)) else {
                continue;
            };
            endpoint(painter, a.endpoint, t, f, color);
        }
        let pos = match a.at {
            Anchor::World(p) => to_screen(p),
            Anchor::Screen(s) => Some(rect.min + egui::vec2(s.x as f32, s.y as f32) * rect.size()),
        };
        let Some(pos) = pos else { continue };
        if a.text.is_empty() {
            continue;
        }
        let galley = painter.layout_no_wrap(
            a.text.clone(),
            egui::FontId::new(a.font_size.clamp(4.0, 96.0), family(a.font.as_deref())),
            color,
        );
        let size = galley.size() + egui::vec2(6.0, 2.0);
        // Along the dimension line: turned to the line (kept readable, never upside
        // down), above it, centered on it or past its end.
        let line = a
            .along
            .filter(|(_, al)| *al != DimensionAlign::Screen)
            .and_then(|([p, q], al)| Some((to_screen(p)?, to_screen(q)?, al)));
        match line {
            Some((p, q, align)) if (q - p).length() > 1.0 => {
                let mut dir = (q - p).normalized();
                if dir.x < 0.0 || (dir.x == 0.0 && dir.y > 0.0) {
                    dir = -dir;
                }
                // Screen y points down: the normal towards the top of the text.
                let up = egui::vec2(dir.y, -dir.x);
                let mid = p + (q - p) * 0.5;
                let far = if (q - p).dot(dir) > 0.0 { q } else { p };
                let center = match align {
                    DimensionAlign::Above => mid + up * (size.y / 2.0 + 2.0),
                    DimensionAlign::Outside => far + dir * (size.x / 2.0 + 6.0),
                    DimensionAlign::Centered | DimensionAlign::Screen => mid,
                };
                let half = dir * (size.x / 2.0) + up * (size.y / 2.0);
                let half2 = dir * (size.x / 2.0) - up * (size.y / 2.0);
                painter.add(egui::Shape::convex_polygon(
                    vec![center - half, center + half2, center + half, center - half2],
                    egui::Color32::from_white_alpha(220),
                    egui::Stroke::NONE,
                ));
                // The galley turns about its top left corner.
                let top_left =
                    center - dir * (galley.size().x / 2.0) + up * (galley.size().y / 2.0);
                let angle = dir.y.atan2(dir.x);
                painter
                    .add(egui::epaint::TextShape::new(top_left, galley, color).with_angle(angle));
            }
            _ => {
                let r = egui::Rect::from_center_size(pos, size);
                painter.rect_filled(r, 2.0, egui::Color32::from_white_alpha(220));
                painter.galley(r.min + egui::vec2(3.0, 1.0), galley, color);
            }
        }
    }
}

/// A line's end at `tip` (coming from `from`) drawn as `style`; sizes in points are
/// drafts.
fn endpoint(
    painter: &egui::Painter,
    style: Endpoint,
    tip: egui::Pos2,
    from: egui::Pos2,
    color: egui::Color32,
) {
    let dir = (from - tip).normalized();
    if !dir.x.is_finite() {
        if style == Endpoint::Dot {
            painter.circle_filled(tip, 2.5, color);
        }
        return;
    }
    let side = egui::vec2(-dir.y, dir.x) * ARROW.1;
    let base = tip + dir * ARROW.0;
    let stroke = egui::Stroke::new(1.2, color);
    match style {
        Endpoint::None => {}
        Endpoint::Dot => {
            painter.circle_filled(tip, 2.5, color);
        }
        Endpoint::Slash => {
            // A short stroke at 45 degrees across the line.
            let d = (dir + egui::vec2(-dir.y, dir.x)).normalized() * 5.0;
            painter.line_segment([tip - d, tip + d], egui::Stroke::new(1.6, color));
        }
        Endpoint::ClosedArrow => {
            painter.add(egui::Shape::convex_polygon(
                vec![tip, base + side, base - side],
                color,
                egui::Stroke::NONE,
            ));
        }
        Endpoint::OpenArrow => {
            painter.line_segment([tip, base + side * 1.3], stroke);
            painter.line_segment([tip, base - side * 1.3], stroke);
        }
    }
}

/// The sorted keys of the dimensions among `annotations` that `expert` hides for this
/// view (none when neither hiding option is on).
pub fn hidden_by_view(
    annotations: &[Annotation],
    expert: &DimensionExpert,
    camera: &mt_render::Camera,
    viewport: DVec2,
    ppp: f64,
) -> Vec<(bool, usize)> {
    if !expert.hide_foreshortened && !expert.hide_small {
        return Vec::new();
    }
    let mut keys: Vec<_> = annotations
        .iter()
        .filter_map(|a| a.dim)
        .filter(|d| d.hidden(expert, camera, viewport, ppp))
        .map(|d| d.key)
        .collect();
    keys.sort_unstable();
    keys
}

impl crate::App {
    /// Rebuilds the scene when the dimensions hidden by view (Expert Settings) change
    /// as the view moves. Waits while a tool shows a live preview.
    pub(crate) fn update_dimension_view(
        &mut self,
        device: &egui_wgpu::wgpu::Device,
        viewport: DVec2,
        ppp: f64,
    ) {
        let Some(doc) = self.document.as_ref() else {
            return;
        };
        let want = hidden_by_view(
            &self.scene.annotations,
            &doc.model.dimension_expert,
            &self.camera,
            viewport,
            ppp,
        );
        if want == doc.dims_hidden || self.live.is_some() {
            return;
        }
        if let Some(doc) = self.document.as_mut() {
            doc.dims_hidden = want;
        }
        self.rebuild(device);
    }
}

/// The Text tool's inline editor at the label being placed. Enter adds the text to
/// the model (one undo step), Esc drops it. Returns whether the model changed.
pub fn text_editor(
    ui: &egui::Ui,
    rect: egui::Rect,
    to_screen: &impl Fn(DVec3) -> Option<egui::Pos2>,
    ctx_xf: DMat4,
    draw: &mut dyn DrawTool,
    doc: &mut Document,
) -> bool {
    let units = doc.units;
    let Some(draft) = draw.text_draft() else {
        return false;
    };
    if draft.needs_screen_pos {
        let Some(p) = ui.input(|i| i.pointer.latest_pos()) else {
            return false;
        };
        let f = (p - rect.min) / rect.size();
        draft.text.point = DVec3::new(f64::from(f.x), f64::from(f.y), 0.0);
        draft.needs_screen_pos = false;
    }
    let pos = if draft.text.screen {
        Some(
            rect.min
                + egui::vec2(draft.text.point.x as f32, draft.text.point.y as f32) * rect.size(),
        )
    } else {
        to_screen(ctx_xf.transform_point3(draft.text.point + draft.text.leader))
    };
    let Some(pos) = pos else { return false };
    let buf = draft
        .edited
        .get_or_insert_with(|| draft.label.format(units));
    let focus = !draft.focused;
    draft.focused = true;
    let (enter, escape) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    let response = egui::Area::new(egui::Id::new("mt_annot_text_editor"))
        .fixed_pos(pos)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            let r = ui.add(
                egui::TextEdit::singleline(buf)
                    .desired_width(160.0)
                    .font(egui::FontId::proportional(FONT_SIZE)),
            );
            if focus {
                r.request_focus();
            }
            r
        })
        .inner;
    if escape {
        draw.cancel();
        false
    } else if enter && response.lost_focus() {
        doc.edit(|ents| draw.finish_text(ents))
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        let d = Dimension::linear(DVec3::ZERO, DVec3::X * 24.0, DVec3::Y);
        assert_eq!(dimension_label(&d, LengthUnit::Inch), "24\"");
        assert_eq!(dimension_label(&d, LengthUnit::Foot), "2'");
        let r = Dimension {
            kind: DimensionKind::Radius,
            ..d.clone()
        };
        assert_eq!(dimension_label(&r, LengthUnit::Inch), "R24\"");
        let t = Dimension {
            text: Some("Wall".into()),
            ..d
        };
        assert_eq!(dimension_label(&t, LengthUnit::Inch), "Wall");
        assert_eq!(area_label(144.0, LengthUnit::Foot), "1 ft²");
        assert_eq!(area_label(12.5, LengthUnit::Inch), "12.5 in²");
        assert_eq!(
            point_label(DVec3::new(1.0, 2.0, 0.0), LengthUnit::Inch),
            "(1\", 2\", 0\")"
        );
    }

    /// Shapes painted for one dimension with `endpoint` and `align`.
    fn painted(endpoint: Endpoint, align: DimensionAlign) -> usize {
        let a = Annotation {
            owner: None,
            text: "10".into(),
            at: Anchor::World(DVec3::new(50.0, 0.0, 0.0)),
            arrows: vec![
                [DVec3::ZERO, DVec3::new(100.0, 0.0, 0.0)],
                [DVec3::new(100.0, 0.0, 0.0), DVec3::ZERO],
            ],
            endpoint,
            font_size: 13.0,
            color: [0, 0, 0],
            along: Some(([DVec3::ZERO, DVec3::new(100.0, 0.0, 0.0)], align)),
            font: None,
            dim: None,
            view_hidden: false,
        };
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0));
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let p = ui.ctx().layer_painter(egui::LayerId::background());
            let to_screen = |q: DVec3| Some(egui::pos2(q.x as f32 + 100.0, 150.0 - q.y as f32));
            paint(
                &p,
                &to_screen,
                rect,
                std::slice::from_ref(&a),
                &|_| false,
                &mut |_| egui::FontFamily::Proportional,
            );
        });
        out.textures_delta.clear();
        out.shapes.len()
    }

    #[test]
    fn endpoints_and_aligned_text_are_painted() {
        let none = painted(Endpoint::None, DimensionAlign::Screen);
        for e in [
            Endpoint::Slash,
            Endpoint::Dot,
            Endpoint::ClosedArrow,
            Endpoint::OpenArrow,
        ] {
            assert!(painted(e, DimensionAlign::Screen) > none, "{e:?}");
        }
        // Along the line: a turned background and text instead of a rectangle.
        assert_eq!(painted(Endpoint::None, DimensionAlign::Above), none);
    }

    #[test]
    fn delete_and_move() {
        let mut e = Entities::new();
        for x in [1.0, 2.0, 3.0] {
            e.dimensions
                .push(Dimension::linear(DVec3::ZERO, DVec3::X * x, DVec3::Y));
        }
        e.texts
            .push(mt_kernel::Text::leader(DVec3::ZERO, DVec3::Z, "a".into()));
        e.texts.push(mt_kernel::Text::screen(0.5, 0.5, "s".into()));
        let xf = DMat4::from_translation(DVec3::Z * 5.0);
        transform(
            &mut e,
            &[Owner::Dimension(1), Owner::Text(0), Owner::Text(1)],
            xf,
            false,
        );
        assert_eq!(e.dimensions[1].a, DVec3::Z * 5.0);
        assert_eq!(e.dimensions[1].offset, DVec3::Y, "offsets are vectors");
        assert_eq!(e.texts[0].point, DVec3::Z * 5.0);
        assert_eq!(
            e.texts[1].point,
            DVec3::new(0.5, 0.5, 0.0),
            "screen text stays"
        );
        transform(&mut e, &[Owner::Text(0)], xf, true);
        assert_eq!(e.texts.len(), 3);
        delete(
            &mut e,
            &[Owner::Dimension(0), Owner::Dimension(2), Owner::Text(1)],
        );
        assert_eq!(e.dimensions.len(), 1);
        assert_eq!(e.dimensions[0].b, DVec3::new(2.0, 0.0, 5.0));
        assert_eq!(e.texts.len(), 2);
    }

    #[test]
    fn expert_settings_highlight_prefix_and_hide_by_view() {
        let mut doc = Document::untitled();
        doc.edit(|e| {
            mt_kernel::add_line(e, DVec3::ZERO, DVec3::X * 10.0);
            // On the edge, then off any geometry.
            e.dimensions.push(Dimension::linear(
                DVec3::ZERO,
                DVec3::X * 10.0,
                DVec3::Y * 2.0,
            ));
            e.dimensions.push(Dimension {
                kind: DimensionKind::Diameter,
                ..Dimension::linear(DVec3::Y * 50.0, DVec3::new(10.0, 50.0, 0.0), DVec3::Y)
            });
            true
        });
        let red = doc.model.dimension_expert.highlight_color;
        let scene = doc.scene();
        assert_eq!(scene.annotations[0].color, [0, 0, 0]);
        assert_eq!(scene.annotations[1].color, red);
        assert!(scene.annotations[1].text.starts_with("DIA "));
        doc.model.dimension_expert.radial_prefix = false;
        doc.model.dimension_expert.highlight_non_associated = false;
        let scene = doc.scene();
        assert_eq!(scene.annotations[1].color, [0, 0, 0]);
        assert_eq!(scene.annotations[1].text, "10\"");

        // Seen from the top both show; looking along red, both are end-on.
        let ex = doc.model.dimension_expert;
        let viewport = DVec2::new(800.0, 600.0);
        let top = mt_render::Camera {
            eye: DVec3::new(5.0, 20.0, 100.0),
            target: DVec3::new(5.0, 20.0, 0.0),
            fov_deg: 35.0,
            perspective: true,
            two_point: false,
        };
        assert!(hidden_by_view(&scene.annotations, &ex, &top, viewport, 1.0).is_empty());
        let along = mt_render::Camera {
            eye: DVec3::new(-200.0, 20.0, 1.0),
            target: DVec3::new(5.0, 20.0, 1.0),
            ..top
        };
        let keys = hidden_by_view(&scene.annotations, &ex, &along, viewport, 1.0);
        assert_eq!(keys, [(true, 0), (true, 1)]);
        // Far away they are too small.
        let far = mt_render::Camera {
            eye: DVec3::new(5.0, 20.0, 1e5),
            ..top
        };
        assert_eq!(
            hidden_by_view(&scene.annotations, &ex, &far, viewport, 1.0).len(),
            2
        );
        // Hidden ones lose their lines and labels, and do not pick.
        doc.dims_hidden = keys;
        let scene = doc.scene();
        assert!(
            scene
                .edge_owner
                .iter()
                .all(|o| !matches!(o, Some(Owner::Dimension(_))))
        );
        assert!(scene.annotations.iter().all(|a| a.view_hidden));
        let project = |p: DVec3| Some(p.truncate() * 10.0);
        assert_eq!(
            pick(&scene, project, DVec2::new(50.0, 20.0), viewport),
            None
        );
        // Both options off: nothing is hidden.
        let off = DimensionExpert {
            hide_foreshortened: false,
            hide_small: false,
            ..ex
        };
        assert!(hidden_by_view(&scene.annotations, &off, &along, viewport, 1.0).is_empty());
    }

    #[test]
    fn scene_has_lines_labels_and_picks() {
        let mut doc = Document::untitled();
        doc.edit(|e| {
            e.dimensions.push(Dimension::linear(
                DVec3::ZERO,
                DVec3::X * 10.0,
                DVec3::Y * 2.0,
            ));
            e.texts
                .push(mt_kernel::Text::screen(0.25, 0.5, "Title".into()));
            true
        });
        let scene = doc.scene();
        let dim_lines = scene
            .edge_owner
            .iter()
            .filter(|o| **o == Some(Owner::Dimension(0)))
            .count();
        assert_eq!(dim_lines, 3);
        assert_eq!(scene.annotations.len(), 2);
        assert_eq!(scene.annotations[0].text, "10\"");
        assert_eq!(scene.annotations[0].arrows.len(), 2);
        let viewport = DVec2::new(800.0, 600.0);
        let project = |p: DVec3| Some(p.truncate() * 10.0);
        assert_eq!(
            pick(&scene, project, DVec2::new(205.0, 300.0), viewport),
            Some(Owner::Text(0))
        );
        assert_eq!(
            pick(&scene, project, DVec2::new(50.0, 20.0), viewport),
            Some(Owner::Dimension(0)),
            "the label at the dimension line's middle"
        );
        assert_eq!(
            pick(&scene, project, DVec2::new(400.0, 50.0), viewport),
            None
        );
    }
}
