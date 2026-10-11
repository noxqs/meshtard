//! The Paint Bucket and its modes (FEATURES.md 50, help page "Replacing material").
//!
//! - Click: paints the clicked face side, or the clicked group or component; when the
//!   clicked entity is selected, the whole selection.
//! - Alt (Windows): samples the material under the cursor and makes it current.
//! - Shift, All Matching: every face side in the model that uses the clicked side's
//!   material (every definition, whatever context is open), or every instance with
//!   the clicked instance's material.
//! - Ctrl, All Connected: the faces connected to the clicked one (through shared
//!   edges) that use its material.
//! - Ctrl+Shift, All on Same Object: the faces of the clicked face's group or
//!   component (the open context) that use its material.
//!
//! Faces "use" a material on the side that was clicked (front or back): a mode paints
//! the same side of the faces it reaches (unverified how SketchUp treats faces whose
//! other side shows the material). Each click is one undo step.

use glam::DVec3;
use mt_infer::Inference;
use mt_infer::units::LengthUnit;
use mt_kernel::{Entities, FaceId, MaterialId, Model};

use crate::cursor::Mods;
use crate::document::Owner;
use crate::modify::{Ctx, Input, ModelEdit, ModifyTool};

/// Which faces a click paints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    /// The clicked side (or the selection).
    One,
    /// Shift: every face side in the model with the same material.
    AllMatching,
    /// Ctrl: connected faces with the same material.
    Connected,
    /// Ctrl+Shift: faces of the same object (context) with the same material.
    SameObject,
}

impl Fill {
    pub fn from_mods(ctrl: bool, shift: bool) -> Self {
        match (ctrl, shift) {
            (false, false) => Fill::One,
            (false, true) => Fill::AllMatching,
            (true, false) => Fill::Connected,
            (true, true) => Fill::SameObject,
        }
    }
}

/// The material on one side of a face.
fn side(ents: &Entities, f: FaceId, back: bool) -> Option<Option<MaterialId>> {
    let a = &ents.faces.get(f)?.attrs;
    Some(if back { a.back_material } else { a.material })
}

/// Faces reached from `start` by Ctrl (connected) or Ctrl+Shift (same object) whose
/// side `back` uses the same material as `start`'s.
pub fn reach(ents: &Entities, start: FaceId, back: bool, fill: Fill) -> Vec<FaceId> {
    let Some(m) = side(ents, start, back) else {
        return Vec::new();
    };
    match fill {
        Fill::One | Fill::AllMatching => vec![start],
        Fill::SameObject => ents
            .faces
            .keys()
            .filter(|&f| side(ents, f, back) == Some(m))
            .collect(),
        Fill::Connected => {
            let mut seen = std::collections::HashSet::from([start]);
            let mut stack = vec![start];
            let mut out = Vec::new();
            while let Some(f) = stack.pop() {
                out.push(f);
                for l in ents.faces[f].loops() {
                    for c in &l.coedges {
                        let Some(e) = ents.edges.get(c.edge) else {
                            continue;
                        };
                        for &g in &e.faces {
                            if side(ents, g, back) == Some(m) && seen.insert(g) {
                                stack.push(g);
                            }
                        }
                    }
                }
            }
            out
        }
    }
}

/// Shift: replaces `from` by `to` on side `back` of every face (`instances`: on every
/// group and component instead), in the root, every definition and `active` (the open
/// context, taken out of the model while it is edited). Returns whether anything
/// changed.
pub fn replace_everywhere(
    model: &mut Model,
    active: &mut Entities,
    from: Option<MaterialId>,
    to: Option<MaterialId>,
    back: bool,
    instances: bool,
) -> bool {
    let mut changed = false;
    let mut one = |ents: &mut Entities| {
        if instances {
            for i in &mut ents.instances {
                if i.material == from {
                    changed |= i.material != to;
                    i.material = to;
                }
            }
            return;
        }
        for f in ents.faces.values_mut() {
            let slot = if back {
                &mut f.attrs.back_material
            } else {
                &mut f.attrs.material
            };
            if *slot == from {
                changed |= *slot != to;
                *slot = to;
            }
        }
    };
    one(active);
    one(&mut model.root);
    for d in model.definitions.values_mut() {
        one(&mut d.entities);
    }
    changed
}

#[derive(Default)]
pub struct PaintBucket {
    material: Option<usize>,
    held: Mods,
    /// The material shown under the cursor (see [`Input::material`]).
    under: Option<Option<usize>>,
    /// Alt-sampled material for the app to make current.
    sampled: Option<Option<usize>>,
    /// Shift's model-wide replacement, made by the app as one undo step.
    edit: Option<ModelEdit>,
}

impl std::fmt::Debug for PaintBucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaintBucket")
            .field("material", &self.material)
            .field("held", &self.held)
            .finish()
    }
}

impl PaintBucket {
    /// The clicked side of face `f` is the back (the camera looks at its back).
    fn clicked_back(ctx: &Ctx, f: FaceId, inf: &Inference) -> bool {
        ctx.ents
            .faces
            .get(f)
            .is_some_and(|face| (ctx.eye - inf.point).dot(face.normal) < 0.0)
    }

    /// Paints with modifiers held as `held`. Returns whether the active context
    /// changed (Shift's change is left in `self.edit`).
    fn paint(&mut self, ctx: &mut Ctx, inf: &Inference, held: Mods) -> bool {
        let Some(picked) = ctx.picked else {
            return false;
        };
        if held.alt {
            // Sample the material under the cursor: the one shown (also inside groups
            // and components), else the clicked side's or instance's.
            let m = match picked {
                _ if self.under.is_some() => self.under.flatten(),
                Owner::Face(f) => side(ctx.ents, f, Self::clicked_back(ctx, f, inf)).flatten(),
                Owner::Instance(i) => ctx.ents.instances.get(i).and_then(|i| i.material),
                _ => return false,
            };
            self.material = m;
            self.sampled = Some(m);
            return false;
        }
        let m = self.material;
        let fill = Fill::from_mods(held.ctrl, held.shift);
        match (fill, picked) {
            (Fill::AllMatching, Owner::Face(f)) => {
                let back = Self::clicked_back(ctx, f, inf);
                let from = side(ctx.ents, f, back).flatten();
                self.edit = Some(Box::new(move |model, active| {
                    replace_everywhere(model, active, from, m, back, false);
                    Vec::new()
                }));
                return false;
            }
            (Fill::AllMatching, Owner::Instance(i)) => {
                let Some(from) = ctx.ents.instances.get(i).map(|i| i.material) else {
                    return false;
                };
                self.edit = Some(Box::new(move |model, active| {
                    replace_everywhere(model, active, from, m, false, true);
                    Vec::new()
                }));
                return false;
            }
            (Fill::Connected | Fill::SameObject, Owner::Face(f)) => {
                let back = Self::clicked_back(ctx, f, inf);
                let mut changed = false;
                for g in reach(ctx.ents, f, back, fill) {
                    let a = &mut ctx.ents.faces[g].attrs;
                    let slot = if back {
                        &mut a.back_material
                    } else {
                        &mut a.material
                    };
                    changed |= *slot != m;
                    *slot = m;
                }
                return changed;
            }
            _ => {}
        }
        let targets: Vec<Owner> = if ctx.selection.contains(&picked) {
            ctx.selection.to_vec()
        } else {
            vec![picked]
        };
        let mut changed = false;
        for t in targets {
            match t {
                Owner::Face(f) => {
                    // The side facing the camera (front for the rest of a selection).
                    let back = t == picked && Self::clicked_back(ctx, f, inf);
                    let Some(face) = ctx.ents.faces.get_mut(f) else {
                        continue;
                    };
                    let slot = if back {
                        &mut face.attrs.back_material
                    } else {
                        &mut face.attrs.material
                    };
                    changed |= *slot != m;
                    *slot = m;
                }
                Owner::Instance(i) => {
                    if let Some(inst) = ctx.ents.instances.get_mut(i) {
                        changed |= inst.material != m;
                        inst.material = m;
                    }
                }
                Owner::Edge(_) | Owner::Section(_) | Owner::Dimension(_) | Owner::Text(_) => {}
            }
        }
        changed
    }
}

impl ModifyTool for PaintBucket {
    fn status(&self) -> String {
        "Select a face or object to paint. Alt = sample material, Shift = all matching, \
         Ctrl = all connected, Ctrl+Shift = all on same object."
            .into()
    }

    fn measurement(&self, _cur: Option<&Inference>, _units: LengthUnit) -> (&'static str, String) {
        ("", String::new())
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    fn input(&mut self, i: &Input) {
        self.held = i.held;
        self.under = i.material;
    }

    fn click(&mut self, ctx: &mut Ctx, inf: &Inference) -> bool {
        let held = self.held;
        self.paint(ctx, inf, held)
    }

    fn typed(
        &mut self,
        _ctx: &mut Ctx,
        _cur: Option<&Inference>,
        _text: &str,
        _units: LengthUnit,
    ) -> bool {
        false
    }

    fn preview(&self, _ents: &Entities, _cur: &Inference) -> Vec<Vec<DVec3>> {
        Vec::new()
    }

    fn cancel(&mut self) {}

    fn set_material(&mut self, material: Option<usize>) {
        self.material = material;
    }

    fn take_sampled_material(&mut self) -> Option<Option<usize>> {
        self.sampled.take()
    }

    fn take_model_edit(&mut self) -> Option<ModelEdit> {
        self.edit.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_infer::Kind;

    fn inf(p: DVec3) -> Inference {
        Inference {
            point: p,
            kind: Kind::OnFace,
            edge: None,
            guide: None,
            locked: false,
            normal: None,
            segment: None,
            container: None,
        }
    }

    /// A closed 10" cube (six faces, outward normals) and a separate square at x = 30.
    fn cube_and_square() -> (Entities, FaceId) {
        let mut ents = Entities::default();
        let q = [
            DVec3::ZERO,
            DVec3::new(10.0, 0.0, 0.0),
            DVec3::new(10.0, 10.0, 0.0),
            DVec3::new(0.0, 10.0, 0.0),
        ];
        for i in 0..4 {
            mt_kernel::add_line(&mut ents, q[i], q[(i + 1) % 4]);
        }
        let base = ents.faces.keys().next().unwrap();
        // Up, whichever way the drawn face looks.
        let d = 10.0 * ents.faces[base].normal.z.signum();
        mt_kernel::push_pull(&mut ents, base, d, false).unwrap();
        for i in 0..4 {
            let o = DVec3::new(30.0, 0.0, 0.0);
            mt_kernel::add_line(&mut ents, q[i] + o, q[(i + 1) % 4] + o);
        }
        let square = ents
            .faces
            .iter()
            .find(|(_, f)| ents.loop_points(&f.outer)[0].x >= 30.0)
            .map(|(id, _)| id)
            .unwrap();
        (ents, square)
    }

    fn top(ents: &Entities) -> FaceId {
        ents.faces
            .iter()
            .find(|(_, f)| f.normal.z > 0.9 && ents.loop_points(&f.outer)[0].z > 9.0)
            .map(|(id, _)| id)
            .unwrap()
    }

    fn click(pb: &mut PaintBucket, ents: &mut Entities, f: FaceId, held: Mods) -> bool {
        let p = ents.loop_points(&ents.faces[f].outer)[0];
        let n = ents.faces[f].normal;
        let mut ctx = Ctx {
            ents,
            picked: Some(Owner::Face(f)),
            selection: &[],
            ctrl: false,
            eye: p + n * 100.0,
        };
        pb.paint(&mut ctx, &inf(p), held)
    }

    fn fronts(ents: &Entities, m: Option<usize>) -> usize {
        ents.faces
            .values()
            .filter(|f| f.attrs.material == m)
            .count()
    }

    #[test]
    fn plain_click_paints_one_face() {
        let (mut ents, _) = cube_and_square();
        let mut pb = PaintBucket::default();
        pb.set_material(Some(0));
        let t = top(&ents);
        assert!(click(&mut pb, &mut ents, t, Mods::default()));
        assert_eq!(fronts(&ents, Some(0)), 1);
        assert_eq!(ents.faces[t].attrs.material, Some(0));
    }

    #[test]
    fn ctrl_paints_connected_faces_with_the_same_material() {
        let (mut ents, _) = cube_and_square();
        let t = top(&ents);
        // One side of the cube already red (1): Ctrl from the top paints the five
        // default faces of the cube, not the red one, not the separate square.
        let side = ents
            .faces
            .iter()
            .find(|(_, f)| f.normal.x < -0.9)
            .map(|(id, _)| id)
            .unwrap();
        ents.faces[side].attrs.material = Some(1);
        let mut pb = PaintBucket::default();
        pb.set_material(Some(0));
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        assert!(click(&mut pb, &mut ents, t, ctrl));
        assert_eq!(fronts(&ents, Some(0)), 5);
        assert_eq!(ents.faces[side].attrs.material, Some(1));
        assert_eq!(fronts(&ents, None), 1, "the square stays");
    }

    #[test]
    fn ctrl_shift_paints_the_whole_context_with_the_same_material() {
        let (mut ents, square) = cube_and_square();
        let t = top(&ents);
        let mut pb = PaintBucket::default();
        pb.set_material(Some(2));
        let held = Mods {
            ctrl: true,
            shift: true,
            alt: false,
        };
        assert!(click(&mut pb, &mut ents, t, held));
        assert_eq!(fronts(&ents, Some(2)), 7, "cube and square");
        assert_eq!(ents.faces[square].attrs.material, Some(2));
        // Backs untouched.
        assert!(ents.faces.values().all(|f| f.attrs.back_material.is_none()));
    }

    #[test]
    fn shift_replaces_the_material_in_the_whole_model() {
        let (mut ents, _) = cube_and_square();
        let t = top(&ents);
        ents.faces[t].attrs.material = Some(1);
        // A definition with a face in material 1 and one in material 3.
        let mut model = Model::default();
        let (mut def, _) = cube_and_square();
        let mut it = def.faces.keys();
        let (a, b) = (it.next().unwrap(), it.next().unwrap());
        def.faces[a].attrs.material = Some(1);
        def.faces[b].attrs.material = Some(3);
        model.definitions.insert(mt_kernel::Definition {
            name: "Box".into(),
            entities: def,
            ..Default::default()
        });
        let mut pb = PaintBucket::default();
        pb.set_material(Some(4));
        let shift = Mods {
            shift: true,
            ..Mods::default()
        };
        assert!(
            !click(&mut pb, &mut ents, t, shift),
            "left to the model edit"
        );
        let edit = pb.take_model_edit().expect("a model-wide edit");
        edit(&mut model, &mut ents);
        assert_eq!(ents.faces[t].attrs.material, Some(4));
        assert_eq!(fronts(&ents, Some(4)), 1);
        let def = &model.definitions.values().next().unwrap().entities;
        assert_eq!(def.faces[a].attrs.material, Some(4));
        assert_eq!(def.faces[b].attrs.material, Some(3));
    }

    #[test]
    fn alt_samples_the_clicked_side() {
        let (mut ents, _) = cube_and_square();
        let t = top(&ents);
        ents.faces[t].attrs.material = Some(5);
        let mut pb = PaintBucket::default();
        pb.set_material(Some(0));
        let alt = Mods {
            alt: true,
            ..Mods::default()
        };
        assert!(
            !click(&mut pb, &mut ents, t, alt),
            "sampling changes nothing"
        );
        assert_eq!(pb.take_sampled_material(), Some(Some(5)));
        assert_eq!(pb.take_sampled_material(), None);
        // The sampled material paints next.
        let other = ents
            .faces
            .iter()
            .find(|(_, f)| f.normal.x > 0.9)
            .map(|(id, _)| id)
            .unwrap();
        assert!(click(&mut pb, &mut ents, other, Mods::default()));
        assert_eq!(ents.faces[other].attrs.material, Some(5));
    }

    #[test]
    fn alt_samples_what_is_shown_inside_a_group() {
        let (mut ents, _) = cube_and_square();
        ents.instances.push(mt_kernel::Instance {
            definition: Default::default(),
            transform: glam::DMat4::IDENTITY,
            name: String::new(),
            hidden: false,
            locked: false,
            layer: None,
            material: None,
        });
        let mut pb = PaintBucket::default();
        // The app reports the material shown under the cursor (a face inside).
        pb.input(&Input {
            held: Mods {
                alt: true,
                ..Mods::default()
            },
            press: false,
            ray: None,
            cur: None,
            material: Some(Some(7)),
        });
        let mut ctx = Ctx {
            ents: &mut ents,
            picked: Some(Owner::Instance(0)),
            selection: &[],
            ctrl: false,
            eye: DVec3::ZERO,
        };
        assert!(!pb.click(&mut ctx, &inf(DVec3::ZERO)));
        assert_eq!(pb.take_sampled_material(), Some(Some(7)));
    }
}
