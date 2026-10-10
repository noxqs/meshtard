//! The open model: loading a `.skp` into the kernel and turning it into renderer input.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use glam::{DMat3, DMat4, DVec3};
use mt_kernel::{DefinitionId, Entities, MaterialId, Model, import, tessellate};
use mt_render::{Camera, FaceVertex, Segment};

/// Edge color and width of the default style. Draft; check against SketchUp 2026.
const EDGE_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const EDGE_WIDTH: f32 = 1.0;

pub struct Document {
    pub path: PathBuf,
    pub model: Model,
    /// The view the file was saved with.
    saved_camera: Option<mt_skp::ViewCamera>,
}

/// Everything the viewport draws for the model.
#[derive(Default)]
pub struct Scene {
    pub edges: Vec<Segment>,
    pub vertices: Vec<FaceVertex>,
    pub indices: Vec<u32>,
}

impl Document {
    pub fn open(path: &Path) -> Result<Self, mt_skp::Error> {
        let src = mt_skp::read(path)?;
        Ok(Self {
            path: path.to_owned(),
            model: import::from_skp(&src),
            saved_camera: src.camera,
        })
    }

    pub fn title(&self) -> String {
        let name = self
            .path
            .file_name()
            .map_or_else(|| "Untitled".into(), |n| n.to_string_lossy().into_owned());
        format!("{name} - meshtard")
    }

    /// The view the model was saved with, which SketchUp reopens at.
    pub fn saved_camera(&self) -> Option<Camera> {
        let c = self.saved_camera.as_ref()?;
        let mut camera = Camera {
            eye: c.eye.into(),
            target: c.target.into(),
            fov_deg: c.fov,
            perspective: !c.parallel,
        };
        // Parallel views store their visible height; our camera derives it from the
        // field of view at the target distance.
        let distance = camera.distance();
        if c.parallel && c.ortho_height > 0.0 && distance > 0.0 {
            camera.fov_deg = (2.0 * (c.ortho_height / 2.0 / distance).atan()).to_degrees();
        }
        Some(camera)
    }

    /// Visible faces and edges in world space. Hidden entities, entities on hidden tags,
    /// and soft edges are left out.
    pub fn scene(&self) -> Scene {
        let mut scene = Scene::default();
        let mut active = HashSet::new();
        self.collect(
            &self.model.root,
            DMat4::IDENTITY,
            None,
            &mut active,
            &mut scene,
        );
        scene
    }

    fn color(&self, m: Option<MaterialId>, fallback: [u8; 3]) -> [f32; 4] {
        let (rgb, a) = match m.and_then(|i| self.model.materials.get(i)) {
            Some(mat) => (mat.color, mat.opacity as f32),
            None => (fallback, 1.0),
        };
        [
            f32::from(rgb[0]) / 255.0,
            f32::from(rgb[1]) / 255.0,
            f32::from(rgb[2]) / 255.0,
            a,
        ]
    }

    fn layer_visible(&self, layer: Option<usize>) -> bool {
        layer.is_none_or(|l| self.model.layers.get(l).is_none_or(|x| x.visible))
    }

    fn collect(
        &self,
        ents: &Entities,
        xf: DMat4,
        inherited: Option<MaterialId>,
        active: &mut HashSet<DefinitionId>,
        out: &mut Scene,
    ) {
        let world = |p: DVec3| {
            let q = xf.transform_point3(p);
            [q.x as f32, q.y as f32, q.z as f32]
        };
        for e in ents.edges.values() {
            if e.attrs.hidden || e.attrs.soft || !self.layer_visible(e.attrs.layer) {
                continue;
            }
            out.edges.push(Segment {
                a: world(ents.pos(e.start)),
                b: world(ents.pos(e.end)),
                color: EDGE_COLOR,
                style: [EDGE_WIDTH, 0.0],
            });
        }

        let normal_xf = DMat3::from_mat4(xf).inverse().transpose();
        for (f, face) in &ents.faces {
            if face.attrs.hidden || !self.layer_visible(face.attrs.layer) {
                continue;
            }
            let front = self.color(face.attrs.material.or(inherited), self.model.front_color);
            let back = self.color(
                face.attrs.back_material.or(inherited),
                self.model.back_color,
            );
            let n = (normal_xf * face.normal).normalize_or_zero();
            let normal = [n.x as f32, n.y as f32, n.z as f32];
            for [a, b, c] in tessellate::triangulate(ents, f) {
                let (a, b, c) = (
                    xf.transform_point3(a),
                    xf.transform_point3(b),
                    xf.transform_point3(c),
                );
                // A mirroring transform flips the winding; keep it counter-clockwise
                // around the front normal.
                let tri = if (b - a).cross(c - a).dot(n) < 0.0 {
                    [a, c, b]
                } else {
                    [a, b, c]
                };
                for p in tri {
                    out.indices.push(out.vertices.len() as u32);
                    out.vertices.push(FaceVertex {
                        position: [p.x as f32, p.y as f32, p.z as f32],
                        normal,
                        front,
                        back,
                    });
                }
            }
        }

        for i in &ents.instances {
            if i.hidden || !self.layer_visible(i.layer) {
                continue;
            }
            let Some(def) = self.model.definitions.get(i.definition) else {
                continue;
            };
            // A definition containing itself would recurse forever; skip it.
            if !active.insert(i.definition) {
                continue;
            }
            self.collect(
                &def.entities,
                xf * i.transform,
                i.material.or(inherited),
                active,
                out,
            );
            active.remove(&i.definition);
        }
    }

    /// Camera looking at the whole model from SketchUp's default direction, for files
    /// without a saved view.
    pub fn extents_camera(&self, fov_deg: f64) -> Option<Camera> {
        let scene = self.scene();
        let points = scene
            .edges
            .iter()
            .flat_map(|s| [s.a, s.b])
            .chain(scene.vertices.iter().map(|v| v.position));
        let mut min = DVec3::splat(f64::INFINITY);
        let mut max = DVec3::splat(f64::NEG_INFINITY);
        let mut any = false;
        for p in points {
            let p = DVec3::new(p[0].into(), p[1].into(), p[2].into());
            min = min.min(p);
            max = max.max(p);
            any = true;
        }
        if !any {
            return None;
        }
        let center = (min + max) / 2.0;
        let radius = ((max - min).length() / 2.0).max(1.0);
        let distance = radius / (fov_deg.to_radians() / 2.0).sin();
        let dir = DVec3::new(-0.5, -0.8, 0.35).normalize();
        Some(Camera {
            eye: center + dir * distance,
            target: center,
            fov_deg,
            perspective: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/openskp")
            .join(name)
    }

    #[test]
    fn opens_vff_file_with_saved_view() {
        let doc = Document::open(&corpus("Untitled.skp")).unwrap();
        assert_eq!(doc.title(), "Untitled.skp - meshtard");
        let cam = doc.saved_camera().unwrap();
        assert!((cam.eye.x - 452.09).abs() < 0.01);
        let scene = doc.scene();
        assert!(!scene.edges.is_empty());
        assert!(!scene.indices.is_empty());
        assert_eq!(scene.indices.len() % 3, 0);
    }

    #[test]
    fn instances_are_placed_in_world_space() {
        let doc = Document::open(&corpus("Untitled.skp")).unwrap();
        let root_only = doc.model.root.edges.len();
        assert!(doc.scene().edges.len() > root_only);
    }

    #[test]
    fn every_corpus_file_builds_a_scene() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/openskp");
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|x| x == "skp") {
                let scene = Document::open(&path).unwrap().scene();
                assert!(
                    scene
                        .indices
                        .iter()
                        .all(|&i| (i as usize) < scene.vertices.len())
                );
            }
        }
    }
}
