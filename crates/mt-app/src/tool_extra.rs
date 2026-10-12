//! Tool modifiers of FEATURES.md 57 and 62 that need the app: Ctrl for the drawing
//! tools (Tape Measure's modes), Ctrl+plus / minus segments, double clicks of the
//! drawing tools (Tape Measure guide point, 2 Point Arc repeat, Text area label), the
//! component name under the Text tool, Tape Measure rescale (confirmation dialog),
//! the section plane name prompt, Orbit's double click and Undo cancelling a tool
//! that is mid-operation (release notes 2024.0 [doc]).
//!
//! The dialogs' wording and layout, and where a double click with Orbit centers the
//! view (the point under the cursor becomes the target, the view direction stays),
//! are from memory, unverified.

use eframe::egui;
use egui_wgpu::wgpu;
use glam::DVec2;

use crate::App;
use crate::document::Owner;
use crate::nav::Tool;

impl App {
    /// Before a drawing tool's click handling: Ctrl tapped (pressed and released with
    /// no other key) goes to the tool, Ctrl+plus / minus change its segments, and the
    /// tool learns what is under the cursor.
    pub(crate) fn draw_tool_pre(
        &mut self,
        ui: &egui::Ui,
        cursor: Option<DVec2>,
        viewport_px: DVec2,
    ) {
        let (ctrl, other_key, plus, minus) = ui.input(|i| {
            let any_key = i
                .events
                .iter()
                .any(|e| matches!(e, egui::Event::Key { pressed: true, key, .. } if !matches!(key, egui::Key::Plus | egui::Key::Minus | egui::Key::Equals)));
            let k = |key| i.modifiers.command && i.key_pressed(key);
            (
                i.modifiers.command,
                any_key,
                k(egui::Key::Plus) || k(egui::Key::Equals),
                k(egui::Key::Minus),
            )
        });
        let Some(draw) = self.draw.as_mut() else {
            return;
        };
        if plus || minus {
            draw.step_segments(plus);
        }
        match (ctrl, self.extra.ctrl_tap) {
            // The frame Ctrl goes down can carry a key event for Ctrl itself.
            (true, None) => self.extra.ctrl_tap = Some(true),
            (true, Some(true)) if other_key || plus || minus => self.extra.ctrl_tap = Some(false),
            (false, Some(tap)) => {
                if tap {
                    draw.toggle_ctrl();
                }
                self.extra.ctrl_tap = None;
            }
            _ => {}
        }
        let instance_name = cursor.and_then(|c| {
            let hit = crate::selection::pick(
                &self.scene,
                self.bvh.as_ref(),
                &self.camera,
                c,
                viewport_px,
            );
            let Some(Owner::Instance(i)) = hit else {
                return None;
            };
            let doc = self.document.as_ref()?;
            let inst = doc.active().instances.get(i)?;
            let def = doc.model.definitions.get(inst.definition)?;
            Some(if inst.name.is_empty() {
                def.name.clone()
            } else {
                inst.name.clone()
            })
        });
        let units = self
            .document
            .as_ref()
            .map_or(mt_infer::units::LengthUnit::Inch, |d| d.units);
        draw.hover(&crate::tools::DrawHover {
            instance_name,
            units,
            alt: ui.input(|i| i.modifiers.alt),
        });
    }

    /// After a drawing tool's click: its double click, and a rescale it asks for.
    /// Returns whether the model changed.
    pub(crate) fn draw_tool_post(
        &mut self,
        response: &egui::Response,
        inf: &mt_infer::Inference,
    ) -> bool {
        let (Some(draw), Some(doc)) = (self.draw.as_mut(), self.document.as_mut()) else {
            return false;
        };
        let changed = response.double_clicked() && doc.edit(|ents| draw.double_click(ents, inf));
        if let Some((measured, wanted)) = draw.take_rescale() {
            let what = if doc.context.is_empty() {
                "the model"
            } else {
                "the active group or component"
            };
            self.extra.rescale = Some((
                wanted / measured,
                format!(
                    "Do you want to resize {what}? (scale {:.4})",
                    wanted / measured
                ),
            ));
        }
        changed
    }

    /// Edit > Undo while a tool is mid-operation cancels the operation instead
    /// [doc]. Returns whether it did.
    pub(crate) fn undo_cancels_tool(&mut self) -> bool {
        let busy_draw = self.draw.as_ref().is_some_and(|d| d.anchor().is_some());
        let busy_modify = matches!(self.tool, Tool::Modify(_))
            && self.modify.as_ref().is_some_and(|m| m.anchor().is_some());
        if !(busy_draw || busy_modify) {
            return false;
        }
        if let Some(d) = self.draw.as_mut() {
            d.cancel();
        }
        if busy_modify && let Some(m) = self.modify.as_mut() {
            m.cancel();
        }
        self.shift_lock.reset(&mut self.engine);
        self.live = None;
        true
    }

    /// Orbit: a double click centers the view on the point under the cursor; a
    /// double click with the middle button does it in any tool.
    pub(crate) fn orbit_double_click(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        viewport_px: DVec2,
    ) {
        let left = self.tool == Tool::Orbit && response.double_clicked();
        if !(left || response.double_clicked_by(egui::PointerButton::Middle)) {
            return;
        }
        let Some(p) = response.interact_pointer_pos() else {
            return;
        };
        let ppp = ui.ctx().pixels_per_point();
        let c = DVec2::new(
            f64::from((p.x - rect.min.x) * ppp),
            f64::from((p.y - rect.min.y) * ppp),
        );
        let ray = self.camera.ray(c, viewport_px);
        let Some(hit) = self.bvh.ray(ray.origin, ray.dir) else {
            return;
        };
        let point = ray.origin + ray.dir * hit.t;
        let mut cam = self.camera.clone();
        let shift = point - cam.target;
        cam.target = point;
        cam.eye += shift;
        self.go(cam);
    }

    /// The Tape Measure rescale question and the section plane name prompt; takes a
    /// section plane placed by the Section Plane tool.
    pub(crate) fn tool_extra_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        // Ctrl+plus / minus are the drawing tools' (segments), not egui's zoom, while
        // one is active.
        let zoom_keys = self.draw.is_none();
        if ctx.options(|o| o.zoom_with_keyboard) != zoom_keys {
            ctx.options_mut(|o| o.zoom_with_keyboard = zoom_keys);
        }
        if let Some(sp) = self.modify.as_mut().and_then(|m| m.take_section()) {
            if self.extra.section_no_prompt {
                self.add_section(sp, frame);
            } else {
                self.extra.section = Some(sp);
            }
        }
        if let Some((factor, text)) = self.extra.rescale.clone() {
            let r = mt_ui::Dialog::new("rescale", "Meshtard")
                .width(320.0)
                .show(ctx, |ui| {
                    mt_ui::dialog::message(ui, &text, None);
                    mt_ui::dialog::buttons(ui, &["Yes", "No"], Some(0))
                });
            let answer = if r.closed { Some(1) } else { r.inner };
            if let Some(a) = answer {
                self.extra.rescale = None;
                if a == 0
                    && let Some(doc) = self.document.as_mut()
                    && doc.edit(|ents| {
                        crate::context_extra::scale_all(ents, factor);
                        true
                    })
                    && let Some(rs) = frame.wgpu_render_state()
                {
                    let device = rs.device.clone();
                    self.rebuild(&device);
                }
            }
        }
        if let Some(mut sp) = self.extra.section.take() {
            let mut no_prompt = self.extra.section_no_prompt;
            let r = mt_ui::Dialog::new("name_section", "Name Section Plane")
                .width(300.0)
                .show(ctx, |ui| {
                    crate::export::dialog_visuals(ui);
                    egui::Grid::new("mt_name_section")
                        .num_columns(2)
                        .spacing([8.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("Name:");
                            ui.add(egui::TextEdit::singleline(&mut sp.name).desired_width(200.0));
                            ui.end_row();
                            ui.label("Symbol:");
                            ui.add(egui::TextEdit::singleline(&mut sp.symbol).desired_width(60.0));
                            ui.end_row();
                        });
                    ui.add_space(4.0);
                    ui.checkbox(&mut no_prompt, "Please don't ask again");
                    crate::presentation::ok_cancel(ui, "Place")
                });
            match crate::presentation::answer(r) {
                None => self.extra.section = Some(sp),
                Some(false) => {}
                Some(true) => {
                    self.extra.section_no_prompt = no_prompt;
                    self.add_section(sp, frame);
                }
            }
        }
    }

    fn add_section(&mut self, sp: mt_kernel::SectionPlane, frame: &eframe::Frame) {
        if let Some(doc) = self.document.as_mut()
            && doc.edit(|ents| {
                ents.section_planes.push(sp);
                ents.active_section = Some(ents.section_planes.len() - 1);
                true
            })
            && let Some(rs) = frame.wgpu_render_state()
        {
            let device: wgpu::Device = rs.device.clone();
            self.rebuild(&device);
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::{DMat4, DVec3};
    use mt_infer::units::LengthUnit;
    use mt_infer::{Inference, Kind};
    use mt_kernel::{Entities, add_line};

    use crate::document::Owner;
    use crate::modify::{Ctx, ModifyTool};
    use crate::tools::{
        ArcSegments, Coords, DrawHover, DrawTool, TapeMeasure, TextTool, fillet_arc, for_command,
        parse_coords,
    };

    const IN: LengthUnit = LengthUnit::Inch;

    fn p(x: f64, y: f64, z: f64) -> DVec3 {
        DVec3::new(x, y, z)
    }

    fn inf(point: DVec3) -> Inference {
        Inference {
            point,
            kind: Kind::Free,
            edge: None,
            guide: None,
            locked: false,
            normal: None,
            segment: None,
            container: None,
        }
    }

    fn ctx<'a>(ents: &'a mut Entities, picked: Option<Owner>, sel: &'a [Owner]) -> Ctx<'a> {
        Ctx {
            ents,
            picked,
            selection: sel,
            ctrl: false,
            eye: DVec3::ZERO,
        }
    }

    fn square(e: &mut Entities, o: DVec3, s: f64) -> mt_kernel::FaceId {
        let q = [o, o + p(s, 0.0, 0.0), o + p(s, s, 0.0), o + p(0.0, s, 0.0)];
        for i in 0..4 {
            add_line(e, q[i], q[(i + 1) % 4]);
        }
        e.faces
            .iter()
            .find(|(_, f)| {
                e.loop_points(&f.outer)
                    .iter()
                    .all(|v| v.x >= o.x - 1e-9 && v.x <= o.x + s + 1e-9)
            })
            .map(|(id, _)| id)
            .unwrap()
    }

    #[test]
    fn typed_coordinates() {
        assert_eq!(
            parse_coords("[1,2,3]", IN),
            Some(Coords::Absolute(p(1.0, 2.0, 3.0)))
        );
        assert_eq!(
            parse_coords("<1m; 0; 0>", IN),
            Some(Coords::Relative(p(1000.0 / 25.4, 0.0, 0.0)))
        );
        assert_eq!(parse_coords("[1,2]", IN), None);
        assert_eq!(parse_coords("12", IN), None);
        // Line: an absolute start, then a relative end.
        let mut e = Entities::new();
        let mut line = for_command("tool.line").unwrap();
        assert!(!line.typed(&mut e, None, "[1,1,0]", IN));
        assert!(line.typed(&mut e, None, "<10,0,0>", IN));
        assert!(line.typed(&mut e, None, "[11,5,0]", IN));
        let mut pts: Vec<DVec3> = e.vertices.values().map(|v| v.pos).collect();
        pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
        assert_eq!(
            pts,
            vec![p(1.0, 1.0, 0.0), p(11.0, 1.0, 0.0), p(11.0, 5.0, 0.0)]
        );
        // Move: absolute takes the base point there, relative moves by the vector.
        let mut e = Entities::new();
        add_line(&mut e, p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
        let sel: Vec<Owner> = e.edges.keys().map(Owner::Edge).collect();
        let mut mv = crate::modify::for_command("tool.move").unwrap();
        mv.click(&mut ctx(&mut e, None, &sel), &inf(p(1.0, 0.0, 0.0)));
        assert!(mv.typed(&mut ctx(&mut e, None, &sel), None, "[5,5,5]", IN));
        assert!(e.vertices.values().any(|v| v.pos == p(5.0, 5.0, 5.0)));
        mv.click(&mut ctx(&mut e, None, &sel), &inf(p(5.0, 5.0, 5.0)));
        assert!(mv.typed(&mut ctx(&mut e, None, &sel), None, "<0,0,-5>", IN));
        assert!(e.vertices.values().any(|v| v.pos == p(5.0, 5.0, 0.0)));
    }

    #[test]
    fn rotate_takes_a_slope() {
        let mut e = Entities::new();
        add_line(&mut e, p(1.0, 0.0, 0.0), p(2.0, 0.0, 0.0));
        let sel: Vec<Owner> = e.edges.keys().map(Owner::Edge).collect();
        let mut rot = crate::modify::for_command("tool.rotate").unwrap();
        for q in [DVec3::ZERO, p(1.0, 0.0, 0.0)] {
            rot.click(&mut ctx(&mut e, None, &sel), &inf(q));
        }
        // 1:1 is 45 degrees.
        assert!(rot.typed(
            &mut ctx(&mut e, None, &sel),
            Some(&inf(p(1.0, 1.0, 0.0))),
            "1:1",
            IN
        ));
        let h = 2f64.sqrt();
        assert!(
            e.vertices
                .values()
                .any(|v| (v.pos - p(h, h, 0.0)).length() < 1e-9)
        );
    }

    #[test]
    fn arc_segments_from_a_circle_and_ctrl_steps() {
        let mut s = ArcSegments::default();
        assert_eq!(s.count(1.0), crate::tools::ARC_SEGMENTS);
        assert!(s.parse("20c"));
        assert_eq!(s.count(std::f64::consts::PI), 10);
        assert_eq!(s.count(std::f64::consts::FRAC_PI_2), 5);
        s.step(true);
        // 21 sides, half of them rounded up.
        assert_eq!(s.count(std::f64::consts::PI), 11);
        assert!(s.parse("8s"));
        assert_eq!(s.count(0.1), 8);
        s.step(false);
        assert_eq!(s.count(0.1), 7);
        assert!(!s.parse("8x"));
        // Polygon tool: Ctrl+plus adds a side.
        let mut poly = for_command("tool.polygon").unwrap();
        assert!(poly.step_segments(true));
        assert_eq!(poly.measurement(None, IN), ("Sides", "7".into()));
    }

    #[test]
    fn two_point_arc_repeats_as_a_fillet() {
        let (t1, t2, arc) = fillet_arc(
            DVec3::ZERO,
            p(10.0, 0.0, 0.0),
            p(0.0, 10.0, 0.0),
            2.0,
            ArcSegments::default(),
        )
        .unwrap();
        assert!((t1 - p(2.0, 0.0, 0.0)).length() < 1e-9);
        assert!((t2 - p(0.0, 2.0, 0.0)).length() < 1e-9);
        assert!((arc.center - p(2.0, 2.0, 0.0)).length() < 1e-9);
        assert!((arc.points().last().unwrap().distance(t2)) < 1e-9);
        assert!(
            fillet_arc(
                DVec3::ZERO,
                p(1.0, 0.0, 0.0),
                p(0.0, 1.0, 0.0),
                2.0,
                ArcSegments::default()
            )
            .is_none()
        );

        // Draw an arc of radius 5 (a half circle on a 10" chord), then double-click
        // the corner of an L: the corner is rounded with radius 5.
        let mut e = Entities::new();
        add_line(&mut e, p(0.0, 50.0, 0.0), p(20.0, 50.0, 0.0));
        add_line(&mut e, p(0.0, 50.0, 0.0), p(0.0, 70.0, 0.0));
        let mut arc = for_command("tool.arc_2pt").unwrap();
        arc.click(&mut e, &inf(p(100.0, 0.0, 0.0)));
        arc.click(&mut e, &inf(p(110.0, 0.0, 0.0)));
        assert!(arc.typed(&mut e, Some(&inf(p(105.0, 5.0, 0.0))), "5", IN));
        let corner = p(0.0, 50.0, 0.0);
        arc.click(&mut e, &inf(corner));
        arc.click(&mut e, &inf(corner));
        assert!(arc.double_click(&mut e, &inf(corner)));
        assert!(
            e.find_vertex(corner).is_none(),
            "the corner's waste is gone"
        );
        assert!(e.find_vertex(p(5.0, 50.0, 0.0)).is_some());
        assert!(e.find_vertex(p(0.0, 55.0, 0.0)).is_some());
    }

    #[test]
    fn tape_measure_modes_guide_point_and_rescale() {
        let mut e = Entities::new();
        let mut tape = TapeMeasure::default();
        let on_edge = Inference {
            kind: Kind::OnEdge,
            segment: Some([DVec3::ZERO, p(10.0, 0.0, 0.0)]),
            ..inf(p(5.0, 0.0, 0.0))
        };
        tape.click(&mut e, &on_edge);
        assert!(tape.click(&mut e, &inf(p(5.0, 3.0, 0.0))));
        assert!(e.guides[0].dir.is_some(), "Guide Line mode from an edge");
        tape.toggle_ctrl();
        tape.click(&mut e, &on_edge);
        assert!(tape.click(&mut e, &inf(p(5.0, 3.0, 0.0))));
        assert!(e.guides[1].dir.is_none(), "Guide Point mode");
        tape.toggle_ctrl();
        tape.click(&mut e, &inf(DVec3::ZERO));
        assert!(!tape.click(&mut e, &inf(p(4.0, 0.0, 0.0))), "Measure mode");
        assert_eq!(e.guides.len(), 2);
        assert!(tape.status().contains("Measure"));
        // Typed after measuring 4": resize to 8".
        assert!(!tape.typed(&mut e, None, "8", IN));
        assert_eq!(tape.take_rescale(), Some((4.0, 8.0)));
        assert_eq!(tape.take_rescale(), None);
        // A double click drops a guide point.
        tape.click(&mut e, &inf(p(1.0, 1.0, 0.0)));
        tape.click(&mut e, &inf(p(1.0, 1.0, 0.0)));
        assert!(tape.double_click(&mut e, &inf(p(1.0, 1.0, 0.0))));
        assert_eq!(e.guides.last().unwrap().point, p(1.0, 1.0, 0.0));
    }

    #[test]
    fn text_component_name_area_label_and_live_value() {
        let mut e = Entities::new();
        let f = square(&mut e, DVec3::ZERO, 12.0);
        let mut text = TextTool::default();
        text.hover(&DrawHover {
            instance_name: None,
            units: LengthUnit::Foot,
            alt: false,
        });
        let on_face = Inference {
            kind: Kind::OnFace,
            normal: Some(DVec3::Z),
            ..inf(p(6.0, 6.0, 0.0))
        };
        text.click(&mut e, &on_face);
        text.click(&mut e, &on_face);
        assert!(text.double_click(&mut e, &on_face));
        assert_eq!(e.texts[0].text, "1 ft²");
        assert_eq!(
            e.texts[0].style.as_ref().map(|s| s.leader),
            Some(mt_kernel::model_data::LeaderKind::Hidden)
        );
        // A leader on a component says its name.
        text.hover(&DrawHover {
            instance_name: Some("Chair".into()),
            units: IN,
            alt: false,
        });
        let in_comp = Inference {
            container: Some(mt_infer::Container::Component),
            ..on_face
        };
        text.click(&mut e, &in_comp);
        text.click(&mut e, &inf(p(20.0, 20.0, 0.0)));
        assert_eq!(text.text_draft().unwrap().label.format(IN), "Chair");
        // `<>` shows the face's area as it is now.
        let t = mt_kernel::Text::leader(p(6.0, 6.0, 0.0), p(5.0, 5.0, 0.0), "Floor: <>".into());
        assert_eq!(crate::annot::live_text(&e, &t, IN), "Floor: 144 in²");
        crate::modify::apply(
            &mut e,
            &[Owner::Face(f)],
            DMat4::from_scale(p(2.0, 1.0, 1.0)),
            false,
        );
        assert_eq!(crate::annot::live_text(&e, &t, IN), "Floor: 288 in²");
        let on_edge = mt_kernel::Text::leader(p(3.0, 0.0, 0.0), DVec3::Z, "<>".into());
        assert_eq!(crate::annot::live_text(&e, &on_edge, IN), "24\"");
    }

    #[test]
    fn offset_overlap_and_repeat() {
        let mut e = Entities::new();
        let a = square(&mut e, DVec3::ZERO, 10.0);
        let b = square(&mut e, p(20.0, 0.0, 0.0), 10.0);
        let mut off = crate::modify::for_command("tool.offset").unwrap();
        off.click(
            &mut ctx(&mut e, Some(Owner::Face(a)), &[]),
            &inf(p(5.0, 0.0, 0.0)),
        );
        assert!(off.typed(
            &mut ctx(&mut e, None, &[]),
            Some(&inf(p(5.0, 5.0, 0.0))),
            "2",
            IN
        ));
        let edges = e.edges.len();
        // Double click on the other square: the same 2" inward.
        let mut c = ctx(&mut e, Some(Owner::Face(b)), &[]);
        off.click(&mut c, &inf(p(25.0, 0.0, 0.0)));
        off.click(&mut c, &inf(p(25.0, 0.0, 0.0)));
        assert!(off.double_click(&mut c, &inf(p(25.0, 0.0, 0.0))));
        assert_eq!(e.edges.len(), edges + 4);
        assert!(e.find_vertex(p(22.0, 2.0, 0.0)).is_some());
        // Overlapping: inward past the middle the trimmed offset vanishes, the
        // overlapping one is kept (crossed over).
        let mut t = Entities::new();
        let f = square(&mut t, DVec3::ZERO, 10.0);
        let mut u = t.clone();
        assert!(mt_kernel::offset_face(&mut t, f, 6.0).is_empty());
        assert!(!mt_kernel::offset_face_overlapping(&mut u, f, 6.0).is_empty());
    }

    #[test]
    fn section_plane_locks_and_prompt() {
        let mut e = Entities::new();
        let mut sp = crate::modify::for_command("tool.section_plane").unwrap();
        assert!(sp.arrow(mt_infer::Axis::Red));
        let face = Inference {
            normal: Some(DVec3::Z),
            ..inf(p(1.0, 2.0, 3.0))
        };
        assert!(
            !sp.click(&mut ctx(&mut e, None, &[]), &face),
            "added after the prompt"
        );
        let placed = sp.take_section().unwrap();
        assert_eq!(placed.normal, DVec3::X);
        assert_eq!(placed.name, "Section Plane 1");
        assert!(sp.take_section().is_none());
        assert!(sp.arrow(mt_infer::Axis::Red), "pressed again: unlocked");
        sp.click(&mut ctx(&mut e, None, &[]), &face);
        assert_eq!(sp.take_section().unwrap().normal, DVec3::Z);
        assert!(e.section_planes.is_empty());
    }

    #[test]
    fn scale_modifiers_are_held() {
        use crate::modify::Input;
        let mut e = Entities::new();
        let f = square(&mut e, DVec3::ZERO, 10.0);
        let sel = [Owner::Face(f)];
        let mut sc = crate::modify::for_command("tool.scale").unwrap();
        let project = |q: DVec3| Some(glam::DVec2::new(q.x * 10.0, q.y * 10.0));
        let no_bounds = |_: usize| None;
        let held = |ctrl, shift| crate::cursor::Mods {
            ctrl,
            shift,
            alt: false,
        };
        let input = |sc: &mut Box<dyn ModifyTool>, ctrl, shift| {
            sc.input(&Input {
                held: held(ctrl, shift),
                press: false,
                ray: None,
                cur: None,
                material: None,
            })
        };
        input(&mut sc, true, false);
        sc.toggle_ctrl();
        sc.hover(&crate::modify::Hover {
            ents: &e,
            selection: &sel,
            instance_bounds: &no_bounds,
            project: &project,
            cursor: Some(glam::DVec2::new(100.0, 100.0)),
        });
        // Corner grip at (10, 10), Ctrl held: about the center (5, 5).
        sc.click(&mut ctx(&mut e, None, &sel), &inf(p(10.0, 10.0, 0.0)));
        let (_, xf, _) = sc.live(&inf(p(15.0, 15.0, 0.0))).unwrap();
        assert!((xf.transform_point3(p(5.0, 5.0, 0.0)) - p(5.0, 5.0, 0.0)).length() < 1e-9);
        assert!((xf.transform_point3(p(10.0, 10.0, 0.0)) - p(15.0, 15.0, 0.0)).length() < 1e-9);
        // Shift held too: the corner scales each axis on its own.
        input(&mut sc, true, true);
        let (_, xf, _) = sc.live(&inf(p(15.0, 10.0, 0.0))).unwrap();
        assert!((xf.transform_point3(p(10.0, 10.0, 0.0)) - p(15.0, 10.0, 0.0)).length() < 1e-9);
        // Released: the opposite corner stays, uniform again.
        input(&mut sc, false, false);
        let (_, xf, _) = sc.live(&inf(p(20.0, 20.0, 0.0))).unwrap();
        assert!(xf.transform_point3(DVec3::ZERO).length() < 1e-9);
    }

    #[test]
    fn zoom_shift_drag_changes_the_field_of_view() {
        let mut c = mt_render::Camera {
            eye: p(-300.0, -500.0, 120.0),
            target: DVec3::ZERO,
            fov_deg: 35.0,
            perspective: true,
            two_point: false,
        };
        let eye = c.eye;
        crate::nav::apply(
            crate::nav::Tool::Zoom,
            &mut c,
            &crate::nav::Input {
                left_drag: glam::DVec2::new(0.0, 50.0),
                shift: true,
                ..Default::default()
            },
            glam::DVec2::new(800.0, 600.0),
        );
        assert_eq!(c.eye, eye);
        assert!((c.fov_deg - 40.0).abs() < 1e-9);
    }

    #[test]
    fn tag_all_instances_reaches_nested_ones() {
        let mut m = mt_kernel::Model::default();
        let d = m.definitions.insert(Default::default());
        let inst = mt_kernel::Instance {
            definition: d,
            transform: DMat4::IDENTITY,
            name: String::new(),
            hidden: false,
            locked: false,
            layer: None,
            material: None,
            attrs: Default::default(),
        };
        let outer = m.definitions.insert(Default::default());
        m.definitions[outer].entities.instances.push(inst.clone());
        m.root.instances.push(inst);
        assert!(crate::pick_tool::tag_all_instances(&mut m, d, Some(1)));
        assert_eq!(m.root.instances[0].layer, Some(1));
        assert_eq!(m.definitions[outer].entities.instances[0].layer, Some(1));
        assert!(!crate::pick_tool::tag_all_instances(&mut m, d, Some(1)));
    }
}
