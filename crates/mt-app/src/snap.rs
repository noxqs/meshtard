//! What the inference engine snaps to, built from the drawn scene (FEATURES.md 49).
//!
//! Every visible edge and face is snapped to: the open context's own geometry, the
//! geometry inside its groups and components, and the faded rest of the model. Points
//! inside a group or component carry it ([`mt_infer::Container`]): the overlay draws
//! them magenta with an "in Group" / "in Component" tooltip, as SketchUp's help
//! describes. The active section plane's cut lines give "On Section".

use glam::{DMat4, DVec3};
use mt_infer::Geometry;

use crate::document::{Document, Owner, Scene};

/// The snap geometry of `scene` (world space); `ctx_xf` is the open context's
/// transform (its arcs give the "center" inference).
pub fn geometry(scene: &Scene, doc: Option<&Document>, ctx_xf: DMat4) -> Geometry {
    let v3 = |p: [f32; 3]| DVec3::new(p[0].into(), p[1].into(), p[2].into());
    // Dimension and text lines and hidden geometry are not snapped to.
    let kept: Vec<usize> = scene
        .edge_owner
        .iter()
        .enumerate()
        .filter(|(i, o)| {
            !matches!(o, Some(Owner::Dimension(_) | Owner::Text(_)))
                && !scene.hidden_edges.contains(i)
        })
        .map(|(i, _)| i)
        .collect();
    let mut geometry = Geometry::from_edges(
        kept.iter()
            .map(|&i| [v3(scene.edges[i].a), v3(scene.edges[i].b)])
            .collect(),
    );
    geometry.edge_container = kept
        .iter()
        .map(|i| scene.edge_container.get(i).copied())
        .collect();
    geometry.tri_container = (0..scene.triangles.len())
        .map(|i| scene.tri_container.get(&i).copied())
        .collect();
    let Some(doc) = doc else {
        return geometry;
    };
    if doc.model.style.guides {
        for g in &doc.model.root.guides {
            match g.dir {
                Some(d) => geometry.guide_lines.push((g.point, d)),
                None => geometry.guide_points.push(g.point),
            }
        }
    }
    // Arcs and circles of the open context, for the "center" inference. Indices are
    // into the kept edges.
    let ents = doc.active();
    let mut arcs: std::collections::BTreeMap<_, (mt_kernel::Arc, Vec<usize>)> =
        std::collections::BTreeMap::new();
    for (k, &i) in kept.iter().enumerate() {
        let Some(Some(Owner::Edge(e))) = scene.edge_owner.get(i) else {
            continue;
        };
        let Some(c) = ents.edges.get(*e).and_then(|e| e.attrs.curve) else {
            continue;
        };
        if let Some(arc) = ents.curves.get(c).and_then(|c| c.arc) {
            arcs.entry(c).or_insert((arc, Vec::new())).1.push(k);
        }
    }
    for (arc, edges) in arcs.into_values() {
        let curve = mt_infer::ArcCurve {
            center: ctx_xf.transform_point3(arc.center),
            normal: ctx_xf.transform_vector3(arc.normal).normalize_or_zero(),
            radius: ctx_xf.transform_vector3(arc.x_axis * arc.radius).length(),
        };
        geometry.add_arc(curve, edges);
    }
    geometry
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_infer::{Container, Engine, Kind, View};
    use mt_kernel::bvh::Bvh;
    use mt_render::Camera;

    /// A 100 x 100 square face drawn in the open context, and one inside a group 200
    /// to the right, and one inside a component 200 behind.
    fn model() -> Document {
        let mut doc = Document::untitled();
        let sq = |ents: &mut mt_kernel::Entities, o: DVec3| {
            let p = [
                o,
                o + DVec3::X * 100.0,
                o + DVec3::new(100.0, 100.0, 0.0),
                o + DVec3::Y * 100.0,
            ];
            for i in 0..4 {
                mt_kernel::add_line(ents, p[i], p[(i + 1) % 4]);
            }
        };
        doc.edit_model(|m, root| {
            sq(root, DVec3::ZERO);
            for (o, group) in [(DVec3::X * 200.0, true), (DVec3::Y * 200.0, false)] {
                let mut ents = mt_kernel::Entities::default();
                sq(&mut ents, DVec3::ZERO);
                let d = m.definitions.insert(mt_kernel::Definition {
                    name: if group { "Group#1" } else { "Box" }.into(),
                    is_group: group,
                    entities: ents,
                    ..Default::default()
                });
                root.instances.push(mt_kernel::Instance {
                    definition: d,
                    transform: DMat4::from_translation(o),
                    name: String::new(),
                    hidden: false,
                    locked: false,
                    layer: None,
                    material: None,
                });
            }
            true
        });
        doc
    }

    fn infer(doc: &Document, p: DVec3) -> mt_infer::Inference {
        let scene = doc.scene();
        let g = geometry(&scene, Some(doc), doc.context_transform());
        let bvh = Bvh::build(scene.triangles.clone());
        let camera = Camera {
            eye: DVec3::new(150.0, -400.0, 500.0),
            target: DVec3::new(150.0, 150.0, 0.0),
            fov_deg: 35.0,
            perspective: true,
            two_point: false,
        };
        let viewport = glam::DVec2::new(800.0, 600.0);
        let cursor = camera.project(p, viewport).unwrap();
        Engine::default().infer(
            &g,
            &bvh,
            View {
                camera: &camera,
                viewport,
            },
            cursor,
            None,
        )
    }

    #[test]
    fn context_and_nested_geometry_is_inferred() {
        let doc = model();
        let i = infer(&doc, DVec3::new(100.0, 100.0, 0.0));
        assert_eq!((i.kind, i.container), (Kind::Endpoint, None));
        let i = infer(&doc, DVec3::new(250.0, 0.0, 0.0));
        assert_eq!(
            (i.kind, i.container),
            (Kind::Midpoint, Some(Container::Group))
        );
        assert_eq!(i.label(), "Midpoint in Group");
        let i = infer(&doc, DVec3::new(60.0, 260.0, 0.0));
        assert_eq!(i.kind, Kind::OnFace);
        assert_eq!(i.label(), "On Face in Component");
    }

    #[test]
    fn inside_an_open_group_its_own_geometry_is_plain() {
        let mut doc = model();
        let group = doc
            .active()
            .instances
            .iter()
            .position(|i| doc.model.definitions[i.definition].is_group)
            .unwrap();
        assert!(doc.open_instance(group));
        // The open group's own points: plain.
        let i = infer(&doc, DVec3::new(300.0, 100.0, 0.0));
        assert_eq!((i.kind, i.container), (Kind::Endpoint, None));
        let i = infer(&doc, DVec3::new(260.0, 60.0, 0.0));
        assert_eq!((i.kind, i.container), (Kind::OnFace, None));
        // The component outside it is still inferred, in magenta.
        let i = infer(&doc, DVec3::new(100.0, 250.0, 0.0));
        assert_eq!(
            (i.kind, i.container),
            (Kind::Midpoint, Some(Container::Component))
        );
        // The rest of the model's loose geometry: plain.
        let i = infer(&doc, DVec3::new(0.0, 0.0, 0.0));
        assert_eq!((i.kind, i.container), (Kind::Endpoint, None));
    }
}
