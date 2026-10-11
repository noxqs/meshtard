//! Entity Info: the fields shown for the selection ([`describe`]) and the edits typed
//! into them ([`apply`]), each one undo step.
//!
//! Per entity type (SketchUp 2026 as remembered; row order and labels unverified):
//! - Edge: Length (typed, the end moves along the edge [inf]), Soft, Smooth, Hidden.
//! - Arc / Circle: Radius, Segments (the curve is redrawn), Length, Soft, Smooth,
//!   Hidden. Polyline curves: Segments and Length only.
//! - Face: front and back material swatches (click paints that side with the current
//!   material [inf]), Area, Hidden, Cast / Receive Shadows.
//! - Group / Component / Image: Instance name, Definition name (components), Hidden,
//!   Locked, Cast / Receive Shadows, Advanced Attributes (components: Price, Size, URL,
//!   Type on the definition, Status and Owner on the instance), solid volume.
//! - Section Plane: Name, Symbol, Hidden. Dimension and Text: Text, Hidden (the model
//!   keeps no color, font or arrow style for them, so those rows are left out).

use mt_infer::units::{LengthUnit, parse_length};
use mt_kernel::entity_edit::{set_arc_radius, set_arc_segments, set_edge_length};
use mt_ui::{EntityInfo, InfoEdit, InfoValue};

use crate::document::{Document, Owner};

/// What an Entity Info edit did.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Applied {
    Unchanged,
    Changed,
    /// Changed, and the selection is now these entities (a redrawn curve).
    Reselect(Vec<Owner>),
}

/// The arc curve (or other curve) all of `items` are edges of, if any.
fn selected_curve(doc: &Document, items: &[Owner]) -> Option<mt_kernel::CurveId> {
    let root = doc.active();
    let curve_of = |o: &Owner| match o {
        Owner::Edge(e) => root.edges.get(*e).and_then(|x| x.attrs.curve),
        _ => None,
    };
    let c = curve_of(items.first()?)?;
    (root.curves.contains_key(c) && items.iter().all(|o| curve_of(o) == Some(c))).then_some(c)
}

fn check(info: &mut EntityInfo, key: &str, label: &str, on: bool) {
    info.field(key, label, InfoValue::Check(on));
}

/// What Entity Info shows for `items` of the active context.
pub(crate) fn describe(doc: &Document, items: &[Owner]) -> EntityInfo {
    let root = doc.active();
    let units = doc.units;
    let mut info = EntityInfo::default();
    if let Some(c) = selected_curve(doc, items) {
        let edges: Vec<_> = items
            .iter()
            .filter_map(|o| match o {
                Owner::Edge(e) => root.edges.get(*e),
                _ => None,
            })
            .collect();
        let length: f64 = edges
            .iter()
            .map(|e| root.pos(e.start).distance(root.pos(e.end)))
            .sum();
        match root.curves[c].arc {
            Some(arc) => {
                info.title = if arc.is_circle() { "Circle" } else { "Arc" }.into();
                info.field(
                    "radius",
                    "Radius",
                    InfoValue::Text(units.format(arc.radius)),
                );
                let min = if arc.is_circle() { 3 } else { 2 };
                info.field(
                    "segments",
                    "Segments",
                    InfoValue::Integer {
                        value: items.len() as i64,
                        min,
                        max: 999,
                    },
                );
            }
            None => {
                info.title = "Curve".into();
                info.rows.push(("Segments".into(), items.len().to_string()));
            }
        }
        info.rows.push(("Length".into(), units.format(length)));
        let all = |f: fn(&mt_kernel::EdgeAttrs) -> bool| edges.iter().all(|e| f(&e.attrs));
        check(&mut info, "soft", "Soft", all(|a| a.soft));
        check(&mut info, "smooth", "Smooth", all(|a| a.smooth));
        check(&mut info, "hidden", "Hidden", all(|a| a.hidden));
        return info;
    }
    match items {
        [] => EntityInfo::no_selection(),
        [Owner::Edge(e)] if root.edges.contains_key(*e) => {
            let edge = &root.edges[*e];
            let len = root.pos(edge.start).distance(root.pos(edge.end));
            info.title = "Edge".into();
            info.field("length", "Length", InfoValue::Text(units.format(len)));
            check(&mut info, "soft", "Soft", edge.attrs.soft);
            check(&mut info, "smooth", "Smooth", edge.attrs.smooth);
            check(&mut info, "hidden", "Hidden", edge.attrs.hidden);
            info
        }
        [Owner::Face(f)] if root.faces.contains_key(*f) => {
            let face = &root.faces[*f];
            let tris = mt_kernel::tessellate::triangulate(root, *f);
            let area: f64 = tris
                .iter()
                .map(|[a, b, c]| (*b - *a).cross(*c - *a).length() / 2.0)
                .sum();
            info.title = "Face".into();
            let swatch = |m: Option<usize>| {
                InfoValue::Swatch(m.and_then(|i| doc.model.materials.get(i)).map(|m| m.color))
            };
            info.field("front", "Front", swatch(face.attrs.material));
            info.field("back", "Back", swatch(face.attrs.back_material));
            check(&mut info, "hidden", "Hidden", face.attrs.hidden);
            check(
                &mut info,
                "cast_shadows",
                "Cast Shadows",
                face.attrs.shadows.cast,
            );
            check(
                &mut info,
                "receive_shadows",
                "Receive Shadows",
                face.attrs.shadows.receive,
            );
            info.rows.push(("Area".into(), format_area(area, units)));
            info
        }
        [Owner::Instance(i)] if *i < root.instances.len() => {
            let inst = &root.instances[*i];
            let def = doc.model.definitions.get(inst.definition);
            let group = def.is_some_and(|d| d.is_group);
            let image = def.is_some_and(|d| d.is_image);
            info.title = if image {
                "Image"
            } else if group {
                "Group"
            } else {
                "Component"
            }
            .into();
            info.field(
                "instance_name",
                "Instance",
                InfoValue::Text(inst.name.clone()),
            );
            let component = !group && !image;
            if component {
                let name = def.map(|d| d.name.clone()).unwrap_or_default();
                info.field("definition_name", "Definition", InfoValue::Text(name));
            }
            check(&mut info, "hidden", "Hidden", inst.hidden);
            check(&mut info, "locked", "Locked", inst.locked);
            let shadows = inst.attrs.shadows;
            check(&mut info, "cast_shadows", "Cast Shadows", shadows.cast);
            check(
                &mut info,
                "receive_shadows",
                "Receive Shadows",
                shadows.receive,
            );
            if component && let Some(d) = def {
                let a = &d.attributes;
                info.field("advanced", "Advanced Attributes", InfoValue::Section(false));
                for (key, label, value) in [
                    ("price", "Price", &a.price),
                    ("size", "Size", &a.size),
                    ("url", "URL", &a.url),
                    ("type", "Type", &a.kind),
                    ("status", "Status", &inst.attrs.status),
                    ("owner", "Owner", &inst.attrs.owner),
                ] {
                    info.field(key, label, InfoValue::Text(value.clone()));
                }
            }
            crate::solid_tools::describe(doc, inst, &mut info);
            info
        }
        [Owner::Section(i)] if *i < root.section_planes.len() => {
            let sp = &root.section_planes[*i];
            info.title = "Section Plane".into();
            info.field("name", "Name", InfoValue::Text(sp.name.clone()));
            info.field("symbol", "Symbol", InfoValue::Text(sp.symbol.clone()));
            check(&mut info, "hidden", "Hidden", sp.hidden);
            let active = root.active_section == Some(*i);
            info.rows
                .push(("Active".into(), if active { "Yes" } else { "No" }.into()));
            info
        }
        [Owner::Dimension(i)] if *i < root.dimensions.len() => {
            let d = &root.dimensions[*i];
            info.title = "Dimension".into();
            info.field(
                "text",
                "Text",
                InfoValue::Text(crate::annot::dimension_label(d, units)),
            );
            check(&mut info, "hidden", "Hidden", d.hidden);
            info
        }
        [Owner::Text(i)] if *i < root.texts.len() => {
            let t = &root.texts[*i];
            info.title = "Text".into();
            info.field("text", "Text", InfoValue::Text(t.text.clone()));
            check(&mut info, "hidden", "Hidden", t.hidden);
            info
        }
        _ => EntityInfo {
            title: format!("{} Entities", items.len()),
            ..Default::default()
        },
    }
}

/// An area in square inches shown in `unit`s squared, e.g. "12.50 m²" (SketchUp's
/// format and precision unverified).
pub(crate) fn format_area(square_inches: f64, unit: LengthUnit) -> String {
    // The open model's Model Info > Units area unit and precision.
    if let Some(o) = mt_infer::units::display_options() {
        return mt_infer::units::format_area(square_inches, &o);
    }
    let name = match unit {
        LengthUnit::Inch => "in",
        LengthUnit::Foot => "ft",
        LengthUnit::Millimeter => "mm",
        LengthUnit::Centimeter => "cm",
        LengthUnit::Meter => "m",
    };
    let v = square_inches / unit.inches().powi(2);
    format!("{v:.2} {name}²")
}

/// Applies an Entity Info edit of field `key` to `items` of the active context as one
/// undo step. `material` is the current material (for the face swatches). An error
/// is a message for the status bar (e.g. an unreadable length).
pub(crate) fn apply(
    doc: &mut Document,
    items: &[Owner],
    key: &str,
    edit: &InfoEdit,
    material: Option<usize>,
) -> Result<Applied, String> {
    let units = doc.units;
    let changed = |c: bool| {
        if c {
            Applied::Changed
        } else {
            Applied::Unchanged
        }
    };
    let length = |t: &str| {
        parse_length(t, units)
            .filter(|v| *v > 0.0)
            .ok_or_else(|| format!("Invalid length: {t}"))
    };
    let curve = selected_curve(doc, items);
    match (key, edit) {
        ("length", InfoEdit::Text(t)) => {
            let [Owner::Edge(e)] = items else {
                return Ok(Applied::Unchanged);
            };
            let len = length(t)?;
            Ok(changed(doc.edit(|ents| set_edge_length(ents, *e, len))))
        }
        ("radius", InfoEdit::Text(t)) => {
            let Some(c) = curve else {
                return Ok(Applied::Unchanged);
            };
            let r = length(t)?;
            Ok(changed(doc.edit(|ents| set_arc_radius(ents, c, r))))
        }
        ("segments", InfoEdit::Integer(n)) => {
            let Some(c) = curve else {
                return Ok(Applied::Unchanged);
            };
            let mut edges = Vec::new();
            doc.edit(|ents| {
                edges = set_arc_segments(ents, c, (*n).max(0) as usize).unwrap_or_default();
                !edges.is_empty()
            });
            Ok(if edges.is_empty() {
                Applied::Unchanged
            } else {
                Applied::Reselect(edges.into_iter().map(Owner::Edge).collect())
            })
        }
        ("front" | "back", InfoEdit::Swatch) => {
            let [Owner::Face(f)] = items else {
                return Ok(Applied::Unchanged);
            };
            let back = key == "back";
            Ok(changed(doc.edit(|ents| {
                let Some(face) = ents.faces.get_mut(*f) else {
                    return false;
                };
                let side = if back {
                    &mut face.attrs.back_material
                } else {
                    &mut face.attrs.material
                };
                let was = std::mem::replace(side, material);
                was != material
            })))
        }
        (_, InfoEdit::Check(on)) => Ok(changed(set_flag(doc, items, key, *on))),
        (_, InfoEdit::Text(t)) => set_text(doc, items, key, t).map(changed),
        _ => Ok(Applied::Unchanged),
    }
}

/// A check box of Entity Info on every selected entity that has it.
fn set_flag(doc: &mut Document, items: &[Owner], key: &str, on: bool) -> bool {
    doc.edit(|ents| {
        let mut changed = false;
        let mut set = |b: &mut bool| {
            changed |= *b != on;
            *b = on;
        };
        for o in items {
            match *o {
                Owner::Edge(e) => {
                    if let Some(x) = ents.edges.get_mut(e) {
                        match key {
                            "soft" => set(&mut x.attrs.soft),
                            "smooth" => set(&mut x.attrs.smooth),
                            "hidden" => set(&mut x.attrs.hidden),
                            _ => {}
                        }
                    }
                }
                Owner::Face(f) => {
                    if let Some(x) = ents.faces.get_mut(f) {
                        match key {
                            "hidden" => set(&mut x.attrs.hidden),
                            "cast_shadows" => set(&mut x.attrs.shadows.cast),
                            "receive_shadows" => set(&mut x.attrs.shadows.receive),
                            _ => {}
                        }
                    }
                }
                Owner::Instance(i) => {
                    if let Some(x) = ents.instances.get_mut(i) {
                        match key {
                            "hidden" => set(&mut x.hidden),
                            "locked" => set(&mut x.locked),
                            "cast_shadows" => set(&mut x.attrs.shadows.cast),
                            "receive_shadows" => set(&mut x.attrs.shadows.receive),
                            _ => {}
                        }
                    }
                }
                Owner::Section(i) => {
                    if let Some(x) = ents.section_planes.get_mut(i)
                        && key == "hidden"
                    {
                        set(&mut x.hidden);
                    }
                }
                Owner::Dimension(i) => {
                    if let Some(x) = ents.dimensions.get_mut(i)
                        && key == "hidden"
                    {
                        set(&mut x.hidden);
                    }
                }
                Owner::Text(i) => {
                    if let Some(x) = ents.texts.get_mut(i)
                        && key == "hidden"
                    {
                        set(&mut x.hidden);
                    }
                }
            }
        }
        changed
    })
}

/// A text field of Entity Info (names, attributes, annotation text).
fn set_text(doc: &mut Document, items: &[Owner], key: &str, text: &str) -> Result<bool, String> {
    let text = text.to_owned();
    let replace = |s: &mut String| {
        let changed = *s != text;
        s.clone_from(&text);
        changed
    };
    Ok(match items {
        [Owner::Instance(i)] => {
            let i = *i;
            match key {
                "instance_name" => doc.edit(|ents| {
                    ents.instances
                        .get_mut(i)
                        .is_some_and(|x| replace(&mut x.name))
                }),
                "status" => doc.edit(|ents| {
                    ents.instances
                        .get_mut(i)
                        .is_some_and(|x| replace(&mut x.attrs.status))
                }),
                "owner" => doc.edit(|ents| {
                    ents.instances
                        .get_mut(i)
                        .is_some_and(|x| replace(&mut x.attrs.owner))
                }),
                "definition_name" | "price" | "size" | "url" | "type" => {
                    if key == "definition_name" && text.is_empty() {
                        return Err("A component definition needs a name.".into());
                    }
                    doc.edit_model(|m, ents| {
                        let Some(d) = ents
                            .instances
                            .get(i)
                            .and_then(|x| m.definitions.get_mut(x.definition))
                        else {
                            return false;
                        };
                        let a = &mut d.attributes;
                        replace(match key {
                            "definition_name" => &mut d.name,
                            "price" => &mut a.price,
                            "size" => &mut a.size,
                            "url" => &mut a.url,
                            _ => &mut a.kind,
                        })
                    })
                }
                _ => false,
            }
        }
        [Owner::Section(i)] => {
            let i = *i;
            doc.edit(|ents| {
                ents.section_planes.get_mut(i).is_some_and(|x| match key {
                    "name" => replace(&mut x.name),
                    "symbol" => replace(&mut x.symbol),
                    _ => false,
                })
            })
        }
        [Owner::Dimension(i)] if key == "text" => {
            let i = *i;
            // An empty text goes back to the measured value.
            let custom = (!text.is_empty()).then(|| text.clone());
            doc.edit(|ents| {
                ents.dimensions.get_mut(i).is_some_and(|d| {
                    let changed = d.text != custom;
                    d.text.clone_from(&custom);
                    changed
                })
            })
        }
        [Owner::Text(i)] if key == "text" => {
            let i = *i;
            doc.edit(|ents| ents.texts.get_mut(i).is_some_and(|t| replace(&mut t.text)))
        }
        _ => false,
    })
}

impl crate::App {
    /// What the Entity Info panel shows for the current selection.
    pub(crate) fn entity_info(&self) -> EntityInfo {
        match &self.document {
            Some(doc) => describe(doc, self.selection.items()),
            None => EntityInfo::no_selection(),
        }
    }

    /// Applies an Entity Info edit. Returns whether the scene must be rebuilt.
    pub(crate) fn entity_info_edit(&mut self, key: &str, edit: &InfoEdit) -> bool {
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        let items = self.selection.items().to_vec();
        match apply(doc, &items, key, edit, self.current_material) {
            Ok(Applied::Unchanged) => false,
            Ok(Applied::Changed) => {
                // Hidden or erased entities leave the selection only when gone.
                self.selection
                    .retain(|o| crate::selection::exists(doc.active(), o));
                true
            }
            Ok(Applied::Reselect(items)) => {
                self.selection.clear();
                self.selection.apply(crate::selection::Mode::Replace, items);
                true
            }
            Err(message) => {
                self.message = Some(message);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    fn doc_with(f: impl FnOnce(&mut mt_kernel::Entities)) -> Document {
        let mut doc = Document::untitled();
        doc.edit(|e| {
            f(e);
            true
        });
        doc
    }

    fn field<'a>(info: &'a EntityInfo, key: &str) -> &'a InfoValue {
        &info
            .fields
            .iter()
            .find(|f| f.key == key)
            .unwrap_or_else(|| panic!("no field {key} in {info:?}"))
            .value
    }

    fn text(t: &str) -> InfoEdit {
        InfoEdit::Text(t.into())
    }

    #[test]
    fn edge_length_soft_and_smooth() {
        let mut doc = doc_with(|e| {
            mt_kernel::add_line(e, DVec3::ZERO, DVec3::new(10.0, 0.0, 0.0));
        });
        doc.units = LengthUnit::Inch;
        let e = doc.active().edges.keys().next().unwrap();
        let items = [Owner::Edge(e)];
        let info = describe(&doc, &items);
        assert_eq!(info.title, "Edge");
        assert_eq!(
            field(&info, "length"),
            &InfoValue::Text(LengthUnit::Inch.format(10.0))
        );
        let undo_before = doc.can_undo();
        assert_eq!(
            apply(&mut doc, &items, "length", &text("25"), None),
            Ok(Applied::Changed)
        );
        assert!(doc.can_undo() && undo_before);
        let x = &doc.active().edges[e];
        assert!((doc.active().pos(x.start).distance(doc.active().pos(x.end)) - 25.0).abs() < 1e-9);
        assert!(apply(&mut doc, &items, "length", &text("abc"), None).is_err());
        assert!(apply(&mut doc, &items, "length", &text("-3"), None).is_err());
        assert_eq!(
            apply(&mut doc, &items, "soft", &InfoEdit::Check(true), None),
            Ok(Applied::Changed)
        );
        assert_eq!(
            apply(&mut doc, &items, "soft", &InfoEdit::Check(true), None),
            Ok(Applied::Unchanged)
        );
        apply(&mut doc, &items, "smooth", &InfoEdit::Check(true), None).unwrap();
        let info = describe(&doc, &items);
        assert_eq!(field(&info, "soft"), &InfoValue::Check(true));
        assert_eq!(field(&info, "smooth"), &InfoValue::Check(true));
        // One undo step per edit: undo takes smooth back off, keeps soft.
        assert!(doc.undo());
        let info = describe(&doc, &items);
        assert_eq!(field(&info, "smooth"), &InfoValue::Check(false));
        assert_eq!(field(&info, "soft"), &InfoValue::Check(true));
    }

    #[test]
    fn circle_radius_and_segments() {
        let arc = mt_kernel::Arc {
            center: DVec3::ZERO,
            normal: DVec3::Z,
            x_axis: DVec3::X,
            radius: 10.0,
            start_angle: 0.0,
            end_angle: std::f64::consts::TAU,
            segments: 24,
        };
        let mut doc = doc_with(|e| {
            mt_kernel::add_curve(e, &arc.points(), Some(arc));
        });
        doc.units = LengthUnit::Inch;
        let items: Vec<Owner> = doc.active().edges.keys().map(Owner::Edge).collect();
        let info = describe(&doc, &items);
        assert_eq!(info.title, "Circle");
        assert_eq!(
            field(&info, "segments"),
            &InfoValue::Integer {
                value: 24,
                min: 3,
                max: 999
            }
        );
        assert_eq!(
            apply(&mut doc, &items, "radius", &text("5"), None),
            Ok(Applied::Changed)
        );
        let c = doc.active().curves.values().next().unwrap();
        assert!((c.arc.unwrap().radius - 5.0).abs() < 1e-9);
        let Ok(Applied::Reselect(items)) =
            apply(&mut doc, &items, "segments", &InfoEdit::Integer(8), None)
        else {
            panic!("segments redraw the circle");
        };
        assert_eq!(items.len(), 8);
        let info = describe(&doc, &items);
        assert_eq!(info.title, "Circle");
        assert_eq!(
            field(&info, "radius"),
            &InfoValue::Text(LengthUnit::Inch.format(5.0))
        );
        assert_eq!(doc.active().faces.len(), 1);
    }

    #[test]
    fn face_swatches_and_shadows() {
        let mut doc = doc_with(|e| {
            let p = [
                DVec3::ZERO,
                DVec3::new(10.0, 0.0, 0.0),
                DVec3::new(10.0, 10.0, 0.0),
                DVec3::new(0.0, 10.0, 0.0),
            ];
            for i in 0..4 {
                mt_kernel::add_line(e, p[i], p[(i + 1) % 4]);
            }
        });
        doc.model.materials.push(mt_kernel::Material {
            name: "Red".into(),
            color: [200, 0, 0],
            opacity: 1.0,
            texture: None,
        });
        let f = doc.active().faces.keys().next().unwrap();
        let items = [Owner::Face(f)];
        let info = describe(&doc, &items);
        assert_eq!(field(&info, "front"), &InfoValue::Swatch(None));
        assert_eq!(info.rows[0].0, "Area");
        apply(&mut doc, &items, "back", &InfoEdit::Swatch, Some(0)).unwrap();
        apply(
            &mut doc,
            &items,
            "cast_shadows",
            &InfoEdit::Check(false),
            None,
        )
        .unwrap();
        let info = describe(&doc, &items);
        assert_eq!(field(&info, "back"), &InfoValue::Swatch(Some([200, 0, 0])));
        assert_eq!(field(&info, "front"), &InfoValue::Swatch(None));
        assert_eq!(field(&info, "cast_shadows"), &InfoValue::Check(false));
        assert_eq!(field(&info, "receive_shadows"), &InfoValue::Check(true));
    }

    #[test]
    fn component_names_flags_and_advanced_attributes() {
        let mut doc = Document::untitled();
        let i = crate::solid_tools::tests::add_box(&mut doc, DVec3::ZERO, DVec3::ONE);
        let d = doc.active().instances[i].definition;
        doc.model.definitions[d].is_group = false;
        doc.model.definitions[d].name = "Box".into();
        let items = [Owner::Instance(i)];
        let info = describe(&doc, &items);
        assert_eq!(info.title, "Solid Component");
        assert_eq!(
            field(&info, "definition_name"),
            &InfoValue::Text("Box".into())
        );
        assert_eq!(field(&info, "advanced"), &InfoValue::Section(false));
        for (key, value) in [
            ("instance_name", "Left"),
            ("definition_name", "Crate"),
            ("price", "12.50"),
            ("url", "https://example.com"),
            ("type", "Furniture"),
            ("status", "New"),
            ("owner", "Me"),
        ] {
            assert_eq!(
                apply(&mut doc, &items, key, &text(value), None),
                Ok(Applied::Changed),
                "{key}"
            );
        }
        assert!(apply(&mut doc, &items, "definition_name", &text(""), None).is_err());
        apply(&mut doc, &items, "locked", &InfoEdit::Check(true), None).unwrap();
        apply(
            &mut doc,
            &items,
            "receive_shadows",
            &InfoEdit::Check(false),
            None,
        )
        .unwrap();
        let inst = &doc.active().instances[i];
        assert_eq!(inst.name, "Left");
        assert!(inst.locked && !inst.attrs.shadows.receive && inst.attrs.shadows.cast);
        assert_eq!(
            (inst.attrs.status.as_str(), inst.attrs.owner.as_str()),
            ("New", "Me")
        );
        let def = &doc.model.definitions[d];
        assert_eq!(def.name, "Crate");
        assert_eq!(def.attributes.price, "12.50");
        assert_eq!(def.attributes.kind, "Furniture");
        let info = describe(&doc, &items);
        assert_eq!(
            field(&info, "url"),
            &InfoValue::Text("https://example.com".into())
        );
        // A group has no definition name or advanced attributes.
        doc.model.definitions[d].is_group = true;
        let info = describe(&doc, &items);
        assert!(
            !info
                .fields
                .iter()
                .any(|f| f.key == "definition_name" || f.key == "price")
        );
    }

    #[test]
    fn section_plane_name_and_symbol() {
        let mut doc = doc_with(|e| {
            e.section_planes.push(mt_kernel::SectionPlane {
                point: DVec3::ZERO,
                normal: DVec3::X,
                name: "Section Plane 1".into(),
                symbol: String::new(),
                hidden: false,
            });
        });
        let items = [Owner::Section(0)];
        apply(&mut doc, &items, "name", &text("North"), None).unwrap();
        apply(&mut doc, &items, "symbol", &text("A1"), None).unwrap();
        let info = describe(&doc, &items);
        assert_eq!(field(&info, "name"), &InfoValue::Text("North".into()));
        assert_eq!(field(&info, "symbol"), &InfoValue::Text("A1".into()));
    }

    #[test]
    fn areas() {
        assert_eq!(format_area(144.0, LengthUnit::Inch), "144.00 in²");
        assert_eq!(format_area(144.0, LengthUnit::Foot), "1.00 ft²");
        assert_eq!(format_area(1e6 / 645.16, LengthUnit::Meter), "1.00 m²");
    }
}
