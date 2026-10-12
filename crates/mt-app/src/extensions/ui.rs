//! The extension windows and the app glue: Extension Manager, Extension Warehouse,
//! the install prompt (permissions), Window > Extension Console, script input
//! dialogs and messages; running commands, click tools and observers each frame.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use eframe::egui;

use super::catalog::{self, Catalog, Entry};
use super::manifest::Permission;
use super::package::{self, Package};
use super::runtime::{Field, FieldKind, Outcome};
use super::{Extensions, intern};
use crate::App;
use crate::nav::Tool;

/// What the windows remember between frames.
#[derive(Default)]
pub(crate) struct UiState {
    /// A script's status text for the status bar.
    pub status: Option<String>,
    pub manager_open: bool,
    pub selected: Option<usize>,
    pub catalog_url: String,
    pub warehouse: Option<Warehouse>,
    pub install: Option<InstallPrompt>,
    /// Toolbar icons by command id, and the commands they were made for.
    icons: HashMap<String, egui::TextureHandle>,
    icons_for: Option<String>,
    history_pos: Option<usize>,
}

pub(crate) enum CatalogState {
    Loading(Receiver<Result<Catalog, String>>),
    Ready(Catalog),
    Failed(String),
}

/// A package being fetched, and its catalog entry.
pub(crate) type Download = (Entry, Receiver<Result<Vec<u8>, String>>);

pub(crate) struct Warehouse {
    pub url: String,
    pub state: CatalogState,
    pub query: String,
    pub selected: Option<usize>,
    /// A download being fetched, and its entry.
    pub download: Option<Download>,
    pub error: Option<String>,
}

pub(crate) struct InstallPrompt {
    pub pkg: Package,
    /// Where it came from (file, or URL with the checksum checked).
    pub source: String,
    /// One per sensitive permission: allowed.
    pub allow: Vec<(Permission, bool)>,
    /// The version installed now, if any.
    pub replaces: Option<String>,
}

impl InstallPrompt {
    pub(crate) fn new(pkg: Package, source: String, ext: &Extensions) -> Self {
        let allow = pkg
            .manifest
            .sensitive()
            .into_iter()
            .map(|p| (p, false))
            .collect();
        let replaces = ext
            .installed
            .iter()
            .filter_map(|i| i.manifest.as_ref().ok())
            .find(|m| m.id == pkg.manifest.id)
            .map(|m| m.version.clone());
        Self {
            pkg,
            source,
            allow,
            replaces,
        }
    }
}

/// The text a field shows (lengths in the model's units).
pub(crate) fn field_text(
    f: &Field,
    units: Option<(mt_infer::units::LengthUnit, &mt_infer::units::UnitsOptions)>,
) -> String {
    match &f.kind {
        FieldKind::Text(s) => s.clone(),
        FieldKind::Number(x) => trim_float(*x),
        FieldKind::Length(x) => match units {
            Some((u, o)) => mt_infer::units::format_length(*x, u, o),
            None => trim_float(*x),
        },
        FieldKind::Choice(_, _) => String::new(),
    }
}

fn trim_float(x: f64) -> String {
    let s = format!("{x:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Reads the dialog's text back into its fields; an error names a bad field.
pub(crate) fn read_fields(
    fields: &mut [Field],
    texts: &[String],
    initial: &[String],
    unit: mt_infer::units::LengthUnit,
) -> Result<(), String> {
    for (i, (f, t)) in fields.iter_mut().zip(texts).enumerate() {
        if initial.get(i) == Some(t) {
            continue;
        }
        match &mut f.kind {
            FieldKind::Text(s) => *s = t.clone(),
            FieldKind::Number(x) => {
                *x = t
                    .trim()
                    .replace(',', ".")
                    .parse()
                    .map_err(|_| format!("{}: not a number", f.label))?;
            }
            FieldKind::Length(x) => {
                // Shown lengths may be marked approximate ("~ 35.43\"").
                let t = t.trim().trim_start_matches('~');
                *x = mt_infer::units::parse_length(t, unit)
                    .ok_or_else(|| format!("{}: not a length", f.label))?;
            }
            FieldKind::Choice(_, _) => {}
        }
    }
    Ok(())
}

impl App {
    /// Loads the installed extensions after the shell exists (App::new). Failures
    /// are reported in the manager and the console; startup goes on.
    pub(crate) fn load_extensions(&mut self) {
        let root = package::extensions_dir();
        let config = package::Config::load();
        self.extensions = Extensions::new(root, config);
        self.extensions.ui.catalog_url = self.extensions.config.catalog_url.clone();
        self.extensions.load_all();
        self.sync_extensions();
        // The extension toolbar's saved place (it did not exist when the saved
        // layout was first applied).
        if let (Some(shell), Some(layout)) =
            (self.shell.as_mut(), crate::settings::load_toolbar_layout())
        {
            shell.set_toolbar_layout(layout);
        }
    }

    /// Puts the loaded extensions' commands into the menus, the toolbar and the
    /// shortcut table.
    pub(crate) fn sync_extensions(&mut self) {
        if self.extensions.apply_to_spec(&mut self.spec) {
            if let Some(shell) = self.shell.as_mut() {
                shell.sync_toolbars(&self.spec);
            }
            for (id, key) in self.extensions.default_shortcuts() {
                let taken = (self.spec.keymap.bindings.values()).any(|v| *v == key);
                if !self.spec.keymap.bindings.contains_key(&id) && !taken {
                    self.spec.keymap.bindings.insert(id.clone(), key.clone());
                    self.prefs.defaults.insert(id, key);
                }
            }
            self.add_extension_shortcuts();
            self.extensions.ui.icons_for = None;
        }
    }

    /// The shortcut table with the extension commands added.
    pub(crate) fn add_extension_shortcuts(&mut self) {
        let mut ids: Vec<&'static str> = crate::IMPLEMENTED.to_vec();
        ids.extend(self.extensions.command_ids().iter().map(|s| intern(s)));
        self.shortcuts = crate::preferences::shortcut_table(&self.spec, &ids);
    }

    pub(crate) fn extensions_command(
        &mut self,
        ctx: &egui::Context,
        frame: &eframe::Frame,
        id: &str,
    ) -> bool {
        match id {
            "extensions.manager" => {
                self.extensions.ui.manager_open = true;
                return true;
            }
            "extensions.warehouse" => {
                self.open_warehouse(ctx);
                return true;
            }
            "window.extension_console" => {
                self.extensions.console_open = !self.extensions.console_open;
                return true;
            }
            _ => {}
        }
        if super::runtime::split_command_id(id).is_none() {
            return false;
        }
        let sel = self.selection.items().to_vec();
        if let Some(out) = self
            .extensions
            .run_command(id, self.document.as_mut(), &sel)
        {
            self.apply_outcomes(frame, vec![out]);
        } else if self.extensions.command(id).is_some() {
            // A click tool: start from Select's clean state, then switch.
            self.run_command(ctx, frame, "tool.select");
            if let Some(tool) = self.extensions.start_tool(id) {
                self.tool = Tool::Draw(intern(id));
                self.draw = Some(Box::new(tool));
            }
        }
        true
    }

    /// Applies what scripts did: rebuilds after model changes, sets the selection,
    /// shows status text.
    fn apply_outcomes(&mut self, frame: &eframe::Frame, outs: Vec<Outcome>) {
        let mut rebuild = false;
        for out in outs {
            rebuild |= out.changed;
            if let Some(sel) = out.selection {
                self.selection.clear();
                self.selection.apply(crate::selection::Mode::Replace, sel);
                rebuild = true;
            }
        }
        if let Some(s) = self.extensions.ui.status.take() {
            self.message = Some(s);
        }
        if rebuild {
            if let Some(doc) = &self.document {
                let ents = doc.active();
                self.selection.retain(|o| crate::selection::exists(ents, o));
            }
            if let Some(rs) = frame.wgpu_render_state() {
                let device = rs.device.clone();
                self.rebuild(&device);
            }
        }
        // Commands registered while running.
        self.sync_extensions();
    }

    /// Every frame: HTTP answers, the click tool, observers, then the windows.
    pub(crate) fn extensions_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        // A click tool that is no longer the active tool ends.
        if self.extensions.tool.is_some()
            && !matches!(self.tool, Tool::Draw(id) if super::runtime::split_command_id(id).is_some())
        {
            self.extensions.tool = None;
        }
        let sel = self.selection.items().to_vec();
        let mut outs = self.extensions.poll_http(self.document.as_mut(), &sel);
        outs.extend(self.extensions.tool_events(self.document.as_mut(), &sel));
        if !outs.is_empty() {
            self.apply_outcomes(frame, outs);
        }
        let sel = self.selection.items().to_vec();
        let (outs, wait) =
            self.extensions
                .observe(self.document.as_mut(), &sel, std::time::Instant::now());
        if !outs.is_empty() {
            self.apply_outcomes(frame, outs);
        }
        if let Some(w) = wait {
            ctx.request_repaint_after(w);
        }
        if !self.extensions.http.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.extension_icons(ctx);
        self.extension_errors();
        self.extension_manager_ui(ctx);
        self.warehouse_ui(ctx);
        self.install_prompt_ui(ctx);
        self.console_ui(ctx, frame);
        self.input_dialog_ui(ctx, frame);
        self.message_ui(ctx);
    }

    /// Loads the toolbar PNGs of extension commands into textures for the shell.
    fn extension_icons(&mut self, ctx: &egui::Context) {
        let ids = self.extensions.command_ids().join("|");
        if self.extensions.ui.icons_for.as_deref() == Some(ids.as_str()) {
            return;
        }
        let mut icons = HashMap::new();
        for id in self.extensions.command_ids() {
            let Some((ext, cmd)) = self.extensions.command(&id) else {
                continue;
            };
            let Some(icon) = &cmd.icon else { continue };
            let Some(s) = self
                .extensions
                .scripts
                .iter()
                .find(|s| s.manifest.id == ext)
            else {
                continue;
            };
            match image::open(s.dir.join(icon)) {
                Ok(img) => {
                    let img = img.into_rgba8();
                    let size = [img.width() as usize, img.height() as usize];
                    let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                    let tex = ctx.load_texture(
                        format!("mt_ext_icon_{id}"),
                        color,
                        egui::TextureOptions::LINEAR,
                    );
                    icons.insert(id, tex);
                }
                Err(e) => self
                    .extensions
                    .log_line(&ext, &format!("icon {icon}: {e}"), true),
            }
        }
        if let Some(shell) = self.shell.as_mut() {
            shell.set_custom_icons(icons.iter().map(|(k, v)| (k.clone(), v.id())).collect());
        }
        self.extensions.ui.icons = icons;
        self.extensions.ui.icons_for = Some(ids);
    }

    /// Script errors go to the app's error dialog, one at a time.
    fn extension_errors(&mut self) {
        if self.error.is_some() || self.extensions.errors.is_empty() {
            return;
        }
        let (name, e) = self.extensions.errors.remove(0);
        self.message = Some(format!("{name}: {e}"));
        self.error = Some(crate::errors::ErrorMessage::new(
            format!("The extension \"{name}\" stopped with an error."),
            format!("{e}\n\nSee Window > Extension Console for the log."),
        ));
    }

    fn message_ui(&mut self, ctx: &egui::Context) {
        let Some((title, text)) = self.extensions.messages.first().cloned() else {
            return;
        };
        let r = mt_ui::Dialog::new("ext_message", &title)
            .width(380.0)
            .show(ctx, |ui| {
                ui.label(&text);
                mt_ui::dialog::buttons(ui, &["OK"], Some(0))
            });
        if r.closed || r.inner.is_some() {
            self.extensions.messages.remove(0);
            mt_ui::dialog::reset(ctx, "ext_message");
        }
    }

    fn input_dialog_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(p) = self.extensions.inputs.first_mut() else {
            return;
        };
        let units = self.document.as_ref().map(|d| (d.units, d.unit_options));
        // Length fields start in the model's units.
        if p.texts.len() != p.fields.len() {
            p.texts = p
                .fields
                .iter()
                .map(|f| field_text(f, units.as_ref().map(|(u, o)| (*u, o))))
                .collect();
            p.initial = p.texts.clone();
        }
        let err_id = egui::Id::new("mt_ext_input_err");
        let error: Option<String> = ctx.data(|d| d.get_temp::<String>(err_id));
        let title = p.title.clone();
        let r = mt_ui::Dialog::new("ext_input", &title)
            .width(340.0)
            .show(ctx, |ui| {
                egui::Grid::new("mt_ext_input_grid")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        for (i, f) in p.fields.iter_mut().enumerate() {
                            ui.label(&f.label);
                            match &mut f.kind {
                                FieldKind::Choice(options, sel) => {
                                    egui::ComboBox::from_id_salt(("mt_ext_choice", i))
                                        .selected_text(options[*sel].clone())
                                        .show_ui(ui, |ui| {
                                            for (k, o) in options.iter().enumerate() {
                                                ui.selectable_value(sel, k, o);
                                            }
                                        });
                                }
                                _ => {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut p.texts[i])
                                            .desired_width(180.0),
                                    );
                                }
                            }
                            ui.end_row();
                        }
                    });
                if let Some(e) = &error {
                    ui.colored_label(egui::Color32::from_rgb(196, 43, 28), e);
                }
                mt_ui::dialog::buttons(ui, &["OK", "Cancel"], Some(0))
            });
        let ok = r.inner == Some(0);
        let cancel = r.closed || r.inner == Some(1);
        if ok {
            let unit = units.map_or(mt_infer::units::LengthUnit::Inch, |(u, _)| u);
            let texts = p.texts.clone();
            if let Err(e) = read_fields(&mut p.fields, &texts, &p.initial, unit) {
                ctx.data_mut(|d| d.insert_temp(err_id, e));
                ctx.request_repaint();
                return;
            }
        }
        if ok || cancel {
            ctx.data_mut(|d| d.remove_temp::<String>(err_id));
            mt_ui::dialog::reset(ctx, "ext_input");
            let sel = self.selection.items().to_vec();
            if let Some(out) = self
                .extensions
                .answer_input(ok, self.document.as_mut(), &sel)
            {
                self.apply_outcomes(frame, vec![out]);
            }
        }
    }

    fn console_ui(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if !self.extensions.console_open {
            return;
        }
        let mut run: Option<String> = None;
        let mut clear = false;
        let ext = &mut self.extensions;
        let r = mt_ui::Dialog::new("ext_console", "Extension Console")
            .width(600.0)
            .modal(false)
            .show(ctx, |ui| {
                ui.weak(
                    "Meshtard addition: a Rhai console for the active model (not SketchUp's Ruby Console).",
                );
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for l in ext.log.borrow().iter() {
                            let text = egui::RichText::new(format!("[{}] {}", l.source, l.text))
                                .monospace();
                            if l.error {
                                ui.label(text.color(egui::Color32::from_rgb(196, 43, 28)));
                            } else {
                                ui.label(text);
                            }
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut ext.console_input)
                            .font(egui::TextStyle::Monospace)
                            .hint_text("Rhai, e.g. faces().len()")
                            .desired_width(470.0),
                    );
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        run = Some(ext.console_input.clone());
                        edit.request_focus();
                    }
                    if edit.has_focus() {
                        let (up, down) = ui.input(|i| {
                            (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown))
                        });
                        let n = ext.console_history.len();
                        if (up || down) && n > 0 {
                            let pos = match (ext.ui.history_pos, up) {
                                (None, true) => n - 1,
                                (Some(p), true) => p.saturating_sub(1),
                                (Some(p), false) => (p + 1).min(n - 1),
                                (None, false) => n - 1,
                            };
                            ext.ui.history_pos = Some(pos);
                            ext.console_input = ext.console_history[pos].clone();
                        }
                    }
                    if ui.button("Run").clicked() {
                        run = Some(ext.console_input.clone());
                    }
                    if ui.button("Clear").clicked() {
                        clear = true;
                    }
                });
            });
        if r.closed {
            self.extensions.console_open = false;
        }
        if clear {
            self.extensions.log.borrow_mut().clear();
        }
        if let Some(line) = run.filter(|l| !l.trim().is_empty()) {
            self.extensions.console_history.push(line.clone());
            self.extensions.ui.history_pos = None;
            self.extensions.console_input.clear();
            let sel = self.selection.items().to_vec();
            let out = self
                .extensions
                .console_eval(&line, self.document.as_mut(), &sel);
            self.apply_outcomes(frame, vec![out]);
        }
    }

    fn extension_manager_ui(&mut self, ctx: &egui::Context) {
        if !self.extensions.ui.manager_open {
            return;
        }
        #[derive(Default)]
        struct Act {
            toggle: Option<(String, bool)>,
            uninstall: Option<(PathBuf, Option<String>)>,
            from_file: bool,
            from_folder: bool,
            warehouse: bool,
            console: bool,
            reload: bool,
            save_url: bool,
        }
        let mut act = Act::default();
        let ext = &mut self.extensions;
        let r = mt_ui::Dialog::new("ext_manager", "Extension Manager")
            .width(560.0)
            .modal(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    act.from_file = ui.button("Install from File...").clicked();
                    act.from_folder = ui.button("Install from Folder...").clicked();
                    act.warehouse = ui.button("Extension Warehouse").clicked();
                    act.console = ui.button("Console").clicked();
                    act.reload = ui.button("Reload All").clicked();
                });
                ui.separator();
                if ext.installed.is_empty() {
                    ui.weak("No extensions installed.");
                }
                egui::ScrollArea::vertical()
                    .id_salt("mt_ext_list")
                    .max_height(200.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for (i, item) in ext.installed.iter().enumerate() {
                            ui.horizontal(|ui| {
                                let mut on = item.enabled;
                                let id = item.manifest.as_ref().ok().map(|m| m.id.clone());
                                if ui
                                    .add_enabled(id.is_some(), egui::Checkbox::without_text(&mut on))
                                    .on_hover_text("Enabled")
                                    .changed()
                                    && let Some(id) = id
                                {
                                    act.toggle = Some((id, on));
                                }
                                let label = match &item.manifest {
                                    Ok(m) => format!("{}  {}  ({})", m.name, m.version, m.author),
                                    Err(_) => item.dir.file_name().map_or_else(String::new, |n| {
                                        n.to_string_lossy().into_owned()
                                    }),
                                };
                                let label = if item.error.is_some() {
                                    egui::RichText::new(format!("{label}  (error)"))
                                        .color(egui::Color32::from_rgb(196, 43, 28))
                                } else {
                                    egui::RichText::new(label)
                                };
                                if ui
                                    .selectable_label(ext.ui.selected == Some(i), label)
                                    .clicked()
                                {
                                    ext.ui.selected = Some(i);
                                }
                            });
                        }
                    });
                if let Some(item) = ext.ui.selected.and_then(|i| ext.installed.get(i)) {
                    ui.separator();
                    match &item.manifest {
                        Ok(m) => {
                            ui.strong(&m.name);
                            if !m.description.is_empty() {
                                ui.label(&m.description);
                            }
                            egui::Grid::new("mt_ext_details").num_columns(2).show(ui, |ui| {
                                for (k, v) in [
                                    ("Id", m.id.as_str()),
                                    ("Version", m.version.as_str()),
                                    ("Author", m.author.as_str()),
                                    ("License", m.license.as_str()),
                                    ("Homepage", m.homepage.as_str()),
                                ] {
                                    if !v.is_empty() {
                                        ui.label(k);
                                        ui.label(v);
                                        ui.end_row();
                                    }
                                }
                                ui.label("Folder");
                                ui.label(item.dir.display().to_string());
                                ui.end_row();
                            });
                            ui.label("Permissions:");
                            let approved = ext
                                .config
                                .extensions
                                .get(&m.id)
                                .map(|s| s.approved.clone())
                                .unwrap_or_default();
                            for p in &m.permissions {
                                let granted = !p.sensitive() || approved.contains(p);
                                let mark = if granted { "granted" } else { "not approved" };
                                ui.label(format!("  {} - {} ({mark})", p.key(), p.describe()));
                            }
                            if m.permissions.is_empty() {
                                ui.weak("  none");
                            }
                        }
                        Err(_) => {
                            ui.label(item.dir.display().to_string());
                        }
                    }
                    if let Some(e) = &item.error {
                        ui.colored_label(egui::Color32::from_rgb(196, 43, 28), e);
                    }
                    if ui.button("Uninstall").clicked() {
                        act.uninstall = Some((
                            item.dir.clone(),
                            item.manifest.as_ref().ok().map(|m| m.id.clone()),
                        ));
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Warehouse catalog URL:");
                    ui.add(
                        egui::TextEdit::singleline(&mut ext.ui.catalog_url)
                            .hint_text(catalog::DEFAULT_CATALOG)
                            .desired_width(300.0),
                    );
                    act.save_url = ui.button("Save").clicked();
                });
                ui.weak("Empty: the Meshtard repository's catalog. file:// URLs and local paths work too.");
            });
        if r.closed {
            self.extensions.ui.manager_open = false;
        }
        let mut changed = false;
        if let Some((id, on)) = act.toggle {
            self.extensions.set_enabled(&id, on);
            changed = true;
        }
        if let Some((dir, id)) = act.uninstall {
            match self.extensions.uninstall(&dir, id.as_deref()) {
                Ok(()) => self.extensions.ui.selected = None,
                Err(e) => self.extensions.errors.push(("Extension Manager".into(), e)),
            }
            changed = true;
        }
        if act.reload {
            self.extensions.load_all();
            changed = true;
        }
        if act.save_url {
            self.extensions.config.catalog_url = self.extensions.ui.catalog_url.trim().to_owned();
            self.extensions.config.save();
        }
        if act.console {
            self.extensions.console_open = true;
        }
        if act.warehouse {
            self.open_warehouse(ctx);
        }
        let picked = if act.from_file {
            crate::filedialog::open(&[("Meshtard extension", &["mtx"])])
        } else if act.from_folder {
            crate::filedialog::pick_folder(None)
        } else {
            None
        };
        if let Some(path) = picked {
            match Package::read(&path) {
                Ok(pkg) => {
                    let p = InstallPrompt::new(pkg, path.display().to_string(), &self.extensions);
                    self.extensions.ui.install = Some(p);
                }
                Err(e) => self
                    .extensions
                    .errors
                    .push(("Install".into(), format!("{}: {e}", path.display()))),
            }
        }
        if changed {
            self.after_extensions_changed();
        }
    }

    fn after_extensions_changed(&mut self) {
        // The click tool of an unloaded extension ends.
        if let Some(t) = &self.extensions.tool
            && self.extensions.command(&t.command).is_none()
        {
            self.extensions.tool = None;
            self.tool = Tool::Select;
            self.draw = None;
        }
        self.sync_extensions();
    }

    pub(crate) fn open_warehouse(&mut self, ctx: &egui::Context) {
        let url = match self.extensions.config.catalog_url.trim() {
            "" => catalog::DEFAULT_CATALOG.to_owned(),
            u => u.to_owned(),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx2 = ctx.clone();
        let u = url.clone();
        std::thread::spawn(move || {
            let r = catalog::fetch(&u).and_then(|b| Catalog::parse(&String::from_utf8_lossy(&b)));
            let _ = tx.send(r);
            ctx2.request_repaint();
        });
        self.extensions.ui.warehouse = Some(Warehouse {
            url,
            state: CatalogState::Loading(rx),
            query: String::new(),
            selected: None,
            download: None,
            error: None,
        });
    }

    fn warehouse_ui(&mut self, ctx: &egui::Context) {
        let Some(w) = self.extensions.ui.warehouse.as_mut() else {
            return;
        };
        if let CatalogState::Loading(rx) = &w.state
            && let Ok(r) = rx.try_recv()
        {
            w.state = match r {
                Ok(c) => CatalogState::Ready(c),
                Err(e) => CatalogState::Failed(e),
            };
        }
        // A finished download: verify, read, ask for permissions.
        let mut prompt = None;
        if let Some((entry, rx)) = &w.download
            && let Ok(r) = rx.try_recv()
        {
            let entry = entry.clone();
            w.download = None;
            let checked = r.and_then(|bytes| {
                catalog::verify(&entry, &bytes)?;
                let pkg = Package::from_zip(&bytes)?;
                if pkg.manifest.id != entry.id || pkg.manifest.version != entry.version {
                    return Err(format!(
                        "the package is {} {}, the catalog says {} {}; not installed",
                        pkg.manifest.id, pkg.manifest.version, entry.id, entry.version
                    ));
                }
                Ok(pkg)
            });
            match checked {
                Ok(pkg) => {
                    prompt = Some((
                        pkg,
                        format!(
                            "{} (SHA-256 verified)",
                            catalog::resolve(&w.url, &entry.url)
                        ),
                    ));
                }
                Err(e) => w.error = Some(e),
            }
        }
        let mut install: Option<Entry> = None;
        let mut retry = false;
        let r = mt_ui::Dialog::new("ext_warehouse", "Extension Warehouse")
            .width(560.0)
            .modal(false)
            .show(ctx, |ui| {
                ui.weak(format!("Catalog: {}", w.url));
                match &w.state {
                    CatalogState::Loading(_) => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Loading the catalog...");
                        });
                    }
                    CatalogState::Failed(e) => {
                        ui.colored_label(egui::Color32::from_rgb(196, 43, 28), e);
                        ui.label("Set another catalog URL in the Extension Manager.");
                        retry = ui.button("Retry").clicked();
                    }
                    CatalogState::Ready(c) => {
                        ui.horizontal(|ui| {
                            ui.label("Search:");
                            ui.add(egui::TextEdit::singleline(&mut w.query).desired_width(300.0));
                        });
                        let shown = catalog::search(&c.extensions, &w.query);
                        egui::ScrollArea::vertical()
                            .id_salt("mt_wh_list")
                            .max_height(180.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                for i in shown {
                                    let e = &c.extensions[i];
                                    let text = format!("{}  {}  - {}", e.name, e.version, e.author);
                                    if ui.selectable_label(w.selected == Some(i), text).clicked() {
                                        w.selected = Some(i);
                                    }
                                }
                            });
                        if let Some(e) = w.selected.and_then(|i| c.extensions.get(i)) {
                            ui.separator();
                            ui.strong(&e.name);
                            ui.label(&e.description);
                            egui::Grid::new("mt_wh_details")
                                .num_columns(2)
                                .show(ui, |ui| {
                                    for (k, v) in [
                                        ("Id", e.id.clone()),
                                        ("Version", e.version.clone()),
                                        ("Author", e.author.clone()),
                                        ("License", e.license.clone()),
                                        ("Size", crate::library::format_size(e.size)),
                                        ("Permissions", e.permissions.join(", ")),
                                        ("SHA-256", e.sha256.clone()),
                                    ] {
                                        ui.label(k);
                                        ui.label(v);
                                        ui.end_row();
                                    }
                                });
                            if w.download.is_some() {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label("Downloading...");
                                });
                            } else if ui.button("Install...").clicked() {
                                install = Some(e.clone());
                            }
                        }
                    }
                }
                if let Some(e) = &w.error {
                    ui.colored_label(egui::Color32::from_rgb(196, 43, 28), e);
                }
            });
        if let Some(e) = install {
            w.error = None;
            let url = catalog::resolve(&w.url, &e.url);
            let (tx, rx) = std::sync::mpsc::channel();
            let ctx2 = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(catalog::fetch(&url));
                ctx2.request_repaint();
            });
            w.download = Some((e, rx));
        }
        let closed = r.closed;
        if closed {
            self.extensions.ui.warehouse = None;
        }
        if retry {
            self.open_warehouse(ctx);
        }
        if let Some((pkg, source)) = prompt {
            self.extensions.ui.install = Some(InstallPrompt::new(pkg, source, &self.extensions));
        }
    }

    fn install_prompt_ui(&mut self, ctx: &egui::Context) {
        let Some(p) = self.extensions.ui.install.as_mut() else {
            return;
        };
        let m = p.pkg.manifest.clone();
        let r = mt_ui::Dialog::new("ext_install", "Install Extension")
            .width(440.0)
            .show(ctx, |ui| {
                ui.strong(format!("{} {}", m.name, m.version));
                if !m.author.is_empty() {
                    ui.label(format!("by {}", m.author));
                }
                if !m.description.is_empty() {
                    ui.label(&m.description);
                }
                if !m.license.is_empty() {
                    ui.label(format!("License: {}", m.license));
                }
                ui.weak(format!("From: {}", p.source));
                if let Some(v) = &p.replaces {
                    ui.label(format!("Replaces the installed version {v}."));
                }
                ui.add_space(6.0);
                ui.label("This extension asks to:");
                for perm in &m.permissions {
                    if perm.sensitive() {
                        let allow = p
                            .allow
                            .iter_mut()
                            .find(|(q, _)| q == perm)
                            .map(|(_, a)| a);
                        if let Some(a) = allow {
                            ui.checkbox(a, format!("{} (allow)", perm.describe()));
                        }
                    } else {
                        ui.label(format!("  - {}", perm.describe()));
                    }
                }
                if m.permissions.is_empty() {
                    ui.weak("  nothing (no permissions)");
                }
                ui.weak("Extensions run sandboxed Rhai scripts; internet and file access only when allowed here.");
                mt_ui::dialog::buttons(ui, &["Install", "Cancel"], Some(0))
            });
        let install = r.inner == Some(0);
        if r.closed || r.inner.is_some() {
            mt_ui::dialog::reset(ctx, "ext_install");
            let p = self.extensions.ui.install.take().expect("shown");
            if install {
                let approved = p
                    .allow
                    .iter()
                    .filter(|(_, a)| *a)
                    .map(|(q, _)| *q)
                    .collect();
                match self.extensions.install(&p.pkg, approved) {
                    Ok(()) => {
                        self.message = Some(format!("Installed {} {}.", m.name, m.version));
                        self.extensions.ui.manager_open = true;
                    }
                    Err(e) => self.extensions.errors.push(("Install".into(), e)),
                }
                self.after_extensions_changed();
            }
        }
    }
}
