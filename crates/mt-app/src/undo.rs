//! Undo steps that share what did not change (spec/PERFORMANCE.md).
//!
//! A step holds the whole model, but each entities collection (the root's and each
//! definition's) is an [`Arc`] that consecutive steps share while the collection stays
//! the same. [`Shared`] keeps the copy of each collection made for the last step it
//! was in; the next step reuses it after checking it still equals the model's
//! collection, so only the collections an edit changed are copied again. The check
//! compares the contents, so a change made without an undo step (Scenes, Texture
//! Position's preview, tests) is never lost: it simply fails the check. The settings
//! (everything but the collections) are small and copied whole into every step.

use std::sync::Arc;

use mt_kernel::spatial::FxMap;
use mt_kernel::{DefinitionId, Entities, Model};

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
    root: Arc<Entities>,
    /// The definitions' collections, in `shell.definitions` order.
    defs: Vec<(DefinitionId, Arc<Entities>)>,
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
        m.root = (*self.root).clone();
        for (k, e) in &self.defs {
            m.definitions[*k].entities = (**e).clone();
        }
        m
    }
}

/// The copies the next step can share; see the module documentation.
#[derive(Default)]
pub struct Shared {
    copies: FxMap<Ctx, Arc<Entities>>,
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

/// Above this much work in total ([`Entities::size`]), comparisons run on several
/// threads.
const PARALLEL_MIN: usize = 20_000;

/// Whether each pair of collections is equal, compared on several threads when there
/// is much to compare.
fn equal_all(pairs: &[(&Entities, &Entities)]) -> Vec<bool> {
    let total: usize = pairs.iter().map(|(a, _)| a.size()).sum();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    if total < PARALLEL_MIN || pairs.len() < 2 || threads < 2 {
        return pairs.iter().map(|(a, b)| a == b).collect();
    }
    // Consecutive runs of about equal work, one per thread.
    let share = total.div_ceil(threads);
    let mut chunks: Vec<&[(&Entities, &Entities)]> = Vec::new();
    let (mut start, mut work) = (0, 0);
    for (i, (a, _)) in pairs.iter().enumerate() {
        work += a.size();
        if work >= share {
            chunks.push(&pairs[start..=i]);
            (start, work) = (i + 1, 0);
        }
    }
    if start < pairs.len() {
        chunks.push(&pairs[start..]);
    }
    std::thread::scope(|s| {
        let handles: Vec<_> = chunks
            .iter()
            .map(|c| s.spawn(move || c.iter().map(|(a, b)| a == b).collect::<Vec<_>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("comparison thread"))
            .collect()
    })
}

impl Shared {
    /// Records that `ctx` was (or may have been) changed: its copy is not offered to
    /// the next step without comparing. Dropping it skips a comparison bound to fail.
    pub fn changed(&mut self, ctx: Ctx) {
        if let Some(a) = self.copies.remove(&ctx) {
            let size = a.size();
            discard(a, size);
        }
    }

    /// Forgets the copies of definitions `model` no longer has.
    fn prune(&mut self, model: &Model) {
        let gone: Vec<Ctx> = (self.copies.keys())
            .filter(|c| matches!(c, Ctx::Def(d) if !model.definitions.contains_key(*d)))
            .copied()
            .collect();
        for c in gone {
            self.changed(c);
        }
    }

    /// The current state of `model` as a step.
    pub fn take(&mut self, model: &mut Model) -> Step {
        let shell = shell_of(model);
        let colls: Vec<(Ctx, &Entities)> = std::iter::once((Ctx::Root, &model.root))
            .chain((model.definitions.iter()).map(|(k, d)| (Ctx::Def(k), &d.entities)))
            .collect();
        // Which copies still equal the collections.
        let mut same = vec![false; colls.len()];
        {
            let (index, pairs): (Vec<usize>, Vec<(&Entities, &Entities)>) = (colls.iter())
                .enumerate()
                .filter_map(|(i, (c, e))| self.copies.get(c).map(|a| (i, (*e, &**a))))
                .unzip();
            for (i, s) in index.into_iter().zip(equal_all(&pairs)) {
                same[i] = s;
            }
        }
        let mut copies: Vec<Arc<Entities>> = Vec::with_capacity(colls.len());
        for ((ctx, ents), same) in colls.iter().zip(same) {
            if same {
                copies.push(self.copies[ctx].clone());
            } else {
                let a = Arc::new((*ents).clone());
                if let Some(old) = self.copies.insert(*ctx, a.clone()) {
                    let size = old.size();
                    discard(old, size);
                }
                copies.push(a);
            }
        }
        let mut copies = copies.into_iter();
        let root = copies.next().expect("the root's copy");
        let defs = model.definitions.keys().zip(copies).collect();
        self.prune(model);
        Step { shell, root, defs }
    }

    /// Makes `step` the model and returns the state it replaced as a step. Nothing
    /// is copied for collections the two states share (they stay in place, with their
    /// spatial index) or that only one of them holds (they are moved).
    pub fn restore(&mut self, model: &mut Model, step: Step) -> Step {
        let Step { shell, root, defs } = step;
        let mut old = std::mem::replace(model, shell);
        let mut current: FxMap<Ctx, Entities> = FxMap::default();
        current.insert(Ctx::Root, std::mem::take(&mut old.root));
        for (k, d) in old.definitions.iter_mut() {
            current.insert(Ctx::Def(k), std::mem::take(&mut d.entities));
        }
        let targets: Vec<(Ctx, Arc<Entities>)> = std::iter::once((Ctx::Root, root))
            .chain(defs.into_iter().map(|(k, a)| (Ctx::Def(k), a)))
            .collect();
        // Collections the step shares with what is there now stay in place (the
        // comparison catches changes made since the copy).
        let mut keep = vec![false; targets.len()];
        {
            let (index, pairs): (Vec<usize>, Vec<(&Entities, &Entities)>) = (targets.iter())
                .enumerate()
                .filter(|(_, (c, t))| self.copies.get(c).is_some_and(|a| Arc::ptr_eq(a, t)))
                .filter_map(|(i, (c, t))| current.get(c).map(|e| (i, (e, &**t))))
                .unzip();
            for (i, s) in index.into_iter().zip(equal_all(&pairs)) {
                keep[i] = s;
            }
        }
        let mut now: FxMap<Ctx, Arc<Entities>> = FxMap::default();
        for ((ctx, target), keep) in targets.into_iter().zip(keep) {
            let content = match current.remove(&ctx) {
                Some(e) if keep => {
                    now.insert(ctx, target);
                    e
                }
                Some(e) => {
                    now.insert(ctx, Arc::new(e));
                    self.unwrap(ctx, target)
                }
                None => self.unwrap(ctx, target),
            };
            match ctx {
                Ctx::Root => model.root = content,
                Ctx::Def(k) => model.definitions[k].entities = content,
            }
        }
        // Definitions only the replaced state has.
        for (ctx, e) in current {
            now.insert(ctx, Arc::new(e));
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

    /// The content of `target`, which becomes collection `ctx`: moved out when no other
    /// step holds it, else copied (and `target` is then the shared copy of `ctx`).
    fn unwrap(&mut self, ctx: Ctx, target: Arc<Entities>) -> Entities {
        self.changed(ctx);
        match Arc::try_unwrap(target) {
            Ok(e) => e,
            Err(target) => {
                let e = (*target).clone();
                self.copies.insert(ctx, target);
                e
            }
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
}
