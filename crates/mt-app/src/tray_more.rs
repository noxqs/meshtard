//! The app side of FEATURES.md 60's panel work: tag folders and dashes, Color by
//! Tag, material Duplicate / Area, component Select Instances / Replace Selected /
//! behaviors, Outliner rename / eye / reparent, scene details, properties to save
//! and reorder, and In Model styles. The model data is in `mt_kernel::model_data`
//! (model-only: not read from or written to .skp files).
//!
//! Undo: tag, folder, component, Outliner, scene and style list edits are one undo
//! step each; Color by Tag and applying a style or scene are display changes and not
//! undo steps (as the Styles panel's settings and tag visibility here).

use glam::DMat4;
use mt_kernel::model_data::{Dashes, NamedStyle, SceneProps, SceneState, TagFolder};
use mt_kernel::{DefinitionId, Entities, GlueTo, Instance, Model, Page};
use mt_ui::Event;

use crate::App;
use crate::document::Owner;
use crate::selection::Mode;

const GLUES: [GlueTo; 5] = [
    GlueTo::None,
    GlueTo::Any,
    GlueTo::Horizontal,
    GlueTo::Vertical,
    GlueTo::Sloped,
];

/// The index of a glue setting in `mt_ui::GLUE_NAMES`.
pub(crate) fn glue_index(g: GlueTo) -> usize {
    GLUES.iter().position(|x| *x == g).unwrap_or(0)
}

/// `SceneProps` as the panel's check boxes and back.
pub(crate) fn props_to_ui(p: SceneProps) -> [bool; 7] {
    [
        p.camera,
        p.hidden_geometry,
        p.visible_tags,
        p.section_planes,
        p.style_fog,
        p.shadows,
        p.axes,
    ]
}

pub(crate) fn props_from_ui(b: [bool; 7]) -> SceneProps {
    SceneProps {
        camera: b[0],
        hidden_geometry: b[1],
        visible_tags: b[2],
        section_planes: b[3],
        style_fog: b[4],
        shadows: b[5],
        axes: b[6],
    }
}

/// The model state a scene saves (everything; `props` says what applies).
pub(crate) fn capture_state(m: &Model) -> SceneState {
    let root = &m.root;
    SceneState {
        style: m.style.clone(),
        style_index: m.active_style,
        shadows: m.shadows.clone(),
        hidden_faces: (root.faces.iter())
            .filter(|(_, f)| f.attrs.hidden)
            .map(|(k, _)| k)
            .collect(),
        hidden_edges: (root.edges.iter())
            .filter(|(_, e)| e.attrs.hidden)
            .map(|(k, _)| k)
            .collect(),
        hidden_instances: (root.instances.iter().enumerate())
            .filter(|(_, i)| i.hidden)
            .map(|(k, _)| k)
            .collect(),
        active_section: root.active_section,
        axes: m.axes,
    }
}

/// A new scene from the current view and model.
pub(crate) fn new_page(m: &Model, name: String, camera: &mt_render::Camera) -> Page {
    let mut page = Page {
        name,
        ..Page::default()
    };
    update_page(&mut page, m, camera, SceneProps::default());
    page
}

/// Updates the properties `which` of `page` from the view and model (Update Scene).
pub(crate) fn update_page(
    page: &mut Page,
    m: &Model,
    camera: &mt_render::Camera,
    which: SceneProps,
) {
    let now = capture_state(m);
    if which.camera {
        page.eye = camera.eye;
        page.target = camera.target;
        page.fov_deg = camera.fov_deg;
        page.perspective = camera.perspective;
    }
    if which.visible_tags {
        page.hidden_layers = (0..m.layers.len())
            .filter(|&i| !m.layers[i].visible)
            .collect();
    }
    let s = &mut page.state;
    if which.style_fog {
        s.style = now.style;
        s.style_index = now.style_index;
    }
    if which.shadows {
        s.shadows = now.shadows;
    }
    if which.hidden_geometry {
        s.hidden_faces = now.hidden_faces;
        s.hidden_edges = now.hidden_edges;
        s.hidden_instances = now.hidden_instances;
    }
    if which.section_planes {
        s.active_section = now.active_section;
    }
    if which.axes {
        s.axes = now.axes;
    }
}

/// Activates `page` on the model for the properties it saves; returns the camera to
/// move to when it saves one.
pub(crate) fn apply_page(m: &mut Model, page: &Page) -> Option<mt_render::Camera> {
    let p = page.props;
    let s = &page.state;
    if p.visible_tags {
        for (k, l) in m.layers.iter_mut().enumerate() {
            l.visible = !page.hidden_layers.contains(&k);
        }
    }
    if p.style_fog {
        m.style = s.style.clone();
        m.active_style = s.style_index.filter(|&i| i < m.styles.len());
    }
    if p.shadows {
        m.shadows = s.shadows.clone();
    }
    if p.hidden_geometry {
        let root = &mut m.root;
        for (k, f) in root.faces.iter_mut() {
            f.attrs.hidden = s.hidden_faces.contains(&k);
        }
        for (k, e) in root.edges.iter_mut() {
            e.attrs.hidden = s.hidden_edges.contains(&k);
        }
        for (k, i) in root.instances.iter_mut().enumerate() {
            i.hidden = s.hidden_instances.contains(&k);
        }
    }
    if p.section_planes {
        m.root.active_section = s
            .active_section
            .filter(|&i| i < m.root.section_planes.len());
    }
    if p.axes {
        m.axes = s.axes;
    }
    p.camera.then_some(mt_render::Camera {
        eye: page.eye,
        target: page.target,
        fov_deg: page.fov_deg,
        perspective: page.perspective,
        two_point: false,
    })
}

/// The next scene the animation plays after `current` (`step` 1) or before it
/// (`step` -1), skipping scenes left out of the animation; `None` when none is in.
pub(crate) fn step_animated(pages: &[Page], current: Option<usize>, step: isize) -> Option<usize> {
    let n = pages.len();
    let mut at = current.filter(|&i| i < n);
    for _ in 0..n {
        let next = crate::animation::step_scene(at, n, step)?;
        if pages[next].include_in_animation {
            return Some(next);
        }
        at = Some(next);
    }
    None
}

/// The entities holding the instance at `path` (its parent's), and the transform from
/// them to world space.
fn parent_of<'a>(m: &'a Model, path: &[usize]) -> Option<(&'a Entities, DMat4)> {
    let (_, up) = path.split_last()?;
    let mut ents = &m.root;
    let mut xf = DMat4::IDENTITY;
    for &i in up {
        let inst = ents.instances.get(i)?;
        xf *= inst.transform;
        ents = &m.definitions.get(inst.definition)?.entities;
    }
    Some((ents, xf))
}

/// The instance at `path` (child indices from the root down).
pub(crate) fn instance_at<'a>(m: &'a Model, path: &[usize]) -> Option<&'a Instance> {
    let (ents, _) = parent_of(m, path)?;
    ents.instances.get(*path.last()?)
}

/// The instance at `path`, mutable.
pub(crate) fn instance_at_mut<'a>(m: &'a mut Model, path: &[usize]) -> Option<&'a mut Instance> {
    let (&last, up) = path.split_last()?;
    let mut def: Option<DefinitionId> = None;
    for &i in up {
        let ents = match def {
            Some(d) => &m.definitions.get(d)?.entities,
            None => &m.root,
        };
        def = Some(ents.instances.get(i)?.definition);
    }
    let ents = match def {
        Some(d) => &mut m.definitions.get_mut(d)?.entities,
        None => &mut m.root,
    };
    ents.instances.get_mut(last)
}

/// Whether definition `outer` holds `inner`, at any depth.
fn holds_definition(m: &Model, outer: DefinitionId, inner: DefinitionId) -> bool {
    let mut stack = vec![(outer, 0)];
    while let Some((d, depth)) = stack.pop() {
        if depth > 64 {
            return true;
        }
        let Some(def) = m.definitions.get(d) else {
            continue;
        };
        for i in &def.entities.instances {
            if i.definition == inner {
                return true;
            }
            stack.push((i.definition, depth + 1));
        }
    }
    false
}

/// Outliner drag and drop: moves the instance at `from` into the group or component
/// at `to` (empty: the model root), keeping where it is in the world. A component
/// target takes it into its definition (all its instances show it). Refused when it
/// would put a definition inside itself.
pub(crate) fn reparent(m: &mut Model, from: &[usize], to: &[usize]) -> bool {
    if !mt_ui::outliner_can_move(from, to) {
        return false;
    }
    let Some((_, from_xf)) = parent_of(m, from) else {
        return false;
    };
    let Some(inst) = instance_at(m, from).cloned() else {
        return false;
    };
    let world = from_xf * inst.transform;
    let (target, to_xf) = if to.is_empty() {
        (None, DMat4::IDENTITY)
    } else {
        let Some((_, up)) = parent_of(m, to) else {
            return false;
        };
        let Some(t) = instance_at(m, to) else {
            return false;
        };
        (Some(t.definition), up * t.transform)
    };
    if let Some(td) = target
        && (td == inst.definition || holds_definition(m, inst.definition, td))
    {
        return false;
    }
    if to_xf.determinant().abs() < 1e-18 {
        return false;
    }
    // Take it out of its parent.
    let (&last, up) = from.split_last().expect("checked");
    let parent_def = if up.is_empty() {
        None
    } else {
        instance_at(m, up).map(|i| i.definition)
    };
    let parent = match parent_def {
        Some(d) => &mut m.definitions[d].entities,
        None => &mut m.root,
    };
    if last >= parent.instances.len() {
        return false;
    }
    let mut moved = parent.instances.remove(last);
    moved.transform = to_xf.inverse() * world;
    let into = match target {
        Some(d) => &mut m.definitions[d].entities,
        None => &mut m.root,
    };
    into.instances.push(moved);
    true
}

/// Area painted with material `mat` in the whole model (square inches): faces with it
/// on either side, and faces without a material inside instances painted with it.
/// Instance scaling is taken into account.
pub(crate) fn material_area(m: &Model, mat: usize) -> f64 {
    fn walk(
        m: &Model,
        ents: &Entities,
        xf: DMat4,
        inherited: Option<usize>,
        mat: usize,
        depth: usize,
    ) -> f64 {
        if depth > 32 {
            return 0.0;
        }
        let mut sum = 0.0;
        for (f, face) in &ents.faces {
            let front = face.attrs.material.or(inherited);
            let back = face.attrs.back_material.or(inherited);
            if front == Some(mat) || back == Some(mat) {
                sum += mt_kernel::tessellate::triangulate(ents, f)
                    .iter()
                    .map(|t| {
                        let [a, b, c] = t.map(|p| xf.transform_point3(p));
                        (b - a).cross(c - a).length() / 2.0
                    })
                    .sum::<f64>();
            }
        }
        for i in &ents.instances {
            if let Some(d) = m.definitions.get(i.definition) {
                let inh = i.material.or(inherited);
                sum += walk(m, &d.entities, xf * i.transform, inh, mat, depth + 1);
            }
        }
        sum
    }
    walk(m, &m.root, DMat4::IDENTITY, None, mat, 0)
}

/// A name not in `taken`: `base`, else `base1`, `base2`... (SketchUp's numbering of
/// duplicates is unverified).
fn unique_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    (1..)
        .map(|n| format!("{base}{n}"))
        .find(|n| !taken(n))
        .expect("unbounded")
}

impl App {
    /// The panel events of this module. Returns whether the scene must be rebuilt.
    pub(crate) fn tray_more_event(&mut self, event: Event) -> bool {
        let camera = self.camera.clone();
        let selection = self.selection.items().to_vec();
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        match event {
            Event::TagFolderAdd { parent, tag } => doc.edit_model(|m, _| {
                let name = unique_name("Folder", |n| m.tag_folders.iter().any(|f| f.name == n));
                m.tag_folders.push(TagFolder {
                    name,
                    parent: parent.filter(|&p| p < m.tag_folders.len()),
                    visible: true,
                });
                let f = m.tag_folders.len() - 1;
                if let Some(l) = tag.filter(|&t| t > 0).and_then(|t| m.layers.get_mut(t)) {
                    l.folder = Some(f);
                }
                true
            }),
            Event::TagFolderDelete(f) => doc.edit_model(|m, _| m.remove_tag_folder(f)),
            Event::TagFolderRename(f, name) => doc.edit_model(|m, _| {
                m.tag_folders.get_mut(f).is_some_and(|x| {
                    let changed = x.name != name;
                    x.name = name;
                    changed
                })
            }),
            Event::TagFolderVisible(f, v) => doc.edit_model(|m, _| {
                m.tag_folders.get_mut(f).is_some_and(|x| {
                    let changed = x.visible != v;
                    x.visible = v;
                    changed
                })
            }),
            Event::TagMove { tag, folder } => doc.edit_model(|m, _| {
                let folder = folder.filter(|&f| f < m.tag_folders.len());
                match m.layers.get_mut(tag).filter(|_| tag > 0) {
                    Some(l) if l.folder != folder => {
                        l.folder = folder;
                        true
                    }
                    _ => false,
                }
            }),
            Event::TagDashes(i, k) => doc.edit_model(|m, _| {
                let d = Dashes::ALL.get(k).copied().unwrap_or_default();
                m.layers.get_mut(i).is_some_and(|l| {
                    let changed = l.dashes != d;
                    l.dashes = d;
                    changed
                })
            }),
            Event::TagPurge => {
                let mut n = 0;
                let changed = doc.edit_whole(|m| {
                    n = mt_kernel::purge::purge_tags(m);
                    n > 0
                });
                self.message = Some(format!("Purged {n} unused tags."));
                if changed {
                    self.active_tag = 0;
                }
                changed
            }
            Event::TagSelectAll(t) => {
                let layer = (t > 0).then_some(t);
                let ents = doc.active();
                let mut items: Vec<Owner> = Vec::new();
                items.extend(
                    (ents.faces.iter())
                        .filter(|(_, f)| f.attrs.layer == layer)
                        .map(|(k, _)| Owner::Face(k)),
                );
                items.extend(
                    (ents.edges.iter())
                        .filter(|(_, e)| e.attrs.layer == layer)
                        .map(|(k, _)| Owner::Edge(k)),
                );
                items.extend(
                    (ents.instances.iter().enumerate())
                        .filter(|(_, i)| i.layer == layer)
                        .map(|(k, _)| Owner::Instance(k)),
                );
                self.selection.apply(Mode::Replace, items);
                true
            }
            Event::ColorByTag(on) => {
                doc.model.color_by_tag = on;
                true
            }
            Event::MaterialDuplicate(i) => {
                let changed = doc.edit_model(|m, _| {
                    let Some(mut copy) = m.materials.get(i).cloned() else {
                        return false;
                    };
                    copy.name =
                        unique_name(&copy.name, |n| m.materials.iter().any(|x| x.name == n));
                    m.materials.push(copy);
                    true
                });
                if changed {
                    self.current_material = Some(doc.model.materials.len() - 1);
                }
                changed
            }
            Event::MaterialArea(i) => {
                if let Some(mat) = doc.model.materials.get(i) {
                    let area = material_area(&doc.model, i);
                    self.message = Some(format!(
                        "Area of {}: {}",
                        mat.name,
                        crate::annot::area_label(area, doc.units)
                    ));
                }
                false
            }
            Event::ComponentSelectInstances(i) => {
                let Some(&d) = crate::tray::component_definitions(&doc.model).get(i) else {
                    return false;
                };
                let items: Vec<Owner> = (doc.active().instances.iter().enumerate())
                    .filter(|(_, x)| x.definition == d)
                    .map(|(k, _)| Owner::Instance(k))
                    .collect();
                if items.is_empty() {
                    self.message = Some("No instances of it in the open context.".into());
                }
                self.selection.apply(Mode::Replace, items);
                true
            }
            Event::ComponentReplaceSelected(i) => {
                let Some(&d) = crate::tray::component_definitions(&doc.model).get(i) else {
                    return false;
                };
                if doc.in_context_path(d) {
                    self.message = Some("A component cannot be placed inside itself.".into());
                    return false;
                }
                let targets: Vec<usize> = selection
                    .iter()
                    .filter_map(|o| match o {
                        Owner::Instance(k) => Some(*k),
                        _ => None,
                    })
                    .collect();
                doc.edit(|ents| {
                    let mut any = false;
                    for k in &targets {
                        if let Some(inst) = ents.instances.get_mut(*k)
                            && !inst.locked
                            && inst.definition != d
                        {
                            inst.definition = d;
                            any = true;
                        }
                    }
                    any
                })
            }
            Event::ComponentBehavior { index, behavior } => {
                let Some(&d) = crate::tray::component_definitions(&doc.model).get(index) else {
                    return false;
                };
                doc.edit_model(|m, _| {
                    let def = &mut m.definitions[d];
                    let before = (
                        def.glue,
                        def.cut_opening,
                        def.always_face_camera,
                        def.shadows_face_sun,
                    );
                    def.glue = GLUES.get(behavior.glue).copied().unwrap_or_default();
                    def.cut_opening = behavior.cut_opening;
                    def.always_face_camera = behavior.always_face_camera;
                    def.shadows_face_sun = behavior.shadows_face_sun;
                    before
                        != (
                            def.glue,
                            def.cut_opening,
                            def.always_face_camera,
                            def.shadows_face_sun,
                        )
                })
            }
            Event::OutlinerRename { path, name } => doc.edit_whole(|m| {
                instance_at_mut(m, &path).is_some_and(|i| {
                    let changed = i.name != name;
                    i.name = name;
                    changed
                })
            }),
            Event::OutlinerVisible { path, visible } => doc.edit_whole(|m| {
                instance_at_mut(m, &path).is_some_and(|i| {
                    let changed = i.hidden == visible;
                    i.hidden = !visible;
                    changed
                })
            }),
            Event::OutlinerMove { from, to } => {
                let moved = doc.edit_whole(|m| reparent(m, &from, &to));
                if moved {
                    // Paths changed: close any open group or component.
                    doc.context.clear();
                    self.selection.clear();
                } else {
                    self.message = Some("It cannot be moved there.".into());
                }
                moved
            }
            Event::SceneDetailsEdit {
                index,
                name,
                description,
                include_in_animation,
            } => doc.edit_model(|m, _| {
                m.pages.get_mut(index).is_some_and(|p| {
                    let changed = p.name != name
                        || p.description != description
                        || p.include_in_animation != include_in_animation;
                    p.name = name;
                    p.description = description;
                    p.include_in_animation = include_in_animation;
                    changed
                })
            }),
            Event::SceneProps { index, props } => doc.edit_model(|m, _| {
                let props = props_from_ui(props);
                m.pages.get_mut(index).is_some_and(|p| {
                    let changed = p.props != props;
                    p.props = props;
                    changed
                })
            }),
            Event::SceneUpdateWith { index, props } => doc.edit_model(|m, _| {
                let Some(mut page) = m.pages.get(index).cloned() else {
                    return false;
                };
                update_page(&mut page, m, &camera, props_from_ui(props));
                m.pages[index] = page;
                true
            }),
            Event::SceneMove { index, up } => {
                let n = doc.model.pages.len();
                let other = if up {
                    index.checked_sub(1)
                } else {
                    Some(index + 1).filter(|&j| j < n)
                };
                let Some(j) = other.filter(|_| index < n) else {
                    return false;
                };
                doc.edit_model(|m, _| {
                    m.pages.swap(index, j);
                    true
                });
                if self.active_scene == Some(index) {
                    self.active_scene = Some(j);
                } else if self.active_scene == Some(j) {
                    self.active_scene = Some(index);
                }
                false
            }
            Event::StyleCreate => {
                doc.edit_model(|m, _| {
                    let n = (1..)
                        .map(|k| format!("Style {k}"))
                        .find(|n| !m.styles.iter().any(|s| &s.name == n))
                        .expect("unbounded");
                    m.styles.push(NamedStyle {
                        name: n,
                        description: String::new(),
                        settings: m.style.clone(),
                    });
                    m.active_style = Some(m.styles.len() - 1);
                    true
                });
                false
            }
            Event::StyleUpdate(i) => doc.edit_model(|m, _| {
                let now = m.style.clone();
                m.styles.get_mut(i).is_some_and(|s| {
                    let changed = s.settings != now;
                    s.settings = now;
                    changed
                })
            }),
            Event::StyleDelete(i) => doc.edit_model(|m, _| {
                if i >= m.styles.len() {
                    return false;
                }
                m.styles.remove(i);
                let fix = |x: Option<usize>| match x {
                    Some(k) if k == i => None,
                    Some(k) if k > i => Some(k - 1),
                    other => other,
                };
                m.active_style = fix(m.active_style);
                for p in &mut m.pages {
                    p.state.style_index = fix(p.state.style_index);
                }
                true
            }),
            Event::StylePurge => {
                let mut purged = 0;
                doc.edit_model(|m, _| {
                    let used = |k: usize| {
                        m.active_style == Some(k)
                            || m.pages.iter().any(|p| p.state.style_index == Some(k))
                    };
                    let keep: Vec<bool> = (0..m.styles.len()).map(used).collect();
                    let map: Vec<Option<usize>> = keep
                        .iter()
                        .scan(0, |next, &k| {
                            Some(k.then(|| {
                                *next += 1;
                                *next - 1
                            }))
                        })
                        .collect();
                    purged = keep.iter().filter(|k| !**k).count();
                    let mut k = 0;
                    m.styles.retain(|_| {
                        k += 1;
                        keep[k - 1]
                    });
                    let fix = |x: Option<usize>| x.and_then(|i| map.get(i).copied().flatten());
                    m.active_style = fix(m.active_style);
                    for p in &mut m.pages {
                        p.state.style_index = fix(p.state.style_index);
                    }
                    purged > 0
                });
                self.message = Some(format!("Purged {purged} unused styles."));
                false
            }
            Event::StyleApply(i) => {
                let Some(s) = doc.model.styles.get(i).map(|s| s.settings.clone()) else {
                    return false;
                };
                // A display change like the Styles panel's settings: not an undo step.
                doc.model.style = s;
                doc.model.active_style = Some(i);
                true
            }
            _ => false,
        }
    }

    /// Scenes panel Add: a scene of the current view and model, made active.
    pub(crate) fn scene_add(&mut self) -> bool {
        let camera = self.camera.clone();
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        let n = doc.model.pages.len() + 1;
        doc.edit_model(|m, _| {
            let page = new_page(m, format!("Scene {n}"), &camera);
            m.pages.push(page);
            true
        });
        self.active_scene = Some(doc.model.pages.len() - 1);
        false
    }

    /// Update Scene without the dialog (View > Animation > Update Scene): updates the
    /// properties the scene saves.
    pub(crate) fn scene_update(&mut self, i: usize) -> bool {
        let camera = self.camera.clone();
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        doc.edit_model(|m, _| {
            let Some(mut page) = m.pages.get(i).cloned() else {
                return false;
            };
            let props = page.props;
            update_page(&mut page, m, &camera, props);
            m.pages[i] = page;
            true
        })
    }

    /// Activates scene `i`: the camera moves there over the transition time; the rest
    /// of what it saves applies at once.
    pub(crate) fn scene_activate(&mut self, i: usize) -> bool {
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        let Some(page) = doc.model.pages.get(i).cloned() else {
            return false;
        };
        self.active_scene = Some(i);
        if let Some(to) = apply_page(&mut doc.model, &page) {
            self.transition = Some(crate::tray::Transition::with_secs(
                self.camera.clone(),
                to,
                self.animation.transition_secs(),
            ));
        }
        self.apply_shadows();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;
    use mt_kernel::{Definition, add_line};

    fn instance(definition: DefinitionId, at: DVec3) -> Instance {
        Instance {
            definition,
            transform: DMat4::from_translation(at),
            name: String::new(),
            hidden: false,
            locked: false,
            layer: None,
            material: None,
            attrs: Default::default(),
        }
    }

    /// Root: group A at (10, 0, 0) rotated a quarter turn, holding nothing; group B
    /// at (0, 5, 0) holding group C at (1, 1, 1) inside it.
    fn model() -> (Model, [DefinitionId; 3]) {
        let mut m = Model::default();
        let mut def = |name: &str| {
            let mut ents = Entities::new();
            add_line(&mut ents, DVec3::ZERO, DVec3::X);
            m.definitions.insert(Definition {
                name: name.into(),
                is_group: true,
                entities: ents,
                ..Default::default()
            })
        };
        let (a, b, c) = (def("A"), def("B"), def("C"));
        let mut ia = instance(a, DVec3::new(10.0, 0.0, 0.0));
        ia.transform *= DMat4::from_rotation_z(std::f64::consts::FRAC_PI_2);
        m.root.instances.push(ia);
        m.root
            .instances
            .push(instance(b, DVec3::new(0.0, 5.0, 0.0)));
        m.definitions[b]
            .entities
            .instances
            .push(instance(c, DVec3::new(1.0, 1.0, 1.0)));
        (m, [a, b, c])
    }

    fn world_of(m: &Model, path: &[usize]) -> DMat4 {
        let (_, xf) = parent_of(m, path).unwrap();
        xf * instance_at(m, path).unwrap().transform
    }

    #[test]
    fn reparent_keeps_world_position() {
        let (mut m, [a, _, c]) = model();
        let before = world_of(&m, &[1, 0]);
        // C (inside B) into A.
        assert!(reparent(&mut m, &[1, 0], &[0]));
        assert!(
            m.definitions
                .values()
                .any(|d| d.entities.instances.is_empty())
        );
        assert_eq!(m.definitions[a].entities.instances[0].definition, c);
        let after = world_of(&m, &[0, 0]);
        assert!(before.abs_diff_eq(after, 1e-9), "{before:?} {after:?}");
        // Back to the root.
        assert!(reparent(&mut m, &[0, 0], &[]));
        assert_eq!(m.root.instances.len(), 3);
        assert!(before.abs_diff_eq(world_of(&m, &[2]), 1e-9));
    }

    #[test]
    fn reparent_refuses_loops() {
        let (mut m, _) = model();
        assert!(!reparent(&mut m, &[1], &[1, 0]), "B into its own child C");
        assert!(!reparent(&mut m, &[1], &[1]), "into itself");
        assert!(!reparent(&mut m, &[0], &[]), "already at the root");
        // A second instance of B: moving B into it would put B inside B.
        let b = m.root.instances[1].definition;
        m.root.instances.push(instance(b, DVec3::ZERO));
        assert!(!reparent(&mut m, &[1], &[2]));
        assert!(!reparent(&mut m, &[5], &[]), "missing path");
    }

    #[test]
    fn scene_properties_saved_and_applied() {
        let mut m = Model::default();
        add_line(&mut m.root, DVec3::ZERO, DVec3::X);
        m.layers.push(mt_kernel::Layer {
            name: "T".into(),
            ..Default::default()
        });
        let cam = mt_render::Camera {
            eye: DVec3::new(1.0, 2.0, 3.0),
            target: DVec3::ZERO,
            fov_deg: 40.0,
            perspective: true,
            two_point: false,
        };
        m.shadows.enabled = true;
        m.style.xray = true;
        m.layers[1].visible = false;
        let e = m.root.edges.keys().next().unwrap();
        m.root.edges[e].attrs.hidden = true;
        let mut page = new_page(&m, "S".into(), &cam);
        assert_eq!(page.hidden_layers, [1]);
        // Change everything, then activate with only some properties saved.
        m.shadows.enabled = false;
        m.style.xray = false;
        m.layers[1].visible = true;
        m.root.edges[e].attrs.hidden = false;
        page.props = SceneProps {
            camera: false,
            style_fog: false,
            ..SceneProps::default()
        };
        assert_eq!(apply_page(&mut m, &page), None, "camera not saved");
        assert!(m.shadows.enabled, "shadows restored");
        assert!(!m.layers[1].visible, "tag visibility restored");
        assert!(m.root.edges[e].attrs.hidden, "hidden geometry restored");
        assert!(!m.style.xray, "style not saved: unchanged");
        page.props = SceneProps::default();
        let to = apply_page(&mut m, &page).expect("camera saved");
        assert_eq!((to.eye, to.fov_deg), (cam.eye, 40.0));
        assert!(m.style.xray);
        // Update with only the camera: the rest keeps what was saved.
        let cam2 = mt_render::Camera {
            eye: DVec3::new(9.0, 9.0, 9.0),
            ..cam
        };
        m.shadows.enabled = false;
        let only_camera = SceneProps {
            camera: true,
            hidden_geometry: false,
            visible_tags: false,
            section_planes: false,
            style_fog: false,
            shadows: false,
            axes: false,
        };
        update_page(&mut page, &m, &cam2, only_camera);
        assert_eq!(page.eye, cam2.eye);
        assert!(page.state.shadows.enabled);
        assert_eq!(props_from_ui(props_to_ui(only_camera)), only_camera);
    }

    #[test]
    fn animation_skips_excluded_scenes() {
        let page = |include| Page {
            include_in_animation: include,
            ..Page::default()
        };
        let pages = [page(true), page(false), page(true)];
        assert_eq!(step_animated(&pages, Some(0), 1), Some(2));
        assert_eq!(step_animated(&pages, Some(2), 1), Some(0));
        assert_eq!(step_animated(&pages, None, 1), Some(0));
        assert_eq!(step_animated(&pages, Some(0), -1), Some(2));
        assert_eq!(step_animated(&[page(false)], None, 1), None);
    }

    #[test]
    fn material_area_counts_painted_faces_and_instances() {
        let mut m = Model::default();
        m.materials.push(mt_kernel::Material {
            name: "Red".into(),
            color: [255, 0, 0],
            opacity: 1.0,
            texture: None,
        });
        // A 10 x 10 square painted at the root.
        let sq = |e: &mut Entities, s: f64| {
            let p = |x: f64, y: f64| DVec3::new(x * s, y * s, 0.0);
            for (a, b) in [
                (p(0.0, 0.0), p(1.0, 0.0)),
                (p(1.0, 0.0), p(1.0, 1.0)),
                (p(1.0, 1.0), p(0.0, 1.0)),
                (p(0.0, 1.0), p(0.0, 0.0)),
            ] {
                add_line(e, a, b);
            }
        };
        sq(&mut m.root, 10.0);
        let f = m.root.faces.keys().next().unwrap();
        m.root.faces[f].attrs.material = Some(0);
        // A 1 x 1 square in a group scaled 2x and painted: 4 more.
        let mut ents = Entities::new();
        sq(&mut ents, 1.0);
        let d = m.definitions.insert(Definition {
            entities: ents,
            ..Default::default()
        });
        let mut inst = instance(d, DVec3::new(50.0, 0.0, 0.0));
        inst.transform *= DMat4::from_scale(DVec3::splat(2.0));
        inst.material = Some(0);
        m.root.instances.push(inst);
        assert!((material_area(&m, 0) - 104.0).abs() < 1e-9);
        assert_eq!(material_area(&m, 1), 0.0);
    }
}
