//! The Default Tray's model panels: Tags, Materials, Outliner, Scenes, Styles and
//! Shadows. Builds `mt_ui::TrayData` from the document and applies the panels' events.

use mt_kernel::{Entities, Layer, Material, Model, Page};
use mt_ui::{Event, MaterialRow, OutlinerKind, OutlinerNode, SelectMode, TagRow, TrayData};

use crate::App;
use crate::document::Owner;
use crate::selection::Mode;

impl App {
    /// What the tray panels show this frame.
    pub(crate) fn tray_data(&self) -> TrayData {
        let mut data = TrayData {
            active_tag: self.active_tag,
            current_material: self.current_material,
            active_scene: self.active_scene,
            scene_tabs: self.scene_tabs,
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
            })
            .collect();
        data.materials = m
            .materials
            .iter()
            .map(|x| MaterialRow {
                name: x.name.clone(),
                color: x.color,
                opacity: (x.opacity * 100.0).round().clamp(0.0, 100.0) as u8,
            })
            .collect();
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
        data.outliner = outliner(m, &m.root, &selected, 0);
        data.scenes = m.pages.iter().map(|p| p.name.clone()).collect();
        data.style = mt_ui::StyleSettings {
            edges: m.style.edges,
            profiles: m.style.profiles,
            profile_width: m.style.profile_width.round() as u8,
            section_cut_width: m.style.section_cut_width.round() as u8,
            face_style: match m.style.face_style {
                mt_kernel::FaceStyle::Wireframe => mt_ui::FaceStyle::Wireframe,
                mt_kernel::FaceStyle::HiddenLine => mt_ui::FaceStyle::HiddenLine,
                mt_kernel::FaceStyle::Shaded => mt_ui::FaceStyle::Shaded,
                mt_kernel::FaceStyle::ShadedWithTextures => mt_ui::FaceStyle::ShadedWithTextures,
                mt_kernel::FaceStyle::Monochrome => mt_ui::FaceStyle::Monochrome,
            },
            x_ray: m.style.xray,
        };
        let layer_of = |o: &Owner| -> Option<usize> {
            let ents = doc.active();
            match *o {
                Owner::Edge(e) => ents.edges.get(e).map(|x| x.attrs.layer),
                Owner::Face(f) => ents.faces.get(f).map(|x| x.attrs.layer),
                Owner::Instance(i) => ents.instances.get(i).map(|x| x.layer),
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
        };
        data
    }

    /// Applies a tray event. Returns whether the scene must be rebuilt.
    pub(crate) fn tray_event(&mut self, event: Event) -> bool {
        let camera = self.camera.clone();
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
                });
                true
            }),
            Event::TagDelete(i) if i > 0 => {
                if self.active_tag == i {
                    self.active_tag = 0;
                }
                doc.edit_model(|m, ents| {
                    if i >= m.layers.len() {
                        return false;
                    }
                    m.layers.remove(i);
                    // Entities on the deleted tag go to Untagged (SketchUp asks; this is
                    // its default choice, unverified).
                    let remap = |l: &mut Option<usize>| {
                        *l = match *l {
                            Some(x) if x == i => None,
                            Some(x) if x > i => Some(x - 1),
                            other => other,
                        }
                    };
                    for e in std::iter::once(ents)
                        .chain(m.definitions.values_mut().map(|d| &mut d.entities))
                        .chain(std::iter::once(&mut m.root))
                    {
                        for x in e.edges.values_mut() {
                            remap(&mut x.attrs.layer);
                        }
                        for x in e.faces.values_mut() {
                            remap(&mut x.attrs.layer);
                        }
                        for x in &mut e.instances {
                            remap(&mut x.layer);
                        }
                    }
                    for p in &mut m.pages {
                        p.hidden_layers.retain(|&x| x != i);
                        p.hidden_layers
                            .iter_mut()
                            .filter(|x| **x > i)
                            .for_each(|x| *x -= 1);
                    }
                    true
                })
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
                        }
                    }
                    !items.is_empty()
                })
            }
            Event::MaterialEdit {
                index,
                name,
                color,
                opacity,
            } => doc.edit_model(|m, _| {
                let Some(x) = m.materials.get_mut(index) else {
                    return false;
                };
                let new = Material {
                    name,
                    color,
                    opacity: f64::from(opacity) / 100.0,
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
            Event::SceneAdd => {
                let n = doc.model.pages.len() + 1;
                doc.edit_model(|m, _| {
                    let page = page(m, format!("Scene {n}"), &camera);
                    m.pages.push(page);
                    true
                });
                self.active_scene = Some(doc.model.pages.len() - 1);
                false
            }
            Event::SceneUpdate(i) => doc.edit_model(|m, _| {
                let Some(name) = m.pages.get(i).map(|p| p.name.clone()) else {
                    return false;
                };
                m.pages[i] = page(m, name, &camera);
                true
            }),
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
            Event::SceneActivate(i) => {
                let Some(p) = doc.model.pages.get(i).cloned() else {
                    return false;
                };
                self.active_scene = Some(i);
                // SketchUp animates the transition; this jumps.
                self.camera.eye = p.eye;
                self.camera.target = p.target;
                self.camera.fov_deg = p.fov_deg;
                self.camera.perspective = p.perspective;
                for (k, l) in doc.model.layers.iter_mut().enumerate() {
                    l.visible = !p.hidden_layers.contains(&k);
                }
                true
            }
            Event::StyleChanged(s) => {
                let st = &mut doc.model.style;
                st.edges = s.edges;
                st.profiles = s.profiles;
                st.profile_width = f32::from(s.profile_width);
                st.section_cut_width = f32::from(s.section_cut_width);
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
                true
            }
            _ => false,
        }
    }
}

/// A scene from the current view and tag visibility.
fn page(m: &Model, name: String, camera: &mt_render::Camera) -> Page {
    Page {
        name,
        eye: camera.eye,
        target: camera.target,
        fov_deg: camera.fov_deg,
        perspective: camera.perspective,
        hidden_layers: m
            .layers
            .iter()
            .enumerate()
            .filter(|(_, l)| !l.visible)
            .map(|(i, _)| i)
            .collect(),
    }
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

fn outliner(m: &Model, ents: &Entities, selected: &[usize], depth: usize) -> Vec<OutlinerNode> {
    ents.instances
        .iter()
        .enumerate()
        .map(|(i, inst)| {
            let def = m.definitions.get(inst.definition);
            let on_hidden_tag = inst
                .layer
                .is_some_and(|l| m.layers.get(l).is_some_and(|x| !x.visible));
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
                    Some(d) if depth < 32 => outliner(m, &d.entities, &[], depth + 1),
                    _ => Vec::new(),
                },
            }
        })
        .collect()
}
