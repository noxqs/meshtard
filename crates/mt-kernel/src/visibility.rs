//! Edit > Hide / Unhide and Edit > Lock / Unlock: the `hidden` flag of entities and the
//! `locked` flag of group and component instances.

use crate::entities::{EdgeId, Entities, FaceId};
use crate::model::Model;

/// A top-level entity of one `Entities`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Item {
    Edge(EdgeId),
    Face(FaceId),
    /// Index into `instances`.
    Instance(usize),
    /// Index into `section_planes`.
    Section(usize),
    /// Index into `dimensions`.
    Dimension(usize),
    /// Index into `texts`.
    Text(usize),
}

/// The `hidden` flag of `item`, if it exists.
fn hidden_flag(ents: &mut Entities, item: Item) -> Option<&mut bool> {
    Some(match item {
        Item::Edge(e) => &mut ents.edges.get_mut(e)?.attrs.hidden,
        Item::Face(f) => &mut ents.faces.get_mut(f)?.attrs.hidden,
        Item::Instance(i) => &mut ents.instances.get_mut(i)?.hidden,
        Item::Section(i) => &mut ents.section_planes.get_mut(i)?.hidden,
        Item::Dimension(i) => &mut ents.dimensions.get_mut(i)?.hidden,
        Item::Text(i) => &mut ents.texts.get_mut(i)?.hidden,
    })
}

/// Sets the hidden flag of `items`; returns the items whose flag changed.
pub fn set_hidden(ents: &mut Entities, items: &[Item], hidden: bool) -> Vec<Item> {
    let mut changed = Vec::new();
    for &item in items {
        if let Some(flag) = hidden_flag(ents, item)
            && *flag != hidden
        {
            *flag = hidden;
            changed.push(item);
        }
    }
    changed
}

/// Every hidden top-level entity of `ents` (nested definitions are not looked into).
pub fn hidden_items(ents: &Entities) -> Vec<Item> {
    let mut out: Vec<Item> = ents
        .edges
        .iter()
        .filter(|(_, e)| e.attrs.hidden)
        .map(|(id, _)| Item::Edge(id))
        .collect();
    out.extend(
        ents.faces
            .iter()
            .filter(|(_, f)| f.attrs.hidden)
            .map(|(id, _)| Item::Face(id)),
    );
    let idx = |flags: Vec<bool>, f: fn(usize) -> Item| {
        flags
            .into_iter()
            .enumerate()
            .filter(|(_, h)| *h)
            .map(move |(i, _)| f(i))
    };
    out.extend(idx(
        ents.instances.iter().map(|i| i.hidden).collect(),
        Item::Instance,
    ));
    out.extend(idx(
        ents.section_planes.iter().map(|s| s.hidden).collect(),
        Item::Section,
    ));
    out.extend(idx(
        ents.dimensions.iter().map(|d| d.hidden).collect(),
        Item::Dimension,
    ));
    out.extend(idx(
        ents.texts.iter().map(|t| t.hidden).collect(),
        Item::Text,
    ));
    out
}

/// Edit > Unhide > All: shows every hidden top-level entity of `ents`. Returns whether
/// anything changed.
pub fn unhide_all(ents: &mut Entities) -> bool {
    let hidden = hidden_items(ents);
    !set_hidden(ents, &hidden, false).is_empty()
}

/// Sets the lock flag of the instances among `items`; returns whether any changed.
pub fn set_locked(ents: &mut Entities, items: &[Item], locked: bool) -> bool {
    let mut changed = false;
    for item in items {
        if let Item::Instance(i) = *item
            && let Some(inst) = ents.instances.get_mut(i)
            && inst.locked != locked
        {
            inst.locked = locked;
            changed = true;
        }
    }
    changed
}

/// Whether `item` is a locked instance.
pub fn is_locked(ents: &Entities, item: Item) -> bool {
    matches!(item, Item::Instance(i) if ents.instances.get(i).is_some_and(|x| x.locked))
}

/// Edit > Unlock > All: unlocks every instance in the model, nested ones included.
pub fn unlock_all(model: &mut Model) -> bool {
    let mut changed = false;
    let lists = std::iter::once(&mut model.root)
        .chain(model.definitions.values_mut().map(|d| &mut d.entities));
    for ents in lists {
        for inst in &mut ents.instances {
            changed |= std::mem::take(&mut inst.locked);
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Instance, add_line};
    use glam::{DMat4, DVec3};

    fn square(ents: &mut Entities) {
        let p = [
            DVec3::ZERO,
            DVec3::X * 10.0,
            DVec3::new(10.0, 10.0, 0.0),
            DVec3::Y * 10.0,
        ];
        for i in 0..4 {
            add_line(ents, p[i], p[(i + 1) % 4]);
        }
    }

    fn instance(model: &mut Model) -> Instance {
        let definition = model.definitions.insert(crate::Definition {
            name: "C".into(),
            ..Default::default()
        });
        Instance {
            definition,
            transform: DMat4::IDENTITY,
            name: String::new(),
            hidden: false,
            locked: false,
            layer: None,
            material: None,
        }
    }

    #[test]
    fn hide_and_unhide() {
        let mut model = Model::default();
        let inst = instance(&mut model);
        let ents = &mut model.root;
        square(ents);
        ents.instances.push(inst);
        let face = ents.faces.keys().next().unwrap();
        let edge = ents.edges.keys().next().unwrap();
        let items = [Item::Face(face), Item::Edge(edge), Item::Instance(0)];
        assert_eq!(set_hidden(ents, &items, true), items.to_vec());
        assert!(ents.faces[face].attrs.hidden && ents.edges[edge].attrs.hidden);
        assert!(ents.instances[0].hidden);
        // Already hidden: nothing changes; missing items are skipped.
        assert!(set_hidden(ents, &[Item::Face(face), Item::Instance(7)], true).is_empty());
        assert_eq!(hidden_items(ents).len(), 3);
        assert_eq!(set_hidden(ents, &[Item::Edge(edge)], false).len(), 1);
        assert!(unhide_all(ents));
        assert!(hidden_items(ents).is_empty());
        assert!(!unhide_all(ents), "nothing left to unhide");
    }

    #[test]
    fn lock_and_unlock() {
        let mut model = Model::default();
        let inst = instance(&mut model);
        model.root.instances.push(inst.clone());
        let mut nested = inst;
        nested.locked = true;
        // A locked instance inside another definition.
        let other = model.definitions.insert(crate::Definition::default());
        model.definitions[other].entities.instances.push(nested);
        assert!(set_locked(&mut model.root, &[Item::Instance(0)], true));
        assert!(!set_locked(&mut model.root, &[Item::Instance(0)], true));
        assert!(is_locked(&model.root, Item::Instance(0)));
        assert!(!is_locked(&model.root, Item::Instance(3)));
        assert!(set_locked(&mut model.root, &[Item::Instance(0)], false));
        model.root.instances[0].locked = true;
        assert!(unlock_all(&mut model));
        assert!(!model.root.instances[0].locked);
        assert!(!model.definitions[other].entities.instances[0].locked);
        assert!(!unlock_all(&mut model));
    }
}
