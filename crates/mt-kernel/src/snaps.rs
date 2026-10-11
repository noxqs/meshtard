//! Component Snaps (SketchUp 2024 and later; help page "Snaps"): disks placed inside a
//! group or component that join front to front when the Move tool brings one to
//! another, or attach to a face.
//!
//! A snap is kept in the definition's [`Entities::snaps`], in definition coordinates,
//! so every instance of a component carries it. Model-only: `.skp` files do not
//! carry snaps here (mt-skp does not read or write them).
//!
//! Joining (help): "Snaps join front to front ... the orientation follows the target
//! Snap's Alignment Direction"; "Snap to face: the Snap's front attaches to the face
//! and orients to the blue axis by default". Here the moved instance turns so its
//! snap's front faces the target's (normals opposite) and the Alignment Directions
//! match, then moves so the two centers meet.

use glam::{DMat3, DMat4, DVec3};

use crate::TOL;
use crate::entities::{DefinitionId, Entities};
use crate::model::Definition;

/// A model's component definitions.
pub type Definitions = slotmap::SlotMap<DefinitionId, Definition>;

/// A snap disk.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snap {
    /// The Central Snap Point.
    pub position: DVec3,
    /// Unit normal of the front side, the side that attaches.
    pub normal: DVec3,
    /// Unit Alignment Direction, in the disk's plane (perpendicular to `normal`).
    pub align: DVec3,
}

impl Snap {
    /// A snap at `position` facing `normal`, aligned per [`default_align`].
    pub fn new(position: DVec3, normal: DVec3) -> Self {
        let normal = normal.normalize_or(DVec3::Z);
        Self {
            position,
            normal,
            align: default_align(normal),
        }
    }

    /// Carried by `xf` (normals by the inverse transpose; the Alignment Direction kept
    /// in the disk's plane).
    pub fn transformed(&self, xf: DMat4) -> Self {
        let m = DMat3::from_mat4(xf);
        let normal = (m.inverse().transpose() * self.normal).normalize_or(self.normal);
        let a = m * self.align;
        let align = (a - normal * a.dot(normal)).normalize_or(default_align(normal));
        Self {
            position: xf.transform_point3(self.position),
            normal,
            align,
        }
    }

    /// The disk turned over (help: Alt while hovering "reverses the Snap's
    /// orientation"); the Alignment Direction stays.
    pub fn reversed(&self) -> Self {
        Self {
            normal: -self.normal,
            ..*self
        }
    }

    /// Sets the Alignment Direction to `dir` flattened into the disk's plane; false
    /// when `dir` is along the normal.
    pub fn set_align(&mut self, dir: DVec3) -> bool {
        let a = (dir - self.normal * dir.dot(self.normal)).normalize_or_zero();
        if a == DVec3::ZERO {
            return false;
        }
        self.align = a;
        true
    }

    /// Right-handed frame: Alignment Direction, normal, their cross product.
    fn frame(&self) -> DMat3 {
        let n = self.normal.normalize_or(DVec3::Z);
        let a = (self.align - n * self.align.dot(n)).normalize_or(default_align(n));
        DMat3::from_cols(a, n, a.cross(n))
    }
}

/// The Alignment Direction a new snap or a face gets: the blue axis flattened into the
/// plane with `normal` [doc: "defaults to the blue axis"]; on a level plane, where blue
/// is the normal, the red axis (unverified).
pub fn default_align(normal: DVec3) -> DVec3 {
    let n = normal.normalize_or(DVec3::Z);
    let flat = |v: DVec3| v - n * v.dot(n);
    let z = flat(DVec3::Z);
    if z.length() > 1e-6 {
        z.normalize()
    } else {
        flat(DVec3::X).normalize_or(DVec3::Y)
    }
}

/// The new transform of an instance (now `inst_xf`) so that its snap `mine`
/// (definition coordinates) joins `target` (context coordinates): the centers meet,
/// the fronts face each other and the Alignment Directions agree. The instance's
/// scale is kept.
pub fn snap_to_snap(inst_xf: DMat4, mine: &Snap, target: &Snap) -> DMat4 {
    let me = mine.transformed(inst_xf);
    let facing = Snap {
        normal: -target.normal,
        ..*target
    };
    let r = facing.frame() * me.frame().transpose();
    DMat4::from_translation(target.position)
        * DMat4::from_mat3(r)
        * DMat4::from_translation(-me.position)
        * inst_xf
}

/// The new transform of an instance whose snap `mine` attaches to the face through
/// `point` with unit `normal` (context coordinates): the snap's front against the
/// face, its Alignment Direction along the face's default ([`default_align`]).
pub fn snap_to_face(inst_xf: DMat4, mine: &Snap, point: DVec3, normal: DVec3) -> DMat4 {
    snap_to_snap(inst_xf, mine, &Snap::new(point, normal))
}

/// The snaps of instance `i` of `ents`, in `ents`' coordinates.
pub fn instance_snaps(defs: &Definitions, ents: &Entities, i: usize) -> Vec<Snap> {
    let Some(inst) = ents.instances.get(i) else {
        return Vec::new();
    };
    defs.get(inst.definition)
        .map(|d| {
            d.entities
                .snaps
                .iter()
                .map(|s| s.transformed(inst.transform))
                .collect()
        })
        .unwrap_or_default()
}

/// Two snaps are joined: same center, fronts facing each other.
pub fn joined(a: &Snap, b: &Snap) -> bool {
    a.position.distance(b.position) <= TOL && a.normal.dot(b.normal) < -1.0 + 1e-6
}

/// Select > Connected by Snaps (RN 2025: "select all entities connected to the
/// selected entity by snaps"): the instances of `ents` reached from `start` through
/// joined snaps, `start` included.
pub fn connected_by_snaps(defs: &Definitions, ents: &Entities, start: &[usize]) -> Vec<usize> {
    let snaps: Vec<Vec<Snap>> = (0..ents.instances.len())
        .map(|i| instance_snaps(defs, ents, i))
        .collect();
    let mut seen: Vec<bool> = vec![false; snaps.len()];
    let mut stack: Vec<usize> = start.iter().copied().filter(|&i| i < snaps.len()).collect();
    let mut out = Vec::new();
    while let Some(i) = stack.pop() {
        if std::mem::replace(&mut seen[i], true) {
            continue;
        }
        out.push(i);
        for (j, other) in snaps.iter().enumerate() {
            if !seen[j]
                && !ents.instances[j].hidden
                && snaps[i].iter().any(|a| other.iter().any(|b| joined(a, b)))
            {
                stack.push(j);
            }
        }
    }
    out.sort_unstable();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::Instance;

    fn close(a: DVec3, b: DVec3) -> bool {
        a.distance(b) < 1e-9
    }

    #[test]
    fn default_alignment_is_blue_flattened() {
        assert!(close(default_align(DVec3::X), DVec3::Z));
        assert!(close(default_align(DVec3::Z), DVec3::X));
        let s = Snap::new(DVec3::ZERO, DVec3::new(1.0, 0.0, 1.0));
        assert!(s.align.dot(s.normal).abs() < 1e-12);
        assert!(s.align.z > 0.0);
    }

    #[test]
    fn snap_to_snap_meets_front_to_front() {
        // A leg's snap on its top (pointing up), a table top's on its underside.
        let mine = Snap::new(DVec3::new(0.0, 0.0, 30.0), DVec3::Z);
        let target = Snap {
            position: DVec3::new(10.0, 5.0, 40.0),
            normal: DVec3::new(0.0, 1.0, 0.0),
            align: DVec3::X,
        };
        // The instance starts turned and scaled.
        let inst = DMat4::from_translation(DVec3::new(-3.0, 2.0, 0.0))
            * DMat4::from_rotation_x(0.4)
            * DMat4::from_scale(DVec3::splat(2.0));
        let xf = snap_to_snap(inst, &mine, &target);
        let moved = mine.transformed(xf);
        assert!(close(moved.position, target.position));
        assert!(close(moved.normal, -target.normal));
        assert!(close(moved.align, target.align));
        // Scale kept, no mirror.
        let m = DMat3::from_mat4(xf);
        assert!((m.determinant() - 8.0).abs() < 1e-9);
        // Already joined: nothing changes.
        let again = snap_to_snap(xf, &mine, &target);
        assert!(again.abs_diff_eq(xf, 1e-9));
        assert!(joined(&moved, &target));
    }

    #[test]
    fn snap_to_face_puts_the_front_against_the_face() {
        let mine = Snap::new(DVec3::new(1.0, 0.0, 0.0), DVec3::X);
        let xf = snap_to_face(DMat4::IDENTITY, &mine, DVec3::new(0.0, 0.0, 10.0), DVec3::Z);
        let moved = mine.transformed(xf);
        assert!(close(moved.position, DVec3::new(0.0, 0.0, 10.0)));
        assert!(close(moved.normal, -DVec3::Z));
        assert!(close(moved.align, DVec3::X));
    }

    #[test]
    fn connected_by_snaps_follows_joined_snaps() {
        let mut defs = slotmap::SlotMap::with_key();
        let mut e = Entities::new();
        e.snaps = vec![
            Snap::new(DVec3::new(0.0, 0.0, 0.0), -DVec3::X),
            Snap::new(DVec3::new(10.0, 0.0, 0.0), DVec3::X),
        ];
        let d = defs.insert(Definition {
            entities: e,
            ..Default::default()
        });
        let mut ents = Entities::new();
        for x in [0.0, 10.0, 20.0, 50.0] {
            ents.instances.push(Instance {
                definition: d,
                transform: DMat4::from_translation(DVec3::new(x, 0.0, 0.0)),
                name: String::new(),
                hidden: false,
                locked: false,
                layer: None,
                material: None,
                attrs: Default::default(),
            });
        }
        assert_eq!(connected_by_snaps(&defs, &ents, &[1]), vec![0, 1, 2]);
        assert_eq!(connected_by_snaps(&defs, &ents, &[3]), vec![3]);
    }
}
