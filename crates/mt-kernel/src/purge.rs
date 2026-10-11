//! Model Info > Statistics: entity counts and Purge Unused (component definitions,
//! materials and tags no entity uses).

use std::collections::BTreeSet;

use crate::entities::{DefinitionId, Entities};
use crate::model::Model;

/// Entity counts of the whole model. Each definition's contents count once, however
/// many instances it has (SketchUp's "Entire model" may count per instance;
/// unverified).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Statistics {
    pub edges: usize,
    pub faces: usize,
    /// Group instances.
    pub groups: usize,
    /// Component instances.
    pub component_instances: usize,
    /// Component definitions (groups' definitions are not counted).
    pub component_definitions: usize,
    pub guides: usize,
    pub dimensions: usize,
    pub texts: usize,
    pub section_planes: usize,
    pub materials: usize,
    /// Tags, `Layer0` (Untagged) included.
    pub tags: usize,
}

/// What a purge removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Purged {
    pub definitions: usize,
    pub materials: usize,
    pub tags: usize,
}

impl Purged {
    pub fn any(&self) -> bool {
        self.definitions + self.materials + self.tags > 0
    }
}

fn all_entities(model: &Model) -> impl Iterator<Item = &Entities> {
    std::iter::once(&model.root).chain(model.definitions.values().map(|d| &d.entities))
}

fn all_entities_mut(model: &mut Model) -> impl Iterator<Item = &mut Entities> {
    std::iter::once(&mut model.root).chain(model.definitions.values_mut().map(|d| &mut d.entities))
}

pub fn statistics(model: &Model) -> Statistics {
    let mut s = Statistics {
        component_definitions: model.definitions.values().filter(|d| !d.is_group).count(),
        materials: model.materials.len(),
        tags: model.layers.len(),
        ..Default::default()
    };
    for ents in all_entities(model) {
        s.edges += ents.edges.len();
        s.faces += ents.faces.len();
        for inst in &ents.instances {
            match model.definitions.get(inst.definition) {
                Some(d) if d.is_group => s.groups += 1,
                _ => s.component_instances += 1,
            }
        }
        s.guides += ents.guides.len();
        s.dimensions += ents.dimensions.len();
        s.texts += ents.texts.len();
        s.section_planes += ents.section_planes.len();
    }
    s
}

/// Definitions with no instance in the model root or inside a used definition.
pub fn unused_definitions(model: &Model) -> Vec<DefinitionId> {
    let mut used = BTreeSet::new();
    let mut stack: Vec<&Entities> = vec![&model.root];
    while let Some(ents) = stack.pop() {
        for inst in &ents.instances {
            if used.insert(inst.definition)
                && let Some(d) = model.definitions.get(inst.definition)
            {
                stack.push(&d.entities);
            }
        }
    }
    model
        .definitions
        .keys()
        .filter(|k| !used.contains(k))
        .collect()
}

pub fn purge_definitions(model: &mut Model) -> usize {
    let unused = unused_definitions(model);
    for &d in &unused {
        model.definitions.remove(d);
    }
    unused.len()
}

/// Old index to new index after dropping the unused ones of `len` items.
fn remap(len: usize, used: &BTreeSet<usize>, keep_first: bool) -> Vec<Option<usize>> {
    let mut next = 0;
    (0..len)
        .map(|i| {
            (used.contains(&i) || (keep_first && i == 0)).then(|| {
                next += 1;
                next - 1
            })
        })
        .collect()
}

pub fn purge_materials(model: &mut Model) -> usize {
    let mut used = BTreeSet::new();
    for ents in all_entities(model) {
        for f in ents.faces.values() {
            used.extend(f.attrs.material);
            used.extend(f.attrs.back_material);
        }
        used.extend(ents.instances.iter().filter_map(|i| i.material));
    }
    let map = remap(model.materials.len(), &used, false);
    let before = model.materials.len();
    let mut i = 0;
    model.materials.retain(|_| {
        i += 1;
        map[i - 1].is_some()
    });
    let fix = |m: &mut Option<usize>| *m = m.and_then(|m| map.get(m).copied().flatten());
    for ents in all_entities_mut(model) {
        for f in ents.faces.values_mut() {
            fix(&mut f.attrs.material);
            fix(&mut f.attrs.back_material);
        }
        for inst in &mut ents.instances {
            fix(&mut inst.material);
        }
    }
    before - model.materials.len()
}

/// Tags no entity is on; `Layer0` (Untagged) always stays.
pub fn purge_tags(model: &mut Model) -> usize {
    let mut used = BTreeSet::new();
    for ents in all_entities(model) {
        used.extend(ents.edges.values().filter_map(|e| e.attrs.layer));
        used.extend(ents.faces.values().filter_map(|f| f.attrs.layer));
        used.extend(ents.instances.iter().filter_map(|i| i.layer));
    }
    let map = remap(model.layers.len(), &used, true);
    let before = model.layers.len();
    let mut i = 0;
    model.layers.retain(|_| {
        i += 1;
        map[i - 1].is_some()
    });
    let fix = |l: &mut Option<usize>| *l = l.and_then(|l| map.get(l).copied().flatten());
    for ents in all_entities_mut(model) {
        for e in ents.edges.values_mut() {
            fix(&mut e.attrs.layer);
        }
        for f in ents.faces.values_mut() {
            fix(&mut f.attrs.layer);
        }
        for inst in &mut ents.instances {
            fix(&mut inst.layer);
        }
    }
    for page in &mut model.pages {
        page.hidden_layers = page
            .hidden_layers
            .iter()
            .filter_map(|&l| map.get(l).copied().flatten())
            .collect();
    }
    before - model.layers.len()
}

/// Purge Unused: definitions first, so materials and tags only they used go too.
pub fn purge_all(model: &mut Model) -> Purged {
    Purged {
        definitions: purge_definitions(model),
        materials: purge_materials(model),
        tags: purge_tags(model),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::Instance;
    use crate::model::{Definition, Layer, Material, Page};
    use glam::{DMat4, DVec3};

    fn material(name: &str) -> Material {
        Material {
            name: name.into(),
            color: [1, 2, 3],
            opacity: 1.0,
            texture: None,
        }
    }

    fn layer(name: &str) -> Layer {
        Layer {
            name: name.into(),
            color: [0; 3],
            visible: true,
            ..Layer::default()
        }
    }

    fn instance(def: DefinitionId) -> Instance {
        Instance {
            definition: def,
            transform: DMat4::IDENTITY,
            name: String::new(),
            hidden: false,
            locked: false,
            layer: None,
            material: None,
            attrs: Default::default(),
        }
    }

    /// Root: a line on tag 2 and an instance of `used` (painted material 2), which
    /// holds an instance of `nested`. `unused` holds a line on tag 1 and an instance
    /// of `only_in_unused` painted with material 1. Material 0 is used by nothing.
    fn model() -> (Model, [DefinitionId; 4]) {
        let mut m = Model {
            materials: vec![material("a"), material("b"), material("c")],
            ..Default::default()
        };
        m.layers.extend([layer("t1"), layer("t2")]);
        let nested = m.definitions.insert(Definition {
            name: "nested".into(),
            ..Default::default()
        });
        let only_in_unused = m.definitions.insert(Definition {
            name: "only".into(),
            ..Default::default()
        });
        let mut used_ents = Entities::new();
        used_ents.instances.push(instance(nested));
        let used = m.definitions.insert(Definition {
            name: "used".into(),
            entities: used_ents,
            ..Default::default()
        });
        let mut unused_ents = Entities::new();
        let e = crate::add_line(&mut unused_ents, DVec3::ZERO, DVec3::X);
        unused_ents.edges[e[0]].attrs.layer = Some(1);
        unused_ents.instances.push(Instance {
            material: Some(1),
            ..instance(only_in_unused)
        });
        let unused = m.definitions.insert(Definition {
            name: "unused".into(),
            entities: unused_ents,
            ..Default::default()
        });
        let e = crate::add_line(&mut m.root, DVec3::ZERO, DVec3::Y);
        m.root.edges[e[0]].attrs.layer = Some(2);
        m.root.instances.push(Instance {
            material: Some(2),
            ..instance(used)
        });
        m.pages.push(Page {
            name: "s".into(),
            eye: DVec3::ONE,
            target: DVec3::ZERO,
            fov_deg: 35.0,
            perspective: true,
            hidden_layers: vec![1, 2],
            ..Default::default()
        });
        (m, [nested, only_in_unused, used, unused])
    }

    #[test]
    fn unused_definitions_include_ones_only_unused_ones_use() {
        let (m, [_, only, _, unused]) = model();
        let mut found = unused_definitions(&m);
        found.sort();
        let mut want = vec![only, unused];
        want.sort();
        assert_eq!(found, want);
    }

    #[test]
    fn purge_all_removes_and_remaps() {
        let (mut m, [nested, _, used, _]) = model();
        let before = statistics(&m);
        assert_eq!(before.component_definitions, 4);
        assert_eq!(before.edges, 2);
        assert_eq!(before.tags, 3);
        let purged = purge_all(&mut m);
        assert_eq!(
            purged,
            Purged {
                definitions: 2,
                materials: 2,
                tags: 1
            }
        );
        assert!(m.definitions.contains_key(nested) && m.definitions.contains_key(used));
        // Material "c" is now index 0, tag "t2" index 1.
        assert_eq!(m.materials.len(), 1);
        assert_eq!(m.materials[0].name, "c");
        assert_eq!(m.root.instances[0].material, Some(0));
        let names: Vec<_> = m.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Layer0", "t2"]);
        let edge = m.root.edges.values().next().unwrap();
        assert_eq!(edge.attrs.layer, Some(1));
        assert_eq!(m.pages[0].hidden_layers, [1]);
        // Nothing left to purge.
        assert!(!purge_all(&mut m).any());
    }
}
