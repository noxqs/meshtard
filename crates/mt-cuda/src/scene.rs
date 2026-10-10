//! World-space triangles of a whole model, for building BVHs in tests and benchmarks.

use std::collections::HashSet;

use glam::{DMat4, DVec3};
use mt_kernel::entities::{DefinitionId, Entities};
use mt_kernel::{Model, tessellate};

/// All face triangles of the model, component instances expanded, hidden ones included.
pub fn world_triangles(model: &Model) -> Vec<[DVec3; 3]> {
    let mut out = Vec::new();
    collect(
        model,
        &model.root,
        DMat4::IDENTITY,
        &mut HashSet::new(),
        &mut out,
    );
    out
}

fn collect(
    model: &Model,
    ents: &Entities,
    xf: DMat4,
    active: &mut HashSet<DefinitionId>,
    out: &mut Vec<[DVec3; 3]>,
) {
    for f in ents.faces.keys() {
        for t in tessellate::triangulate(ents, f) {
            out.push(t.map(|p| xf.transform_point3(p)));
        }
    }
    for i in &ents.instances {
        let Some(def) = model.definitions.get(i.definition) else {
            continue;
        };
        // A definition containing itself would recurse forever.
        if !active.insert(i.definition) {
            continue;
        }
        collect(model, &def.entities, xf * i.transform, active, out);
        active.remove(&i.definition);
    }
}

/// Reads a `.skp` file and returns its world triangles.
pub fn read_triangles(path: impl AsRef<std::path::Path>) -> Result<Vec<[DVec3; 3]>, String> {
    let src = mt_skp::read(path).map_err(|e| format!("{e:?}"))?;
    Ok(world_triangles(&mt_kernel::import::from_skp(&src)))
}
