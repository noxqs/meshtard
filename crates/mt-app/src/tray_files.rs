//! Tray panel parts of LEFTOVERS.md 29, app side: `.skm` material files (Open
//! Material..., Save As...), favorite collections, writing either kind of `.style`
//! file, and the Components Statistics tab for the whole model or the selection.
//!
//! Favorites are kept in `favorites.txt` in the config folder (`kind<TAB>key` lines,
//! the key being the collection's folder or `builtin:<name>`); a favorite folder is
//! listed even when it is not under a collection root. From memory of SketchUp,
//! unverified: favorites as a Details menu pair and a dropdown section, and the
//! Statistics scopes ("All geometry").

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use mt_kernel::{DefinitionId, Entities, Model};
use mt_ui::{CollectionKind, Event, StatsScope};

use crate::App;
use crate::document::Owner;

fn favorites_path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("favorites.txt"))
}

const KINDS: [&str; 3] = ["materials", "components", "styles"];

/// Favorite collection keys per kind, from `text` (`favorites.txt`).
pub(crate) fn parse_favorites(text: &str) -> [Vec<String>; 3] {
    let mut out: [Vec<String>; 3] = Default::default();
    for line in text.lines() {
        if let Some((k, key)) = line.split_once('\t')
            && let Some(i) = KINDS.iter().position(|x| *x == k)
            && !key.is_empty()
            && !out[i].iter().any(|x| x == key)
        {
            out[i].push(key.to_owned());
        }
    }
    out
}

/// `favorites.txt`'s text for `favs`.
pub(crate) fn favorites_text(favs: &[Vec<String>; 3]) -> String {
    let mut text = String::new();
    for (k, keys) in favs.iter().enumerate() {
        for key in keys {
            text.push_str(&format!("{}\t{key}\n", KINDS[k]));
        }
    }
    text
}

impl crate::tray_extra::TrayLib {
    /// The favorite collections, read on first use.
    pub(crate) fn favorites(&mut self) -> &mut [Vec<String>; 3] {
        self.favorites.get_or_insert_with(|| {
            let text = favorites_path()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .unwrap_or_default();
            parse_favorites(&text)
        })
    }
}

/// Statistics of some entities: counts per `tray_extra::STAT_ROWS` row (materials and
/// tags as sets).
#[derive(Default, Clone)]
struct Tally {
    n: [usize; 10],
    materials: BTreeSet<usize>,
    tags: BTreeSet<usize>,
}

impl Tally {
    fn add(&mut self, o: &Tally) {
        for (a, b) in self.n.iter_mut().zip(o.n) {
            *a += b;
        }
        self.materials.extend(&o.materials);
        self.tags.extend(&o.tags);
    }
}

/// Counts what `ents` holds (only the entities `pick` takes; guides and guide points
/// only with no filter, as they cannot be selected here) and returns the definitions
/// of the instances counted, one entry per instance.
fn own(
    m: &Model,
    ents: &Entities,
    pick: Option<&dyn Fn(Owner) -> bool>,
) -> (Tally, Vec<DefinitionId>) {
    let take = |o: Owner| pick.is_none_or(|p| p(o));
    let mut t = Tally::default();
    for (k, f) in ents.faces.iter() {
        if take(Owner::Face(k)) {
            t.n[1] += 1;
            t.materials.extend(f.attrs.material);
            t.materials.extend(f.attrs.back_material);
            t.tags.extend(f.attrs.layer);
        }
    }
    for (k, e) in ents.edges.iter() {
        if take(Owner::Edge(k)) {
            t.n[0] += 1;
            t.tags.extend(e.attrs.layer);
        }
    }
    if pick.is_none() {
        t.n[5] += ents.guides.iter().filter(|g| g.dir.is_some()).count();
        t.n[6] += ents.guides.iter().filter(|g| g.dir.is_none()).count();
    }
    t.n[7] += (0..ents.section_planes.len())
        .filter(|&i| take(Owner::Section(i)))
        .count();
    t.n[8] += (0..ents.dimensions.len())
        .filter(|&i| take(Owner::Dimension(i)))
        .count();
    t.n[9] += (0..ents.texts.len())
        .filter(|&i| take(Owner::Text(i)))
        .count();
    let mut defs = Vec::new();
    for (k, i) in ents.instances.iter().enumerate() {
        if !take(Owner::Instance(k)) {
            continue;
        }
        let Some(d) = m.definitions.get(i.definition) else {
            continue;
        };
        t.materials.extend(i.material);
        t.tags.extend(i.layer);
        t.n[if d.is_image {
            4
        } else if d.is_group {
            3
        } else {
            2
        }] += 1;
        defs.push(i.definition);
    }
    (t, defs)
}

/// Everything definition `d` holds with nested instances expanded (each instance
/// counted), memoized per definition (`depth` stops cycles).
fn expanded(
    m: &Model,
    d: DefinitionId,
    memo: &mut HashMap<DefinitionId, Tally>,
    depth: usize,
) -> Tally {
    if let Some(t) = memo.get(&d) {
        return t.clone();
    }
    let Some(def) = m.definitions.get(d) else {
        return Tally::default();
    };
    let (mut t, defs) = own(m, &def.entities, None);
    if depth < 32 {
        for c in defs {
            let sub = expanded(m, c, memo, depth + 1);
            t.add(&sub);
        }
    }
    memo.insert(d, t.clone());
    t
}

/// The Statistics rows (label, own count, count with nested contents expanded) of
/// `ents` (the root, or the open context's entities `pick` takes).
pub(crate) fn stats_of(
    m: &Model,
    ents: &Entities,
    pick: Option<&dyn Fn(Owner) -> bool>,
) -> Vec<(String, usize, usize)> {
    let (top, defs) = own(m, ents, pick);
    let mut all = top.clone();
    let mut memo = HashMap::new();
    for d in defs {
        let sub = expanded(m, d, &mut memo, 0);
        all.add(&sub);
    }
    crate::tray_extra::STAT_ROWS
        .iter()
        .enumerate()
        .map(|(k, label)| {
            let (a, b) = match k {
                10 => (top.materials.len(), all.materials.len()),
                11 => (top.tags.len(), all.tags.len()),
                k => (top.n[k], all.n[k]),
            };
            ((*label).to_owned(), a, b)
        })
        .collect()
}

/// The cached model-wide or selection Statistics: key and rows.
#[derive(Default)]
pub(crate) struct StatsCache {
    /// What the Statistics tab shows this frame (`Shell::stats_scope`).
    pub(crate) scope: Option<StatsScope>,
    key: Option<(u64, StatsScope, u64)>,
    rows: Vec<(String, usize, usize)>,
}

impl App {
    /// Fills `data.model_stats` while the Statistics tab shows the whole model or the
    /// selection (recounted after a model or selection change).
    pub(crate) fn tray_data_files(&mut self, data: &mut mt_ui::TrayData) {
        let Some(scope) = self.stats_cache.scope else {
            return;
        };
        let Some(doc) = &self.document else { return };
        let sel_key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            self.selection.items().hash(&mut h);
            doc.context.hash(&mut h);
            h.finish()
        };
        let key = (doc.state_id(), scope, sel_key);
        if self.stats_cache.key != Some(key) {
            let m = &doc.model;
            self.stats_cache.rows = match scope {
                StatsScope::Selection => {
                    let sel = &self.selection;
                    let pick = |o: Owner| sel.contains(&o);
                    stats_of(m, doc.active(), Some(&pick))
                }
                _ => stats_of(m, &m.root, None),
            };
            self.stats_cache.key = Some(key);
        }
        data.model_stats.clone_from(&self.stats_cache.rows);
    }

    /// Writes material `i` to a `.skm` file the user picks.
    fn save_material(&mut self, i: usize) {
        let Some(m) = (self.document.as_ref()).and_then(|d| d.model.materials.get(i).cloned())
        else {
            return;
        };
        let filters: [(&str, &[&str]); 1] = [("SketchUp Materials (*.skm)", &["skm"])];
        let Some(path) = crate::filedialog::save(&filters, &format!("{}.skm", m.name)) else {
            return;
        };
        match crate::skm::write(&m)
            .and_then(|d| std::fs::write(&path, d).map_err(|e| e.to_string()))
        {
            Ok(()) => {
                self.message = Some(format!("Saved {}.", path.display()));
                // The file may be in a collection: list it.
                self.tray_lib.collections[0] = None;
            }
            Err(e) => self.file_error("save", &path, e),
        }
    }

    /// Reads a `.skm` file the user picks into the model and makes it current.
    fn open_material(&mut self) -> bool {
        let filters: [(&str, &[&str]); 1] = [("SketchUp Materials (*.skm)", &["skm"])];
        let Some(path) = crate::filedialog::open(&filters) else {
            return false;
        };
        let read = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|d| crate::errors::guard(|| crate::skm::read(&d)));
        let mat = match read {
            Ok(s) => s.material,
            Err(e) => {
                self.file_error("open", &path, e);
                return false;
            }
        };
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        // A material of that name is replaced (SketchUp asks; unverified).
        let mut index = 0;
        doc.edit_model(|m, _| {
            index = match m.materials.iter().position(|x| x.name == mat.name) {
                Some(i) => {
                    m.materials[i] = mat.clone();
                    i
                }
                None => {
                    m.materials.push(mat.clone());
                    m.materials.len() - 1
                }
            };
            true
        });
        self.make_current_material(index);
        true
    }

    /// Writes In Model style `i` as a SketchUp `.style` archive or our text file.
    fn save_style(&mut self, i: usize, sketchup: bool) {
        let Some(style) = (self.document.as_ref()).and_then(|d| d.model.styles.get(i).cloned())
        else {
            return;
        };
        let filters: [(&str, &[&str]); 1] = [(
            if sketchup {
                "SketchUp Styles (*.style)"
            } else {
                "meshtard Text Styles (*.style)"
            },
            &["style"],
        )];
        let Some(path) = crate::filedialog::save(&filters, &format!("{}.style", style.name)) else {
            return;
        };
        let data = if sketchup {
            let thumb = self
                .tray_lib
                .render
                .style_png(&format!("model:{i}"), &style.settings);
            crate::style_zip::write(&style, thumb.as_deref())
        } else {
            Ok(crate::tray_extra::style_file_text(&style).into_bytes())
        };
        match data.and_then(|d| std::fs::write(&path, d).map_err(|e| e.to_string())) {
            Ok(()) => {
                self.message = Some(format!("Saved {}.", path.display()));
                self.tray_lib.collections[2] = None;
                self.tray_lib.render.forget_collections();
            }
            Err(e) => self.file_error("save", &path, e),
        }
    }

    /// The events of this module (`Err`: not one of them).
    pub(crate) fn tray_files_event(&mut self, event: Event) -> Result<bool, Event> {
        match event {
            Event::MaterialOpenFile => Ok(self.open_material()),
            Event::MaterialSaveFile(i) => {
                self.save_material(i);
                Ok(false)
            }
            Event::StyleSaveFile { index, sketchup } => {
                self.save_style(index, sketchup);
                Ok(false)
            }
            Event::CollectionFavorite {
                kind,
                collection,
                add,
            } => {
                let k = match kind {
                    CollectionKind::Materials => 0,
                    CollectionKind::Components => 1,
                    CollectionKind::Styles => 2,
                };
                let Some(key) =
                    (self.tray_lib.collections(k).get(collection)).map(|c| c.key.clone())
                else {
                    return Ok(false);
                };
                let favs = &mut self.tray_lib.favorites()[k];
                if add && !favs.contains(&key) {
                    favs.push(key);
                } else if !add {
                    favs.retain(|f| *f != key);
                }
                let text = favorites_text(self.tray_lib.favorites());
                if let Some(path) = favorites_path() {
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    if let Err(e) = std::fs::write(&path, text) {
                        log::warn!("could not save {}: {e}", path.display());
                    }
                }
                Ok(false)
            }
            e => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{DMat4, DVec3};
    use mt_kernel::{Instance, add_line};

    #[test]
    fn favorites_text_round_trip() {
        let favs = parse_favorites(
            "materials\t/a\nstyles\tbuiltin:Default Styles\nbogus\tx\nmaterials\t/a\ncomponents\t\n",
        );
        assert_eq!(favs[0], ["/a"]);
        assert!(favs[1].is_empty());
        assert_eq!(favs[2], ["builtin:Default Styles"]);
        assert_eq!(parse_favorites(&favorites_text(&favs)), favs);
    }

    fn instance(definition: DefinitionId) -> Instance {
        Instance {
            definition,
            transform: DMat4::IDENTITY,
            name: String::new(),
            hidden: false,
            locked: false,
            layer: None,
            material: None,
            attrs: Default::default(),
        }
    }

    #[test]
    fn model_and_selection_statistics_count_nested_contents() {
        let mut m = Model::default();
        // A definition with one edge, placed twice inside a group placed once.
        let mut leaf = mt_kernel::Definition {
            name: "Leaf".into(),
            ..Default::default()
        };
        add_line(&mut leaf.entities, DVec3::ZERO, DVec3::X);
        let leaf = m.definitions.insert(leaf);
        let mut group = mt_kernel::Definition {
            name: "Group".into(),
            is_group: true,
            ..Default::default()
        };
        group.entities.instances.push(instance(leaf));
        group.entities.instances.push(instance(leaf));
        let group = m.definitions.insert(group);
        m.root.instances.push(instance(group));
        add_line(&mut m.root, DVec3::Y, DVec3::Z);
        let rows = stats_of(&m, &m.root, None);
        let row = |r: &[(String, usize, usize)], k: usize| (r[k].1, r[k].2);
        // Edges: one in the root; with the nested ones 1 + 2.
        assert_eq!(row(&rows, 0), (1, 3));
        // Components: none directly, two nested; groups: one.
        assert_eq!(row(&rows, 2), (0, 2));
        assert_eq!(row(&rows, 3), (1, 1));
        // Only the group selected.
        let pick = |o: Owner| o == Owner::Instance(0);
        let sel = stats_of(&m, &m.root, Some(&pick));
        assert_eq!(row(&sel, 0), (0, 2));
        assert_eq!(row(&sel, 3), (1, 1));
    }
}
