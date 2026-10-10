//! The open model: loading a `.skp` and turning it into renderer input.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use glam::DVec3;
use mt_render::{Camera, Segment};
use mt_skp::{Definition, Model, transform};

/// Edge color and width of the default style. Draft; check against SketchUp 2026.
const EDGE_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const EDGE_WIDTH: f32 = 1.0;

pub struct Document {
    pub path: PathBuf,
    pub model: Model,
}

impl Document {
    pub fn open(path: &Path) -> Result<Self, mt_skp::Error> {
        Ok(Self {
            path: path.to_owned(),
            model: mt_skp::read(path)?,
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
        let c = self.model.camera.as_ref()?;
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

    /// Visible edges of the whole model in world space: hidden and soft edges, hidden
    /// instances and instances on hidden tags are left out.
    pub fn edge_segments(&self) -> Vec<Segment> {
        let hidden_layers: HashSet<&str> = self
            .model
            .layers
            .iter()
            .filter(|l| l.hidden)
            .map(|l| l.name.as_str())
            .collect();
        let mut out = Vec::new();
        let mut active = HashSet::new();
        self.collect(&self.model.root, &[], &hidden_layers, &mut active, &mut out);
        out
    }

    fn collect(
        &self,
        def: &Definition,
        matrix: &[f64],
        hidden_layers: &HashSet<&str>,
        active: &mut HashSet<i64>,
        out: &mut Vec<Segment>,
    ) {
        let world = |id: i64| {
            def.vertices.get(&id).map(|v| {
                let p = transform::transform_point(matrix, [v.x, v.y, v.z]);
                [p[0] as f32, p[1] as f32, p[2] as f32]
            })
        };
        for e in def.edges.values() {
            if e.hidden || e.soft {
                continue;
            }
            if let (Some(a), Some(b)) = (world(e.v1_id), world(e.v2_id)) {
                out.push(Segment {
                    a,
                    b,
                    color: EDGE_COLOR,
                    style: [EDGE_WIDTH, 0.0],
                });
            }
        }
        for i in &def.instances {
            if i.hidden || hidden_layers.contains(i.layer.as_str()) {
                continue;
            }
            let Some(id) = i.ref_idx else { continue };
            let Some(child) = self.model.definitions.get(&id) else {
                continue;
            };
            // A definition containing itself would recurse forever; skip it.
            if !active.insert(id) {
                continue;
            }
            let m = transform::multiply_matrices(matrix, &i.matrix);
            self.collect(child, &m, hidden_layers, active, out);
            active.remove(&id);
        }
    }

    /// Camera looking at all edges from SketchUp's default direction, for files without
    /// a saved view.
    pub fn extents_camera(&self, fov_deg: f64) -> Option<Camera> {
        let segments = self.edge_segments();
        let mut min = DVec3::splat(f64::INFINITY);
        let mut max = DVec3::splat(f64::NEG_INFINITY);
        for s in &segments {
            for p in [s.a, s.b] {
                let p = DVec3::new(p[0].into(), p[1].into(), p[2].into());
                min = min.min(p);
                max = max.max(p);
            }
        }
        if segments.is_empty() {
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
        assert!(!doc.edge_segments().is_empty());
    }

    #[test]
    fn instances_are_placed_in_world_space() {
        let doc = Document::open(&corpus("Untitled.skp")).unwrap();
        let root_only = doc.model.root.edges.len();
        assert!(doc.edge_segments().len() > root_only);
    }
}
