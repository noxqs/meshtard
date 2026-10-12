//! Window > Preferences (FEATURES.md 43): the pages Accessibility, Compatibility,
//! Drawing, Files, General, Graphics, Shortcuts, Template and Workspace, and what the
//! General page runs: auto-save to a recovery file, backups, Ask to Purge at save,
//! automatic Fix Problems and the Welcome window at start.
//!
//! Shortcuts changed here are saved to `shortcuts.toml` in the user config folder
//! (next to `toolbars.toml`, see `settings`) as the differences from the built-in
//! keymap (`""` removes a shortcut), and applied at startup. The other pages' settings
//! go to `preferences.toml` (General's keys at the top, a table per other page).
//!
//! Option lists and the defaults marked "help" come from SketchUp's help pages
//! (read 2026-10-11); page layouts, wording and the other defaults are from memory and
//! unverified. Auto-save is on every 5 minutes (help) and writes a recovery file
//! (`welcome`); the backup is the previous file copied to `<name>.skb` on save.
//! Ask to Purge All asks before File > Save / Save As when the model has unused
//! components, materials or tags (the prompt's wording is unverified; the "Save
//! changes?" question's save does not ask). Fix Problems runs on open, auto-save and
//! save (help: on by default) and only tells when Notify is on (help: off).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use mt_spec::{Shortcut, Spec};

use crate::App;
use crate::click_style::{ClickStyle, Drawing};
use crate::prefs_pages::More;

/// Preferences > General.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct General {
    pub autosave: bool,
    pub autosave_minutes: u32,
    pub backup: bool,
    /// Ask to Purge All at save (help: on).
    pub ask_purge: bool,
    /// Automatically fix problems on open, auto-save and save (help: on).
    pub fix_problems: bool,
    /// Notify me when problems are fixed (help: off).
    pub notify_fixed: bool,
    /// Show the Welcome window at start (help: on).
    pub welcome: bool,
    /// Warnings answered with "don't show again" (Reset All Warning Messages clears).
    pub dismissed: BTreeSet<String>,
}

impl Default for General {
    fn default() -> Self {
        Self {
            autosave: true,
            autosave_minutes: 5,
            backup: true,
            ask_purge: true,
            fix_problems: true,
            notify_fixed: false,
            welcome: true,
            dismissed: BTreeSet::new(),
        }
    }
}

/// The section plane name prompt's "don't ask again" (`tool_extra`).
pub(crate) const WARN_SECTION_NAME: &str = "section_name";
/// The "problems fixed" notice's "don't show again".
pub(crate) const WARN_FIXED: &str = "problems_fixed";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Accessibility,
    Applications,
    Compatibility,
    Drawing,
    Files,
    General,
    Graphics,
    Shortcuts,
    Template,
    Workspace,
}

impl Page {
    /// In SketchUp's (alphabetical) order.
    const ALL: [(Page, &'static str); 10] = [
        (Page::Accessibility, "Accessibility"),
        (Page::Applications, "Applications"),
        (Page::Compatibility, "Compatibility"),
        (Page::Drawing, "Drawing"),
        (Page::Files, "Files"),
        (Page::General, "General"),
        (Page::Graphics, "Graphics"),
        (Page::Shortcuts, "Shortcuts"),
        (Page::Template, "Template"),
        (Page::Workspace, "Workspace"),
    ];
}

#[derive(Debug, Clone)]
struct Dialog {
    page: Page,
    filter: String,
    selected: Option<String>,
    /// Waiting for the keys of the selected command's new shortcut.
    capturing: bool,
    /// A pressed shortcut another command has: the shortcut and that command,
    /// waiting for Keep Original or Replace.
    conflict: Option<(Shortcut, String)>,
    note: Option<String>,
    /// Files > Import / Export: include the shortcuts.
    with_shortcuts: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Prefs {
    pub general: General,
    pub drawing: Drawing,
    pub more: More,
    /// The pointer between frames, for the click style.
    pub pointer: crate::click_style::Pointer,
    /// The built-in keymap (Reset All goes back to it).
    pub(crate) defaults: BTreeMap<String, String>,
    /// The spec's axis colors (Accessibility's defaults for the drawn axes).
    spec_axes: [[f32; 3]; 3],
    dialog: Option<Dialog>,
    /// Help > Keyboard Shortcuts while open.
    pub(crate) shortcut_list: Option<crate::shortcut_list::ListDialog>,
    /// Auto-save: since when the model has changes not auto-saved yet.
    dirty_since: Option<Instant>,
    /// The document state last auto-saved.
    pub autosaved: Option<u64>,
    /// This session's recovery file of the current model, while it exists.
    pub recovery_written: Option<PathBuf>,
    /// The Welcome window while open.
    pub welcome: Option<crate::welcome::Welcome>,
    /// The first frame's start-up checks are done.
    started: bool,
    /// File > Save or Save As waiting for the Ask to Purge answer.
    purge_prompt: Option<String>,
    /// "Problems fixed" message waiting to be shown.
    notice: Option<String>,
    /// Graphics: what the adapter supports and the sample count in use.
    pub samples_supported: Vec<u32>,
    pub samples_in_use: u32,
    pub adapter_details: String,
    /// The toolbar icon size given to the shell.
    icons_applied: Option<bool>,
}

/// The shortcut table the app matches key presses against: implemented commands
/// with a shortcut whose key egui knows.
pub(crate) fn shortcut_table(
    spec: &Spec,
    implemented: &[&'static str],
) -> Vec<(Shortcut, egui::Key, &'static str)> {
    implemented
        .iter()
        .filter_map(|&id| {
            let shortcut = spec.keymap.shortcut(id)?;
            let key = egui::Key::from_name(&shortcut.key)?;
            Some((shortcut, key, id))
        })
        .collect()
}

/// Modifier keys, which egui also reports as key presses: a shortcut needs another
/// key.
fn is_modifier(key: egui::Key) -> bool {
    use egui::Key as K;
    matches!(
        key,
        K::ShiftLeft
            | K::ShiftRight
            | K::ControlLeft
            | K::ControlRight
            | K::AltLeft
            | K::AltRight
            | K::SuperLeft
            | K::SuperRight
    )
}

/// Reads `shortcuts.toml` text: command id to shortcut, `""` for removed.
pub(crate) fn parse_overrides(text: &str) -> Result<BTreeMap<String, String>, String> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let Some(bindings) = table.get("bindings") else {
        return Ok(BTreeMap::new());
    };
    let bindings = bindings.as_table().ok_or("`bindings` is not a table")?;
    bindings
        .iter()
        .map(|(id, v)| {
            let s = v.as_str().ok_or(format!("`{id}`: not a string"))?;
            if !s.is_empty() {
                Shortcut::parse(s).map_err(|e| format!("`{id}`: {e}"))?;
            }
            Ok((id.clone(), s.to_owned()))
        })
        .collect()
}

pub(crate) fn overrides_text(overrides: &BTreeMap<String, String>) -> String {
    let bindings: toml::Table = overrides
        .iter()
        .map(|(k, v)| (k.clone(), toml::Value::String(v.clone())))
        .collect();
    let mut table = toml::Table::new();
    table.insert("bindings".into(), toml::Value::Table(bindings));
    format!(
        "# meshtard keyboard shortcuts that differ from the built-in ones (\"\" = none).\n{table}"
    )
}

/// The differences of `current` from `defaults`.
pub(crate) fn diff(
    defaults: &BTreeMap<String, String>,
    current: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for id in defaults.keys().filter(|id| !current.contains_key(*id)) {
        out.insert(id.clone(), String::new());
    }
    for (id, s) in current {
        if defaults.get(id) != Some(s) {
            out.insert(id.clone(), s.clone());
        }
    }
    out
}

/// The command other than `id` that has shortcut `s`.
pub(crate) fn holder(
    bindings: &BTreeMap<String, String>,
    id: &str,
    s: &Shortcut,
) -> Option<String> {
    bindings
        .iter()
        .find(|(k, v)| *k != id && Shortcut::parse(v).ok().as_ref() == Some(s))
        .map(|(k, _)| k.clone())
}

/// Gives `id` the shortcut `to` (none: removes it). A command that had the same
/// shortcut loses it; returns that command.
pub(crate) fn rebind(
    bindings: &mut BTreeMap<String, String>,
    id: &str,
    to: Option<&Shortcut>,
) -> Option<String> {
    let Some(to) = to else {
        bindings.remove(id);
        return None;
    };
    let other = holder(bindings, id, to);
    if let Some(o) = &other {
        bindings.remove(o);
    }
    bindings.insert(id.to_owned(), to.to_string());
    other
}

/// Applies saved overrides to a keymap.
pub(crate) fn apply_overrides(
    bindings: &mut BTreeMap<String, String>,
    overrides: &BTreeMap<String, String>,
) {
    for (id, s) in overrides {
        let shortcut = Shortcut::parse(s).ok();
        rebind(bindings, id, shortcut.as_ref());
    }
}

fn general_from_table(t: &toml::Table) -> General {
    let mut g = General::default();
    let flag = |k: &str| t.get(k).and_then(toml::Value::as_bool);
    if let Some(v) = flag("autosave") {
        g.autosave = v;
    }
    if let Some(v) = t.get("autosave_minutes").and_then(toml::Value::as_integer) {
        g.autosave_minutes = v.clamp(1, 999) as u32;
    }
    for (key, field) in [
        ("backup", &mut g.backup),
        ("ask_purge", &mut g.ask_purge),
        ("fix_problems", &mut g.fix_problems),
        ("notify_fixed", &mut g.notify_fixed),
        ("show_welcome", &mut g.welcome),
    ] {
        if let Some(v) = flag(key) {
            *field = v;
        }
    }
    g.dismissed = t
        .get("dismissed_warnings")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    g
}

fn drawing_from_table(t: &toml::Table) -> Drawing {
    let mut d = Drawing::default();
    let Some(t) = t.get("drawing").and_then(toml::Value::as_table) else {
        return d;
    };
    if let Some(s) = t
        .get("click_style")
        .and_then(toml::Value::as_str)
        .and_then(ClickStyle::from_key)
    {
        d.click_style = s;
    }
    for (key, field) in [
        ("continue_line", &mut d.continue_line),
        ("crosshairs", &mut d.crosshairs),
        ("disable_prepick_push_pull", &mut d.no_prepick),
    ] {
        if let Some(v) = t.get(key).and_then(toml::Value::as_bool) {
            *field = v;
        }
    }
    d
}

/// All settings as `preferences.toml` text.
fn settings_text(g: &General, d: &Drawing, m: &More) -> String {
    let mut t = toml::Table::new();
    t.insert("autosave".into(), g.autosave.into());
    t.insert(
        "autosave_minutes".into(),
        i64::from(g.autosave_minutes).into(),
    );
    t.insert("backup".into(), g.backup.into());
    t.insert("ask_purge".into(), g.ask_purge.into());
    t.insert("fix_problems".into(), g.fix_problems.into());
    t.insert("notify_fixed".into(), g.notify_fixed.into());
    t.insert("show_welcome".into(), g.welcome.into());
    t.insert(
        "dismissed_warnings".into(),
        toml::Value::Array(g.dismissed.iter().map(|s| s.as_str().into()).collect()),
    );
    let mut dr = toml::Table::new();
    dr.insert("click_style".into(), d.click_style.key().into());
    dr.insert("continue_line".into(), d.continue_line.into());
    dr.insert("crosshairs".into(), d.crosshairs.into());
    dr.insert("disable_prepick_push_pull".into(), d.no_prepick.into());
    t.insert("drawing".into(), dr.into());
    m.write(&mut t);
    t.to_string()
}

fn settings_from_table(t: &toml::Table) -> (General, Drawing, More) {
    (general_from_table(t), drawing_from_table(t), More::read(t))
}

fn settings_from_text(text: &str) -> (General, Drawing, More) {
    settings_from_table(&text.parse().unwrap_or_default())
}

/// The backup of the model at `path`, made when saving over it.
pub(crate) fn backup_path(path: &Path) -> PathBuf {
    path.with_extension("skb")
}

fn config_file(name: &str) -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join(name))
}

fn write_config(name: &str, text: &str) {
    let Some(path) = config_file(name) else {
        return;
    };
    let result = std::fs::create_dir_all(path.parent().expect("has a parent"))
        .and_then(|()| std::fs::write(&path, text));
    if let Err(e) = result {
        log::warn!("could not save {}: {e}", path.display());
    }
}

/// Loads saved preferences and applies saved shortcuts and axis colors to `spec`.
pub(crate) fn load(spec: &mut Spec) -> Prefs {
    let defaults = spec.keymap.bindings.clone();
    if let Some(text) = config_file("shortcuts.toml").and_then(|p| std::fs::read_to_string(p).ok())
    {
        match parse_overrides(&text) {
            Ok(o) => apply_overrides(&mut spec.keymap.bindings, &o),
            Err(e) => log::warn!("ignoring saved shortcuts: {e}"),
        }
    }
    let (general, drawing, more) = config_file("preferences.toml")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| settings_from_text(&t))
        .unwrap_or_default();
    let a = &spec.default_model.axes;
    let prefs = Prefs {
        general,
        drawing,
        more,
        defaults,
        spec_axes: [a.x, a.y, a.z],
        ..Default::default()
    };
    prefs.apply_axes(spec);
    prefs.more.apply_globals();
    prefs
}

impl Prefs {
    /// Accessibility's axis colors into the spec the viewport style is built from.
    fn apply_axes(&self, spec: &mut Spec) {
        use crate::prefs_pages::Swatch;
        let axes = &mut spec.default_model.axes;
        for (i, (s, out)) in [Swatch::X, Swatch::Y, Swatch::Z]
            .into_iter()
            .zip([&mut axes.x, &mut axes.y, &mut axes.z])
            .enumerate()
        {
            *out = self
                .more
                .palette
                .get(s)
                .map_or(self.spec_axes[i], |c| c.map(|v| f32::from(v) / 255.0));
        }
    }

    /// Graphics: the adapter's details and multisample counts; returns the count to
    /// create the viewport with.
    pub(crate) fn set_adapter(&mut self, adapter: &egui_wgpu::wgpu::Adapter) -> u32 {
        self.adapter_details = crate::prefs_pages::engine_details(&adapter.get_info());
        self.samples_supported = mt_render::viewport::supported_samples(adapter);
        self.samples_in_use =
            crate::prefs_pages::usable_samples(self.more.msaa, &self.samples_supported);
        self.samples_in_use
    }

    /// The mouse wheel's direction (Compatibility > Invert).
    pub(crate) fn wheel_sign(&self) -> f64 {
        if self.more.invert_wheel { -1.0 } else { 1.0 }
    }

    /// Before File > Save writes over `path`: keeps the old file as a backup.
    pub(crate) fn backup(&self, path: &Path) {
        if self.general.backup
            && path.exists()
            && let Err(e) = std::fs::copy(path, backup_path(path))
        {
            log::warn!("could not back up {}: {e}", path.display());
        }
    }

    /// After the model was saved to `path`: no recovery file is needed and the file is
    /// a recent one.
    pub(crate) fn after_save(&mut self, path: &Path) {
        if let Some(r) = self.recovery_written.take() {
            crate::welcome::remove_recovery(&r);
        }
        crate::welcome::add_recent(path);
    }

    /// Fix Problems when Automatically Fix Problems is on; returns the notice to show.
    fn auto_fix(&self, doc: &mut crate::document::Document) -> Option<String> {
        if !self.general.fix_problems || crate::fix_problems::count_problems(&doc.model) == 0 {
            return None;
        }
        let mut report = crate::fix_problems::Report::default();
        doc.edit_whole(|m| {
            report = crate::fix_problems::fix_model(m);
            report.fixed > 0
        });
        log::info!("fix problems: {}", report.message());
        (self.general.notify_fixed && !self.general.dismissed.contains(WARN_FIXED))
            .then(|| report.message())
    }
}

/// Unused components, materials or tags that Purge All would remove.
fn has_unused(model: &mt_kernel::Model) -> bool {
    let mut m = model.clone();
    mt_kernel::purge::purge_all(&mut m).any()
}

impl App {
    pub(crate) fn open_preferences(&mut self) {
        self.prefs.dialog.get_or_insert(Dialog {
            page: Page::General,
            filter: String::new(),
            selected: None,
            capturing: false,
            conflict: None,
            note: None,
            with_shortcuts: true,
        });
    }

    /// Writes `preferences.toml`.
    pub(crate) fn save_preferences(&self) {
        let p = &self.prefs;
        write_config(
            "preferences.toml",
            &settings_text(&p.general, &p.drawing, &p.more),
        );
    }

    /// Applies a changed keymap: the shortcut table, menus (they read the spec) and
    /// the saved file.
    fn keymap_changed(&mut self) {
        self.add_extension_shortcuts();
        let overrides = diff(&self.prefs.defaults, &self.spec.keymap.bindings);
        write_config("shortcuts.toml", &overrides_text(&overrides));
    }

    /// File > Save, Save As (from the menu or a shortcut): asks to purge first when
    /// Ask to Purge All is on and something would be purged. Returns false when the
    /// question is shown (the save follows its answer).
    pub(crate) fn save_command(&mut self, ctx: &egui::Context, id: &str) {
        let ask = id != "file.save_copy_as"
            && self.prefs.general.ask_purge
            && self.document.as_ref().is_some_and(|d| has_unused(&d.model));
        if ask {
            self.prefs.purge_prompt = Some(id.to_owned());
        } else {
            self.save(ctx, id);
        }
    }

    /// Runs the General page's checks before a save: Fix Problems, and the backup.
    pub(crate) fn before_save(&mut self, path: &Path) {
        if let Some(doc) = self.document.as_mut()
            && let Some(n) = self.prefs.auto_fix(doc)
        {
            self.prefs.notice = Some(n);
        }
        self.prefs.backup(path);
    }

    /// A model was opened (`install`): Fix Problems, and the recent files list.
    pub(crate) fn after_open(&mut self) {
        let Some(doc) = self.document.as_mut() else {
            return;
        };
        if doc.has_file {
            crate::welcome::add_recent(&doc.path);
        }
        if let Some(n) = self.prefs.auto_fix(doc) {
            self.prefs.notice = Some(n);
        }
    }

    /// The app is closing normally: the recovery file goes.
    pub(crate) fn preferences_exit(&mut self) {
        if let Some(r) = self.prefs.recovery_written.take() {
            crate::welcome::remove_recovery(&r);
        }
    }

    /// Auto-saves the model to its recovery file when it has had unsaved changes for
    /// the set time; removes the recovery file once the model has none.
    fn autosave_tick(&mut self, ctx: &egui::Context) {
        let Some(doc) = self.document.as_mut() else {
            return;
        };
        let pid = std::process::id();
        let path = crate::welcome::recovery_dir()
            .map(|d| d.join(crate::welcome::recovery_name(&doc.path, pid)));
        let modified = doc.is_modified();
        // Saved, discarded or renamed: the old recovery file is not needed.
        if let Some(old) = &self.prefs.recovery_written
            && (!modified || path.as_ref() != Some(old))
        {
            crate::welcome::remove_recovery(old);
            self.prefs.recovery_written = None;
        }
        let g = &self.prefs.general;
        let (Some(path), true, true) = (path, g.autosave, modified) else {
            self.prefs.dirty_since = None;
            return;
        };
        if self.prefs.autosaved == Some(doc.state_id()) {
            self.prefs.dirty_since = None;
            return;
        }
        let now = Instant::now();
        let since = *self.prefs.dirty_since.get_or_insert(now);
        let every = Duration::from_secs(u64::from(g.autosave_minutes.max(1)) * 60);
        let elapsed = now - since;
        if elapsed < every {
            ctx.request_repaint_after(every - elapsed);
            return;
        }
        if let Some(n) = self.prefs.auto_fix(doc) {
            self.prefs.notice = Some(n);
        }
        let origin = doc.has_file.then(|| doc.path.clone());
        let result = doc.to_skp().map_err(|e| e.to_string()).and_then(|bytes| {
            crate::welcome::write_recovery(&path, &bytes, origin.as_deref())
                .map_err(|e| e.to_string())
        });
        match result {
            Ok(()) => {
                log::info!("auto-saved to {}", path.display());
                self.prefs.recovery_written = Some(path);
            }
            Err(e) => self.message = Some(format!("Could not auto-save {}: {e}", path.display())),
        }
        self.prefs.autosaved = Some(doc.state_id());
        self.prefs.dirty_since = None;
    }

    /// First frame: the Welcome window (when it is on and no model was given on the
    /// command line, or when there are recovered files) and the icon size.
    fn startup(&mut self, ctx: &egui::Context) {
        if self.prefs.icons_applied != Some(self.prefs.more.large_icons)
            && let Some(shell) = &mut self.shell
        {
            shell.set_large_icons(&self.spec, self.prefs.more.large_icons);
            self.prefs.icons_applied = Some(self.prefs.more.large_icons);
        }
        // The section prompt's "don't ask again" is kept with the warnings.
        let dismissed = self.prefs.general.dismissed.contains(WARN_SECTION_NAME);
        if !self.prefs.started {
            self.extra.section_no_prompt = dismissed;
        } else if self.extra.section_no_prompt != dismissed {
            if self.extra.section_no_prompt {
                self.prefs
                    .general
                    .dismissed
                    .insert(WARN_SECTION_NAME.into());
            } else {
                self.prefs.general.dismissed.remove(WARN_SECTION_NAME);
            }
            self.save_preferences();
        }
        if self.prefs.started || !self.splash.is_done() {
            if !self.splash.is_done() {
                ctx.request_repaint();
            }
            return;
        }
        self.prefs.started = true;
        let w = crate::welcome::Welcome::load();
        if (self.prefs.general.welcome
            && self.loading.is_none()
            && self.document.as_ref().is_none_or(|d| !d.has_file))
            || w.has_recovered()
        {
            self.prefs.welcome = Some(w);
        }
    }

    /// Per frame: auto-save, the start-up checks, the Preferences dialog, the Welcome
    /// window, the Ask to Purge question and the problems-fixed notice.
    pub(crate) fn preferences_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        self.autosave_tick(ctx);
        self.startup(ctx);
        self.welcome_ui(ctx, frame);
        self.purge_prompt_ui(ctx, frame);
        self.notice_ui(ctx);
        let Some(mut d) = self.prefs.dialog.take() else {
            return;
        };
        let mut general = self.prefs.general.clone();
        let mut drawing = self.prefs.drawing.clone();
        let mut more = self.prefs.more.clone();
        let mut keymap_changed = false;
        let mut files_action = None;
        let capturing = d.capturing;
        let r = mt_ui::Dialog::new("preferences", "Preferences")
            .width(600.0)
            .modal(false)
            .show(ctx, |ui| {
                crate::export::dialog_visuals(ui);
                // A fixed-height body: the page list on the left, the page on the right.
                let size = egui::vec2(ui.available_width(), 380.0);
                let layout = egui::Layout::left_to_right(egui::Align::Min);
                ui.allocate_ui_with_layout(size, layout, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(110.0);
                        for (p, label) in Page::ALL {
                            if ui.selectable_label(d.page == p, label).clicked() {
                                d.page = p;
                                d.capturing = false;
                            }
                        }
                    });
                    ui.separator();
                    ui.vertical(|ui| match d.page {
                        Page::Drawing => {
                            drawing_page(ui, &mut drawing);
                            crate::prefs_pages::drawing_more(ui, &mut more);
                        }
                        Page::Applications => crate::prefs_pages::applications_page(ui, &mut more),
                        Page::General => general_page(ui, &mut general),
                        Page::Shortcuts => {
                            keymap_changed |= self.shortcuts_page(ui, &mut d, &mut files_action);
                        }
                        Page::Template => self.template_page(ui),
                        Page::Workspace => self.workspace_page(ui, &mut more),
                        Page::Accessibility => {
                            crate::prefs_pages::accessibility_page(ui, &mut more);
                        }
                        Page::Graphics => self.graphics_page(ui, &mut more),
                        Page::Compatibility => {
                            crate::prefs_pages::compatibility_page(ui, &mut more);
                        }
                        Page::Files => {
                            files_action = crate::prefs_pages::files_page(
                                ui,
                                &mut more,
                                &mut d.with_shortcuts,
                            );
                        }
                    });
                });
            });
        // Escape while recording a shortcut cancels the recording, not the dialog.
        let open = !r.closed || (capturing && ctx.input(|i| i.key_pressed(egui::Key::Escape)));
        self.set_settings(general, drawing, more, frame);
        if keymap_changed {
            self.keymap_changed();
        }
        match files_action {
            Some(crate::prefs_pages::FilesAction::Export { shortcuts }) => {
                self.export_preferences(shortcuts);
            }
            Some(crate::prefs_pages::FilesAction::Import { shortcuts }) => {
                self.import_preferences(shortcuts, frame);
            }
            None => {}
        }
        if open {
            self.prefs.dialog = Some(d);
        }
    }

    /// Takes changed settings: saves them and applies what changed.
    fn set_settings(
        &mut self,
        general: General,
        drawing: Drawing,
        more: More,
        frame: &eframe::Frame,
    ) {
        let p = &self.prefs;
        if general == p.general && drawing == p.drawing && more == p.more {
            return;
        }
        if general.dismissed.is_empty() && !p.general.dismissed.is_empty() {
            self.extra.section_no_prompt = false;
        }
        let colors = more.palette != p.more.palette;
        let highlight = more.bbox_highlight != p.more.bbox_highlight;
        self.prefs.general = general;
        self.prefs.drawing = drawing;
        self.prefs.more = more;
        self.prefs.more.apply_globals();
        self.save_preferences();
        if (colors || highlight)
            && let Some(rs) = frame.wgpu_render_state()
        {
            self.prefs.apply_axes(&mut self.spec);
            let device = rs.device.clone();
            self.rebuild(&device);
        }
    }

    /// Files > Export: writes all preferences (and the shortcuts) to a chosen file.
    fn export_preferences(&mut self, shortcuts: bool) {
        let Some(path) =
            crate::filedialog::save(&[("Preferences", &["toml"])], "meshtard-preferences.toml")
        else {
            return;
        };
        let p = &self.prefs;
        let overrides: toml::Table = diff(&p.defaults, &self.spec.keymap.bindings)
            .into_iter()
            .map(|(k, v)| (k, toml::Value::String(v)))
            .collect();
        let text = crate::prefs_pages::export_text(
            &settings_text(&p.general, &p.drawing, &p.more),
            shortcuts.then_some(&overrides),
        );
        self.message = Some(match std::fs::write(&path, text) {
            Ok(()) => format!("Exported the preferences to {}.", path.display()),
            Err(e) => format!("Could not write {}: {e}", path.display()),
        });
    }

    /// Files > Import: reads a preferences file written by Export.
    fn import_preferences(&mut self, shortcuts: bool, frame: &eframe::Frame) {
        let Some(path) = crate::filedialog::open(&[("Preferences", &["toml"])]) else {
            return;
        };
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| crate::prefs_pages::import_text(&t));
        let (table, keys) = match parsed {
            Ok(x) => x,
            Err(e) => {
                self.message = Some(format!("Could not import {}: {e}", path.display()));
                return;
            }
        };
        let (general, drawing, more) = settings_from_table(&table);
        self.set_settings(general, drawing, more, frame);
        if shortcuts && let Some(keys) = keys {
            let overrides: BTreeMap<String, String> = keys
                .into_iter()
                .filter_map(|(k, v)| Some((k, v.as_str()?.to_owned())))
                .collect();
            self.spec.keymap.bindings = self.prefs.defaults.clone();
            apply_overrides(&mut self.spec.keymap.bindings, &overrides);
            self.keymap_changed();
        }
        self.message = Some(format!("Imported the preferences from {}.", path.display()));
    }

    /// Ask to Purge All before a save.
    fn purge_prompt_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(id) = self.prefs.purge_prompt.clone() else {
            return;
        };
        let r = mt_ui::Dialog::new("ask_purge", "Meshtard")
            .width(340.0)
            .show(ctx, |ui| {
                mt_ui::dialog::message(
                    ui,
                    "Purge unused items before saving?",
                    Some("The model has components, materials or tags that are not used."),
                );
                mt_ui::dialog::buttons(ui, &["Purge", "Don't Purge", "Cancel"], Some(0))
            });
        let answer = if r.closed { Some(2) } else { r.inner };
        let Some(answer) = answer else { return };
        self.prefs.purge_prompt = None;
        if answer == 2 {
            return;
        }
        if answer == 0
            && let Some(doc) = self.document.as_mut()
            && doc.edit_whole(|m| mt_kernel::purge::purge_all(m).any())
        {
            // Indices into the purged lists may be stale (as Model Info's Purge).
            self.current_material = None;
            self.current_component = None;
            self.active_tag = 0;
            if let Some(rs) = frame.wgpu_render_state() {
                let device = rs.device.clone();
                self.rebuild(&device);
            }
        }
        self.save(ctx, &id);
    }

    /// Notify Me When Problems Are Fixed.
    fn notice_ui(&mut self, ctx: &egui::Context) {
        let Some(text) = self.prefs.notice.clone() else {
            return;
        };
        let id = egui::Id::new("mt_fixed_dont");
        let mut dont = ctx.data(|m| m.get_temp::<bool>(id)).unwrap_or(false);
        let r = mt_ui::Dialog::new("problems_fixed", "Meshtard")
            .width(320.0)
            .show(ctx, |ui| {
                mt_ui::dialog::message(ui, "Meshtard fixed problems in the model.", Some(&text));
                ui.add_space(6.0);
                ui.checkbox(&mut dont, "Don't show this again");
                mt_ui::dialog::buttons(ui, &["OK"], Some(0))
            });
        ctx.data_mut(|m| m.insert_temp(id, dont));
        if r.closed || r.inner.is_some() {
            if dont {
                self.prefs.general.dismissed.insert(WARN_FIXED.into());
                self.save_preferences();
            }
            ctx.data_mut(|m| m.remove::<bool>(id));
            self.prefs.notice = None;
        }
    }
}

/// A small grey note under an option.
fn note(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).small().weak());
}

/// The Drawing page.
fn drawing_page(ui: &mut egui::Ui, d: &mut Drawing) {
    ui.strong("Click Style");
    ui.add_space(2.0);
    for (style, label, _) in ClickStyle::ALL {
        if ui.radio(d.click_style == style, label).clicked() {
            d.click_style = style;
        }
    }
    ui.add_enabled(
        d.click_style != ClickStyle::ClickDragRelease,
        egui::Checkbox::new(&mut d.continue_line, "Continue line drawing"),
    );
    note(
        ui,
        "Auto detect: a press, drag and release draws from the press to the release; a \
         click places one point.",
    );
    ui.add_space(8.0);
    ui.strong("Miscellaneous");
    ui.checkbox(&mut d.crosshairs, "Display crosshairs");
    ui.checkbox(&mut d.no_prepick, "Disable pre-pick on Push/Pull");
}

/// The General page.
fn general_page(ui: &mut egui::Ui, g: &mut General) {
    ui.strong("Saving");
    ui.checkbox(&mut g.backup, "Create backup");
    ui.horizontal(|ui| {
        ui.checkbox(&mut g.autosave, "Auto-save every");
        ui.add_enabled(
            g.autosave,
            egui::DragValue::new(&mut g.autosave_minutes).range(1..=999),
        );
        ui.label("minutes");
    });
    ui.checkbox(&mut g.ask_purge, "Ask to purge all at save");
    ui.add_space(6.0);
    ui.strong("Check for Problems");
    ui.checkbox(
        &mut g.fix_problems,
        "Automatically fix problems on open, auto-save and save",
    );
    ui.add_enabled(
        g.fix_problems,
        egui::Checkbox::new(&mut g.notify_fixed, "Notify me when problems are fixed"),
    );
    ui.add_space(6.0);
    ui.strong("Warning Messages");
    if ui
        .add_enabled(
            !g.dismissed.is_empty(),
            egui::Button::new("Reset All Warning Messages"),
        )
        .clicked()
    {
        g.dismissed.clear();
    }
    ui.add_space(6.0);
    ui.strong("Startup");
    ui.checkbox(&mut g.welcome, "Show Welcome window");
}

impl App {
    /// The Shortcuts page. Returns whether the keymap changed; Import / Export go to
    /// `action` (they read and write the whole preferences file, shortcuts included,
    /// like SketchUp's Preferences.dat).
    ///
    /// Layout after SketchUp for Windows (help: creating-and-editing-shortcuts,
    /// importing-and-exporting-shortcuts [doc]): Filter, the Function list with the
    /// Assigned key, a selected command's key set by pressing it, a warning with
    /// Keep Original / Replace when another command has that key, Remove, Reset All,
    /// Import and Export. SketchUp types the key into an Add Shortcut box and adds it
    /// with +; here the key is recorded when pressed. Show All opens Help > Keyboard
    /// Shortcuts (this app's own).
    fn shortcuts_page(
        &mut self,
        ui: &mut egui::Ui,
        d: &mut Dialog,
        action: &mut Option<crate::prefs_pages::FilesAction>,
    ) -> bool {
        use crate::shortcut_list::{key_label, matches};
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Filter:");
            ui.text_edit_singleline(&mut d.filter);
        });
        let filter = d.filter.to_lowercase();
        let rows = self.shortcut_rows();
        egui::ScrollArea::vertical()
            .max_height(230.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("mt_pref_shortcuts")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("Function");
                        ui.strong("Assigned");
                        ui.end_row();
                        for r in &rows {
                            let key = key_label(&self.spec.keymap.bindings, &r.id);
                            if !matches(r, &key, &filter) {
                                continue;
                            }
                            let selected = d.selected.as_deref() == Some(r.id.as_str());
                            let resp = ui
                                .selectable_label(selected, &r.function)
                                .on_hover_text(&r.id);
                            if resp.clicked() {
                                d.selected = Some(r.id.clone());
                                d.capturing = false;
                                d.conflict = None;
                            }
                            ui.label(key);
                            ui.end_row();
                        }
                    });
            });
        ui.separator();
        let function = |id: &str| {
            rows.iter()
                .find(|r| r.id == id)
                .map_or_else(|| id.to_owned(), |r| r.function.clone())
        };
        if let Some(id) = d.selected.clone() {
            if let Some((s, other)) = d.conflict.clone() {
                ui.label(format!(
                    "{} is already assigned to {}.",
                    s.label(),
                    function(&other)
                ));
                ui.horizontal(|ui| {
                    if ui.button("Keep Original").clicked() {
                        d.conflict = None;
                    }
                    if ui.button("Replace").clicked() {
                        rebind(&mut self.spec.keymap.bindings, &id, Some(&s));
                        d.note = Some(format!(
                            "{} was removed from {}.",
                            s.label(),
                            function(&other)
                        ));
                        d.conflict = None;
                        changed = true;
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    ui.label(format!("{}:", function(&id)));
                    let current = key_label(&self.spec.keymap.bindings, &id);
                    let text = if d.capturing {
                        "Press keys...".to_owned()
                    } else if current.is_empty() {
                        "(none)".to_owned()
                    } else {
                        current
                    };
                    let r = ui
                        .button(text)
                        .on_hover_text("Click, then press the new shortcut (Escape cancels).");
                    if r.clicked() {
                        d.capturing = true;
                        d.note = None;
                    }
                    if d.capturing {
                        // Focus keeps the app's shortcuts and Measurements typing away
                        // from the keys pressed.
                        r.request_focus();
                        if let Some(pressed) = captured(ui) {
                            d.capturing = false;
                            r.surrender_focus();
                            if let Some(s) = pressed {
                                match holder(&self.spec.keymap.bindings, &id, &s) {
                                    Some(other) => d.conflict = Some((s, other)),
                                    None => {
                                        rebind(&mut self.spec.keymap.bindings, &id, Some(&s));
                                        changed = true;
                                    }
                                }
                            }
                        }
                    }
                    if ui.button("Remove").clicked() {
                        rebind(&mut self.spec.keymap.bindings, &id, None);
                        changed = true;
                    }
                });
            }
        } else {
            ui.label("Select a function to change its shortcut.");
        }
        if let Some(note) = &d.note {
            ui.label(egui::RichText::new(note).small());
        }
        ui.horizontal(|ui| {
            if ui.button("Reset All").clicked() {
                self.spec.keymap.bindings = self.prefs.defaults.clone();
                d.conflict = None;
                d.note = Some("All shortcuts are back to the defaults.".into());
                changed = true;
            }
            if ui.button("Import...").clicked() {
                *action = Some(crate::prefs_pages::FilesAction::Import { shortcuts: true });
            }
            if ui.button("Export...").clicked() {
                *action = Some(crate::prefs_pages::FilesAction::Export { shortcuts: true });
            }
            if ui.button("Show All...").clicked() {
                self.open_shortcut_list();
            }
        });
        changed
    }
}

/// While recording a shortcut: the first non-modifier key pressed this frame;
/// `Some(None)` for a plain Escape (cancel).
fn captured(ui: &egui::Ui) -> Option<Option<Shortcut>> {
    let (key, m) = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } if !is_modifier(*key) => Some((*key, *modifiers)),
            _ => None,
        })
    })?;
    if key == egui::Key::Escape && !m.any() {
        return Some(None);
    }
    Some(Some(Shortcut {
        ctrl: m.command,
        shift: m.shift,
        alt: m.alt,
        key: key.name().to_owned(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect()
    }

    #[test]
    fn overrides_round_trip_through_text() {
        let o = map(&[("tool.line", "Ctrl+Shift+L"), ("tool.move", "")]);
        let text = overrides_text(&o);
        assert_eq!(parse_overrides(&text).unwrap(), o);
        assert!(parse_overrides("[bindings]\n\"tool.line\" = \"Hyper+L\"").is_err());
        assert!(parse_overrides("").unwrap().is_empty());
    }

    #[test]
    fn rebinding_moves_a_taken_shortcut() {
        let defaults = map(&[("tool.line", "L"), ("tool.move", "M"), ("tool.eraser", "E")]);
        let mut b = defaults.clone();
        let m = Shortcut::parse("M").unwrap();
        assert_eq!(
            rebind(&mut b, "tool.line", Some(&m)),
            Some("tool.move".into())
        );
        assert_eq!(b, map(&[("tool.line", "M"), ("tool.eraser", "E")]));
        rebind(&mut b, "tool.eraser", None);
        let o = diff(&defaults, &b);
        assert_eq!(
            o,
            map(&[("tool.line", "M"), ("tool.move", ""), ("tool.eraser", "")])
        );
        // Applying the saved differences to the defaults gives the same keymap.
        let mut again = defaults.clone();
        apply_overrides(&mut again, &o);
        assert_eq!(again, b);
    }

    /// The Shortcuts page asks Keep Original / Replace when the key is taken.
    #[test]
    fn holder_finds_the_other_command() {
        let b = map(&[("tool.line", "L"), ("tool.move", "M")]);
        let m = Shortcut::parse("M").unwrap();
        assert_eq!(holder(&b, "tool.line", &m), Some("tool.move".into()));
        // A command's own key is no conflict.
        assert_eq!(holder(&b, "tool.move", &m), None);
        assert_eq!(
            holder(&b, "tool.line", &Shortcut::parse("K").unwrap()),
            None
        );
    }

    #[test]
    fn settings_round_trip() {
        let g = General {
            autosave: false,
            autosave_minutes: 12,
            backup: false,
            ask_purge: false,
            fix_problems: false,
            notify_fixed: true,
            welcome: false,
            dismissed: [WARN_FIXED.to_owned()].into(),
        };
        let d = Drawing {
            click_style: ClickStyle::ClickMoveClick,
            continue_line: false,
            crosshairs: true,
            no_prepick: true,
        };
        let m = More {
            invert_wheel: true,
            ..Default::default()
        };
        let back = settings_from_text(&settings_text(&g, &d, &m));
        assert_eq!(back, (g, d, m));
        let (g, d, m) = settings_from_text("nonsense =");
        assert_eq!(g, General::default());
        assert_eq!(d, Drawing::default());
        assert_eq!(m, More::default());
        // A file from before the other pages still reads its General keys.
        let (g, _, _) = settings_from_text(
            "autosave = false
backup = true
",
        );
        assert!(!g.autosave && g.backup && g.welcome);
    }

    #[test]
    fn backup_name() {
        let p = Path::new("/models/house.skp");
        assert_eq!(backup_path(p), Path::new("/models/house.skb"));
    }
}
