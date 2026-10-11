//! Tools > Axes: moves and turns the model's drawing axes (`Model::axes`). The axes
//! are drawn there, and the inference engine's axis directions, axis locks, "on axis",
//! origin and ground plane follow them (`mt_infer::Axes`).
//!
//! The tool is [`crate::component_dialog::AxesPick`] without a dialog: click the
//! origin, a point on the red axis and a point toward the green axis. Setting the axes
//! is an undo step (unverified that SketchUp records it). Not saved in .skp files:
//! mt-skp does not read or write the model's axes yet. Inside an open group the tool
//! still sets the model's axes (SketchUp may set the group's own axes; unverified).

use egui_wgpu::wgpu;
use glam::{DMat4, DVec3};

use crate::App;
use crate::nav::Tool;

/// The inference engine's axes for the model's axes transform.
pub fn axes_of(m: DMat4) -> mt_infer::Axes {
    let dir = |v: DVec3| m.transform_vector3(v).normalize_or_zero();
    mt_infer::Axes {
        origin: m.transform_point3(DVec3::ZERO),
        dirs: [dir(DVec3::X), dir(DVec3::Y), dir(DVec3::Z)],
    }
}

/// Passes the model's axes to the drawn axes and the inference engine.
pub fn apply(
    model: &mt_kernel::Model,
    style: &mut mt_render::Style,
    engine: &mut mt_infer::Engine,
) {
    let a = axes_of(model.axes);
    engine.axes = a;
    if let Some(s) = style.axes.as_mut() {
        s.origin = a.origin.as_vec3().to_array();
        s.dirs = a.dirs.map(|v| v.as_vec3().to_array());
    }
}

/// World-space axes from a frame picked in the open context (`ctx_xf`: context to
/// world), re-orthonormalized in case the context is scaled.
pub fn world_frame(ctx_xf: DMat4, local: DMat4) -> Option<DMat4> {
    let w = ctx_xf * local;
    crate::component_dialog::axes_frame(
        w.transform_point3(DVec3::ZERO),
        w.transform_point3(DVec3::X),
        w.transform_point3(DVec3::Y),
    )
}

impl App {
    /// The Axes tool finished with `local` (open-context space): sets the model's
    /// axes and goes back to Select.
    pub(crate) fn set_model_axes(&mut self, local: DMat4, device: &wgpu::Device) {
        // Context menu > Change Axes uses the same tool for a component's axes.
        if self.finish_change_axes(local, device) {
            return;
        }
        self.modify = None;
        self.tool = Tool::Select;
        self.engine.unlock();
        let Some(frame) = world_frame(self.ctx_xf, local) else {
            return;
        };
        if let Some(doc) = &mut self.document
            && doc.edit_model(|m, _| {
                let changed = m.axes != frame;
                m.axes = frame;
                changed
            })
        {
            self.rebuild(device);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modify::{Ctx, ModifyTool};

    #[test]
    fn axes_tool_picks_three_points() {
        let mut t = crate::component_dialog::AxesPick::model_axes();
        let mut ents = mt_kernel::Entities::new();
        let mut ctx = Ctx {
            ents: &mut ents,
            picked: None,
            selection: &[],
            ctrl: false,
            eye: DVec3::ZERO,
        };
        let at = |p: DVec3| mt_infer::Inference {
            point: p,
            kind: mt_infer::Kind::Free,
            edge: None,
            guide: None,
            locked: false,
            normal: None,
            segment: None,
            container: None,
        };
        let o = DVec3::new(10.0, 0.0, 0.0);
        for p in [o, o + DVec3::Y * 5.0, o - DVec3::X * 3.0] {
            assert!(t.take_model_axes().is_none());
            assert!(!t.click(&mut ctx, &at(p)), "the model changes later");
        }
        assert!(t.take_axes().is_none(), "no component dialog");
        let local = t.take_model_axes().expect("three points");
        // Picked inside a group moved up by 5.
        let w = world_frame(DMat4::from_translation(DVec3::Z * 5.0), local).unwrap();
        let a = axes_of(w);
        assert!((a.origin - DVec3::new(10.0, 0.0, 5.0)).length() < 1e-12);
        assert!((a.dirs[0] - DVec3::Y).length() < 1e-12);
        assert!((a.dirs[1] + DVec3::X).length() < 1e-12);
        assert!((a.dirs[2] - DVec3::Z).length() < 1e-12);

        let mut model = mt_kernel::Model::default();
        assert_eq!(axes_of(model.axes), mt_infer::Axes::default());
        model.axes = w;
        let mut engine = mt_infer::Engine::default();
        let mut style =
            mt_render::Style::from_spec(&mt_spec::Spec::builtin().unwrap().default_model);
        apply(&model, &mut style, &mut engine);
        assert_eq!(engine.axes, a);
        assert_eq!(style.axes.unwrap().origin, [10.0, 0.0, 5.0]);
    }
}
