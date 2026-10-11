//! Click tools that act on whole entities picked in the view: the Solid Tools (click
//! the first solid, then the second) and the Tag tool (click entities to move them to
//! the active tag; Alt+click samples an entity's tag, Shift+click replaces the tag of
//! every entity tagged like the clicked one, Ctrl+click tags every instance of the
//! clicked component; help page "Controlling visibility with tags" [doc]).
//!
//! Unverified against SketchUp 2026: the cursor feedback (a 1 / 2 badge for the pick
//! and a circle-slash badge over non-solids, see `cursor.rs`, plus a label by the
//! cursor that SketchUp may not show), Alt to sample (from memory of the 2025 Tag
//! tool), how far Replace All reaches (here the open context), and that a click on a
//! selected entity tags the whole selection (as the Paint Bucket does).

use eframe::egui;
use egui_wgpu::wgpu;
use glam::DVec2;
use mt_kernel::Entities;

use crate::App;
use crate::document::Owner;
use crate::nav::Tool;
use crate::selection::{self, Mode};
use crate::solid_tools::{self, SolidOp, SolidPick};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickKind {
    Solid(SolidOp),
    Tag,
}

/// The active pick tool's state.
#[derive(Debug, Clone)]
pub struct PickTool {
    pub kind: PickKind,
    /// Solid Tools: the first solid picked (instance index of the open context).
    pub first: Option<usize>,
    /// Label drawn by the cursor and whether it is a valid target.
    hover: Option<(egui::Pos2, String, bool)>,
}

impl PickTool {
    pub fn for_command(id: &str) -> Option<Self> {
        let kind = match id {
            "tool.tag" => PickKind::Tag,
            _ => PickKind::Solid(SolidOp::from_command(id)?),
        };
        Some(Self {
            kind,
            first: None,
            hover: None,
        })
    }

    /// The entity under the cursor is not a valid target (Solid Tools over a
    /// non-solid or a locked instance).
    pub fn hover_invalid(&self) -> bool {
        self.hover.as_ref().is_some_and(|h| !h.2)
    }
}

/// Moves `items` to tag `layer` (`None`: Untagged). Returns whether any changed.
pub fn tag_entities(ents: &mut Entities, items: &[Owner], layer: Option<usize>) -> bool {
    let mut changed = false;
    let mut set = |l: &mut Option<usize>| {
        changed |= *l != layer;
        *l = layer;
    };
    for o in items {
        match *o {
            Owner::Edge(e) => {
                if let Some(x) = ents.edges.get_mut(e) {
                    set(&mut x.attrs.layer);
                }
            }
            Owner::Face(f) => {
                if let Some(x) = ents.faces.get_mut(f) {
                    set(&mut x.attrs.layer);
                }
            }
            // Locked groups and components keep their tag (unverified).
            Owner::Instance(i) => {
                if let Some(x) = ents.instances.get_mut(i).filter(|x| !x.locked) {
                    set(&mut x.layer);
                }
            }
            Owner::Section(_) | Owner::Dimension(_) | Owner::Text(_) => {}
        }
    }
    changed
}

/// Tag tool with Ctrl: every unlocked instance of `def` in the model gets `layer`.
pub fn tag_all_instances(
    model: &mut mt_kernel::Model,
    def: mt_kernel::DefinitionId,
    layer: Option<usize>,
) -> bool {
    let mut changed = false;
    let mut fix = |ents: &mut Entities| {
        for i in &mut ents.instances {
            if i.definition == def && !i.locked && i.layer != layer {
                i.layer = layer;
                changed = true;
            }
        }
    };
    fix(&mut model.root);
    for d in model.definitions.values_mut() {
        fix(&mut d.entities);
    }
    changed
}

/// The tag of `o` (0 for Untagged), for sampling.
pub fn tag_of(ents: &Entities, o: Owner) -> Option<usize> {
    match o {
        Owner::Edge(e) => ents.edges.get(e).map(|x| x.attrs.layer),
        Owner::Face(f) => ents.faces.get(f).map(|x| x.attrs.layer),
        Owner::Instance(i) => ents.instances.get(i).map(|x| x.layer),
        Owner::Section(_) | Owner::Dimension(_) | Owner::Text(_) => None,
    }
    .map(|l| l.unwrap_or(0))
}

impl App {
    /// A tool command was chosen: sets up the pick tool for it, if it is one. Outer
    /// Shell with two or more solids selected runs at once and returns to Select.
    pub(crate) fn start_pick_tool(&mut self, frame: &eframe::Frame, id: &str) {
        self.pick = PickTool::for_command(id);
        // A message from the previous tool does not outlive it.
        self.message = None;
        if id != "tool.outer_shell" {
            return;
        }
        let Some(doc) = self.document.as_mut() else {
            return;
        };
        let solids = solid_tools::solid_instances(doc, self.selection.items());
        if solids.len() < 2 {
            return;
        }
        match solid_tools::run(doc, SolidOp::OuterShell, &solids) {
            Ok(made) => {
                self.selection.clear();
                self.selection
                    .apply(Mode::Replace, made.into_iter().map(Owner::Instance));
                self.message = None;
            }
            Err(e) => self.message = Some(e),
        }
        self.pick = None;
        self.tool = Tool::Select;
        if let Some(rs) = frame.wgpu_render_state() {
            let device = rs.device.clone();
            self.rebuild(&device);
        }
    }

    /// Status bar text of the active pick tool.
    pub(crate) fn pick_status(&self) -> String {
        let Some(t) = &self.pick else {
            return String::new();
        };
        match t.kind {
            PickKind::Solid(op) => op.status(t.first.is_none()),
            PickKind::Tag => {
                let name = self
                    .document
                    .as_ref()
                    .and_then(|d| d.model.layers.get(self.active_tag))
                    .map_or("Untagged".to_owned(), |l| {
                        if self.active_tag == 0 {
                            "Untagged".to_owned()
                        } else {
                            l.name.clone()
                        }
                    });
                format!(
                    "Click entities to move them to tag \"{name}\". Alt = sample, Shift = replace all with the clicked tag, Ctrl = all instances."
                )
            }
        }
    }

    /// Mouse and keys of the pick tools.
    pub(crate) fn pick_tool(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        viewport_px: DVec2,
        device: &wgpu::Device,
    ) {
        self.inference = None;
        let Some((kind, first)) = self.pick.as_ref().map(|t| (t.kind, t.first)) else {
            return;
        };
        let ppp = ui.ctx().pixels_per_point();
        let to_px = |p: egui::Pos2| {
            DVec2::new(
                f64::from((p.x - rect.min.x) * ppp),
                f64::from((p.y - rect.min.y) * ppp),
            )
        };
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) && first.is_some() {
            if let Some(t) = self.pick.as_mut() {
                t.first = None;
            }
            self.select(device, Mode::Replace, Vec::new());
        }
        let pos = response.hover_pos();
        let hit = pos.and_then(|p| {
            selection::pick(
                &self.scene,
                self.bvh.as_ref(),
                &self.camera,
                to_px(p),
                viewport_px,
            )
        });
        let clicked = response.clicked_by(egui::PointerButton::Primary);
        let (alt, shift, ctrl) =
            ui.input(|i| (i.modifiers.alt, i.modifiers.shift, i.modifiers.command));
        let Some(doc) = self.document.as_mut() else {
            return;
        };
        match kind {
            PickKind::Solid(op) => {
                let class = hit.map(|o| match o {
                    Owner::Instance(i) => solid_tools::classify(doc, i),
                    _ => SolidPick::NotSolid,
                });
                let hover = match (pos, class) {
                    (Some(p), Some(c)) => {
                        let ok = matches!(c, SolidPick::Solid(_));
                        Some((p, c.label(doc), ok))
                    }
                    _ => None,
                };
                if let Some(t) = self.pick.as_mut() {
                    t.hover = hover;
                }
                if !clicked {
                    return;
                }
                match (class, first) {
                    (Some(SolidPick::Solid(i)), None) => {
                        if let Some(t) = self.pick.as_mut() {
                            t.first = Some(i);
                        }
                        self.message = None;
                        // The first solid stays highlighted.
                        self.select(device, Mode::Replace, vec![Owner::Instance(i)]);
                    }
                    (Some(SolidPick::Solid(i)), Some(a)) if i != a => {
                        if let Some(t) = self.pick.as_mut() {
                            t.first = None;
                        }
                        match solid_tools::run(doc, op, &[a, i]) {
                            Ok(_) => self.message = None,
                            Err(e) => self.message = Some(e),
                        }
                        self.selection.clear();
                        self.rebuild(device);
                    }
                    (Some(SolidPick::NotSolid), _) => {
                        self.message =
                            Some(format!("{}: not a solid group or component.", op.name()));
                    }
                    (Some(SolidPick::Locked), _) => {
                        self.message = Some(format!(
                            "{}: a locked group or component cannot be used.",
                            op.name()
                        ));
                    }
                    _ => {}
                }
            }
            PickKind::Tag => {
                if let Some(t) = self.pick.as_mut() {
                    t.hover = None;
                }
                let Some(o) = hit.filter(|_| clicked) else {
                    return;
                };
                if alt {
                    if let Some(tag) = tag_of(doc.active(), o) {
                        self.active_tag = tag;
                    }
                    return;
                }
                let layer = (self.active_tag > 0).then_some(self.active_tag);
                // Ctrl: every instance of the clicked component, anywhere [doc].
                if ctrl && let Owner::Instance(i) = o {
                    let def = doc.active().instances.get(i).map(|x| x.definition);
                    if let Some(def) = def
                        && doc.edit_whole(|m| tag_all_instances(m, def, layer))
                    {
                        self.message = None;
                        self.rebuild(device);
                    }
                    return;
                }
                let items = if shift {
                    // Shift: Replace All, every entity with the clicked one's tag [doc]
                    // (in the open context; unverified whether SketchUp goes deeper).
                    crate::context_menu::same(doc.active(), &[o], false)
                } else if self.selection.contains(&o) {
                    self.selection.items().to_vec()
                } else {
                    vec![o]
                };
                if doc.edit(|ents| tag_entities(ents, &items, layer)) {
                    self.message = None;
                    self.rebuild(device);
                }
            }
        }
    }

    /// The pick tool's label by the cursor.
    pub(crate) fn pick_overlay(&self, ui: &egui::Ui, rect: egui::Rect) {
        if !matches!(self.tool, Tool::Pick(_)) {
            return;
        }
        let Some((p, text, ok)) = self.pick.as_ref().and_then(|t| t.hover.clone()) else {
            return;
        };
        let color = if ok {
            egui::Color32::from_rgb(0, 90, 0)
        } else {
            egui::Color32::from_rgb(200, 0, 0)
        };
        let painter = ui.painter().with_clip_rect(rect);
        let font = egui::FontId::proportional(12.0);
        let at = p + egui::vec2(16.0, 18.0);
        let galley = painter.layout_no_wrap(text, font, color);
        let bg = egui::Rect::from_min_size(at, galley.size()).expand(2.0);
        painter.rect_filled(bg, 2.0, egui::Color32::from_white_alpha(220));
        painter.galley(at, galley, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    #[test]
    fn tag_tool_tags_and_samples() {
        let mut doc = crate::document::Document::untitled();
        crate::solid_tools::tests::add_box(&mut doc, DVec3::ZERO, DVec3::splat(10.0));
        doc.edit(|ents| !mt_kernel::add_line(ents, DVec3::X * 20.0, DVec3::X * 30.0).is_empty());
        let e = doc.model.root.edges.keys().next().unwrap();
        let items = [Owner::Edge(e), Owner::Instance(0)];
        assert!(doc.edit(|ents| tag_entities(ents, &items, Some(1))));
        assert_eq!(tag_of(doc.active(), Owner::Edge(e)), Some(1));
        assert_eq!(tag_of(doc.active(), Owner::Instance(0)), Some(1));
        assert!(
            !doc.edit(|ents| tag_entities(ents, &items, Some(1))),
            "no change"
        );
        doc.lock(&[Owner::Instance(0)], true);
        assert!(doc.edit(|ents| tag_entities(ents, &items, None)));
        assert_eq!(tag_of(doc.active(), Owner::Edge(e)), Some(0));
        assert_eq!(tag_of(doc.active(), Owner::Instance(0)), Some(1), "locked");
        assert!(doc.undo());
        assert_eq!(tag_of(doc.active(), Owner::Edge(e)), Some(1));
    }

    #[test]
    fn commands_make_pick_tools() {
        assert_eq!(
            PickTool::for_command("tool.solid_trim").map(|t| t.kind),
            Some(PickKind::Solid(SolidOp::Trim))
        );
        assert_eq!(
            PickTool::for_command("tool.tag").map(|t| t.kind),
            Some(PickKind::Tag)
        );
        assert!(PickTool::for_command("tool.line").is_none());
        for id in [
            "tool.tag",
            "tool.outer_shell",
            "tool.solid_union",
            "tool.solid_intersect",
            "tool.solid_subtract",
            "tool.solid_trim",
            "tool.solid_split",
        ] {
            assert!(matches!(Tool::from_command(id), Some(Tool::Pick(x)) if x == id));
            assert!(crate::IMPLEMENTED.contains(&id), "{id}");
        }
    }
}
