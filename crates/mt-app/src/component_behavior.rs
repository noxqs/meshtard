//! Drawing component behaviors (Components panel, Edit tab): "Cut opening" holes in
//! the faces glued components sit on, "Always face camera" instances turned toward the
//! viewer every time the view moves, and "Shadows face sun" shadows cast as if those
//! instances faced the sun. All of it happens when the scene is built; the model keeps
//! its faces and transforms (see [`mt_kernel::behavior`]).
//!
//! Turning toward the camera rebuilds the scene whenever the view moves while the
//! model has such components (not while a tool shows a live preview).

use glam::{DMat4, DVec3};
use mt_kernel::behavior::{Toward, face_toward, opening_loops};
use mt_kernel::{Definition, Entities};
use mt_render::{Camera, FaceVertex};

use crate::App;
use crate::document::{Document, Scene};

/// The view that face-camera components turn to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Facing {
    /// The eye (perspective) or the direction back toward the viewer (parallel).
    pub camera: Toward,
    /// Unit direction toward the sun while shadows are on and it is up.
    pub sun: Option<DVec3>,
}

impl Facing {
    pub fn new(camera: &Camera, sun: Option<DVec3>) -> Self {
        Self {
            camera: if camera.perspective {
                Toward::Point(camera.eye)
            } else {
                Toward::Direction(-camera.forward())
            },
            sun,
        }
    }

    /// Close enough to `other` that the drawn components would not visibly turn:
    /// the eye moved less than `slack`, directions differ by less than ~0.03 degrees.
    fn near(&self, other: &Self, slack: f64) -> bool {
        let same_dir =
            |a: DVec3, b: DVec3| a.normalize_or_zero().dot(b.normalize_or_zero()) > 1.0 - 1e-7;
        let camera = match (self.camera, other.camera) {
            (Toward::Point(a), Toward::Point(b)) => a.distance(b) <= slack,
            (Toward::Direction(a), Toward::Direction(b)) => same_dir(a, b),
            _ => false,
        };
        let sun = match (self.sun, other.sun) {
            (Some(a), Some(b)) => same_dir(a, b),
            (a, b) => a.is_none() && b.is_none(),
        };
        camera && sun
    }
}

/// Where an instance of `def` placed at `world` is drawn, and for "Shadows face sun"
/// the rotation that turns that drawn pose toward the sun for its shadow.
pub fn placed(view: Option<&Facing>, def: &Definition, world: DMat4) -> (DMat4, Option<DMat4>) {
    let Some(view) = view.filter(|_| def.always_face_camera) else {
        return (world, None);
    };
    let drawn = face_toward(world, view.camera) * world;
    let sun = (view.sun)
        .filter(|_| def.shadows_face_sun)
        .map(|s| face_toward(drawn, Toward::Direction(s)));
    (drawn, sun)
}

impl Document {
    /// The holes "Cut opening" components glued in `ents` cut, in its coordinates.
    /// Hidden instances and instances on hidden tags cut nothing (unverified).
    pub(crate) fn cut_openings(&self, ents: &Entities) -> Vec<Vec<DVec3>> {
        let mut out = Vec::new();
        for inst in &ents.instances {
            let Some(def) = self.model.definitions.get(inst.definition) else {
                continue;
            };
            if !def.cut_opening
                || !inst.attrs.glued
                || inst.hidden
                || inst.layer.is_some_and(|l| !self.model.layer_shown(l))
            {
                continue;
            }
            for ring in opening_loops(&def.entities) {
                out.push(
                    ring.into_iter()
                        .map(|p| inst.transform.transform_point3(p))
                        .collect(),
                );
            }
        }
        out
    }

    /// Where `inst` (placed at `world`) is drawn: turned toward the camera when its
    /// component always faces it.
    pub fn drawn_transform(&self, world: DMat4, inst: &mt_kernel::Instance) -> DMat4 {
        match self.model.definitions.get(inst.definition) {
            Some(def) => placed(self.view.as_ref(), def, world).0,
            None => world,
        }
    }

    /// Whether some component always faces the camera (the scene depends on the view).
    pub fn has_face_camera(&self) -> bool {
        self.model
            .definitions
            .values()
            .any(|d| d.always_face_camera)
    }
}

impl Scene {
    /// The face vertices with "Shadows face sun" components turned toward the sun, to
    /// cast shadows with the scene's indices; `None` when there are none.
    pub fn shadow_casters(&self) -> Option<Vec<FaceVertex>> {
        if self.sun_facing.is_empty() {
            return None;
        }
        let mut out = self.vertices.clone();
        // Innermost first (they were recorded as their collection finished).
        for (range, turn) in &self.sun_facing {
            let normal = glam::DMat3::from_mat4(*turn);
            for v in &mut out[range.clone()] {
                let p = turn.transform_point3(glam::Vec3::from(v.position).as_dvec3());
                let n = normal * glam::Vec3::from(v.normal).as_dvec3();
                v.position = p.as_vec3().to_array();
                v.normal = n.normalize_or_zero().as_vec3().to_array();
            }
        }
        Some(out)
    }
}

impl App {
    /// The view face-camera components should turn to now.
    pub(crate) fn facing(&self) -> Facing {
        let sun = self
            .document
            .as_ref()
            .filter(|d| d.model.shadows.enabled)
            .and_then(|_| self.sun_dir())
            .filter(|s| s.z > 0.0)
            .map(DVec3::normalize);
        Facing::new(&self.camera, sun)
    }

    /// Rebuilds the scene when face-camera components would turn: the view (or the
    /// sun) moved since the scene was built. Waits while a tool shows a live preview.
    pub(crate) fn update_facing(&mut self, device: &egui_wgpu::wgpu::Device) {
        let want = self.facing();
        let Some(doc) = self.document.as_ref().filter(|d| d.has_face_camera()) else {
            return;
        };
        let slack = 1e-4 * self.camera.distance().max(1.0);
        if self.live.is_some() || doc.view.is_some_and(|v| v.near(&want, slack)) {
            return;
        }
        self.rebuild(device);
    }

    /// Sets the view the next scene is built for (called by `rebuild`).
    pub(crate) fn sync_facing(&mut self) {
        let want = self.facing();
        if let Some(doc) = &mut self.document {
            doc.view = Some(want);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_kernel::{DefinitionId, Instance, InstanceAttrs};

    fn rect(ents: &mut Entities, x0: f64, y0: f64, x1: f64, y1: f64, z: f64) {
        let p = [
            DVec3::new(x0, y0, z),
            DVec3::new(x1, y0, z),
            DVec3::new(x1, y1, z),
            DVec3::new(x0, y1, z),
        ];
        for k in 0..4 {
            mt_kernel::add_line(ents, p[k], p[(k + 1) % 4]);
        }
    }

    fn area(scene: &Scene) -> f64 {
        scene
            .triangles
            .iter()
            .map(|[a, b, c]| (*b - *a).cross(*c - *a).length() / 2.0)
            .sum()
    }

    fn instance(definition: DefinitionId, transform: DMat4, glued: bool) -> Instance {
        Instance {
            definition,
            transform,
            name: String::new(),
            hidden: false,
            locked: false,
            layer: None,
            material: None,
            attrs: InstanceAttrs {
                glued,
                ..Default::default()
            },
        }
    }

    /// A 100 x 50 floor face with a glued 20 x 20 "window" (an outline only) on it.
    fn floor_with_window(cut: bool, glued: bool) -> Document {
        let mut doc = Document::untitled();
        let mut def = Definition {
            cut_opening: cut,
            ..Default::default()
        };
        rect(&mut def.entities, 0.0, 0.0, 20.0, 20.0, 0.0);
        // Without its own face the window is just edges: erase the auto face.
        let faces: Vec<_> = def.entities.faces.keys().collect();
        for f in faces {
            def.entities.remove_face(f);
        }
        let id = doc.model.definitions.insert(def);
        let root = &mut doc.model.root;
        rect(root, 0.0, 0.0, 100.0, 50.0, 0.0);
        root.instances.push(instance(
            id,
            DMat4::from_translation(DVec3::new(10.0, 10.0, 0.0)),
            glued,
        ));
        doc
    }

    #[test]
    fn cut_opening_shows_a_hole_without_changing_the_face() {
        let doc = floor_with_window(true, true);
        assert!((area(&doc.scene()) - (5000.0 - 400.0)).abs() < 1e-6);
        let f = doc.model.root.faces.values().next().unwrap();
        assert!(f.holes.is_empty(), "the model keeps its face");
        // Not cut without the behavior or once unglued.
        for (cut, glued) in [(false, true), (true, false)] {
            let doc = floor_with_window(cut, glued);
            assert!((area(&doc.scene()) - 5000.0).abs() < 1e-6);
        }
    }

    /// A 10 x 20 "person" card in the red-blue plane at (50, 0, 0).
    fn card(sun_too: bool) -> Document {
        let mut doc = Document::untitled();
        let mut def = Definition {
            always_face_camera: true,
            shadows_face_sun: sun_too,
            ..Default::default()
        };
        let e = &mut def.entities;
        let p = [
            DVec3::new(-5.0, 0.0, 0.0),
            DVec3::new(5.0, 0.0, 0.0),
            DVec3::new(5.0, 0.0, 20.0),
            DVec3::new(-5.0, 0.0, 20.0),
        ];
        for k in 0..4 {
            mt_kernel::add_line(e, p[k], p[(k + 1) % 4]);
        }
        let id = doc.model.definitions.insert(def);
        doc.model.root.instances.push(instance(
            id,
            DMat4::from_translation(DVec3::new(50.0, 0.0, 0.0)),
            false,
        ));
        doc
    }

    /// The card's drawn normal (all its triangles share it, up to sign).
    fn card_plane_normal(points: &[[DVec3; 3]]) -> DVec3 {
        let [a, b, c] = points[0];
        (b - a).cross(c - a).normalize()
    }

    #[test]
    fn face_camera_turns_toward_the_eye() {
        let mut doc = card(false);
        // As placed without a view.
        let n = card_plane_normal(&doc.scene().triangles);
        assert!(n.x.abs() < 1e-9 && n.z.abs() < 1e-9);
        // Eye due east of the card: it turns its face east.
        let camera = Camera {
            eye: DVec3::new(500.0, 0.0, 10.0),
            target: DVec3::new(50.0, 0.0, 10.0),
            fov_deg: 35.0,
            perspective: true,
            two_point: false,
        };
        doc.view = Some(Facing::new(&camera, None));
        let scene = doc.scene();
        let n = card_plane_normal(&scene.triangles);
        assert!(n.y.abs() < 1e-9 && n.z.abs() < 1e-9 && n.x.abs() > 0.999);
        // It stays standing at its origin.
        for t in &scene.triangles {
            for p in t {
                assert!((p.x - 50.0).abs() < 1e-9);
            }
        }
        // The model is unchanged.
        assert_eq!(doc.model.root.instances[0].transform.w_axis.x, 50.0);
        assert!(scene.shadow_casters().is_none());
    }

    #[test]
    fn shadows_face_sun_turns_only_the_casters() {
        let mut doc = card(true);
        let camera = Camera {
            eye: DVec3::new(50.0, -500.0, 10.0),
            target: DVec3::new(50.0, 0.0, 10.0),
            fov_deg: 35.0,
            perspective: true,
            two_point: false,
        };
        // Sun due east, half way up.
        doc.view = Some(Facing::new(
            &camera,
            Some(DVec3::new(1.0, 0.0, 1.0).normalize()),
        ));
        let scene = doc.scene();
        // Drawn facing the camera (south).
        let drawn = card_plane_normal(&scene.triangles);
        assert!(drawn.y.abs() > 0.999);
        // Casting facing the sun (east).
        let casters = scene.shadow_casters().expect("sun-facing casters");
        let tri: Vec<DVec3> = scene.indices[..3]
            .iter()
            .map(|&i| glam::Vec3::from(casters[i as usize].position).as_dvec3())
            .collect();
        let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize();
        assert!(n.x.abs() > 0.999, "{n}");
        // No sun: casts as drawn.
        doc.view = Some(Facing::new(&camera, None));
        assert!(doc.scene().shadow_casters().is_none());
    }

    #[test]
    fn facing_tolerance() {
        let a = Facing {
            camera: Toward::Point(DVec3::ZERO),
            sun: None,
        };
        let b = Facing {
            camera: Toward::Point(DVec3::new(0.5, 0.0, 0.0)),
            sun: None,
        };
        assert!(a.near(&b, 1.0));
        assert!(!a.near(&b, 0.1));
        let c = Facing {
            sun: Some(DVec3::Z),
            ..a
        };
        assert!(!a.near(&c, 1.0));
    }
}
