//! Menu commands without a tool of their own: File > Revert, Edit > Intersect Faces >
//! With Context, and the View > Component Edit toggles' availability.

use eframe::egui;
use egui_wgpu::wgpu;
use mt_kernel::{Entities, FaceId};

use crate::App;
use crate::confirm::Pending;
use crate::document::Owner;

/// Edit > Intersect Faces > With Context: the selected faces against every face of the
/// open context (the selection's own context, not other groups or the model around
/// it). Returns whether edges were added.
///
/// Unverified: SketchUp also intersects selected groups' contents with the context;
/// only loose faces are intersected here.
pub fn intersect_with_context(ents: &mut Entities, items: &[Owner]) -> bool {
    let faces: Vec<FaceId> = items
        .iter()
        .filter_map(|o| match o {
            Owner::Face(f) => Some(*f),
            _ => None,
        })
        .collect();
    if faces.is_empty() {
        return false;
    }
    let with: Vec<FaceId> = ents.faces.keys().collect();
    !mt_kernel::intersect_faces(ents, &faces, &with).is_empty()
}

impl App {
    /// Runs the commands of this module; false if `id` is not one of them.
    pub(crate) fn extra_command(
        &mut self,
        ctx: &egui::Context,
        frame: &eframe::Frame,
        id: &str,
    ) -> bool {
        match id {
            "file.revert" => {
                if self.document.as_ref().is_some_and(|d| d.has_file) {
                    self.request(ctx, frame, Pending::Revert);
                }
            }
            "edit.intersect_context" => {
                let items = self.selection.items().to_vec();
                if let Some(doc) = &mut self.document
                    && doc.edit(|ents| intersect_with_context(ents, &items))
                    && let Some(rs) = frame.wgpu_render_state()
                {
                    let device: wgpu::Device = rs.device.clone();
                    self.selection.clear();
                    self.rebuild(&device);
                }
            }
            _ => return false,
        }
        true
    }

    /// File > Revert: reopens the model from its file (after the question in
    /// `confirm`).
    pub(crate) fn revert(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(path) = self
            .document
            .as_ref()
            .filter(|d| d.has_file)
            .map(|d| d.path.clone())
        else {
            return;
        };
        if let Some(rs) = frame.wgpu_render_state() {
            let device = rs.device.clone();
            self.open(ctx, &device, &path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    #[test]
    fn intersect_with_context_cuts_the_selected_face() {
        let mut ents = Entities::new();
        // A square on the ground and a vertical square crossing it.
        let sq = |ents: &mut Entities, p: [DVec3; 4]| {
            for i in 0..4 {
                mt_kernel::add_line(ents, p[i], p[(i + 1) % 4]);
            }
        };
        sq(
            &mut ents,
            [
                DVec3::ZERO,
                DVec3::new(10.0, 0.0, 0.0),
                DVec3::new(10.0, 10.0, 0.0),
                DVec3::new(0.0, 10.0, 0.0),
            ],
        );
        let ground = ents.faces.keys().next().unwrap();
        sq(
            &mut ents,
            [
                DVec3::new(5.0, -5.0, -5.0),
                DVec3::new(5.0, 15.0, -5.0),
                DVec3::new(5.0, 15.0, 5.0),
                DVec3::new(5.0, -5.0, 5.0),
            ],
        );
        assert_eq!(ents.faces.len(), 2);
        assert!(!intersect_with_context(&mut ents, &[]));
        assert!(intersect_with_context(&mut ents, &[Owner::Face(ground)]));
        assert!(ents.faces.len() >= 3, "the ground face is split");
    }

    #[test]
    fn ids_are_enabled() {
        for id in [
            "file.revert",
            "edit.intersect_context",
            "view.hide_rest_of_model",
            "view.hide_similar_components",
            "window.soften_edges",
            "tool.axes",
        ] {
            assert!(crate::IMPLEMENTED.contains(&id), "{id}");
        }
    }
}
