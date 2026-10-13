//! Undo steps that share what did not change (spec/PERFORMANCE.md).
//!
//! A step holds the whole model: the settings (everything but the geometry, small:
//! copied into every step) and one [`EntitiesSnapshot`] per entities collection (the
//! root's and each definition's). A collection's vertices, edges and faces know which
//! chunks of their storage changed since their last snapshot
//! (`mt_kernel::slot_store`), so a snapshot copies only those chunks and shares the
//! rest with the snapshots before. Every write marks its chunk, whether or not it is
//! made inside an undoable edit, so a change made without an undo step (Scenes,
//! Texture Position's preview, tests) is never lost. [`Shared`] keeps the snapshot of
//! each collection in the last step, which the next step reuses when nothing changed.
//! Undo and redo rewrite only the chunks that differ between the two states.

use std::sync::Arc;

use mt_kernel::spatial::FxMap;
use mt_kernel::{DefinitionId, Entities, EntitiesSnapshot, Model};

/// An entities collection of the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ctx {
    Root,
    Def(DefinitionId),
}

/// A model state: see the module documentation.
pub struct Step {
    /// The model with every entities collection empty.
    shell: Model,
    root: Arc<EntitiesSnapshot>,
    /// The definitions' collections, in `shell.definitions` order.
    defs: Vec<(DefinitionId, Arc<EntitiesSnapshot>)>,
}

impl Step {
    /// The size ([`Entities::size`]) of its collections, shared or not.
    pub fn size(&self) -> usize {
        self.root.size() + self.defs.iter().map(|(_, e)| e.size()).sum::<usize>()
    }

    /// The model this step holds (a copy: tests compare it).
    #[cfg(test)]
    pub fn model(&self) -> Model {
        let mut m = self.shell.clone();
        m.root = self.root.to_entities();
        for (k, e) in &self.defs {
            m.definitions[*k].entities = e.to_entities();
        }
        m
    }
}

/// The snapshots the next step can reuse; see the module documentation.
#[derive(Default)]
pub struct Shared {
    copies: FxMap<Ctx, Arc<EntitiesSnapshot>>,
}

/// `model` with its entities collections left empty.
fn shell_of(model: &mut Model) -> Model {
    let root = std::mem::take(&mut model.root);
    let defs: Vec<Entities> = model
        .definitions
        .values_mut()
        .map(|d| std::mem::take(&mut d.entities))
        .collect();
    let shell = model.clone();
    model.root = root;
    for (d, e) in model.definitions.values_mut().zip(defs) {
        d.entities = e;
    }
    shell
}

/// Drops `x` on another thread when it is large (freeing a big collection takes a
/// noticeable time, and nothing waits for it).
pub fn discard<T: Send + 'static>(x: T, size: usize) {
    if size >= DISCARD_MIN {
        std::thread::spawn(move || drop(x));
    }
}

/// Collections at least this large ([`Entities::size`]) are freed on another thread.
const DISCARD_MIN: usize = 20_000;

impl Shared {
    /// Records that `ctx` was (or may have been) changed. Not needed for correctness
    /// (writes are tracked); it only drops a snapshot the next step will not reuse.
    pub fn changed(&mut self, ctx: Ctx) {
        if let Some(a) = self.copies.remove(&ctx) {
            let size = a.size();
            discard(a, size);
        }
    }

    /// Forgets the snapshots of definitions `model` no longer has.
    fn prune(&mut self, model: &Model) {
        let gone: Vec<Ctx> = (self.copies.keys())
            .filter(|c| matches!(c, Ctx::Def(d) if !model.definitions.contains_key(*d)))
            .copied()
            .collect();
        for c in gone {
            self.changed(c);
        }
    }

    /// The snapshot of collection `ctx` (holding `ents`), remembered for the next step.
    fn freeze(&mut self, ctx: Ctx, ents: &mut Entities) -> Arc<EntitiesSnapshot> {
        let s = ents.freeze(self.copies.get(&ctx));
        if let Some(old) = self.copies.insert(ctx, s.clone())
            && !Arc::ptr_eq(&old, &s)
        {
            let size = old.size();
            discard(old, size);
        }
        s
    }

    /// The current state of `model` as a step.
    pub fn take(&mut self, model: &mut Model) -> Step {
        let shell = shell_of(model);
        let root = self.freeze(Ctx::Root, &mut model.root);
        let defs = (model.definitions.iter_mut())
            .map(|(k, d)| (k, self.freeze(Ctx::Def(k), &mut d.entities)))
            .collect();
        self.prune(model);
        Step { shell, root, defs }
    }

    /// Makes `step` the model and returns the state it replaced as a step. Collections
    /// both states have are rewritten in place where they differ (keeping their
    /// spatial index when they do not).
    pub fn restore(&mut self, model: &mut Model, step: Step) -> Step {
        let Step { shell, root, defs } = step;
        let mut old = std::mem::replace(model, shell);
        let mut current: FxMap<Ctx, Entities> = FxMap::default();
        current.insert(Ctx::Root, std::mem::take(&mut old.root));
        for (k, d) in old.definitions.iter_mut() {
            current.insert(Ctx::Def(k), std::mem::take(&mut d.entities));
        }
        let targets = std::iter::once((Ctx::Root, root))
            .chain(defs.into_iter().map(|(k, a)| (Ctx::Def(k), a)));
        let mut now: FxMap<Ctx, Arc<EntitiesSnapshot>> = FxMap::default();
        for (ctx, target) in targets {
            let content = match current.remove(&ctx) {
                Some(mut e) => {
                    let before = e.swap_to(&target, self.copies.get(&ctx));
                    now.insert(ctx, before);
                    e
                }
                None => target.to_entities(),
            };
            // The collection now holds `target` exactly.
            if let Some(old) = self.copies.insert(ctx, target) {
                let size = old.size();
                discard(old, size);
            }
            match ctx {
                Ctx::Root => model.root = content,
                Ctx::Def(k) => model.definitions[k].entities = content,
            }
        }
        // Definitions only the replaced state has.
        for (ctx, mut e) in current {
            now.insert(ctx, self.freeze(ctx, &mut e));
        }
        self.prune(model);
        let defs = (old.definitions.keys())
            .map(|k| (k, now.remove(&Ctx::Def(k)).expect("every collection")))
            .collect();
        Step {
            shell: old,
            root: now.remove(&Ctx::Root).expect("the root's collection"),
            defs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    /// A model with a line at the root and a group holding another.
    fn model() -> Model {
        let mut m = Model::default();
        mt_kernel::add_line(&mut m.root, DVec3::ZERO, DVec3::X);
        let mut inner = Entities::default();
        mt_kernel::add_line(&mut inner, DVec3::ZERO, DVec3::Y);
        let d = m.definitions.insert(mt_kernel::Definition {
            name: "G".into(),
            entities: inner,
            is_group: true,
            ..Default::default()
        });
        m.root.instances.push(mt_kernel::Instance {
            definition: d,
            transform: glam::DMat4::IDENTITY,
            name: String::new(),
            hidden: false,
            locked: false,
            layer: None,
            material: None,
            attrs: Default::default(),
        });
        m
    }

    fn dump(m: &Model) -> String {
        format!("{:?}", m.clone())
    }

    #[test]
    fn steps_share_what_did_not_change() {
        let mut shared = Shared::default();
        let mut m = model();
        let first = shared.take(&mut m);
        // An edit of the root: the group's collection is shared, the root's is not.
        mt_kernel::add_line(&mut m.root, DVec3::ZERO, DVec3::Z);
        shared.changed(Ctx::Root);
        let second = shared.take(&mut m);
        assert!(Arc::ptr_eq(&first.defs[0].1, &second.defs[0].1));
        assert!(!Arc::ptr_eq(&first.root, &second.root));
        // A change nobody reported is found by the comparison.
        let d = m.root.instances[0].definition;
        mt_kernel::add_line(&mut m.definitions[d].entities, DVec3::X, DVec3::Z);
        let third = shared.take(&mut m);
        assert!(!Arc::ptr_eq(&second.defs[0].1, &third.defs[0].1));
        assert!(Arc::ptr_eq(&second.root, &third.root));
        assert_eq!(dump(&third.model()), dump(&m));
        // Restoring the first step and back again.
        let now = shared.restore(&mut m, first);
        assert_eq!(dump(&m), dump(&model()));
        let first = shared.restore(&mut m, now);
        assert_eq!(dump(&m), dump(&third.model()));
        assert_eq!(dump(&first.model()), dump(&model()));
    }

    /// Within a collection, a step shares the storage chunks an edit did not touch,
    /// and undo rewrites only those.
    #[test]
    fn steps_share_untouched_chunks() {
        let mut shared = Shared::default();
        let mut m = Model::default();
        for i in 0..5000 {
            let x = i as f64;
            mt_kernel::add_line(
                &mut m.root,
                DVec3::new(x, 0.0, 0.0),
                DVec3::new(x, 1.0, 0.0),
            );
        }
        let first = shared.take(&mut m);
        let before = dump(&m);
        // One vertex moved and one edge removed: two chunks of each map change at most.
        let v = m.root.vertices.keys().nth(4321).unwrap();
        m.root.vertices[v].pos.z = 3.0;
        let e = m.root.edges.keys().nth(17).unwrap();
        mt_kernel::erase_edge(&mut m.root, e);
        // Not reported with `changed`: the writes themselves are tracked.
        let second = shared.take(&mut m);
        let (same, all) = (second.root.maps().0).chunks_shared_with(first.root.maps().0);
        assert!(
            all >= 30 && same + 3 >= all,
            "{same} of {all} vertex chunks shared"
        );
        let (same, all) = (second.root.maps().1).chunks_shared_with(first.root.maps().1);
        assert!(
            all >= 15 && same + 1 >= all,
            "{same} of {all} edge chunks shared"
        );
        // The first step still holds the state before the edit.
        assert_eq!(dump(&first.model()), before);
        assert_eq!(dump(&second.model()), dump(&m));
        let now = shared.restore(&mut m, first);
        assert_eq!(dump(&m), before);
        shared.restore(&mut m, now);
        assert_eq!(dump(&m), dump(&second.model()));
    }
}
