//! Edit > Make Component: SketchUp's dialog (name, description, "Glue to", "Set
//! Component Axes"), and the axes picking it can start. Fields SketchUp also shows
//! (Cut opening, Always face camera, Shadows face sun, Replace selection) are left out
//! here (unverified which are on by default).

use eframe::egui;
use glam::{DMat4, DVec3, DVec4};
use mt_infer::Inference;
use mt_infer::units::LengthUnit;
use mt_kernel::{Entities, GlueTo};

use crate::App;
use crate::document::Owner;
use crate::modify::{Ctx, ModifyTool};
use crate::selection::Mode;

/// The dialog's fields while it is open, and the selection it applies to.
#[derive(Debug, Clone)]
pub struct MakeComponent {
    pub name: String,
    pub description: String,
    pub glue: GlueTo,
    pub set_axes: bool,
    pub selection: mt_kernel::group::Selection,
}

const GLUE: [(GlueTo, &str); 5] = [
    (GlueTo::None, "None"),
    (GlueTo::Any, "Any"),
    (GlueTo::Horizontal, "Horizontal"),
    (GlueTo::Vertical, "Vertical"),
    (GlueTo::Sloped, "Sloped"),
];

impl App {
    /// Edit > Make Component (G): opens the dialog for the current selection.
    pub(crate) fn open_make_component(&mut self, selection: mt_kernel::group::Selection) {
        let Some(doc) = &self.document else { return };
        if selection.is_empty() {
            return;
        }
        self.make_component = Some(MakeComponent {
            name: doc.model.unique_definition_name("Component#1"),
            description: String::new(),
            glue: GlueTo::None,
            set_axes: false,
            selection,
        });
    }

    pub(crate) fn make_component_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(mut d) = self.make_component.take() else {
            return;
        };
        let mut done = None;
        egui::Modal::new(egui::Id::new("mt_make_component")).show(ctx, |ui| {
            ui.set_width(360.0);
            // The shell theme draws flat widgets; dialog fields need visible borders.
            let border = egui::Stroke::new(1.0, egui::Color32::from_gray(150));
            let w = &mut ui.visuals_mut().widgets;
            w.inactive.bg_stroke = border;
            w.inactive.bg_fill = egui::Color32::WHITE;
            w.inactive.weak_bg_fill = egui::Color32::from_gray(235);
            w.hovered.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(0, 120, 215));
            ui.visuals_mut().extreme_bg_color = egui::Color32::WHITE;
            ui.strong("Create Component");
            ui.add_space(6.0);
            egui::Grid::new("mt_make_component_grid")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Definition:");
                    ui.text_edit_singleline(&mut d.name);
                    ui.end_row();
                    ui.label("Description:");
                    ui.text_edit_multiline(&mut d.description);
                    ui.end_row();
                    ui.label("Glue to:");
                    egui::ComboBox::from_id_salt("mt_glue")
                        .selected_text(GLUE.iter().find(|g| g.0 == d.glue).map_or("None", |g| g.1))
                        .show_ui(ui, |ui| {
                            for (g, label) in GLUE {
                                ui.selectable_value(&mut d.glue, g, label);
                            }
                        });
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut d.set_axes, "Set Component Axes");
                    ui.end_row();
                });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Create").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    done = Some(true);
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    done = Some(false);
                }
            });
        });
        match done {
            None => self.make_component = Some(d),
            Some(false) => {}
            Some(true) if d.set_axes => {
                // Pick origin, red and green directions, then create.
                self.tool = crate::nav::Tool::Modify("tool.component_axes");
                self.draw = None;
                self.modify = Some(Box::new(AxesPick {
                    points: Vec::new(),
                    frame: None,
                    dialog: Some(d),
                }));
            }
            Some(true) => {
                if let Some(rs) = frame.wgpu_render_state() {
                    let device = rs.device.clone();
                    self.create_component(d, None, &device);
                }
            }
        }
    }

    /// Makes the component from the dialog's choices; `axes` maps definition space to
    /// the open context (None: bounding-box corner, world-aligned).
    pub(crate) fn create_component(
        &mut self,
        d: MakeComponent,
        axes: Option<DMat4>,
        device: &egui_wgpu::wgpu::Device,
    ) {
        let Some(doc) = &mut self.document else {
            return;
        };
        let mut made = None;
        doc.edit_model(|model, ents| {
            let name = if d.name.trim().is_empty() {
                model.unique_definition_name("Component#1")
            } else {
                model.unique_definition_name(d.name.trim())
            };
            made = mt_kernel::group::make_group_with_frame(
                &mut model.definitions,
                ents,
                &d.selection,
                name,
                false,
                axes,
            );
            if let Some(i) = made {
                let def = &mut model.definitions[ents.instances[i].definition];
                def.description = d.description.trim_end().to_owned();
                def.glue = d.glue;
            }
            made.is_some()
        });
        if let Some(i) = made {
            self.selection.clear();
            self.selection.apply(Mode::Replace, [Owner::Instance(i)]);
            self.rebuild(device);
        }
    }
}

/// "Set Component Axes": click the origin, a point along red, a point toward green.
/// Without a dialog it is the Axes tool, picking the model's drawing axes.
#[derive(Debug)]
pub struct AxesPick {
    points: Vec<DVec3>,
    frame: Option<DMat4>,
    dialog: Option<MakeComponent>,
}

impl AxesPick {
    /// The Axes tool (Tools > Axes).
    pub fn model_axes() -> Self {
        Self {
            points: Vec::new(),
            frame: None,
            dialog: None,
        }
    }

    /// The finished dialog and axes, once the third point is placed.
    pub fn take(&mut self) -> Option<(MakeComponent, DMat4)> {
        self.dialog.as_ref()?;
        let f = self.frame.take()?;
        Some((self.dialog.take()?, f))
    }
}

/// Definition-to-parent transform from an origin and points on the red and green axes.
pub fn axes_frame(o: DVec3, red: DVec3, green: DVec3) -> Option<DMat4> {
    let x = (red - o).try_normalize()?;
    let y = (green - o) - x * (green - o).dot(x);
    let y = y.try_normalize()?;
    let z = x.cross(y);
    Some(DMat4::from_cols(
        x.extend(0.0),
        y.extend(0.0),
        z.extend(0.0),
        DVec4::new(o.x, o.y, o.z, 1.0),
    ))
}

impl ModifyTool for AxesPick {
    fn status(&self) -> String {
        if self.dialog.is_none() {
            // The Axes tool (draft wording).
            return match self.points.len() {
                0 => "Select a point for the axes origin.",
                1 => "Select a point on the red axis.",
                _ => "Select a point toward the green axis.",
            }
            .into();
        }
        match self.points.len() {
            0 => "Select origin of the component axes.",
            1 => "Select a point on the red axis.",
            _ => "Select a point toward the green axis.",
        }
        .into()
    }

    fn measurement(&self, _cur: Option<&Inference>, _units: LengthUnit) -> (&'static str, String) {
        ("", String::new())
    }

    fn anchor(&self) -> Option<DVec3> {
        self.points.first().copied()
    }

    fn click(&mut self, _ctx: &mut Ctx, inf: &Inference) -> bool {
        self.points.push(inf.point);
        if self.points.len() == 3 {
            let [o, r, g] = [self.points[0], self.points[1], self.points[2]];
            self.frame = axes_frame(o, r, g);
            if self.frame.is_none() {
                // Collinear picks: start over at the green direction.
                self.points.truncate(2);
            }
        }
        false
    }

    fn typed(
        &mut self,
        _ctx: &mut Ctx,
        _cur: Option<&Inference>,
        _text: &str,
        _units: LengthUnit,
    ) -> bool {
        false
    }

    fn preview(&self, _ents: &Entities, cur: &Inference) -> Vec<Vec<DVec3>> {
        let mut pts = self.points.clone();
        pts.push(cur.point);
        if pts.len() > 1 { vec![pts] } else { Vec::new() }
    }

    fn cancel(&mut self) {
        self.points.clear();
    }

    fn take_axes(&mut self) -> Option<(MakeComponent, DMat4)> {
        self.take()
    }

    fn take_model_axes(&mut self) -> Option<DMat4> {
        if self.dialog.is_some() {
            return None;
        }
        self.frame.take()
    }
}

/// Placement transform for a gluing component on a face with unit normal `n` at `p`:
/// the component's blue axis along the normal, red along world red projected into the
/// face (world green for faces facing red). Unverified against SketchUp's choice.
pub fn glued_frame(p: DVec3, n: DVec3) -> DMat4 {
    let x = DVec3::X - n * DVec3::X.dot(n);
    let x = x
        .try_normalize()
        .unwrap_or_else(|| (DVec3::Y - n * DVec3::Y.dot(n)).normalize());
    let y = n.cross(x);
    DMat4::from_cols(x.extend(0.0), y.extend(0.0), n.extend(0.0), p.extend(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames() {
        let f = axes_frame(
            DVec3::new(1.0, 2.0, 3.0),
            DVec3::new(1.0, 5.0, 3.0),
            DVec3::new(-4.0, 2.0, 3.0),
        )
        .unwrap();
        // Red along +Y, green along -X, blue up.
        assert!((f.transform_point3(DVec3::X) - DVec3::new(1.0, 3.0, 3.0)).length() < 1e-12);
        assert!((f.transform_point3(DVec3::Y) - DVec3::new(0.0, 2.0, 3.0)).length() < 1e-12);
        assert!((f.transform_vector3(DVec3::Z) - DVec3::Z).length() < 1e-12);
        assert!(axes_frame(DVec3::ZERO, DVec3::X, DVec3::X * 2.0).is_none());

        let wall = glued_frame(DVec3::new(0.0, 5.0, 10.0), DVec3::NEG_Y);
        assert!((wall.transform_vector3(DVec3::Z) - DVec3::NEG_Y).length() < 1e-12);
        assert!((wall.transform_vector3(DVec3::X) - DVec3::X).length() < 1e-12);
        let east = glued_frame(DVec3::ZERO, DVec3::X);
        assert!((east.transform_vector3(DVec3::X) - DVec3::Y).length() < 1e-12);
        assert!(GlueTo::Vertical.accepts(DVec3::NEG_Y) && !GlueTo::Vertical.accepts(DVec3::Z));
        assert!(GlueTo::Horizontal.accepts(DVec3::Z) && !GlueTo::Sloped.accepts(DVec3::Z));
    }
}
