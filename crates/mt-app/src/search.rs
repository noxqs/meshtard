//! Help > Search (Shift+S; FEATURES.md 63): a popup with a text field. As you type it
//! lists the matching commands of the menu bar and the right-click menus (label, menu
//! path and a few synonyms per command, so "extrude" finds Push/Pull), with their
//! shortcut, greyed when they cannot run now. Enter or a click runs the highlighted
//! one; Up and Down move; Escape closes.
//!
//! The help page (search-tools-commands-and-extensions) says Search opens from Shift+S,
//! the Help menu and the Getting Started toolbar, finds matches as you type and runs
//! what is picked, and that it understands goals ("transparent") [doc]. The popup's
//! place and look, the ranking and the synonym list are this app's own (unverified).
//! The Getting Started toolbar button is not added (it needs an icon).

use eframe::egui;

use crate::App;

/// Results shown at most.
const MAX_RESULTS: usize = 12;

/// A command Search can find.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: String,
    pub label: String,
    /// Menu path, e.g. `Edit > Entity` or `Context menu > Area`.
    pub path: String,
}

/// Words people use for what a command does (concept matching; own list).
const SYNONYMS: &[(&str, &str)] = &[
    ("tool.push_pull", "extrude pull push extend thicken"),
    ("tool.move", "translate copy shift drag"),
    ("tool.rotate", "turn spin angle"),
    ("tool.scale", "resize size stretch enlarge shrink"),
    ("tool.eraser", "delete remove rubber erase soften"),
    ("tool.offset", "inset outline border"),
    ("tool.follow_me", "sweep lathe path along revolve"),
    (
        "tool.paint_bucket",
        "paint color colour material texture fill",
    ),
    (
        "tool.tape_measure",
        "measure distance guide length ruler rescale",
    ),
    ("tool.protractor", "angle guide measure"),
    ("tool.dimensions", "measure annotate dimension"),
    ("tool.text", "label note annotate leader"),
    ("tool.line", "pencil draw edge segment"),
    ("tool.freehand", "sketch curve draw"),
    ("tool.circle", "round disc"),
    ("tool.section_plane", "cut slice section"),
    ("tool.select", "pick arrow choose"),
    ("tool.tag", "layer tag"),
    ("tool.axes", "origin axes ucs"),
    ("tool.flip", "mirror flip"),
    ("tool.solid_union", "boolean merge combine add"),
    ("tool.solid_subtract", "boolean cut minus difference"),
    ("tool.solid_intersect", "boolean common overlap"),
    ("tool.outer_shell", "boolean merge solid"),
    ("edit.make_component", "block symbol component"),
    ("edit.create_group", "group combine"),
    ("edit.hide", "invisible conceal"),
    ("edit.undo", "back revert mistake"),
    ("edit.delete", "erase remove"),
    ("view.xray", "transparent see through"),
    ("view.wireframe", "transparent lines"),
    ("view.hidden_geometry", "hidden show"),
    ("view.hidden_objects", "hidden show groups components"),
    ("view.shadows", "sun light shade"),
    ("camera.zoom_extents", "fit all everything"),
    ("camera.orbit", "rotate view turn view"),
    ("camera.pan", "slide scroll"),
    ("camera.parallel", "orthographic ortho"),
    ("window.materials", "color colour texture paint"),
    ("window.tags", "layers visibility"),
    ("window.styles", "edges look appearance"),
    ("window.model_info", "units settings statistics"),
    ("file.export_2d", "png jpg image picture screenshot"),
    ("file.export_3d", "obj stl dae fbx"),
    ("file.print", "paper printer"),
    ("edit.area_selection", "area surface square"),
    ("edit.find_center", "center middle"),
];

/// The search popup's state.
#[derive(Debug, Default)]
pub(crate) struct Search {
    pub query: String,
    pub selected: usize,
    focused: bool,
}

/// The commands of the menu bar (with their paths) and the right-click menus.
pub fn entries(spec: &mt_spec::Spec) -> Vec<Entry> {
    fn walk(items: &[mt_spec::MenuItem], path: &str, out: &mut Vec<Entry>) {
        for i in items {
            match i {
                mt_spec::MenuItem::Command { id, label } => {
                    if !out.iter().any(|e| &e.id == id) {
                        out.push(Entry {
                            id: id.clone(),
                            label: label.trim_end_matches("...").to_owned(),
                            path: path.to_owned(),
                        });
                    }
                }
                mt_spec::MenuItem::Submenu { label, items } => {
                    walk(items, &format!("{path} > {label}"), out)
                }
                mt_spec::MenuItem::Separator { .. } => {}
            }
        }
    }
    let mut out = Vec::new();
    for m in &spec.menus.menu {
        walk(&m.items, &m.label, &mut out);
    }
    for (id, label, path) in crate::context_menu::all_items() {
        if !out.iter().any(|e| e.id == id) {
            out.push(Entry {
                id,
                label: label.trim_end_matches("...").to_owned(),
                path,
            });
        }
    }
    out
}

fn synonyms(id: &str) -> &'static str {
    SYNONYMS
        .iter()
        .find(|(x, _)| *x == id)
        .map_or("", |(_, s)| s)
}

/// Whether `needle`'s characters appear in `hay` in order.
fn subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

/// How well `entry` matches `query` (higher is better); `None` when some word of the
/// query matches nothing. A word matches the label (best at the start of a word),
/// the synonyms, the menu path, or loosely as letters in order in the label.
pub fn score(query: &str, entry: &Entry) -> Option<i32> {
    let label = entry.label.to_lowercase();
    let path = entry.path.to_lowercase();
    let syn = synonyms(&entry.id);
    let mut total = 0;
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return None;
    }
    for w in &words {
        let word_start = label
            .split(|c: char| !c.is_alphanumeric())
            .any(|x| x.starts_with(w.as_str()));
        total += if label.starts_with(w.as_str()) {
            100
        } else if word_start {
            80
        } else if label.contains(w.as_str()) {
            60
        } else if syn.split(' ').any(|x| x.starts_with(w.as_str())) {
            50
        } else if path.contains(w.as_str()) {
            30
        } else if w.len() >= 2 && subsequence(w, &label) {
            10
        } else {
            return None;
        };
    }
    // Shorter labels first among equals (the closer match); menu bar commands
    // before right-click items, whose short labels ("Open", "Center") need their
    // submenu to make sense.
    let context = if path.starts_with("context menu") {
        50
    } else {
        0
    };
    Some(total * 10 - label.len() as i32 - context)
}

/// The best matches for `query`, best first.
pub fn find<'a>(query: &str, entries: &'a [Entry]) -> Vec<&'a Entry> {
    let mut hits: Vec<(i32, &Entry)> = entries
        .iter()
        .filter_map(|e| Some((score(query, e)?, e)))
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.label.cmp(&b.1.label)));
    hits.into_iter().map(|(_, e)| e).take(MAX_RESULTS).collect()
}

impl App {
    /// Help > Search: opens the popup (again: closes it).
    pub(crate) fn open_search(&mut self) {
        self.extra.search = match self.extra.search {
            Some(_) => None,
            None => Some(Search::default()),
        };
    }

    /// Whether a found command can run now.
    fn search_enabled(&self, id: &str) -> bool {
        let doc = self.document.as_ref();
        match id {
            "edit.undo" => doc.is_some_and(crate::document::Document::can_undo),
            "edit.redo" => doc.is_some_and(crate::document::Document::can_redo),
            "edit.cut" | "edit.copy" => !self.selection.items().is_empty(),
            "edit.paste" | "edit.paste_in_place" => self.clipboard.is_some(),
            _ => {
                crate::IMPLEMENTED.contains(&id)
                    && doc
                        .and_then(|d| self.extra_facts(d).enabled(id))
                        .unwrap_or(true)
            }
        }
    }

    /// The popup, while open. Runs the picked command.
    pub(crate) fn search_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(mut s) = self.extra.search.take() else {
            return;
        };
        let all = entries(&self.spec);
        let hits: Vec<Entry> = find(&s.query, &all).into_iter().cloned().collect();
        let (up, down, enter, escape) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if down && s.selected + 1 < hits.len() {
            s.selected += 1;
        }
        if up {
            s.selected = s.selected.saturating_sub(1);
        }
        s.selected = s.selected.min(hits.len().saturating_sub(1));
        let enabled: Vec<bool> = hits.iter().map(|h| self.search_enabled(&h.id)).collect();
        let screen = ctx.content_rect();
        let width = 420.0;
        let mut run: Option<String> = None;
        let mut close = escape;
        let area = egui::Area::new(egui::Id::new("mt_search"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(
                screen.center().x - width / 2.0,
                screen.top() + 90.0,
            ))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(width);
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut s.query)
                            .hint_text("Search tools and commands")
                            .desired_width(f32::INFINITY),
                    );
                    if !s.focused {
                        edit.request_focus();
                        s.focused = true;
                    }
                    if edit.changed() {
                        s.selected = 0;
                    }
                    if !hits.is_empty() {
                        ui.add_space(4.0);
                    }
                    for (k, h) in hits.iter().enumerate() {
                        let on = enabled[k];
                        let shortcut = self
                            .spec
                            .keymap
                            .shortcut(&h.id)
                            .map(|x| x.label())
                            .unwrap_or_default();
                        let weak = ui.visuals().weak_text_color();
                        let text = if on { ui.visuals().text_color() } else { weak };
                        let (rect, resp) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 34.0),
                            egui::Sense::click(),
                        );
                        if k == s.selected || resp.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                3.0,
                                ui.visuals().selection.bg_fill.gamma_multiply(0.25),
                            );
                        }
                        let p = ui.painter();
                        p.text(
                            rect.left_top() + egui::vec2(6.0, 3.0),
                            egui::Align2::LEFT_TOP,
                            &h.label,
                            egui::FontId::proportional(13.0),
                            text,
                        );
                        p.text(
                            rect.left_bottom() + egui::vec2(6.0, -3.0),
                            egui::Align2::LEFT_BOTTOM,
                            &h.path,
                            egui::FontId::proportional(10.5),
                            weak,
                        );
                        p.text(
                            rect.right_center() - egui::vec2(6.0, 0.0),
                            egui::Align2::RIGHT_CENTER,
                            &shortcut,
                            egui::FontId::proportional(12.0),
                            weak,
                        );
                        if resp.clicked() && on {
                            run = Some(h.id.clone());
                        }
                    }
                    if !s.query.trim().is_empty() && hits.is_empty() {
                        ui.label(egui::RichText::new("No matches.").weak());
                    }
                });
            });
        if enter
            && let Some(h) = hits.get(s.selected)
            && enabled[s.selected]
        {
            run = Some(h.id.clone());
        }
        // A click outside closes it.
        if ctx.input(|i| i.pointer.any_pressed())
            && !ctx
                .pointer_interact_pos()
                .is_some_and(|p| area.response.rect.contains(p))
        {
            close = true;
        }
        if let Some(id) = run {
            self.run_command(ctx, frame, &id);
        } else if !close {
            self.extra.search = Some(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> mt_spec::Spec {
        mt_spec::Spec::builtin().unwrap()
    }

    #[test]
    fn finds_by_label_synonym_and_path() {
        let all = entries(&spec());
        assert!(all.iter().any(|e| e.id == "help.search"));
        assert!(all.iter().any(|e| e.id == "edit.find_center"));
        let ids = |q: &str| {
            find(q, &all)
                .iter()
                .map(|e| e.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids("push")[0], "tool.push_pull");
        assert_eq!(ids("extrude")[0], "tool.push_pull");
        assert!(ids("transparent").contains(&"view.xray".to_owned()));
        assert_eq!(ids("tape")[0], "tool.tape_measure");
        assert!(ids("zoom ext").contains(&"camera.zoom_extents".to_owned()));
        // Letters in order.
        assert!(ids("pshpl").contains(&"tool.push_pull".to_owned()));
        assert!(ids("qqqzzz").is_empty());
        assert!(ids("   ").is_empty());
        assert!(ids("e").len() <= MAX_RESULTS);
    }

    #[test]
    fn shift_s_is_bound_to_search_only() {
        let spec = spec();
        let s = spec.keymap.shortcut("help.search").unwrap();
        assert!(s.shift && !s.ctrl && !s.alt && s.key == "S");
        let clash: Vec<_> = spec
            .keymap
            .bindings
            .keys()
            .filter(|id| {
                id.as_str() != "help.search" && spec.keymap.shortcut(id) == Some(s.clone())
            })
            .collect();
        assert!(clash.is_empty(), "{clash:?}");
        assert!(crate::IMPLEMENTED.contains(&"help.search"));
    }
}
