//! Context menu items of groups and components: Flip Along, Reset Scale, Reset Skew
//! and Scale Definition.
//!
//! Drafts from memory of SketchUp (`spec/parity/STATUS.md`):
//! - Flip Along > Component's Red / Green / Blue mirrors the instance along its own
//!   axis through the center of its bounding box, so it stays in place (unverified).
//! - Reset Scale removes scaling, skew and mirroring and keeps the rotation and the
//!   instance's origin (unverified: that it also undoes flips).
//! - Reset Skew makes the axes perpendicular again, keeping their lengths (the red
//!   axis keeps its direction, green stays in the red-green plane).
//! - Scale Definition bakes the instance's scale into the definition and resets the
//!   instance. Other instances of the definition are compensated so they look the
//!   same (unverified: SketchUp may let them change).

use glam::{DMat3, DMat4, DVec3};
use slotmap::SlotMap;

use crate::entities::{DefinitionId, EdgeId, Entities, FaceId, Instance};
use crate::model::{Definition, Model};

/// Below this the axes count as unscaled / perpendicular.
const EPS: f64 = 1e-9;

/// Bounding box of a definition's contents in its own space (nested instances
/// included), `None` when it is empty.
pub fn definition_bounds(
    defs: &SlotMap<DefinitionId, Definition>,
    def: DefinitionId,
) -> Option<(DVec3, DVec3)> {
    fn walk(
        defs: &SlotMap<DefinitionId, Definition>,
        def: DefinitionId,
        xf: DMat4,
        depth: usize,
        b: &mut Option<(DVec3, DVec3)>,
    ) {
        let Some(d) = defs.get(def) else { return };
        if depth > 64 {
            return;
        }
        let mut add = |p: DVec3| {
            let q = xf.transform_point3(p);
            *b = Some(match *b {
                Some((lo, hi)) => (lo.min(q), hi.max(q)),
                None => (q, q),
            });
        };
        for e in d.entities.edges.values() {
            add(d.entities.pos(e.start));
            add(d.entities.pos(e.end));
        }
        for n in &d.entities.instances {
            walk(defs, n.definition, xf * n.transform, depth + 1, b);
        }
    }
    let mut b = None;
    walk(defs, def, DMat4::IDENTITY, 0, &mut b);
    b
}

/// Flip Along: mirrors `inst` along its own axis `axis` (0 red, 1 green, 2 blue)
/// through the center of its bounding box.
pub fn flip_along(
    defs: &SlotMap<DefinitionId, Definition>,
    inst: &mut Instance,
    axis: usize,
) -> bool {
    if axis > 2 {
        return false;
    }
    let center =
        definition_bounds(defs, inst.definition).map_or(DVec3::ZERO, |(lo, hi)| (lo + hi) * 0.5);
    let mut s = DVec3::ONE;
    s[axis] = -1.0;
    let m =
        DMat4::from_translation(center) * DMat4::from_scale(s) * DMat4::from_translation(-center);
    inst.transform *= m;
    true
}

fn linear(m: &DMat4) -> DMat3 {
    DMat3::from_mat4(*m)
}

fn with_linear(m: &DMat4, l: DMat3) -> DMat4 {
    let mut out = DMat4::from_mat3(l);
    out.w_axis = m.w_axis;
    out
}

/// The rotation nearest the axes of `l` in SketchUp's sense: red keeps its direction,
/// green stays in the red-green plane, blue is red x green.
fn rotation_of(l: DMat3) -> Option<DMat3> {
    let x = l.x_axis.normalize_or_zero();
    let y = (l.y_axis - x * l.y_axis.dot(x)).normalize_or_zero();
    if x == DVec3::ZERO || y == DVec3::ZERO {
        return None;
    }
    Some(DMat3::from_cols(x, y, x.cross(y)))
}

fn close(a: DMat3, b: DMat3) -> bool {
    a.abs_diff_eq(b, EPS)
}

/// Reset Scale: the instance's axes become a plain rotation (unit, perpendicular,
/// right-handed); its origin stays.
pub fn reset_scale(inst: &mut Instance) -> bool {
    let l = linear(&inst.transform);
    let Some(r) = rotation_of(l) else {
        return false;
    };
    if close(l, r) {
        return false;
    }
    inst.transform = with_linear(&inst.transform, r);
    true
}

/// Reset Skew: makes the instance's axes perpendicular, keeping each axis's length
/// and the handedness.
pub fn reset_skew(inst: &mut Instance) -> bool {
    let l = linear(&inst.transform);
    let Some(r) = rotation_of(l) else {
        return false;
    };
    let mirror = l.determinant() < 0.0;
    let z = if mirror { -r.z_axis } else { r.z_axis };
    let out = DMat3::from_cols(
        r.x_axis * l.x_axis.length(),
        r.y_axis * l.y_axis.length(),
        z * l.z_axis.length(),
    );
    if close(l, out) {
        return false;
    }
    inst.transform = with_linear(&inst.transform, out);
    true
}

/// Whether the instance is scaled, skewed or mirrored (Reset Scale applies).
pub fn is_scaled(inst: &Instance) -> bool {
    let l = linear(&inst.transform);
    rotation_of(l).is_some_and(|r| !close(l, r))
}

/// Whether the instance's axes are not perpendicular (Reset Skew applies).
pub fn is_skewed(inst: &Instance) -> bool {
    let l = linear(&inst.transform);
    let (x, y, z) = (
        l.x_axis.normalize_or_zero(),
        l.y_axis.normalize_or_zero(),
        l.z_axis.normalize_or_zero(),
    );
    x.dot(y).abs() > EPS || x.dot(z).abs() > EPS || y.dot(z).abs() > EPS
}

/// `src` with every point carried by `xf` (geometry, nested instances, guides,
/// section planes, dimensions and texts).
pub fn transform_entities(src: &Entities, xf: DMat4) -> Entities {
    let mut out = Entities::new();
    let edges: Vec<EdgeId> = src.edges.keys().collect();
    let faces: Vec<FaceId> = src.faces.keys().collect();
    crate::group::copy_raw(&mut out, src, &edges, &faces, xf);
    let normal_xf = linear(&xf).inverse().transpose();
    out.instances = src
        .instances
        .iter()
        .map(|n| Instance {
            transform: xf * n.transform,
            ..n.clone()
        })
        .collect();
    out.guides = src
        .guides
        .iter()
        .map(|g| crate::entities::Guide {
            point: xf.transform_point3(g.point),
            dir: g.dir.map(|d| xf.transform_vector3(d).normalize_or_zero()),
        })
        .collect();
    out.section_planes = src
        .section_planes
        .iter()
        .map(|s| crate::entities::SectionPlane {
            point: xf.transform_point3(s.point),
            normal: (normal_xf * s.normal).normalize_or_zero(),
            ..s.clone()
        })
        .collect();
    out.active_section = src.active_section;
    out.snaps = src.snaps.iter().map(|s| s.transformed(xf)).collect();
    out.dimensions = src
        .dimensions
        .iter()
        .map(|d| crate::annotation::Dimension {
            a: xf.transform_point3(d.a),
            b: xf.transform_point3(d.b),
            offset: xf.transform_vector3(d.offset),
            ..d.clone()
        })
        .collect();
    out.texts = src
        .texts
        .iter()
        .map(|t| {
            if t.screen {
                t.clone()
            } else {
                crate::annotation::Text {
                    point: xf.transform_point3(t.point),
                    leader: xf.transform_vector3(t.leader),
                    ..t.clone()
                }
            }
        })
        .collect();
    out
}

/// Scale Definition: bakes the scale (and skew, mirror) of instance `index` of `ents`
/// into its definition, leaving the instance with a plain rotation. Every other
/// instance of the definition (in `model` or `ents`, the context taken out of it) is
/// compensated so nothing moves in the world.
pub fn scale_definition(model: &mut Model, ents: &mut Entities, index: usize) -> bool {
    let Some(inst) = ents.instances.get(index) else {
        return false;
    };
    let def = inst.definition;
    let l = linear(&inst.transform);
    let Some(r) = rotation_of(l) else {
        return false;
    };
    if close(l, r) || l.determinant().abs() < EPS {
        return false;
    }
    // transform = [r | t] * k, with k the scale to bake in.
    let k = DMat4::from_mat3(r.transpose() * l);
    let undo = k.inverse();
    let Some(d) = model.definitions.get_mut(def) else {
        return false;
    };
    d.entities = transform_entities(&d.entities, k);
    let fix = |e: &mut Entities| {
        for i in e.instances.iter_mut().filter(|i| i.definition == def) {
            i.transform *= undo;
        }
    };
    fix(ents);
    fix(&mut model.root);
    for d in model.definitions.values_mut() {
        fix(&mut d.entities);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Checks, add_line, validate};

    fn p(x: f64, y: f64, z: f64) -> DVec3 {
        DVec3::new(x, y, z)
    }

    /// A model with a 10 x 4 x 0 rectangle component placed twice in the root.
    fn model() -> (Model, DefinitionId) {
        let mut m = Model::default();
        let mut e = Entities::new();
        let c = [
            p(0.0, 0.0, 0.0),
            p(10.0, 0.0, 0.0),
            p(10.0, 4.0, 0.0),
            p(0.0, 4.0, 0.0),
        ];
        for i in 0..4 {
            add_line(&mut e, c[i], c[(i + 1) % 4]);
        }
        let def = m.definitions.insert(Definition {
            name: "Box".into(),
            entities: e,
            ..Default::default()
        });
        for xf in [
            DMat4::from_translation(p(5.0, 5.0, 0.0)),
            DMat4::from_translation(p(-20.0, 0.0, 0.0)),
        ] {
            m.root.instances.push(Instance {
                definition: def,
                transform: xf,
                name: String::new(),
                hidden: false,
                locked: false,
                layer: None,
                material: None,
                attrs: Default::default(),
            });
        }
        (m, def)
    }

    /// World positions of the corners of instance `i` (sorted).
    fn world(m: &Model, ents: &Entities, i: usize) -> Vec<[i64; 3]> {
        let inst = &ents.instances[i];
        let d = &m.definitions[inst.definition].entities;
        let mut v: Vec<[i64; 3]> = d
            .vertices
            .values()
            .map(|x| {
                let q = inst.transform.transform_point3(x.pos);
                [q.x, q.y, q.z].map(|c| (c * 1e6).round() as i64)
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn flip_along_stays_in_place() {
        let (mut m, def) = model();
        let before = world(&m, &m.root, 0);
        let bounds = definition_bounds(&m.definitions, def).unwrap();
        assert_eq!(bounds, (p(0.0, 0.0, 0.0), p(10.0, 4.0, 0.0)));
        for axis in 0..3 {
            let mut inst = m.root.instances[0].clone();
            assert!(flip_along(&m.definitions, &mut inst, axis));
            assert!(inst.transform.determinant() < 0.0);
            m.root.instances[0] = inst;
            // A rectangle's corners map onto themselves.
            assert_eq!(world(&m, &m.root, 0), before);
            let o = m.root.instances[0].transform.transform_point3(DVec3::ZERO);
            if axis == 0 {
                assert!(o.abs_diff_eq(p(15.0, 5.0, 0.0), 1e-12));
            }
            // Flip back.
            let mut inst = m.root.instances[0].clone();
            flip_along(&m.definitions, &mut inst, axis);
            m.root.instances[0] = inst;
        }
        assert!(
            m.root.instances[0]
                .transform
                .abs_diff_eq(DMat4::from_translation(p(5.0, 5.0, 0.0)), 1e-12)
        );
    }

    #[test]
    fn reset_scale_and_skew() {
        let (mut m, _) = model();
        let rot = DMat4::from_rotation_z(0.5);
        let t = DMat4::from_translation(p(1.0, 2.0, 3.0));
        let inst = &mut m.root.instances[0];
        inst.transform = t * rot * DMat4::from_scale(p(2.0, 3.0, 1.0));
        assert!(is_scaled(inst));
        assert!(!is_skewed(inst));
        assert!(!reset_skew(inst));
        assert!(reset_scale(inst));
        assert!(inst.transform.abs_diff_eq(t * rot, 1e-12));
        assert!(!reset_scale(inst));
        assert!(!is_scaled(inst));

        // A skew: green leans towards red.
        let skew = DMat4::from_cols(
            DVec3::X.extend(0.0) * 2.0,
            p(1.0, 1.0, 0.0).extend(0.0),
            DVec3::Z.extend(0.0),
            glam::DVec4::W,
        );
        inst.transform = t * rot * skew;
        assert!(is_skewed(inst));
        assert!(reset_skew(inst));
        assert!(!is_skewed(inst));
        let l = DMat3::from_mat4(inst.transform);
        assert!((l.x_axis.length() - 2.0).abs() < 1e-12);
        assert!((l.y_axis.length() - 2f64.sqrt()).abs() < 1e-12);
        assert!(l.determinant() > 0.0);
        assert!(
            inst.transform
                .w_axis
                .truncate()
                .abs_diff_eq(p(1.0, 2.0, 3.0), 1e-12)
        );

        // A mirror keeps its handedness through Reset Skew; Reset Scale removes it.
        inst.transform = DMat4::from_scale(p(-1.0, 1.0, 1.0));
        assert!(!reset_skew(inst));
        assert!(reset_scale(inst));
        assert!(DMat3::from_mat4(inst.transform).determinant() > 0.0);
    }

    #[test]
    fn scale_definition_keeps_world_geometry() {
        let (mut m, def) = model();
        let xf = DMat4::from_translation(p(5.0, 5.0, 0.0))
            * DMat4::from_rotation_z(0.3)
            * DMat4::from_scale(p(2.0, 0.5, 3.0));
        m.root.instances[0].transform = xf;
        let before = [world(&m, &m.root, 0), world(&m, &m.root, 1)];
        let mut root = std::mem::take(&mut m.root);
        assert!(scale_definition(&mut m, &mut root, 0));
        m.root = root;
        assert_eq!([world(&m, &m.root, 0), world(&m, &m.root, 1)], before);
        assert!(!is_scaled(&m.root.instances[0]));
        let (lo, hi) = definition_bounds(&m.definitions, def).unwrap();
        assert!((hi - lo).abs_diff_eq(p(20.0, 2.0, 0.0), 1e-9));
        let errs = validate(&m.definitions[def].entities, Checks::ALL);
        assert!(errs.is_empty(), "{errs:#?}");
        assert_eq!(m.definitions[def].entities.faces.len(), 1);
        // Nothing left to bake.
        let mut root = std::mem::take(&mut m.root);
        assert!(!scale_definition(&mut m, &mut root, 0));
    }
}
