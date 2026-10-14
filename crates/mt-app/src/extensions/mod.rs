//! Meshtard extensions (Extensions > Extension Manager, Extension Warehouse, Window >
//! Extension Console): Rhai scripts with a manifest and permissions. Design and API:
//! `spec/EXTENSIONS.md`. Not SketchUp's Ruby API (`differs` in STATUS.md).
//!
//! - [`manifest`]: `extension.toml` and permissions.
//! - [`package`]: `.mtx` / folder packages, the install folder, enabled state.
//! - [`runtime`]: the Rhai engines, the model facade, budgets and rollback.
//! - [`catalog`]: the Extension Warehouse catalog, SHA-256 checks and updates.
//! - [`signing`]: ed25519 signatures of packages and catalogs, trusted keys.
//! - [`ui`]: the windows, dialogs and the app glue.

pub(crate) mod catalog;
pub(crate) mod manifest;
pub(crate) mod package;
pub(crate) mod runtime;
pub(crate) mod signing;
pub(crate) mod ui;

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;
use std::time::Instant;

use glam::DVec3;
use rhai::Dynamic;

use crate::document::{Document, Owner};
use manifest::{Manifest, Permission, ToolCursor};
use package::Config;
use runtime::{Action, CommandDef, LogLine, ObserverKind, Outcome, Request, Script};
use signing::SigStatus;

/// An installed extension as the Extension Manager lists it.
#[derive(Debug)]
pub(crate) struct Installed {
    pub dir: PathBuf,
    pub manifest: Result<Manifest, String>,
    pub enabled: bool,
    /// Why it did not load.
    pub error: Option<String>,
    /// Its `signature.json` checked against the installed files.
    pub signature: SigStatus,
}

/// An input dialog a script asked for.
#[derive(Debug)]
pub(crate) struct PendingInput {
    pub ext: String,
    pub title: String,
    pub fields: Vec<runtime::Field>,
    pub callback: rhai::FnPtr,
    /// Edit buffers for the text, number and length fields.
    pub texts: Vec<String>,
    /// The texts as first shown (an unchanged field keeps its exact value).
    pub initial: Vec<String>,
}

/// A click tool from an extension, while active.
pub(crate) struct ToolSession {
    pub ext: String,
    pub command: String,
    pub shared: Rc<RefCell<ToolShared>>,
}

#[derive(Debug, Default)]
pub(crate) struct ToolShared {
    pub status: String,
    pub clicks: Vec<mt_infer::Inference>,
    pub hover: Option<mt_infer::Inference>,
    pub last_moved: Option<DVec3>,
    pub preview: Vec<Vec<DVec3>>,
}

/// The drawing-tool face of an extension's click tool: it queues clicks and
/// remembers the cursor; [`Extensions::tool_events`] runs the script.
pub(crate) struct ScriptTool {
    pub shared: Rc<RefCell<ToolShared>>,
}

impl crate::tools::DrawTool for ScriptTool {
    fn anchor(&self) -> Option<DVec3> {
        None
    }
    fn status(&self) -> String {
        self.shared.borrow().status.clone()
    }
    fn measurement(
        &self,
        _cur: Option<&mt_infer::Inference>,
        _units: mt_infer::units::LengthUnit,
    ) -> (&'static str, String) {
        ("", String::new())
    }
    fn click(&mut self, _ents: &mut mt_kernel::Entities, inf: &mt_infer::Inference) -> bool {
        self.shared.borrow_mut().clicks.push(*inf);
        false
    }
    fn typed(
        &mut self,
        _ents: &mut mt_kernel::Entities,
        _cur: Option<&mt_infer::Inference>,
        _text: &str,
        _units: mt_infer::units::LengthUnit,
    ) -> bool {
        false
    }
    fn preview(&self, cur: &mt_infer::Inference) -> Vec<Vec<DVec3>> {
        let mut s = self.shared.borrow_mut();
        s.hover = Some(*cur);
        s.preview.clone()
    }
    fn cancel(&mut self) {
        let mut s = self.shared.borrow_mut();
        s.preview.clear();
        s.last_moved = None;
    }
}

/// Observer bookkeeping per (extension, observer index).
#[derive(Debug, Default)]
struct ObserverState {
    last_fired: Option<Instant>,
    pending: bool,
}

/// An HTTP GET a script started (runs on a thread; the callback stays here).
pub(crate) struct HttpJob {
    ext: String,
    callback: rhai::FnPtr,
    rx: std::sync::mpsc::Receiver<Result<String, String>>,
}

#[derive(Default)]
pub(crate) struct Extensions {
    pub config: Config,
    /// Where extensions are installed (tests use their own folder).
    pub root: Option<PathBuf>,
    pub installed: Vec<Installed>,
    /// Loaded (enabled) extensions.
    pub scripts: Vec<Script>,
    pub log: runtime::Log,
    /// Window > Extension Console.
    pub console: Option<Script>,
    pub console_open: bool,
    pub console_input: String,
    pub console_history: Vec<String>,
    pub inputs: Vec<PendingInput>,
    /// (title, text) waiting for OK.
    pub messages: Vec<(String, String)>,
    /// Script errors waiting for OK: (extension name, error).
    pub errors: Vec<(String, String)>,
    pub tool: Option<ToolSession>,
    pub http: Vec<HttpJob>,
    observers: BTreeMap<(String, usize), ObserverState>,
    last_selection: Vec<Owner>,
    last_state: Option<u64>,
    /// The spec's own menus and toolbars, before extensions added theirs.
    base: Option<(Vec<mt_spec::Menu>, Vec<mt_spec::Toolbar>)>,
    /// Signature of the registered commands the menus were built from.
    menu_signature: String,
    pub ui: ui::UiState,
}

/// Command ids handed to the shortcut table, which wants `&'static str`.
static INTERNED: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

pub(crate) fn intern(s: &str) -> &'static str {
    let mut v = INTERNED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(x) = v.iter().find(|x| **x == s) {
        return x;
    }
    let x: &'static str = Box::leak(s.to_owned().into_boxed_str());
    v.push(x);
    x
}

/// Top-level menu an extension command goes in when its path names no existing one.
const DEFAULT_MENU: &str = "Extensions";
/// The toolbar extension buttons go on.
pub(crate) const TOOLBAR_ID: &str = "extension_commands";

impl Extensions {
    /// A manager over `root` (installed extensions) with `config`.
    pub(crate) fn new(root: Option<PathBuf>, config: Config) -> Self {
        Self {
            root,
            config,
            ..Default::default()
        }
    }

    /// Scans the install folder and loads every enabled extension. A broken one is
    /// reported in the manager and the console; it never stops the others.
    pub(crate) fn load_all(&mut self) {
        self.scripts.clear();
        self.installed.clear();
        let Some(root) = self.root.clone() else {
            return;
        };
        for (dir, manifest) in package::installed(&root) {
            let id = manifest.as_ref().ok().map(|m| m.id.clone());
            let enabled = id
                .as_ref()
                .is_none_or(|id| self.config.extensions.get(id).is_none_or(|s| s.enabled));
            let signature = match package::Package::from_folder(&dir) {
                Ok(p) => signing::verify_package(&p.files, &self.trusted_keys()),
                Err(_) => SigStatus::Unsigned,
            };
            let mut item = Installed {
                dir,
                manifest,
                enabled,
                error: None,
                signature,
            };
            match &item.manifest {
                Err(e) => item.error = Some(e.clone()),
                Ok(m) if enabled => {
                    if let Err(e) = self.load_one(m.clone(), &item.dir.clone()) {
                        item.error = Some(e);
                    }
                }
                Ok(_) => {}
            }
            self.installed.push(item);
        }
    }

    fn load_one(&mut self, m: Manifest, dir: &Path) -> Result<(), String> {
        let state = self
            .config
            .extensions
            .get(&m.id)
            .cloned()
            .unwrap_or_default();
        let granted = runtime::granted(&m, &state.approved, &state.revoked);
        let name = m.name.clone();
        match Script::load(m, dir, granted, self.log.clone()) {
            Ok(mut s) => {
                let requests = s.take_requests();
                let ext = s.manifest.id.clone();
                self.scripts.push(s);
                self.handle_requests(&ext, requests);
                self.log_line("meshtard", &format!("loaded {name}"), false);
                Ok(())
            }
            Err(e) => {
                let msg = format!("{name} did not load: {e}");
                self.log_line("meshtard", &msg, true);
                Err(e)
            }
        }
    }

    pub(crate) fn log_line(&self, source: &str, text: &str, error: bool) {
        self.log.borrow_mut().push(LogLine {
            source: source.to_owned(),
            text: text.to_owned(),
            error,
        });
    }

    fn script_index(&self, ext: &str) -> Option<usize> {
        self.scripts.iter().position(|s| s.manifest.id == ext)
    }

    fn ext_name(&self, ext: &str) -> String {
        self.scripts
            .iter()
            .find(|s| s.manifest.id == ext)
            .map_or_else(|| ext.to_owned(), |s| s.manifest.name.clone())
    }

    /// The app ids of every loaded command (for enabling menu items).
    pub(crate) fn command_ids(&self) -> Vec<String> {
        let mut out = Vec::new();
        for s in &self.scripts {
            for c in &s.host.borrow().commands {
                out.push(runtime::app_command_id(&s.manifest.id, &c.id));
            }
        }
        out
    }

    /// A command by its app id: (extension, its definition).
    pub(crate) fn command(&self, id: &str) -> Option<(String, CommandDef)> {
        let (ext, cmd) = runtime::split_command_id(id)?;
        let s = self.scripts.iter().find(|s| s.manifest.id == ext)?;
        let c = s
            .host
            .borrow()
            .commands
            .iter()
            .find(|c| c.id == cmd)?
            .clone();
        Some((ext.to_owned(), c))
    }

    /// Calls `f` in extension `ext`, then deals with what the script asked for.
    pub(crate) fn call(
        &mut self,
        ext: &str,
        f: &rhai::FnPtr,
        args: Vec<Dynamic>,
        doc: Option<&mut Document>,
        selection: &[Owner],
    ) -> Outcome {
        let Some(i) = self.script_index(ext) else {
            return Outcome {
                error: Some(format!("extension {ext} is not loaded")),
                ..Default::default()
            };
        };
        let mut out = self.scripts[i].call(f, args, doc, selection);
        self.finish(ext, &mut out);
        out
    }

    fn finish(&mut self, ext: &str, out: &mut Outcome) {
        if let Some(e) = &out.error {
            self.errors.push((self.ext_name(ext), e.clone()));
        }
        let requests = std::mem::take(&mut out.requests);
        self.handle_requests(ext, requests);
    }

    fn handle_requests(&mut self, ext: &str, requests: Vec<Request>) {
        for r in requests {
            match r {
                Request::Message(t) => self.messages.push((self.ext_name(ext), t)),
                Request::Status(t) => {
                    if let Some(tool) = self.tool.as_ref().filter(|t| t.ext == ext) {
                        tool.shared.borrow_mut().status = t.clone();
                    }
                    self.ui.status = Some(t);
                }
                Request::Input {
                    title,
                    fields,
                    callback,
                } => {
                    self.inputs.push(PendingInput {
                        ext: ext.to_owned(),
                        title,
                        fields,
                        callback,
                        texts: Vec::new(),
                        initial: Vec::new(),
                    });
                }
                Request::HttpGet { url, callback } => {
                    let (tx, rx) = std::sync::mpsc::channel();
                    self.log_line(ext, &format!("GET {url}"), false);
                    std::thread::spawn(move || {
                        let r = catalog::fetch(&url).and_then(|b| {
                            if b.len() > 8 << 20 {
                                Err("the answer is larger than 8 MB".into())
                            } else {
                                Ok(String::from_utf8_lossy(&b).into_owned())
                            }
                        });
                        let _ = tx.send(r);
                    });
                    self.http.push(HttpJob {
                        ext: ext.to_owned(),
                        callback,
                        rx,
                    });
                }
            }
        }
    }

    /// Runs a menu command or toolbar button. Tools are started by the app
    /// ([`Extensions::start_tool`]); this returns `None` for them.
    pub(crate) fn run_command(
        &mut self,
        id: &str,
        doc: Option<&mut Document>,
        selection: &[Owner],
    ) -> Option<Outcome> {
        let (ext, cmd) = self.command(id)?;
        match &cmd.action {
            Action::Run(f) => Some(self.call(&ext, f, Vec::new(), doc, selection)),
            Action::Tool(_) => None,
        }
    }

    /// Makes the click tool of command `id` the active one.
    pub(crate) fn start_tool(&mut self, id: &str) -> Option<ScriptTool> {
        let (ext, cmd) = self.command(id)?;
        let Action::Tool(t) = &cmd.action else {
            return None;
        };
        let shared = Rc::new(RefCell::new(ToolShared {
            status: t.status.clone(),
            ..Default::default()
        }));
        self.tool = Some(ToolSession {
            ext,
            command: id.to_owned(),
            shared: shared.clone(),
        });
        Some(ScriptTool { shared })
    }

    /// Runs the active click tool's `on_click` for queued clicks and `on_move` when
    /// the cursor moved. Returns the outcomes (the app rebuilds after a change).
    pub(crate) fn tool_events(
        &mut self,
        mut doc: Option<&mut Document>,
        selection: &[Owner],
    ) -> Vec<Outcome> {
        let Some(session) = &self.tool else {
            return Vec::new();
        };
        let (ext, shared) = (session.ext.clone(), session.shared.clone());
        let Some((_, cmd)) = self.command(&session.command) else {
            self.tool = None;
            return Vec::new();
        };
        let Action::Tool(t) = cmd.action else {
            return Vec::new();
        };
        let mut outs = Vec::new();
        let clicks = std::mem::take(&mut shared.borrow_mut().clicks);
        for inf in clicks {
            if let Some(f) = &t.on_click {
                let args = vec![
                    runtime::point_dynamic(inf.point),
                    Dynamic::from_map(runtime::inference_map(&inf)),
                ];
                outs.push(self.call(&ext, f, args, doc.as_deref_mut(), selection));
            }
        }
        let hover = shared.borrow().hover;
        if let (Some(inf), Some(f)) = (hover, &t.on_move)
            && shared.borrow().last_moved != Some(inf.point)
        {
            shared.borrow_mut().last_moved = Some(inf.point);
            let args = vec![
                runtime::point_dynamic(inf.point),
                Dynamic::from_map(runtime::inference_map(&inf)),
            ];
            let out = self.call(&ext, f, args, doc, selection);
            shared.borrow_mut().preview = out
                .value
                .as_ref()
                .map(runtime::polylines)
                .unwrap_or_default();
            outs.push(out);
        }
        outs
    }

    /// Fires observers whose event happened and whose throttle time passed. Returns
    /// the outcomes and, when an observer still waits, how long until it may fire.
    pub(crate) fn observe(
        &mut self,
        mut doc: Option<&mut Document>,
        selection: &[Owner],
        now: Instant,
    ) -> (Vec<Outcome>, Option<std::time::Duration>) {
        let state = doc.as_deref().map(Document::state);
        let sel_changed = selection != self.last_selection.as_slice();
        let model_changed = state != self.last_state;
        self.last_selection = selection.to_vec();
        self.last_state = state;
        let mut due = Vec::new();
        let mut wait: Option<std::time::Duration> = None;
        for s in &self.scripts {
            for (i, o) in s.host.borrow().observers.iter().enumerate() {
                let st = self
                    .observers
                    .entry((s.manifest.id.clone(), i))
                    .or_default();
                if (o.kind == ObserverKind::Selection && sel_changed)
                    || (o.kind == ObserverKind::Model && model_changed)
                {
                    st.pending = true;
                }
                if !st.pending {
                    continue;
                }
                let ready = st.last_fired.map_or(now, |t| t + o.throttle);
                if ready <= now {
                    st.pending = false;
                    st.last_fired = Some(now);
                    due.push((s.manifest.id.clone(), o.callback.clone()));
                } else {
                    let w = ready - now;
                    wait = Some(wait.map_or(w, |x: std::time::Duration| x.min(w)));
                }
            }
        }
        let mut outs = Vec::new();
        for (ext, f) in due {
            outs.push(self.call(&ext, &f, Vec::new(), doc.as_deref_mut(), selection));
        }
        // An observer's own change does not wake the observers again.
        self.last_state = doc.as_deref().map(Document::state);
        if let Some(sel) = outs.iter().rev().find_map(|o| o.selection.clone()) {
            self.last_selection = sel;
        }
        (outs, wait)
    }

    /// HTTP answers that came in: runs their callbacks.
    pub(crate) fn poll_http(
        &mut self,
        mut doc: Option<&mut Document>,
        selection: &[Owner],
    ) -> Vec<Outcome> {
        let mut done = Vec::new();
        self.http.retain(|j| match j.rx.try_recv() {
            Ok(r) => {
                done.push((j.ext.clone(), j.callback.clone(), r));
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(_) => false,
        });
        let mut outs = Vec::new();
        for (ext, f, r) in done {
            let mut m = rhai::Map::new();
            m.insert("ok".into(), r.is_ok().into());
            match r {
                Ok(body) => {
                    m.insert("body".into(), body.into());
                    m.insert("error".into(), "".into());
                }
                Err(e) => {
                    m.insert("body".into(), "".into());
                    m.insert("error".into(), e.into());
                }
            }
            outs.push(self.call(
                &ext,
                &f,
                vec![Dynamic::from_map(m)],
                doc.as_deref_mut(),
                selection,
            ));
        }
        outs
    }

    /// Answers the first input dialog (OK with its field values, or Cancel).
    pub(crate) fn answer_input(
        &mut self,
        ok: bool,
        doc: Option<&mut Document>,
        selection: &[Owner],
    ) -> Option<Outcome> {
        if self.inputs.is_empty() {
            return None;
        }
        let p = self.inputs.remove(0);
        if !ok {
            return None;
        }
        let values = runtime::field_values(&p.fields);
        Some(self.call(
            &p.ext,
            &p.callback,
            vec![Dynamic::from_map(values)],
            doc,
            selection,
        ))
    }

    /// Evaluates a console line against the model.
    pub(crate) fn console_eval(
        &mut self,
        line: &str,
        doc: Option<&mut Document>,
        selection: &[Owner],
    ) -> Outcome {
        let log = self.log.clone();
        let console = self.console.get_or_insert_with(|| Script::console(log));
        self.log.borrow_mut().push(LogLine {
            source: "console".into(),
            text: format!("> {line}"),
            error: false,
        });
        let mut out = console.eval(line, doc, selection);
        if let Some(v) = out.value.as_ref().and_then(runtime::show_value) {
            self.log_line("console", &v, false);
        }
        // Console errors stay in the console (no dialog).
        out.error = None;
        let requests = std::mem::take(&mut out.requests);
        self.handle_requests("console", requests);
        out
    }

    /// Installs a package: writes it, records it enabled with the approved
    /// permissions, and (re)loads it.
    pub(crate) fn install(
        &mut self,
        pkg: &package::Package,
        approved: Vec<manifest::Permission>,
    ) -> Result<(), String> {
        let root = self.root.clone().ok_or("no data folder for extensions")?;
        std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
        // A broken signature is never installed (callers check first; this is the
        // last line).
        if let SigStatus::Invalid(e) = signing::verify_package(&pkg.files, &self.trusted_keys()) {
            return Err(format!("not installed: invalid signature ({e})"));
        }
        // Unload first (an update replaces its folder).
        self.scripts.retain(|s| s.manifest.id != pkg.manifest.id);
        pkg.install(&root)?;
        // Permissions taken back in the manager stay taken back after an update.
        let revoked = (self.config.extensions.get(&pkg.manifest.id))
            .map(|s| s.revoked.clone())
            .unwrap_or_default();
        self.config.extensions.insert(
            pkg.manifest.id.clone(),
            package::ExtState {
                enabled: true,
                approved,
                revoked,
            },
        );
        self.config.save_if(self.root.is_some() && cfg!(not(test)));
        self.load_all();
        self.log_line(
            "meshtard",
            &format!("installed {} {}", pkg.manifest.name, pkg.manifest.version),
            false,
        );
        Ok(())
    }

    /// Enables or disables an installed extension (kept between runs).
    pub(crate) fn set_enabled(&mut self, id: &str, enabled: bool) {
        let e = self.config.extensions.entry(id.to_owned()).or_default();
        e.enabled = enabled;
        self.config.save_if(cfg!(not(test)));
        self.load_all();
    }

    /// Grants or takes back one declared permission of an installed extension
    /// (Extension Manager), then reloads it. Sensitive permissions are approved or
    /// unapproved; the others are revoked or restored.
    pub(crate) fn set_permission(&mut self, id: &str, p: Permission, granted: bool) {
        // No entry yet means enabled (see `load_all`).
        let e =
            (self.config.extensions.entry(id.to_owned())).or_insert_with(|| package::ExtState {
                enabled: true,
                ..Default::default()
            });
        e.approved.retain(|q| *q != p);
        e.revoked.retain(|q| *q != p);
        if granted {
            if p.sensitive() {
                e.approved.push(p);
            }
        } else {
            e.revoked.push(p);
        }
        e.approved.sort();
        e.revoked.sort();
        self.config.save_if(cfg!(not(test)));
        self.load_all();
    }

    /// Whether a declared permission is granted now.
    pub(crate) fn is_granted(&self, id: &str, p: Permission) -> bool {
        let s = self.config.extensions.get(id);
        let revoked = s.is_some_and(|s| s.revoked.contains(&p));
        let approved = s.is_some_and(|s| s.approved.contains(&p));
        !revoked && (!p.sensitive() || approved)
    }

    /// The Warehouse catalog URL (the user's, else the default).
    pub(crate) fn catalog_url(&self) -> String {
        match self.config.catalog_url.trim() {
            "" => catalog::DEFAULT_CATALOG.to_owned(),
            u => u.to_owned(),
        }
    }

    /// The built-in keys and the user's.
    pub(crate) fn trusted_keys(&self) -> Vec<signing::TrustedKey> {
        signing::trusted_keys(&self.config.trusted_keys)
    }

    /// Adds a trusted key (Extension Manager > Trusted keys).
    pub(crate) fn add_trusted_key(&mut self, name: &str, hex: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 {
            return Err("give the key a name (1-80 characters)".into());
        }
        let key = signing::parse_public_key(hex.trim())?;
        if self.trusted_keys().iter().any(|k| k.key == key) {
            return Err("that key is trusted already".into());
        }
        self.config
            .trusted_keys
            .insert(name.to_owned(), signing::public_key_hex(&key));
        self.config.save_if(cfg!(not(test)));
        self.load_all();
        Ok(())
    }

    pub(crate) fn remove_trusted_key(&mut self, name: &str) {
        self.config.trusted_keys.remove(name);
        self.config.save_if(cfg!(not(test)));
        self.load_all();
    }

    /// Checks a package's signature; a broken one is an error.
    pub(crate) fn check_signature(&self, pkg: &package::Package) -> Result<SigStatus, String> {
        match signing::verify_package(&pkg.files, &self.trusted_keys()) {
            SigStatus::Invalid(e) => Err(format!(
                "the package's signature is invalid ({e}); not installed"
            )),
            s => Ok(s),
        }
    }

    /// Installed (id, version) of every readable manifest.
    pub(crate) fn installed_versions(&self) -> Vec<(String, String)> {
        self.installed
            .iter()
            .filter_map(|i| i.manifest.as_ref().ok())
            .map(|m| (m.id.clone(), m.version.clone()))
            .collect()
    }

    /// The custom cursor of the click tool with app command id `id`: the image's
    /// path and the cursor.
    pub(crate) fn tool_cursor(&self, id: &str) -> Option<(PathBuf, ToolCursor)> {
        let (ext, cmd) = self.command(id)?;
        let runtime::Action::Tool(t) = &cmd.action else {
            return None;
        };
        let s = self.scripts.iter().find(|s| s.manifest.id == ext)?;
        let c = t
            .cursor
            .clone()
            .or_else(|| s.manifest.cursors.get(&cmd.id).cloned())?;
        Some((s.dir.join(&c.image), c))
    }

    /// Removes an installed extension's folder and settings.
    pub(crate) fn uninstall(&mut self, dir: &Path, id: Option<&str>) -> Result<(), String> {
        let root = self.root.clone().ok_or("no extensions folder")?;
        // Only folders directly inside the extensions folder.
        if dir.parent() != Some(root.as_path()) {
            return Err("not an installed extension".into());
        }
        std::fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        if let Some(id) = id {
            self.config.extensions.remove(id);
            self.config.save_if(cfg!(not(test)));
        }
        self.load_all();
        Ok(())
    }

    /// Builds the menus and the extension toolbar into `spec` from the spec's own
    /// ones plus every loaded command. Returns whether anything changed.
    pub(crate) fn apply_to_spec(&mut self, spec: &mut mt_spec::Spec) -> bool {
        let sig: String = self.command_ids().into_iter().collect::<Vec<_>>().join("|");
        if self.base.is_some() && sig == self.menu_signature {
            return false;
        }
        self.menu_signature = sig;
        let (menus, toolbars) = self
            .base
            .get_or_insert_with(|| (spec.menus.menu.clone(), spec.toolbars.toolbar.clone()))
            .clone();
        spec.menus.menu = menus;
        spec.toolbars.toolbar = toolbars;
        let mut buttons = Vec::new();
        let mut separated = HashSet::new();
        for s in &self.scripts {
            for c in &s.host.borrow().commands {
                let id = runtime::app_command_id(&s.manifest.id, &c.id);
                if !c.menu.is_empty() {
                    add_menu_item(spec, &c.menu, &id, &c.label, &mut separated);
                }
                if c.toolbar {
                    buttons.push(mt_spec::ToolbarItem::Button(mt_spec::ToolbarButton {
                        id: id.clone(),
                        tooltip: c.tooltip.clone(),
                        flyout: Vec::new(),
                    }));
                }
                if c.menu.is_empty() {
                    // Toolbar-only commands still need a label (tooltips, Preferences >
                    // Shortcuts): a hidden entry would be odd, so they go in the
                    // Extensions menu too.
                    add_menu_item(
                        spec,
                        &[DEFAULT_MENU.to_owned(), s.manifest.name.clone()],
                        &id,
                        &c.label,
                        &mut separated,
                    );
                }
            }
        }
        if !buttons.is_empty() {
            spec.toolbars.toolbar.push(mt_spec::Toolbar {
                id: TOOLBAR_ID.into(),
                name: "Extensions".into(),
                dock: mt_spec::Dock::Top,
                row: 3,
                columns: 1,
                visible: true,
                items: buttons,
            });
        }
        true
    }

    /// Default shortcuts the loaded commands declare, for commands without one.
    pub(crate) fn default_shortcuts(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for s in &self.scripts {
            for c in &s.host.borrow().commands {
                if let Some(k) = &c.shortcut {
                    out.push((runtime::app_command_id(&s.manifest.id, &c.id), k.clone()));
                }
            }
        }
        out
    }
}

impl Config {
    fn save_if(&self, yes: bool) {
        if yes {
            self.save();
        }
    }
}

fn add_menu_item(
    spec: &mut mt_spec::Spec,
    path: &[String],
    id: &str,
    label: &str,
    separated: &mut HashSet<String>,
) {
    use mt_spec::MenuItem;
    let (top, subs) = path.split_first().expect("non-empty path");
    let i = spec
        .menus
        .menu
        .iter()
        .position(|m| m.label.eq_ignore_ascii_case(top))
        .or_else(|| {
            (spec.menus.menu.iter()).position(|m| m.label.eq_ignore_ascii_case(DEFAULT_MENU))
        });
    let Some(i) = i else { return };
    let menu = &mut spec.menus.menu[i];
    // A separator between the spec's items and the extensions' (once per menu).
    if separated.insert(menu.label.clone()) && !menu.items.is_empty() {
        menu.items.push(MenuItem::Separator { separator: true });
    }
    let mut items = &mut menu.items;
    // The last path part is the item's own label when it matches; otherwise every
    // part names a submenu.
    let subs = match subs.split_last() {
        Some((last, rest)) if last == label => rest,
        _ => subs,
    };
    for name in subs {
        let pos = items
            .iter()
            .position(|it| matches!(it, MenuItem::Submenu { label, .. } if label == name));
        let pos = pos.unwrap_or_else(|| {
            items.push(MenuItem::Submenu {
                label: name.clone(),
                items: Vec::new(),
            });
            items.len() - 1
        });
        let MenuItem::Submenu { items: next, .. } = &mut items[pos] else {
            unreachable!()
        };
        items = next;
    }
    items.push(MenuItem::Command {
        id: id.to_owned(),
        label: label.to_owned(),
    });
}
