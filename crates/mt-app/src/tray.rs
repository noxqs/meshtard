//! The Default Tray's model panels: Tags, Materials, Outliner, Scenes, Styles and
//! Shadows. Builds `mt_ui::TrayData` from the document and applies the panels' events.

use mt_kernel::{Entities, Layer, Material, Model};
use mt_ui::{Event, MaterialRow, OutlinerKind, OutlinerNode, SelectMode, TagRow, TrayData};

use crate::App;
use crate::document::Owner;
use crate::selection::Mode;

impl App {
    /// What the tray panels show this frame.
    pub(crate) fn tray_data(&mut self) -> TrayData {
        let mut data = TrayData {
            active_tag: self.active_tag,
            current_material: self.current_material,
            active_scene: self.active_scene,
            scene_tabs: self.scene_tabs,
            soften: self.soften.settings,
            ..TrayData::default()
        };
        let Some(doc) = &self.document else {
            return data;
        };
        let m = &doc.model;
        data.tags = m
            .layers
            .iter()
            .enumerate()
            .map(|(i, l)| TagRow {
                // SketchUp 2020+ calls Layer0 "Untagged".
                name: if i == 0 {
                    "Untagged".into()
                } else {
                    l.name.clone()
                },
                visible: l.visible,
                color: l.color,
                folder: l.folder,
                dashes: mt_kernel::model_data::Dashes::ALL
                    .iter()
                    .position(|d| *d == l.dashes)
                    .unwrap_or(0),
            })
            .collect();
        data.tag_folders = m
            .tag_folders
            .iter()
            .map(|f| mt_ui::TagFolderRow {
                name: f.name.clone(),
                parent: f.parent,
                visible: f.visible,
            })
            .collect();
        data.color_by_tag = m.color_by_tag;
        data.styles = m.styles.iter().map(|s| s.name.clone()).collect();
        data.active_style = m.active_style;
        data.scene_details = m
            .pages
            .iter()
            .map(|p| mt_ui::SceneDetails {
                description: p.description.clone(),
                include_in_animation: p.include_in_animation,
                props: crate::tray_more::props_to_ui(p.props),
            })
            .collect();
        data.can_replace_component = self
            .selection
            .items()
            .iter()
            .any(|o| matches!(o, Owner::Instance(_)));
        data.materials = m
            .materials
            .iter()
            .map(|x| MaterialRow {
                name: x.name.clone(),
                color: x.color,
                opacity: (x.opacity * 100.0).round().clamp(0.0, 100.0) as u8,
                texture: x
                    .texture
                    .as_ref()
                    .map(|t| crate::material_edit::texture_row(t, doc.units)),
            })
            .collect();
        data.can_position_texture = self.can_position_texture();
        let selected: Vec<usize> = if doc.context.is_empty() {
            self.selection
                .items()
                .iter()
                .filter_map(|o| match o {
                    Owner::Instance(i) => Some(*i),
                    _ => None,
                })
                .collect()
        } else {
            Vec::new()
        };
        let sections: Vec<usize> = if doc.context.is_empty() {
            (self.selection.items().iter())
                .filter_map(|o| match o {
                    Owner::Section(i) => Some(*i),
                    _ => None,
                })
                .collect()
        } else {
            Vec::new()
        };
        data.outliner = outliner(m, &m.root, (&selected, &sections), Some(&doc.context), 0);
        let defs = component_definitions(m);
        data.components = defs
            .iter()
            .map(|&d| {
                let def = &m.definitions[d];
                mt_ui::ComponentRow {
                    name: def.name.clone(),
                    description: def.description.clone(),
                    instances: mt_kernel::group::instance_count(m, d),
                    behavior: mt_ui::ComponentBehavior {
                        glue: crate::tray_more::glue_index(def.glue),
                        cut_opening: def.cut_opening,
                        always_face_camera: def.always_face_camera,
                        shadows_face_sun: def.shadows_face_sun,
                    },
                }
            })
            .collect();
        data.current_component = self.current_component;
        data.scenes = m.pages.iter().map(|p| p.name.clone()).collect();
        data.style = mt_ui::StyleSettings {
            edges: m.style.edges,
            profiles: m.style.profiles,
            profile_width: m.style.profile_width.round() as u8,
            back_edges: m.style.back_edges,
            depth_cue: m.style.depth_cue,
            depth_cue_width: m.style.depth_cue_width.round() as u8,
            extension: m.style.extension,
            extension_length: m.style.extension_length.round() as u8,
            section_cut_width: m.style.section_cut_width.round() as u8,
            section_fill: m.style.section_fill,
            section_fill_color: m.style.section_fill_color,
            face_style: match m.style.face_style {
                mt_kernel::FaceStyle::Wireframe => mt_ui::FaceStyle::Wireframe,
                mt_kernel::FaceStyle::HiddenLine => mt_ui::FaceStyle::HiddenLine,
                mt_kernel::FaceStyle::Shaded => mt_ui::FaceStyle::Shaded,
                mt_kernel::FaceStyle::ShadedWithTextures => mt_ui::FaceStyle::ShadedWithTextures,
                mt_kernel::FaceStyle::Monochrome => mt_ui::FaceStyle::Monochrome,
            },
            x_ray: m.style.xray,
        };
        (data.style_extras, data.watermarks) = crate::style_view::tray_extras(m);
        let fog = &m.style.fog;
        let percent = |x: f32| (x * 100.0).round().clamp(0.0, 100.0) as u8;
        data.fog = mt_ui::FogSettings {
            enabled: fog.enabled,
            start: percent(fog.start),
            end: percent(fog.end),
            use_background: fog.use_background,
            color: fog.color,
        };
        let layer_of = |o: &Owner| -> Option<usize> {
            let ents = doc.active();
            match *o {
                Owner::Edge(e) => ents.edges.get(e).map(|x| x.attrs.layer),
                Owner::Face(f) => ents.faces.get(f).map(|x| x.attrs.layer),
                Owner::Instance(i) => ents.instances.get(i).map(|x| x.layer),
                Owner::Dimension(i) => ents.dimensions.get(i).map(|x| x.layer),
                Owner::Text(i) => ents.texts.get(i).map(|x| x.layer),
                Owner::Section(_) => None,
            }
            .map(|l| l.unwrap_or(0))
        };
        data.entity_tag = self.selection.items().first().and_then(layer_of);
        let s = &m.shadows;
        data.shadows = mt_ui::ShadowSettings {
            enabled: s.enabled,
            minute_of_day: (s.hour * 60 + s.minute) as u16,
            day_of_year: mt_ui::day_of_year(s.month as u16, s.day as u16),
            light: (s.light * 100.0).round() as u8,
            dark: (s.dark * 100.0).round() as u8,
            on_faces: s.on_faces,
            on_ground: s.on_ground,
            utc_offset_minutes: (s.utc_offset * 60.0).round() as i16,
            use_sun_for_shading: s.use_sun_for_shading,
            from_edges: s.from_edges,
        };
        self.tray_data_extra(&mut data);
        data
    }

    /// Applies a tray event. Returns whether the scene must be rebuilt.
    pub(crate) fn tray_event(&mut self, event: Event) -> bool {
        if let Some(rebuild) = self.environment_event(&event) {
            return rebuild;
        }
        let event = match event {
            Event::EntityInfoEdit { key, edit } => return self.entity_info_edit(&key, &edit),
            Event::MaterialTexture { index, action } => {
                return self.material_texture(index, action);
            }
            e @ (Event::StyleExtrasChanged(_)
            | Event::WatermarkAdd
            | Event::WatermarkRemove(_)
            | Event::WatermarkChanged(..)) => return self.style_extras_event(e),
            e @ (Event::OutlinerSelect { .. }
            | Event::OutlinerRename { .. }
            | Event::OutlinerVisible { .. }) => match self.outliner_section_event(e) {
                Ok(rebuild) => return rebuild,
                Err(e) => e,
            },
            e => e,
        };
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        match event {
            Event::TagAdd => doc.edit_model(|m, _| {
                let n = (1..)
                    .find(|n| !m.layers.iter().any(|l| l.name == format!("Tag{n}")))
                    .unwrap_or(1);
                m.layers.push(Layer {
                    name: format!("Tag{n}"),
                    color: tag_color(m.layers.len()),
                    visible: true,
                    ..Layer::default()
                });
                crate::tray_extra::new_tag_in_scenes(m, m.layers.len() - 1);
                true
            }),
            Event::TagDelete(i) if i > 0 => {
                if self.active_tag == i {
                    self.active_tag = 0;
                }
                // Entities on the deleted tag go to Untagged (SketchUp asks; this is
                // its default choice, unverified).
                doc.edit_model(|m, ents| crate::tray_extra::delete_tags(m, ents, &[i]))
            }
            Event::TagRename(i, name) if i > 0 => doc.edit_model(|m, _| {
                m.layers.get_mut(i).is_some_and(|l| {
                    l.name = name;
                    true
                })
            }),
            Event::TagVisible(i, v) => doc.edit_model(|m, _| {
                m.layers.get_mut(i).is_some_and(|l| {
                    let changed = l.visible != v;
                    l.visible = v;
                    changed
                })
            }),
            Event::TagColor(i, c) => doc.edit_model(|m, _| {
                m.layers.get_mut(i).is_some_and(|l| {
                    l.color = c;
                    true
                })
            }),
            Event::TagActive(i) => {
                self.active_tag = i;
                false
            }
            Event::MaterialPick(i) => {
                self.current_material = Some(i);
                if let Some(t) = self.modify.as_mut() {
                    t.set_material(Some(i));
                }
                false
            }
            Event::MaterialDefault => {
                self.current_material = None;
                if let Some(t) = self.modify.as_mut() {
                    t.set_material(None);
                }
                false
            }
            Event::MaterialCreate => {
                doc.edit_model(|m, _| {
                    let n = (1..)
                        .map(|n| {
                            if n == 1 {
                                "Material".to_owned()
                            } else {
                                format!("Material{n}")
                            }
                        })
                        .find(|name| !m.materials.iter().any(|x| &x.name == name))
                        .unwrap_or_default();
                    // SketchUp's new-material color is unverified; a neutral gray here.
                    m.materials.push(Material {
                        name: n,
                        color: [200, 200, 200],
                        opacity: 1.0,
                        texture: None,
                    });
                    true
                });
                let i = doc.model.materials.len() - 1;
                self.current_material = Some(i);
                if let Some(t) = self.modify.as_mut() {
                    t.set_material(Some(i));
                }
                false
            }
            Event::EntityTag(tag) => {
                let items = self.selection.items().to_vec();
                let layer = (tag > 0).then_some(tag);
                doc.edit(|ents| {
                    for o in &items {
                        match *o {
                            Owner::Edge(e) => {
                                if let Some(x) = ents.edges.get_mut(e) {
                                    x.attrs.layer = layer;
                                }
                            }
                            Owner::Face(f) => {
                                if let Some(x) = ents.faces.get_mut(f) {
                                    x.attrs.layer = layer;
                                }
                            }
                            Owner::Instance(i) => {
                                if let Some(x) = ents.instances.get_mut(i) {
                                    x.layer = layer;
                                }
                            }
                            Owner::Dimension(i) => {
                                if let Some(x) = ents.dimensions.get_mut(i) {
                                    x.layer = layer;
                                }
                            }
                            Owner::Text(i) => {
                                if let Some(x) = ents.texts.get_mut(i) {
                                    x.layer = layer;
                                }
                            }
                            Owner::Section(_) => {}
                        }
                    }
                    !items.is_empty()
                })
            }
            Event::ComponentPick(i) => {
                let Some(&d) = component_definitions(&doc.model).get(i) else {
                    return false;
                };
                self.current_component = Some(i);
                if doc.in_context_path(d) {
                    self.message = Some("A component cannot be placed inside itself.".into());
                    return false;
                }
                self.tool = crate::nav::Tool::Modify("tool.place_component");
                self.draw = None;
                let glue = doc.model.definitions[d].glue;
                self.modify = Some(Box::new(crate::modify::PlaceComponent {
                    definition: d,
                    glue,
                    ..Default::default()
                }));
                false
            }
            Event::ComponentEdit {
                index,
                name,
                description,
            } => {
                let Some(&d) = component_definitions(&doc.model).get(index) else {
                    return false;
                };
                doc.edit_model(|m, _| {
                    let def = &mut m.definitions[d];
                    let changed = def.name != name || def.description != description;
                    def.name = name;
                    def.description = description;
                    changed
                })
            }
            Event::ComponentPurge => doc.edit_model(|m, ents| {
                // Definitions used nowhere (the open context's entities are taken out of
                // the model during the edit, so count them too).
                let used = |m: &Model, d| {
                    mt_kernel::group::instance_count(m, d) > 0
                        || ents.instances.iter().any(|i| i.definition == d)
                };
                let unused: Vec<_> = m.definitions.keys().filter(|&d| !used(m, d)).collect();
                for d in &unused {
                    m.definitions.remove(*d);
                }
                !unused.is_empty()
            }),
            Event::MaterialEdit {
                index,
                name,
                color,
                opacity,
            } => doc.edit_model(|m, _| {
                let Some(x) = m.materials.get_mut(index) else {
                    return false;
                };
                // A recolored texture shows colorized, as in SketchUp.
                let texture = x.texture.clone().map(|mut t| {
                    t.colorize |= color != x.color;
                    t
                });
                let new = Material {
                    name,
                    color,
                    opacity: f64::from(opacity) / 100.0,
                    texture,
                };
                let changed = *x != new;
                *x = new;
                changed
            }),
            Event::OutlinerSelect { path, mode } => {
                let Some((&last, parents)) = path.split_last() else {
                    return false;
                };
                // Selecting a nested entity opens its parent for editing, as SketchUp does.
                if doc.context != parents {
                    doc.context = parents.to_vec();
                    self.selection.clear();
                }
                let mode = match mode {
                    SelectMode::Replace => Mode::Replace,
                    SelectMode::Add => Mode::Add,
                    SelectMode::Toggle => Mode::Toggle,
                    SelectMode::Remove => Mode::Remove,
                };
                self.selection.apply(mode, [Owner::Instance(last)]);
                true
            }
            Event::SceneAdd => self.scene_add(),
            Event::SceneUpdate(i) => self.scene_update(i),
            Event::SceneDelete(i) => {
                self.active_scene = None;
                doc.edit_model(|m, _| {
                    if i < m.pages.len() {
                        m.pages.remove(i);
                        true
                    } else {
                        false
                    }
                })
            }
            // The camera moves there over the View > Animation > Settings transition
            // time (see `App::animate_camera`); tag visibility and the other saved
            // properties change at once (SketchUp fades tags, unverified).
            Event::SceneActivate(i) => self.scene_activate(i),
            Event::StyleChanged(s) => {
                let st = &mut doc.model.style;
                st.edges = s.edges;
                st.profiles = s.profiles;
                st.profile_width = f32::from(s.profile_width);
                st.back_edges = s.back_edges;
                st.depth_cue = s.depth_cue;
                st.depth_cue_width = f32::from(s.depth_cue_width);
                st.extension = s.extension;
                st.extension_length = f32::from(s.extension_length);
                st.section_cut_width = f32::from(s.section_cut_width);
                st.section_fill = s.section_fill;
                st.section_fill_color = s.section_fill_color;
                st.xray = s.x_ray;
                st.face_style = match s.face_style {
                    mt_ui::FaceStyle::Wireframe => mt_kernel::FaceStyle::Wireframe,
                    mt_ui::FaceStyle::HiddenLine => mt_kernel::FaceStyle::HiddenLine,
                    mt_ui::FaceStyle::Shaded => mt_kernel::FaceStyle::Shaded,
                    mt_ui::FaceStyle::ShadedWithTextures => {
                        mt_kernel::FaceStyle::ShadedWithTextures
                    }
                    mt_ui::FaceStyle::Monochrome => mt_kernel::FaceStyle::Monochrome,
                };
                true
            }
            Event::FogChanged(f) => {
                // A display setting like the Styles panel's; not an undo step.
                doc.model.style.fog = mt_kernel::FogSettings {
                    enabled: f.enabled,
                    start: f32::from(f.start) / 100.0,
                    end: f32::from(f.end) / 100.0,
                    use_background: f.use_background,
                    color: f.color,
                };
                true
            }
            Event::SoftenChanged(s) => self.soften_changed(s),
            Event::ShadowsChanged(s) => {
                let (month, day) = mt_ui::month_day(s.day_of_year);
                let sh = &mut doc.model.shadows;
                sh.enabled = s.enabled;
                sh.month = u32::from(month);
                sh.day = u32::from(day);
                sh.hour = u32::from(s.minute_of_day / 60);
                sh.minute = u32::from(s.minute_of_day % 60);
                sh.light = f64::from(s.light) / 100.0;
                sh.dark = f64::from(s.dark) / 100.0;
                sh.on_faces = s.on_faces;
                sh.on_ground = s.on_ground;
                sh.utc_offset = f64::from(s.utc_offset_minutes) / 60.0;
                sh.use_sun_for_shading = s.use_sun_for_shading;
                sh.from_edges = s.from_edges;
                true
            }
            e => match self.tray_extra_event(e) {
                Ok(rebuild) => rebuild,
                Err(e) => self.tray_more_event(e),
            },
        }
    }
}

/// SketchUp's default scene transition time (Model Info > Animation, 2 seconds;
/// unverified).
pub(crate) const SCENE_TRANSITION_SECS: f64 = 2.0;

/// A camera move to an activated scene.
pub(crate) struct Transition {
    from: mt_render::Camera,
    to: mt_render::Camera,
    /// egui time of the first frame; set by the first [`Transition::at`].
    start: Option<f64>,
    /// Duration in seconds (0 jumps).
    secs: f64,
}

impl Transition {
    pub(crate) fn new(from: mt_render::Camera, to: mt_render::Camera) -> Self {
        Self::with_secs(from, to, SCENE_TRANSITION_SECS)
    }

    pub(crate) fn with_secs(from: mt_render::Camera, to: mt_render::Camera, secs: f64) -> Self {
        Self {
            from,
            to,
            start: None,
            secs,
        }
    }

    /// The camera at egui time `now` and whether the move is over.
    pub(crate) fn at(&mut self, now: f64) -> (mt_render::Camera, bool) {
        let start = *self.start.get_or_insert(now);
        if self.secs <= 0.0 {
            return (self.to.clone(), true);
        }
        let t = (now - start) / self.secs;
        let camera = self.from.interpolate(&self.to, mt_render::ease_in_out(t));
        (camera, t >= 1.0)
    }
}

impl App {
    /// Advances a scene transition: sets the camera for this frame and asks for the
    /// next one. Esc cancels it where it is (navigating cancels it too; see
    /// [`App::cancel_transition_if_moved`]).
    pub(crate) fn animate_camera(&mut self, ctx: &eframe::egui::Context) {
        let Some(tr) = self.transition.as_mut() else {
            return;
        };
        if ctx.input(|i| i.key_pressed(eframe::egui::Key::Escape)) {
            self.transition = None;
            return;
        }
        let (camera, done) = tr.at(ctx.input(|i| i.time));
        self.camera = camera;
        if done {
            self.transition = None;
        } else {
            ctx.request_repaint();
        }
    }

    /// Ends a scene transition when navigation moved the camera away from `before`.
    pub(crate) fn cancel_transition_if_moved(&mut self, before: &mt_render::Camera) {
        if self.camera != *before {
            self.transition = None;
        }
    }
}

/// Component definitions in Components panel order (groups and images are not
/// listed).
pub(crate) fn component_definitions(m: &Model) -> Vec<mt_kernel::DefinitionId> {
    m.definitions
        .iter()
        .filter(|(_, d)| !d.is_group && !d.is_image)
        .map(|(k, _)| k)
        .collect()
}

/// Colors for new tags (draft; SketchUp's palette is unverified).
fn tag_color(i: usize) -> [u8; 3] {
    const PALETTE: [[u8; 3]; 6] = [
        [220, 120, 120],
        [120, 180, 220],
        [140, 200, 120],
        [220, 190, 110],
        [180, 140, 210],
        [120, 200, 190],
    ];
    PALETTE[i % PALETTE.len()]
}

/// The Outliner tree below `ents`; `open` is the rest of the open context's path at
/// this level (`None` off the path).
fn outliner(
    m: &Model,
    ents: &Entities,
    (selected, sections): (&[usize], &[usize]),
    open: Option<&[usize]>,
    depth: usize,
) -> Vec<OutlinerNode> {
    let mut nodes: Vec<OutlinerNode> = ents
        .instances
        .iter()
        .enumerate()
        .map(|(i, inst)| {
            let def = m.definitions.get(inst.definition);
            let on_hidden_tag = inst.layer.is_some_and(|l| !m.layer_shown(l));
            let child_open = open.and_then(|p| match p {
                [first, rest @ ..] if *first == i => Some(rest),
                _ => None,
            });
            OutlinerNode {
                name: inst.name.clone(),
                kind: match def {
                    Some(d) if d.is_group => OutlinerKind::Group,
                    d => OutlinerKind::Component {
                        definition: d.map(|d| d.name.clone()).unwrap_or_default(),
                    },
                },
                hidden: inst.hidden || on_hidden_tag,
                selected: depth == 0 && selected.contains(&i),
                // Guard against definitions that contain themselves.
                children: match def {
                    Some(d) if depth < 32 => {
                        outliner(m, &d.entities, (&[], &[]), child_open, depth + 1)
                    }
                    _ => Vec::new(),
                },
                instance_hidden: inst.hidden,
                locked: inst.locked,
                open: child_open.is_some(),
            }
        })
        .collect();
    // Section planes after the groups and components (path index past them).
    nodes.extend(
        ents.section_planes
            .iter()
            .enumerate()
            .map(|(k, s)| OutlinerNode {
                name: s.name.clone(),
                kind: OutlinerKind::SectionPlane {
                    active: ents.active_section == Some(k),
                },
                hidden: s.hidden,
                selected: depth == 0 && sections.contains(&k),
                children: Vec::new(),
                instance_hidden: s.hidden,
                locked: false,
                open: false,
            }),
    );
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    fn cam(eye: DVec3, fov: f64) -> mt_render::Camera {
        mt_render::Camera {
            eye,
            target: DVec3::ZERO,
            fov_deg: fov,
            perspective: true,
            two_point: false,
        }
    }

    #[test]
    fn transition_eases_from_start_to_scene() {
        let from = cam(DVec3::new(100.0, 0.0, 0.0), 30.0);
        let to = cam(DVec3::new(0.0, 100.0, 0.0), 60.0);
        let mut tr = Transition::new(from.clone(), to.clone());
        // The clock starts at the first frame.
        assert_eq!(tr.at(10.0), (from.clone(), false));
        let (mid, done) = tr.at(10.0 + SCENE_TRANSITION_SECS / 2.0);
        assert!(!done);
        assert!((mid.fov_deg - 45.0).abs() < 1e-9);
        // Eased: a tenth of the time covers less than a tenth of the way.
        let (early, _) = tr.at(10.0 + SCENE_TRANSITION_SECS / 10.0);
        assert!(early.fov_deg - 30.0 < 3.0);
        assert_eq!(tr.at(10.0 + SCENE_TRANSITION_SECS), (to.clone(), true));
        assert_eq!(tr.at(100.0), (to, true));
    }
}
