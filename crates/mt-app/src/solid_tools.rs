//! Tools > Solid Tools (Outer Shell, Intersect, Union, Subtract, Trim, Split) on the
//! groups and components of the open context, and the solid rows of Entity Info.
//!
//! The geometry comes from `mt_kernel::solid`; this module picks the operands, removes
//! or keeps them and places each result as a new group in the open context, all in one
//! undo step. Unverified against SketchUp 2026 (`spec/parity/TRIAL_CHECKLIST.md`):
//! - results are groups named `Group#n` (SketchUp makes a group; its naming unverified);
//! - Union, Intersect, Subtract, Split and Outer Shell delete their inputs; Trim keeps
//!   the first solid (operand order as in `mt_kernel::solid`);
//! - a result takes the tag of the input it replaces (the second solid for Subtract and
//!   Trim, the first otherwise).

use mt_infer::units::LengthUnit;
use mt_kernel::solid::{self, SolidError, SolidInput};
use mt_kernel::{Entities, Model};

use crate::document::Document;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolidOp {
    OuterShell,
    Intersect,
    Union,
    Subtract,
    Trim,
    Split,
}

impl SolidOp {
    pub fn from_command(id: &str) -> Option<Self> {
        Some(match id {
            "tool.outer_shell" => Self::OuterShell,
            "tool.solid_intersect" => Self::Intersect,
            "tool.solid_union" => Self::Union,
            "tool.solid_subtract" => Self::Subtract,
            "tool.solid_trim" => Self::Trim,
            "tool.solid_split" => Self::Split,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::OuterShell => "Outer Shell",
            Self::Intersect => "Intersect",
            Self::Union => "Union",
            Self::Subtract => "Subtract",
            Self::Trim => "Trim",
            Self::Split => "Split",
        }
    }

    /// Status bar text while picking the first (`first` = true) or second solid.
    /// Draft wording.
    pub fn status(self, first: bool) -> String {
        let n = self.name();
        match (self, first) {
            (Self::Subtract | Self::Trim, true) => {
                format!("{n}: Select the solid group or component to cut with.")
            }
            (Self::Subtract, false) => {
                format!(
                    "{n}: Select the solid to subtract it from (both are replaced by the result)."
                )
            }
            (Self::Trim, false) => {
                format!("{n}: Select the solid to trim (the first solid is kept).")
            }
            (_, true) => format!("{n}: Select the first solid group or component."),
            (_, false) => format!("{n}: Select the second solid group or component."),
        }
    }
}

/// What an instance of the open context is to the Solid Tools (cursor feedback).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolidPick {
    /// A solid group or component: instance index.
    Solid(usize),
    /// A group or component that is not a closed solid, or other geometry.
    NotSolid,
    /// A locked group or component.
    Locked,
}

impl SolidPick {
    /// Text drawn by the cursor (draft; SketchUp shows a cursor badge instead).
    pub fn label(self, doc: &Document) -> String {
        match self {
            Self::Solid(i) => {
                let group = doc
                    .active()
                    .instances
                    .get(i)
                    .and_then(|x| doc.model.definitions.get(x.definition))
                    .is_some_and(|d| d.is_group);
                if group {
                    "Solid Group"
                } else {
                    "Solid Component"
                }
            }
            Self::NotSolid => "Not a solid",
            Self::Locked => "Locked",
        }
        .into()
    }
}

/// Whether instance `i` of the open context is a usable solid.
pub fn classify(doc: &Document, i: usize) -> SolidPick {
    let Some(inst) = doc.active().instances.get(i) else {
        return SolidPick::NotSolid;
    };
    if inst.locked {
        return SolidPick::Locked;
    }
    match doc.model.definitions.get(inst.definition) {
        Some(d) if solid::is_solid(&d.entities) => SolidPick::Solid(i),
        _ => SolidPick::NotSolid,
    }
}

/// The solid instances among `items` (Outer Shell's selection).
pub fn solid_instances(doc: &Document, items: &[crate::document::Owner]) -> Vec<usize> {
    let mut out: Vec<usize> = items
        .iter()
        .filter_map(|o| match *o {
            crate::document::Owner::Instance(i) => match classify(doc, i) {
                SolidPick::Solid(i) => Some(i),
                _ => None,
            },
            _ => None,
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Runs `op` on instances `operands` of the open context (two, in pick order; any
/// number of at least two for Outer Shell). On success the results are new groups in
/// the open context (one undo step) and their instance indices are returned; on
/// failure nothing changes and the error is the status text.
pub fn run(doc: &mut Document, op: SolidOp, operands: &[usize]) -> Result<Vec<usize>, String> {
    let mut seen = operands.to_vec();
    seen.sort_unstable();
    seen.dedup();
    if seen.len() != operands.len() || operands.len() < 2 {
        return Err(format!("{}: select two different solids.", op.name()));
    }
    if op != SolidOp::OuterShell && operands.len() != 2 {
        return Err(format!("{}: select exactly two solids.", op.name()));
    }
    if let Some(&bad) = operands
        .iter()
        .find(|&&i| !matches!(classify(doc, i), SolidPick::Solid(_)))
    {
        return Err(match classify(doc, bad) {
            SolidPick::Locked => {
                format!("{}: a locked group or component cannot be used.", op.name())
            }
            _ => format!("{}: not a solid group or component.", op.name()),
        });
    }
    let (results, remove, layer) = compute(doc.active(), &doc.model, op, operands)
        .map_err(|e| match e {
            SolidError::NotSolid(_) => format!("{}: not a solid group or component.", op.name()),
            SolidError::Failed => format!(
                "{} failed: the result is not a solid (solids touching only along an edge or at a point?).",
                op.name()
            ),
        })?;
    let mut made = Vec::new();
    doc.edit_model(|model, ents| {
        let mut remove = remove.clone();
        remove.sort_unstable();
        for &i in remove.iter().rev() {
            ents.instances.remove(i);
        }
        for r in &results {
            if let Some(i) = add_group(model, ents, r, layer) {
                made.push(i);
            }
        }
        true
    });
    Ok(made)
}

/// The result pieces, the instances to remove and the tag for the results.
type Computed = (Vec<Entities>, Vec<usize>, Option<usize>);

fn compute(
    ents: &Entities,
    model: &Model,
    op: SolidOp,
    operands: &[usize],
) -> Result<Computed, SolidError> {
    let inputs: Vec<SolidInput> = operands
        .iter()
        .map(|&i| {
            let inst = &ents.instances[i];
            SolidInput {
                ents: &model.definitions[inst.definition].entities,
                transform: inst.transform,
                material: inst.material,
            }
        })
        .collect();
    let layer_of = |k: usize| ents.instances[operands[k]].layer;
    if op == SolidOp::OuterShell {
        return Ok((
            vec![solid::outer_shell(&inputs)?],
            operands.to_vec(),
            layer_of(0),
        ));
    }
    let (a, b) = (inputs[0], inputs[1]);
    let both = operands.to_vec();
    Ok(match op {
        SolidOp::Union => (vec![solid::union(a, b)?], both, layer_of(0)),
        SolidOp::Intersect => (vec![solid::intersect(a, b)?], both, layer_of(0)),
        SolidOp::Subtract => (vec![solid::subtract(a, b)?], both, layer_of(1)),
        SolidOp::Trim => (vec![solid::trim(a, b)?], vec![operands[1]], layer_of(1)),
        SolidOp::Split => {
            let s = solid::split(a, b)?;
            (vec![s.first_only, s.both, s.second_only], both, layer_of(0))
        }
        SolidOp::OuterShell => unreachable!("handled above"),
    })
}

/// Places `result` (open-context coordinates) as a new group; `None` when it is empty.
fn add_group(
    model: &mut Model,
    ents: &mut Entities,
    result: &Entities,
    layer: Option<usize>,
) -> Option<usize> {
    if result.faces.is_empty() {
        return None;
    }
    // Make Group on a scratch copy holding only the result: the group's origin goes
    // to the bounding-box corner as for Edit > Create Group, and the result does not
    // merge with the context's loose geometry.
    let mut scratch = result.clone();
    let sel = mt_kernel::group::Selection {
        edges: scratch.edges.keys().collect(),
        faces: scratch.faces.keys().collect(),
        instances: Vec::new(),
    };
    let name = model.unique_definition_name("Group#1");
    let i = mt_kernel::group::make_group(&mut model.definitions, &mut scratch, &sel, name, true)?;
    let mut inst = scratch.instances.swap_remove(i);
    inst.layer = layer;
    ents.instances.push(inst);
    Some(ents.instances.len() - 1)
}

/// Entity Info for a solid group or component: "Solid Group" / "Solid Component" and
/// its volume in model units. Leaves other instances as they are.
pub fn describe(doc: &Document, inst: &mt_kernel::Instance, info: &mut mt_ui::EntityInfo) {
    let Some(def) = doc.model.definitions.get(inst.definition) else {
        return;
    };
    if !solid::is_solid(&def.entities) {
        return;
    }
    info.title = if def.is_group {
        "Solid Group"
    } else {
        "Solid Component"
    }
    .into();
    // A scaled instance scales the volume by the determinant.
    let scale = glam::DMat3::from_mat4(inst.transform).determinant().abs();
    let v = solid::volume(&def.entities) * scale;
    info.rows
        .push(("Volume".into(), format_volume(v, doc.units)));
}

/// A volume in cubic inches shown in `unit`s cubed, e.g. "1000.00 in³" (SketchUp's
/// format and precision unverified).
pub fn format_volume(cubic_inches: f64, unit: LengthUnit) -> String {
    let name = match unit {
        LengthUnit::Inch => "in",
        LengthUnit::Foot => "ft",
        LengthUnit::Millimeter => "mm",
        LengthUnit::Centimeter => "cm",
        LengthUnit::Meter => "m",
    };
    let v = cubic_inches / unit.inches().powi(3);
    format!("{v:.2} {name}³")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::document::Owner;
    use glam::DVec3;

    /// A closed box from `min` to `max` as a group of the root; returns its index.
    pub(crate) fn add_box(doc: &mut Document, min: DVec3, max: DVec3) -> usize {
        let mut made = None;
        doc.edit_model(|model, ents| {
            let mut scratch = Entities::new();
            let c = |x: usize, y: usize, z: usize| {
                DVec3::new(
                    if x == 0 { min.x } else { max.x },
                    if y == 0 { min.y } else { max.y },
                    if z == 0 { min.z } else { max.z },
                )
            };
            let mut line = |a: DVec3, b: DVec3| {
                mt_kernel::add_line(&mut scratch, a, b);
            };
            for z in 0..2 {
                line(c(0, 0, z), c(1, 0, z));
                line(c(1, 0, z), c(1, 1, z));
                line(c(1, 1, z), c(0, 1, z));
                line(c(0, 1, z), c(0, 0, z));
            }
            for (x, y) in [(0, 0), (1, 0), (1, 1), (0, 1)] {
                line(c(x, y, 0), c(x, y, 1));
            }
            assert!(solid::is_solid(&scratch), "the box closes");
            made = add_group(model, ents, &scratch, None);
            made.is_some()
        });
        made.unwrap()
    }

    fn two_boxes() -> Document {
        let mut doc = Document::untitled();
        add_box(&mut doc, DVec3::ZERO, DVec3::splat(10.0));
        add_box(&mut doc, DVec3::splat(5.0), DVec3::splat(15.0));
        doc
    }

    fn volumes(doc: &Document) -> Vec<f64> {
        doc.model
            .root
            .instances
            .iter()
            .map(|i| {
                let v = solid::volume(&doc.model.definitions[i.definition].entities);
                (v * 1000.0).round() / 1000.0
            })
            .collect()
    }

    #[test]
    fn union_replaces_both_with_one_group_in_one_step() {
        let mut doc = two_boxes();
        assert_eq!(classify(&doc, 0), SolidPick::Solid(0));
        let made = run(&mut doc, SolidOp::Union, &[0, 1]).unwrap();
        assert_eq!(made, vec![0]);
        assert_eq!(volumes(&doc), vec![2000.0 - 125.0]);
        let def = &doc.model.definitions[doc.model.root.instances[0].definition];
        assert!(def.is_group && def.name.starts_with("Group#"));
        assert!(doc.undo());
        assert_eq!(volumes(&doc), vec![1000.0, 1000.0], "one undo step");
    }

    #[test]
    fn subtract_trim_intersect_split_keep_the_right_inputs() {
        let mut doc = two_boxes();
        run(&mut doc, SolidOp::Subtract, &[0, 1]).unwrap();
        assert_eq!(volumes(&doc), vec![1000.0 - 125.0], "second minus first");

        let mut doc = two_boxes();
        run(&mut doc, SolidOp::Trim, &[0, 1]).unwrap();
        assert_eq!(volumes(&doc), vec![1000.0, 875.0], "the first is kept");

        let mut doc = two_boxes();
        run(&mut doc, SolidOp::Intersect, &[0, 1]).unwrap();
        assert_eq!(volumes(&doc), vec![125.0]);

        let mut doc = two_boxes();
        let made = run(&mut doc, SolidOp::Split, &[0, 1]).unwrap();
        assert_eq!(made.len(), 3);
        assert_eq!(volumes(&doc), vec![875.0, 125.0, 875.0]);

        let mut doc = two_boxes();
        run(&mut doc, SolidOp::OuterShell, &[1, 0]).unwrap();
        assert_eq!(volumes(&doc), vec![1875.0]);
    }

    #[test]
    fn not_solid_and_locked_inputs_change_nothing() {
        let mut doc = two_boxes();
        let d = doc.model.root.instances[1].definition;
        let f = doc.model.definitions[d]
            .entities
            .faces
            .keys()
            .next()
            .unwrap();
        doc.model.definitions[d].entities.remove_face(f);
        assert_eq!(classify(&doc, 1), SolidPick::NotSolid);
        let err = run(&mut doc, SolidOp::Union, &[0, 1]).unwrap_err();
        assert!(err.contains("not a solid"), "{err}");
        assert_eq!(doc.model.root.instances.len(), 2);

        let mut doc = two_boxes();
        doc.lock(&[Owner::Instance(0)], true);
        assert_eq!(classify(&doc, 0), SolidPick::Locked);
        assert!(run(&mut doc, SolidOp::Union, &[0, 1]).is_err());
        assert!(run(&mut doc, SolidOp::Union, &[1, 1]).is_err());
        assert_eq!(
            solid_instances(&doc, &[Owner::Instance(0), Owner::Instance(1)]),
            vec![1]
        );
    }

    #[test]
    fn entity_info_names_solids_and_shows_volume() {
        let mut doc = two_boxes();
        // A scaled instance: twice as long in x.
        doc.model.root.instances[0].transform *= glam::DMat4::from_scale(DVec3::new(2.0, 1.0, 1.0));
        let mut info = mt_ui::EntityInfo {
            title: "Group".into(),
            rows: Vec::new(),
        };
        let inst = doc.model.root.instances[0].clone();
        describe(&doc, &inst, &mut info);
        assert_eq!(info.title, "Solid Group");
        assert_eq!(info.rows, vec![("Volume".into(), "2000.00 in³".into())]);
        assert_eq!(format_volume(1e9 / 16387.064, LengthUnit::Meter), "1.00 m³");
    }
}
