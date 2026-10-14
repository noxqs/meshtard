//! Running extension scripts: one Rhai [`Engine`] per extension with the Meshtard API
//! registered (`spec/EXTENSIONS.md`), an operation and time budget per call, and the
//! model facade.
//!
//! A call gets the document's model moved into the extension's [`Host`] while it
//! runs. The first change makes a backup; when the call ends with an error, or the
//! changed context fails `mt_kernel::validate`, the backup is put back (nothing
//! changed). Otherwise the change becomes one undo step through
//! [`Document::edit_whole`].

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use glam::{DMat4, DVec3};
use mt_infer::units::{LengthUnit, UnitsOptions};
use mt_kernel::{DefinitionId, EdgeId, Entities, FaceId, Model};
use rhai::{AST, Array, Dynamic, Engine, EvalAltResult, FnPtr, ImmutableString, Map, Scope};

use super::manifest::{Manifest, Permission, ToolCursor};
use crate::document::{Document, Owner};

type R<T> = Result<T, Box<EvalAltResult>>;

/// Script operations allowed per call (a runaway loop stops here).
pub(crate) const MAX_OPERATIONS: u64 = 5_000_000;
/// Wall-clock time allowed per call.
pub(crate) const MAX_TIME: Duration = Duration::from_secs(3);
/// Default and smallest observer throttle.
pub(crate) const DEFAULT_THROTTLE_MS: i64 = 250;
const MIN_THROTTLE_MS: i64 = 50;

/// A model entity seen by a script: valid during the call that returned it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entity {
    Edge(EdgeId),
    Face(FaceId),
    /// Index into the active context's instances (group or component).
    Instance(usize),
    /// Section planes, dimensions and texts: selectable, not editable from scripts.
    Other(Owner),
}

impl Entity {
    pub(crate) fn from_owner(o: Owner) -> Self {
        match o {
            Owner::Edge(e) => Self::Edge(e),
            Owner::Face(f) => Self::Face(f),
            Owner::Instance(i) => Self::Instance(i),
            o => Self::Other(o),
        }
    }

    pub(crate) fn owner(self) -> Owner {
        match self {
            Self::Edge(e) => Owner::Edge(e),
            Self::Face(f) => Owner::Face(f),
            Self::Instance(i) => Owner::Instance(i),
            Self::Other(o) => o,
        }
    }
}

/// A field of an input dialog.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FieldKind {
    Text(String),
    Number(f64),
    /// A length in inches, shown and typed in the model's units.
    Length(f64),
    Choice(Vec<String>, usize),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Field {
    pub key: String,
    pub label: String,
    pub kind: FieldKind,
}

/// Something a script asked the app to do.
#[derive(Debug, Clone)]
pub(crate) enum Request {
    Message(String),
    Status(String),
    Input {
        title: String,
        fields: Vec<Field>,
        callback: FnPtr,
    },
    HttpGet {
        url: String,
        callback: FnPtr,
    },
}

/// A menu command or toolbar button an extension added.
#[derive(Debug, Clone)]
pub(crate) struct CommandDef {
    /// The extension's own id for it.
    pub id: String,
    pub label: String,
    /// "Extensions > My Tool": top-level menu, then submenus. Empty: no menu item.
    pub menu: Vec<String>,
    pub toolbar: bool,
    pub tooltip: String,
    /// Default shortcut ("Ctrl+Shift+K"), changeable in Preferences > Shortcuts.
    pub shortcut: Option<String>,
    /// PNG in the extension folder for the toolbar button.
    pub icon: Option<String>,
    pub action: Action,
}

#[derive(Debug, Clone)]
pub(crate) enum Action {
    Run(FnPtr),
    Tool(ToolDef),
}

#[derive(Debug, Clone)]
pub(crate) struct ToolDef {
    pub status: String,
    /// `cursor` / `cursor_hotspot` given to `register_tool` (else the manifest's
    /// `[cursors]` entry for the tool, else the default cursor).
    pub cursor: Option<ToolCursor>,
    pub on_click: Option<FnPtr>,
    pub on_move: Option<FnPtr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObserverKind {
    Selection,
    Model,
}

#[derive(Debug, Clone)]
pub(crate) struct Observer {
    pub kind: ObserverKind,
    pub callback: FnPtr,
    pub throttle: Duration,
}

/// One line of the Extension Console.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LogLine {
    pub source: String,
    pub text: String,
    pub error: bool,
}

pub(crate) type Log = Rc<RefCell<Vec<LogLine>>>;

/// What a call did.
#[derive(Debug, Default)]
pub(crate) struct Outcome {
    pub value: Option<Dynamic>,
    /// The model changed (one undo step was recorded).
    pub changed: bool,
    /// The script set the selection.
    pub selection: Option<Vec<Owner>>,
    pub error: Option<String>,
    pub requests: Vec<Request>,
}

/// The state script functions work on, shared with the engine's closures.
pub(crate) struct Host {
    ext_id: String,
    granted: BTreeSet<Permission>,
    dir: PathBuf,
    // During a call.
    model: Option<Model>,
    context: Vec<usize>,
    /// Groups entered with `edit_group` (instance indices below `context`).
    target: Vec<usize>,
    /// Definitions entered with `edit_group` (validated after the call).
    touched: BTreeSet<DefinitionId>,
    units: LengthUnit,
    unit_options: UnitsOptions,
    backup: Option<Model>,
    selection: Vec<Owner>,
    selection_out: Option<Vec<Owner>>,
    requests: Vec<Request>,
    /// Registrations.
    pub commands: Vec<CommandDef>,
    pub observers: Vec<Observer>,
    state: Map,
    log: Log,
}

impl Host {
    /// Whether `p` is granted (declared, not revoked, approved when sensitive).
    #[cfg(test)]
    pub(crate) fn has(&self, p: Permission) -> bool {
        self.granted.contains(&p)
    }

    fn need(&self, p: Permission) -> R<()> {
        if self.granted.contains(&p) {
            Ok(())
        } else {
            Err(format!(
                "permission denied: `{}` was not granted to {} (declare it in extension.toml)",
                p.key(),
                self.ext_id
            )
            .into())
        }
    }

    fn model(&self) -> R<&Model> {
        self.need(Permission::ModelRead)?;
        self.model
            .as_ref()
            .ok_or_else(|| "the model is not available while the extension loads".into())
    }

    /// The context scripts work in: the open one, then groups entered with
    /// `edit_group`.
    fn path(&self) -> Vec<usize> {
        let mut p = self.context.clone();
        p.extend(&self.target);
        p
    }

    fn active(&self) -> R<&Entities> {
        let model = self.model()?;
        Ok(match active_def(model, &self.path()) {
            Some(d) => &model.definitions[d].entities,
            None => &model.root,
        })
    }

    /// Runs a change of the model and the active context's entities.
    fn write<T>(&mut self, f: impl FnOnce(&mut Model, &mut Entities) -> R<T>) -> R<T> {
        self.need(Permission::ModelWrite)?;
        let model = self
            .model
            .as_mut()
            .ok_or("the model is not available while the extension loads")?;
        if self.backup.is_none() {
            self.backup = Some(model.clone());
        }
        let mut path = self.context.clone();
        path.extend(&self.target);
        let def = active_def(model, &path);
        let mut ents = match def {
            Some(d) => std::mem::take(&mut model.definitions[d].entities),
            None => std::mem::take(&mut model.root),
        };
        let r = f(model, &mut ents);
        match def {
            Some(d) => model.definitions[d].entities = ents,
            None => model.root = ents,
        }
        r
    }

    fn log(&self, text: &str, error: bool) {
        self.log.borrow_mut().push(LogLine {
            source: self.ext_id.clone(),
            text: text.to_owned(),
            error,
        });
    }
}

/// The open group's or component's definition along a context path.
fn active_def(model: &Model, path: &[usize]) -> Option<DefinitionId> {
    let mut ents = &model.root;
    let mut def = None;
    for &i in path {
        let d = ents.instances.get(i)?.definition;
        ents = &model.definitions.get(d)?.entities;
        def = Some(d);
    }
    def
}

/// An extension's engine and compiled scripts.
pub(crate) struct Script {
    pub manifest: Manifest,
    pub dir: PathBuf,
    engine: Engine,
    ast: AST,
    /// Persistent variables (the console's REPL).
    scope: Scope<'static>,
    pub host: Rc<RefCell<Host>>,
    deadline: Rc<Cell<Option<Instant>>>,
}

impl Script {
    /// Compiles an installed extension and runs its scripts' top level (where it
    /// registers commands and observers). `granted` is what it may do.
    pub(crate) fn load(
        manifest: Manifest,
        dir: &Path,
        granted: BTreeSet<Permission>,
        log: Log,
    ) -> Result<Self, String> {
        let mut sources = Vec::new();
        for s in &manifest.scripts {
            let text = std::fs::read_to_string(dir.join(s)).map_err(|e| format!("{s}: {e}"))?;
            sources.push((s.clone(), text));
        }
        let mut script = Self::new(manifest, dir, granted, log);
        let mut ast = AST::empty();
        for (name, text) in &sources {
            let a = script
                .engine
                .compile(text)
                .map_err(|e| format!("{name}: {e}"))?;
            ast = ast.merge(&a);
        }
        script.ast = ast;
        let outcome = script.run(None, &[], |engine, ast, scope| {
            engine
                .run_ast_with_scope(scope, ast)
                .map(|()| Dynamic::UNIT)
        });
        if let Some(e) = outcome.error {
            return Err(e);
        }
        // Messages the top level asked for are shown by the caller.
        script.host.borrow_mut().requests = outcome.requests;
        Ok(script)
    }

    /// The Extension Console's REPL: model read and write, no network or files.
    pub(crate) fn console(log: Log) -> Self {
        let manifest = Manifest {
            id: "console".into(),
            name: "Extension Console".into(),
            version: "1.0".into(),
            author: String::new(),
            description: String::new(),
            license: String::new(),
            min_meshtard: None,
            permissions: vec![Permission::ModelRead, Permission::ModelWrite],
            scripts: Vec::new(),
            icon: None,
            homepage: String::new(),
            cursors: Default::default(),
        };
        let granted = manifest.permissions.iter().copied().collect();
        Self::new(manifest, Path::new(""), granted, log)
    }

    fn new(manifest: Manifest, dir: &Path, granted: BTreeSet<Permission>, log: Log) -> Self {
        let host = Rc::new(RefCell::new(Host {
            ext_id: manifest.id.clone(),
            granted,
            dir: dir.to_owned(),
            model: None,
            context: Vec::new(),
            target: Vec::new(),
            touched: BTreeSet::new(),
            units: LengthUnit::Inch,
            unit_options: UnitsOptions::default(),
            backup: None,
            selection: Vec::new(),
            selection_out: None,
            requests: Vec::new(),
            commands: Vec::new(),
            observers: Vec::new(),
            state: Map::new(),
            log,
        }));
        let deadline = Rc::new(Cell::new(None));
        let engine = make_engine(&host, &deadline);
        Self {
            manifest,
            dir: dir.to_owned(),
            engine,
            ast: AST::empty(),
            scope: Scope::new(),
            host,
            deadline,
        }
    }

    /// Requests made while loading (shown once).
    pub(crate) fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.host.borrow_mut().requests)
    }

    /// Calls a script function with `doc` (when there is one) open to it.
    pub(crate) fn call(
        &mut self,
        f: &FnPtr,
        args: Vec<Dynamic>,
        doc: Option<&mut Document>,
        selection: &[Owner],
    ) -> Outcome {
        let f = f.clone();
        self.run(doc, selection, move |engine, ast, _| {
            f.call::<Dynamic>(engine, ast, args)
        })
    }

    /// Evaluates a console line (variables persist between lines).
    pub(crate) fn eval(
        &mut self,
        code: &str,
        doc: Option<&mut Document>,
        selection: &[Owner],
    ) -> Outcome {
        let code = code.to_owned();
        self.run(doc, selection, move |engine, _, scope| {
            engine.eval_with_scope::<Dynamic>(scope, &code)
        })
    }

    fn run(
        &mut self,
        mut doc: Option<&mut Document>,
        selection: &[Owner],
        f: impl FnOnce(&Engine, &AST, &mut Scope<'static>) -> R<Dynamic>,
    ) -> Outcome {
        {
            let mut h = self.host.borrow_mut();
            h.backup = None;
            h.target.clear();
            h.touched.clear();
            h.selection = selection.to_vec();
            h.selection_out = None;
            if let Some(d) = doc.as_deref_mut() {
                h.model = Some(std::mem::take(&mut d.model));
                h.context = d.context.clone();
                h.units = d.units;
                h.unit_options = d.unit_options;
            }
        }
        self.deadline.set(Some(Instant::now() + MAX_TIME));
        let result = f(&self.engine, &self.ast, &mut self.scope);
        self.deadline.set(None);
        let mut h = self.host.borrow_mut();
        let mut out = Outcome {
            requests: std::mem::take(&mut h.requests),
            ..Default::default()
        };
        let mut error = match result {
            Ok(v) => {
                out.value = Some(v);
                None
            }
            Err(e) => Some(describe(&e)),
        };
        let model = h.model.take();
        let backup = h.backup.take();
        if let (Some(d), Some(model)) = (doc, model) {
            match backup {
                None => d.model = model,
                Some(backup) => {
                    if error.is_none() {
                        error = check(&backup, &model, &h.context, &h.touched).err();
                    }
                    if error.is_none() {
                        d.model = backup;
                        out.changed = d.edit_whole(|m| {
                            *m = model;
                            true
                        });
                    } else {
                        // Roll back: the model as before the call.
                        d.model = backup;
                    }
                }
            }
        }
        if error.is_none() {
            out.selection = h.selection_out.take();
        } else {
            out.requests
                .retain(|r| matches!(r, Request::Message(_) | Request::Status(_)));
        }
        if let Some(e) = &error {
            h.log(e, true);
        }
        out.error = error;
        out
    }
}

/// Rhai's error text, with "Too many operations" explained.
fn describe(e: &EvalAltResult) -> String {
    match e {
        EvalAltResult::ErrorTooManyOperations(_) => format!(
            "stopped: the script ran more than {MAX_OPERATIONS} operations (an endless loop?)"
        ),
        EvalAltResult::ErrorTerminated(_, _) => format!(
            "stopped: the script ran longer than {} seconds",
            MAX_TIME.as_secs()
        ),
        e => e.to_string(),
    }
}

/// The changed context and new definitions must pass `mt_kernel::validate` (only
/// problems that were not there before count).
fn check(
    before: &Model,
    after: &Model,
    context: &[usize],
    touched: &BTreeSet<DefinitionId>,
) -> Result<(), String> {
    let checks = mt_kernel::Checks::ALL;
    let errors = |m: &Model, def: Option<DefinitionId>| -> Vec<String> {
        let ents = match def {
            None => Some(&m.root),
            Some(d) => m.definitions.get(d).map(|d| &d.entities),
        };
        ents.map(|e| mt_kernel::validate(e, checks))
            .unwrap_or_default()
    };
    let mut defs: Vec<Option<DefinitionId>> = vec![match context {
        [] => None,
        p => active_def(after, p),
    }];
    defs.extend(touched.iter().map(|d| Some(*d)));
    let mut new = Vec::new();
    for d in defs {
        let old: HashSet<String> = errors(before, d).into_iter().collect();
        new.extend(errors(after, d).into_iter().filter(|e| !old.contains(e)));
    }
    for (id, d) in &after.definitions {
        if !before.definitions.contains_key(id) {
            new.extend(mt_kernel::validate(&d.entities, checks));
        }
    }
    match new.first() {
        None => Ok(()),
        Some(e) => Err(format!(
            "the change left the model invalid ({e}); it was undone"
        )),
    }
}

// ---------------------------------------------------------------------------------
// Conversions.

fn num(d: &Dynamic) -> R<f64> {
    if let Ok(f) = d.as_float() {
        Ok(f)
    } else if let Ok(i) = d.as_int() {
        Ok(i as f64)
    } else {
        Err(format!("expected a number, got {}", d.type_name()).into())
    }
}

fn point(d: &Dynamic) -> R<DVec3> {
    let a = d
        .clone()
        .try_cast::<Array>()
        .filter(|a| a.len() == 3)
        .ok_or("expected a point [x, y, z]")?;
    Ok(DVec3::new(num(&a[0])?, num(&a[1])?, num(&a[2])?))
}

fn to_array(p: DVec3) -> Dynamic {
    Dynamic::from_array(vec![p.x.into(), p.y.into(), p.z.into()])
}

fn entities(d: &Dynamic) -> R<Vec<Entity>> {
    if let Some(e) = d.clone().try_cast::<Entity>() {
        return Ok(vec![e]);
    }
    let a = d
        .clone()
        .try_cast::<Array>()
        .ok_or("expected an entity or an array of entities")?;
    a.iter()
        .map(|x| {
            x.clone()
                .try_cast::<Entity>()
                .ok_or_else(|| "expected an entity".into())
        })
        .collect()
}

fn callback(d: Dynamic) -> R<FnPtr> {
    if d.is_fnptr() {
        return Ok(d.cast::<FnPtr>());
    }
    let name = d
        .into_immutable_string()
        .map_err(|_| "expected a function name or Fn(\"name\")")?;
    FnPtr::new(name.as_str())
}

fn opt_str(m: &Map, key: &str) -> Option<String> {
    m.get(key)
        .and_then(|v| v.clone().into_immutable_string().ok())
        .map(|s| s.to_string())
}

fn flag(m: &Map, key: &str) -> bool {
    m.get(key).and_then(|v| v.as_bool().ok()).unwrap_or(false)
}

/// "Extensions > My Tool" -> ["Extensions", "My Tool"].
pub(crate) fn menu_path(s: &str) -> Vec<String> {
    s.split('>')
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty())
        .collect()
}

fn command_id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn parse_fields(fields: &Array, units: LengthUnit, opts: &UnitsOptions) -> R<Vec<Field>> {
    let _ = (units, opts);
    let mut out = Vec::new();
    for (i, f) in fields.iter().enumerate() {
        let m = f
            .clone()
            .try_cast::<Map>()
            .ok_or("each input field is a map #{ key, label, type, value }")?;
        let key = opt_str(&m, "key").unwrap_or_else(|| format!("field{i}"));
        let label = opt_str(&m, "label").unwrap_or_else(|| key.clone());
        let ty = opt_str(&m, "type").unwrap_or_else(|| "text".into());
        let value = m.get("value").cloned().unwrap_or(Dynamic::UNIT);
        let kind = match ty.as_str() {
            "text" => FieldKind::Text(if value.is_unit() {
                String::new()
            } else {
                value.to_string()
            }),
            "number" => FieldKind::Number(if value.is_unit() { 0.0 } else { num(&value)? }),
            "length" => FieldKind::Length(if value.is_unit() { 0.0 } else { num(&value)? }),
            "choice" => {
                let options: Vec<String> = m
                    .get("options")
                    .and_then(|o| o.clone().try_cast::<Array>())
                    .ok_or("a choice field needs `options`: an array")?
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                if options.is_empty() {
                    return Err("a choice field needs at least one option".into());
                }
                let v = value.to_string();
                let sel = options.iter().position(|o| *o == v).unwrap_or(0);
                FieldKind::Choice(options, sel)
            }
            t => return Err(format!("unknown input field type `{t}`").into()),
        };
        out.push(Field { key, label, kind });
    }
    Ok(out)
}

/// The values of an answered input dialog, as the map its callback gets.
pub(crate) fn field_values(fields: &[Field]) -> Map {
    let mut m = Map::new();
    for f in fields {
        let v: Dynamic = match &f.kind {
            FieldKind::Text(s) => s.clone().into(),
            FieldKind::Number(x) | FieldKind::Length(x) => (*x).into(),
            FieldKind::Choice(o, i) => o[*i].clone().into(),
        };
        m.insert(f.key.as_str().into(), v);
    }
    m
}

fn data_file(h: &Host, name: &str) -> R<PathBuf> {
    h.need(Permission::File)?;
    if !crate::library::safe_relative(name) || name.contains('/') {
        return Err("file names may not contain folders".into());
    }
    let dir = super::package::extension_data_dir(&h.ext_id).ok_or("no data folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(name))
}

// ---------------------------------------------------------------------------------
// The API.

fn make_engine(host: &Rc<RefCell<Host>>, deadline: &Rc<Cell<Option<Instant>>>) -> Engine {
    let mut e = Engine::new();
    // Sandbox: no `import` of files (scripts listed in extension.toml are merged).
    e.set_module_resolver(rhai::module_resolvers::DummyModuleResolver::new());
    e.set_max_operations(MAX_OPERATIONS);
    e.set_max_call_levels(64);
    e.set_max_expr_depths(64, 64);
    e.set_max_string_size(1 << 20);
    e.set_max_array_size(1 << 20);
    e.set_max_map_size(1 << 16);
    let dl = deadline.clone();
    e.on_progress(move |_| match dl.get() {
        Some(t) if Instant::now() > t => Some(Dynamic::UNIT),
        _ => None,
    });
    let h = host.clone();
    e.on_print(move |s| h.borrow().log(s, false));
    let h = host.clone();
    e.on_debug(move |s, _, pos| h.borrow().log(&format!("{s} ({pos})"), false));

    e.register_type_with_name::<Entity>("Entity");
    e.register_fn("==", |a: &mut Entity, b: Entity| *a == b);
    e.register_fn("!=", |a: &mut Entity, b: Entity| *a != b);
    e.register_fn("to_string", |a: &mut Entity| format!("{a:?}"));
    e.register_fn("to_debug", |a: &mut Entity| format!("{a:?}"));

    register_registration(&mut e, host);
    register_read(&mut e, host);
    register_write(&mut e, host);
    register_attributes(&mut e, host);
    register_ui(&mut e, host);
    register_units(&mut e, host);
    register_io(&mut e, host);
    e
}

/// Wraps `host` for a closure.
macro_rules! with_host {
    ($host:expr, |$h:ident| $body:expr) => {{
        let hh = $host.clone();
        move || -> R<_> {
            #[allow(unused_mut)]
            let mut $h = hh.borrow_mut();
            $body
        }
    }};
}

fn register_registration(e: &mut Engine, host: &Rc<RefCell<Host>>) {
    let h = host.clone();
    e.register_fn("register_command", move |m: Map| -> R<()> {
        let mut h = h.borrow_mut();
        let cmd = command_def(&h, &m, false)?;
        h.commands.retain(|c| c.id != cmd.id);
        h.commands.push(cmd);
        Ok(())
    });
    let h = host.clone();
    e.register_fn("register_tool", move |m: Map| -> R<()> {
        let mut h = h.borrow_mut();
        let cmd = command_def(&h, &m, true)?;
        h.commands.retain(|c| c.id != cmd.id);
        h.commands.push(cmd);
        Ok(())
    });
    for (name, kind) in [
        ("on_selection_changed", ObserverKind::Selection),
        ("on_model_changed", ObserverKind::Model),
    ] {
        let h = host.clone();
        e.register_fn(name, move |f: Dynamic| -> R<()> {
            add_observer(&h, kind, f, DEFAULT_THROTTLE_MS)
        });
        let h = host.clone();
        e.register_fn(name, move |f: Dynamic, ms: i64| -> R<()> {
            add_observer(&h, kind, f, ms)
        });
    }
}

fn add_observer(h: &Rc<RefCell<Host>>, kind: ObserverKind, f: Dynamic, ms: i64) -> R<()> {
    let mut h = h.borrow_mut();
    h.need(Permission::ModelRead)?;
    let callback = callback(f)?;
    let throttle = Duration::from_millis(ms.max(MIN_THROTTLE_MS) as u64);
    h.observers.push(Observer {
        kind,
        callback,
        throttle,
    });
    Ok(())
}

fn command_def(h: &Host, m: &Map, tool: bool) -> R<CommandDef> {
    let id = opt_str(m, "id").ok_or("a command needs an `id`")?;
    if !command_id_ok(&id) {
        return Err(format!("command id `{id}`: use letters, digits, `_`, `-`, `.`").into());
    }
    let label = opt_str(m, "label").unwrap_or_else(|| id.clone());
    let menu = opt_str(m, "menu")
        .map(|s| menu_path(&s))
        .unwrap_or_default();
    let toolbar = flag(m, "toolbar");
    if !menu.is_empty() {
        h.need(Permission::UiMenu)?;
    }
    if toolbar {
        h.need(Permission::UiToolbar)?;
    }
    if menu.is_empty() && !toolbar {
        return Err(format!("command `{id}` needs a `menu` path or `toolbar: true`").into());
    }
    let shortcut = opt_str(m, "shortcut").filter(|s| !s.is_empty());
    if let Some(s) = &shortcut {
        mt_spec::Shortcut::parse(s).map_err(|e| format!("shortcut `{s}`: {e}"))?;
    }
    let icon = opt_str(m, "icon");
    if let Some(i) = &icon
        && (!crate::library::safe_relative(i) || !i.ends_with(".png"))
    {
        return Err(format!("icon `{i}` must be a .png file inside the extension").into());
    }
    let get_cb = |k: &str| m.get(k).cloned().map(callback).transpose();
    let action = if tool {
        let cursor = match opt_str(m, "cursor") {
            None => None,
            Some(image) => {
                let hotspot = match m.get("cursor_hotspot") {
                    None => [0, 0],
                    Some(v) => {
                        let a = v
                            .clone()
                            .try_cast::<Array>()
                            .filter(|a| a.len() == 2)
                            .ok_or("cursor_hotspot is [x, y] in pixels")?;
                        [
                            a[0].as_int()
                                .map_err(|_| "cursor_hotspot is [x, y] in pixels")?,
                            a[1].as_int()
                                .map_err(|_| "cursor_hotspot is [x, y] in pixels")?,
                        ]
                    }
                };
                Some(ToolCursor::new(&image, hotspot)?)
            }
        };
        Action::Tool(ToolDef {
            cursor,
            status: opt_str(m, "status").unwrap_or_default(),
            on_click: get_cb("on_click")?,
            on_move: get_cb("on_move")?,
        })
    } else {
        Action::Run(get_cb("callback")?.ok_or("a command needs a `callback`")?)
    };
    Ok(CommandDef {
        tooltip: opt_str(m, "tooltip").unwrap_or_else(|| label.clone()),
        id,
        label,
        menu,
        toolbar,
        shortcut,
        icon,
        action,
    })
}

fn kind_name(model: &Model, ents: &Entities, e: Entity) -> &'static str {
    match e {
        Entity::Edge(_) => "edge",
        Entity::Face(_) => "face",
        Entity::Instance(i) => match ents.instances.get(i) {
            Some(inst)
                if model
                    .definitions
                    .get(inst.definition)
                    .is_some_and(|d| d.is_group) =>
            {
                "group"
            }
            Some(_) => "component",
            None => "deleted",
        },
        Entity::Other(Owner::Section(_)) => "section_plane",
        Entity::Other(Owner::Dimension(_)) => "dimension",
        Entity::Other(Owner::Text(_)) => "text",
        Entity::Other(_) => "other",
    }
}

fn face(ents: &Entities, e: Entity) -> R<FaceId> {
    match e {
        Entity::Face(f) if ents.faces.contains_key(f) => Ok(f),
        _ => Err("expected a face of the active context".into()),
    }
}

fn edge(ents: &Entities, e: Entity) -> R<EdgeId> {
    match e {
        Entity::Edge(x) if ents.edges.contains_key(x) => Ok(x),
        _ => Err("expected an edge of the active context".into()),
    }
}

fn instance(ents: &Entities, e: Entity) -> R<usize> {
    match e {
        Entity::Instance(i) if i < ents.instances.len() => Ok(i),
        _ => Err("expected a group or component of the active context".into()),
    }
}

fn register_read(e: &mut Engine, host: &Rc<RefCell<Host>>) {
    e.register_fn(
        "edges",
        with_host!(host, |h| {
            let ents = h.active()?;
            Ok(ents
                .edges
                .keys()
                .map(|k| Dynamic::from(Entity::Edge(k)))
                .collect::<Array>())
        }),
    );
    e.register_fn(
        "faces",
        with_host!(host, |h| {
            let ents = h.active()?;
            Ok(ents
                .faces
                .keys()
                .map(|k| Dynamic::from(Entity::Face(k)))
                .collect::<Array>())
        }),
    );
    e.register_fn(
        "groups",
        with_host!(host, |h| {
            let ents = h.active()?;
            Ok((0..ents.instances.len())
                .map(|i| Dynamic::from(Entity::Instance(i)))
                .collect::<Array>())
        }),
    );
    e.register_fn(
        "materials",
        with_host!(host, |h| {
            Ok(h.model()?
                .materials
                .iter()
                .map(|m| Dynamic::from(m.name.clone()))
                .collect::<Array>())
        }),
    );
    e.register_fn(
        "tags",
        with_host!(host, |h| {
            Ok(h.model()?
                .layers
                .iter()
                .map(|m| Dynamic::from(m.name.clone()))
                .collect::<Array>())
        }),
    );
    e.register_fn(
        "selection",
        with_host!(host, |h| {
            h.need(Permission::ModelRead)?;
            let sel = h.selection_out.as_ref().unwrap_or(&h.selection);
            Ok(sel
                .iter()
                .map(|o| Dynamic::from(Entity::from_owner(*o)))
                .collect::<Array>())
        }),
    );
    let hh = host.clone();
    e.register_fn("select", move |d: Dynamic| -> R<()> {
        let mut h = hh.borrow_mut();
        h.need(Permission::ModelRead)?;
        let list = entities(&d)?;
        h.selection_out = Some(list.into_iter().map(Entity::owner).collect());
        Ok(())
    });
    e.register_fn(
        "clear_selection",
        with_host!(host, |h| {
            h.need(Permission::ModelRead)?;
            h.selection_out = Some(Vec::new());
            Ok(())
        }),
    );

    // Entity properties.
    let hh = host.clone();
    e.register_get("kind", move |x: &mut Entity| -> R<String> {
        let h = hh.borrow();
        Ok(kind_name(h.model()?, h.active()?, *x).to_owned())
    });
    let hh = host.clone();
    e.register_get("valid", move |x: &mut Entity| -> R<bool> {
        let h = hh.borrow();
        let ents = h.active()?;
        Ok(match *x {
            Entity::Edge(k) => ents.edges.contains_key(k),
            Entity::Face(k) => ents.faces.contains_key(k),
            Entity::Instance(i) => i < ents.instances.len(),
            Entity::Other(_) => true,
        })
    });
    let hh = host.clone();
    e.register_get("start", move |x: &mut Entity| -> R<Dynamic> {
        let h = hh.borrow();
        let ents = h.active()?;
        Ok(to_array(ents.pos(ents.edges[edge(ents, *x)?].start)))
    });
    let hh = host.clone();
    e.register_get("end", move |x: &mut Entity| -> R<Dynamic> {
        let h = hh.borrow();
        let ents = h.active()?;
        Ok(to_array(ents.pos(ents.edges[edge(ents, *x)?].end)))
    });
    let hh = host.clone();
    e.register_get("length", move |x: &mut Entity| -> R<f64> {
        let h = hh.borrow();
        let ents = h.active()?;
        let ed = &ents.edges[edge(ents, *x)?];
        Ok(ents.pos(ed.start).distance(ents.pos(ed.end)))
    });
    let hh = host.clone();
    e.register_get("area", move |x: &mut Entity| -> R<f64> {
        let h = hh.borrow();
        let ents = h.active()?;
        Ok(crate::tools::face_area(ents, face(ents, *x)?))
    });
    let hh = host.clone();
    e.register_get("normal", move |x: &mut Entity| -> R<Dynamic> {
        let h = hh.borrow();
        let ents = h.active()?;
        Ok(to_array(ents.faces[face(ents, *x)?].normal))
    });
    let hh = host.clone();
    e.register_get("vertices", move |x: &mut Entity| -> R<Array> {
        let h = hh.borrow();
        let ents = h.active()?;
        let pts = match *x {
            Entity::Face(_) => ents.loop_points(&ents.faces[face(ents, *x)?].outer),
            _ => {
                let ed = &ents.edges[edge(ents, *x)?];
                vec![ents.pos(ed.start), ents.pos(ed.end)]
            }
        };
        Ok(pts.into_iter().map(to_array).collect())
    });
    let hh = host.clone();
    e.register_get("edges", move |x: &mut Entity| -> R<Array> {
        let h = hh.borrow();
        let ents = h.active()?;
        let f = face(ents, *x)?;
        Ok(ents.faces[f]
            .loops()
            .flat_map(|l| {
                l.coedges
                    .iter()
                    .map(|c| Dynamic::from(Entity::Edge(c.edge)))
            })
            .collect())
    });
    let hh = host.clone();
    e.register_get("faces", move |x: &mut Entity| -> R<Array> {
        let h = hh.borrow();
        let ents = h.active()?;
        let ed = edge(ents, *x)?;
        Ok(ents.edges[ed]
            .faces
            .iter()
            .map(|f| Dynamic::from(Entity::Face(*f)))
            .collect())
    });
    let hh = host.clone();
    e.register_get("material", move |x: &mut Entity| -> R<Dynamic> {
        let h = hh.borrow();
        let model = h.model()?;
        let ents = h.active()?;
        let m = match *x {
            Entity::Face(_) => ents.faces[face(ents, *x)?].attrs.material,
            _ => ents.instances[instance(ents, *x)?].material,
        };
        Ok(m.and_then(|i| model.materials.get(i))
            .map_or(Dynamic::UNIT, |m| m.name.clone().into()))
    });
    let hh = host.clone();
    e.register_get("tag", move |x: &mut Entity| -> R<Dynamic> {
        let h = hh.borrow();
        let model = h.model()?;
        let ents = h.active()?;
        let l = match *x {
            Entity::Face(f) if ents.faces.contains_key(f) => ents.faces[f].attrs.layer,
            Entity::Edge(k) if ents.edges.contains_key(k) => ents.edges[k].attrs.layer,
            _ => ents.instances[instance(ents, *x)?].layer,
        };
        Ok(l.and_then(|i| model.layers.get(i))
            .map_or(Dynamic::UNIT, |m| m.name.clone().into()))
    });
    let hh = host.clone();
    e.register_get("name", move |x: &mut Entity| -> R<String> {
        let h = hh.borrow();
        let ents = h.active()?;
        Ok(ents.instances[instance(ents, *x)?].name.clone())
    });
    let hh = host.clone();
    e.register_get("definition", move |x: &mut Entity| -> R<String> {
        let h = hh.borrow();
        let model = h.model()?;
        let ents = h.active()?;
        let d = ents.instances[instance(ents, *x)?].definition;
        Ok(model
            .definitions
            .get(d)
            .map(|d| d.name.clone())
            .unwrap_or_default())
    });
    let hh = host.clone();
    e.register_get("origin", move |x: &mut Entity| -> R<Dynamic> {
        let h = hh.borrow();
        let ents = h.active()?;
        let t = ents.instances[instance(ents, *x)?].transform;
        Ok(to_array(t.w_axis.truncate()))
    });
    let hh = host.clone();
    e.register_get("bounds", move |x: &mut Entity| -> R<Dynamic> {
        let h = hh.borrow();
        let model = h.model()?;
        let ents = h.active()?;
        let i = instance(ents, *x)?;
        let sel = mt_kernel::group::Selection {
            instances: vec![i],
            ..Default::default()
        };
        Ok(
            match mt_kernel::group::bounds(ents, &sel, &model.definitions) {
                Some((a, b)) => Dynamic::from_array(vec![to_array(a), to_array(b)]),
                None => Dynamic::UNIT,
            },
        )
    });
    let hh = host.clone();
    e.register_get("entity_count", move |x: &mut Entity| -> R<i64> {
        let h = hh.borrow();
        let model = h.model()?;
        let ents = h.active()?;
        let d = ents.instances[instance(ents, *x)?].definition;
        Ok(model.definitions.get(d).map_or(0, |d| {
            (d.entities.edges.len() + d.entities.faces.len() + d.entities.instances.len()) as i64
        }))
    });
}

fn register_write(e: &mut Engine, host: &Rc<RefCell<Host>>) {
    let hh = host.clone();
    e.register_fn("add_line", move |a: Dynamic, b: Dynamic| -> R<Array> {
        let (a, b) = (point(&a)?, point(&b)?);
        hh.borrow_mut().write(|_, ents| {
            Ok(mt_kernel::add_line(ents, a, b)
                .into_iter()
                .map(|e| Dynamic::from(Entity::Edge(e)))
                .collect())
        })
    });
    let hh = host.clone();
    e.register_fn("add_face", move |pts: Array| -> R<Dynamic> {
        let pts: Vec<DVec3> = pts.iter().map(point).collect::<R<_>>()?;
        if pts.len() < 3 {
            return Err("a face needs at least 3 points".into());
        }
        hh.borrow_mut().write(|_, ents| {
            let mut made = Vec::new();
            for i in 0..pts.len() {
                made.extend(mt_kernel::add_line(ents, pts[i], pts[(i + 1) % pts.len()]));
            }
            let on_loop = |f: FaceId| {
                let lp = ents.loop_points(&ents.faces[f].outer);
                pts.iter()
                    .all(|p| lp.iter().any(|q| q.distance(*p) <= mt_kernel::TOL))
            };
            let found = made
                .iter()
                .flat_map(|e| ents.edges[*e].faces.iter().copied())
                .find(|f| on_loop(*f));
            Ok(found.map_or(Dynamic::UNIT, |f| Dynamic::from(Entity::Face(f))))
        })
    });
    let hh = host.clone();
    e.register_fn("push_pull", move |f: Entity, dist: Dynamic| -> R<Dynamic> {
        let dist = num(&dist)?;
        hh.borrow_mut().write(|_, ents| {
            let f = face(ents, f)?;
            Ok(mt_kernel::push_pull(ents, f, dist, false)
                .map_or(Dynamic::UNIT, |f| Dynamic::from(Entity::Face(f))))
        })
    });
    let hh = host.clone();
    e.register_fn("erase", move |d: Dynamic| -> R<()> {
        let list = entities(&d)?;
        hh.borrow_mut().write(|_, ents| {
            let mut instances = Vec::new();
            for x in list {
                match x {
                    Entity::Face(f) => {
                        ents.remove_face(f);
                    }
                    Entity::Edge(k) if ents.edges.contains_key(k) => mt_kernel::erase_edge(ents, k),
                    Entity::Instance(i) => instances.push(i),
                    _ => {}
                }
            }
            instances.sort_unstable();
            instances.dedup();
            for i in instances.into_iter().rev() {
                if ents.instances.get(i).is_some_and(|x| !x.locked) {
                    ents.instances.remove(i);
                }
            }
            Ok(())
        })
    });
    for (name, group) in [("make_group", true), ("make_component", false)] {
        let hh = host.clone();
        e.register_fn(name, move |d: Dynamic, label: &str| -> R<Entity> {
            make_group(&hh, &d, label, group)
        });
        let hh = host.clone();
        e.register_fn(name, move |d: Dynamic| -> R<Entity> {
            make_group(&hh, &d, "", group)
        });
    }
    let hh = host.clone();
    e.register_fn("new_group", move |name: &str| -> R<Entity> {
        let name = name.to_owned();
        hh.borrow_mut().write(|model, ents| {
            let def_name =
                model.unique_definition_name(if name.is_empty() { "Group#1" } else { &name });
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            let definition = model.definitions.insert(mt_kernel::Definition {
                name: def_name,
                guid: format!("{:032x}", t ^ ((model.definitions.len() as u128) << 64)),
                entities: Entities::new(),
                is_group: true,
                ..Default::default()
            });
            ents.instances.push(mt_kernel::Instance {
                definition,
                transform: DMat4::IDENTITY,
                name: name.clone(),
                hidden: false,
                locked: false,
                layer: None,
                material: None,
                attrs: Default::default(),
            });
            Ok(Entity::Instance(ents.instances.len() - 1))
        })
    });
    let hh = host.clone();
    e.register_fn("edit_group", move |x: Entity| -> R<()> {
        let mut h = hh.borrow_mut();
        let ents = h.active()?;
        let i = instance(ents, x)?;
        let inst = &ents.instances[i];
        if inst.locked {
            return Err("the group or component is locked".into());
        }
        let def = inst.definition;
        h.target.push(i);
        h.touched.insert(def);
        Ok(())
    });
    e.register_fn(
        "end_group",
        with_host!(host, |h| {
            h.target.pop().ok_or("end_group without edit_group")?;
            Ok(())
        }),
    );
    let hh = host.clone();
    e.register_fn("set_name", move |x: Entity, name: &str| -> R<()> {
        hh.borrow_mut().write(|_, ents| {
            let i = instance(ents, x)?;
            ents.instances[i].name = name.to_owned();
            Ok(())
        })
    });
    let hh = host.clone();
    e.register_fn("move_by", move |x: Entity, v: Dynamic| -> R<()> {
        let v = point(&v)?;
        hh.borrow_mut().write(|_, ents| {
            let i = instance(ents, x)?;
            if ents.instances[i].locked {
                return Err("the group or component is locked".into());
            }
            let t = &mut ents.instances[i].transform;
            *t = DMat4::from_translation(v) * *t;
            Ok(())
        })
    });
    let hh = host.clone();
    e.register_fn("add_material", move |name: &str, rgb: Array| -> R<String> {
        let c: Vec<f64> = rgb.iter().map(num).collect::<R<_>>()?;
        if c.len() != 3 {
            return Err("a color is [r, g, b] with 0-255 values".into());
        }
        let color = [0, 1, 2].map(|i| c[i].clamp(0.0, 255.0) as u8);
        hh.borrow_mut().write(|model, _| {
            if !model.materials.iter().any(|m| m.name == name) {
                model.materials.push(mt_kernel::Material {
                    name: name.to_owned(),
                    color,
                    opacity: 1.0,
                    texture: None,
                });
            }
            Ok(name.to_owned())
        })
    });
    let hh = host.clone();
    e.register_fn("set_material", move |d: Dynamic, name: Dynamic| -> R<()> {
        let list = entities(&d)?;
        hh.borrow_mut().write(|model, ents| {
            let m = if name.is_unit() {
                None
            } else {
                let n = name.to_string();
                Some(
                    model
                        .materials
                        .iter()
                        .position(|m| m.name == n)
                        .ok_or(format!("no material named `{n}`"))?,
                )
            };
            for x in list {
                match x {
                    Entity::Face(f) => {
                        let f = face(ents, Entity::Face(f))?;
                        ents.faces[f].attrs.material = m;
                    }
                    x => {
                        let i = instance(ents, x)?;
                        ents.instances[i].material = m;
                    }
                }
            }
            Ok(())
        })
    });
    let hh = host.clone();
    e.register_fn("add_tag", move |name: &str| -> R<String> {
        hh.borrow_mut().write(|model, _| {
            if !model.layers.iter().any(|l| l.name == name) {
                model.layers.push(mt_kernel::Layer {
                    name: name.to_owned(),
                    ..Default::default()
                });
            }
            Ok(name.to_owned())
        })
    });
    let hh = host.clone();
    e.register_fn("set_tag", move |d: Dynamic, name: Dynamic| -> R<()> {
        let list = entities(&d)?;
        hh.borrow_mut().write(|model, ents| {
            let l = if name.is_unit() {
                None
            } else {
                let n = name.to_string();
                Some(
                    model
                        .layers
                        .iter()
                        .position(|m| m.name == n)
                        .ok_or(format!("no tag named `{n}`"))?,
                )
            };
            for x in list {
                match x {
                    Entity::Face(f) if ents.faces.contains_key(f) => ents.faces[f].attrs.layer = l,
                    Entity::Edge(k) if ents.edges.contains_key(k) => ents.edges[k].attrs.layer = l,
                    x => {
                        let i = instance(ents, x)?;
                        ents.instances[i].layer = l;
                    }
                }
            }
            Ok(())
        })
    });
}

fn make_group(h: &Rc<RefCell<Host>>, d: &Dynamic, name: &str, group: bool) -> R<Entity> {
    let list = entities(d)?;
    h.borrow_mut().write(|model, ents| {
        let mut sel = mt_kernel::group::Selection::default();
        for x in list {
            match x {
                Entity::Edge(k) if ents.edges.contains_key(k) => sel.edges.push(k),
                Entity::Face(f) if ents.faces.contains_key(f) => sel.faces.push(f),
                Entity::Instance(i) if i < ents.instances.len() => sel.instances.push(i),
                _ => return Err("make_group: an entity is not in the active context".into()),
            }
        }
        // Edges of selected faces go with them.
        for f in sel.faces.clone() {
            for l in ents.faces[f].loops() {
                sel.edges.extend(l.coedges.iter().map(|c| c.edge));
            }
        }
        sel.edges.sort_unstable();
        sel.edges.dedup();
        let base = match (name.is_empty(), group) {
            (false, _) => name.to_owned(),
            (true, true) => "Group#1".to_owned(),
            (true, false) => "Component#1".to_owned(),
        };
        let def_name = model.unique_definition_name(&base);
        let i = mt_kernel::group::make_group(&mut model.definitions, ents, &sel, def_name, group)
            .ok_or("make_group: nothing to group")?;
        Ok(Entity::Instance(i))
    })
}

// ---------------------------------------------------------------------------------
// Attribute dictionaries (groups and components; saved in .skp).

/// Longest dictionary name or key, and most dictionaries per entity and entries per
/// dictionary (keeps a runaway script from bloating the model).
const MAX_ATTR_NAME: usize = 255;
const MAX_DICTS: usize = 256;
const MAX_ENTRIES: usize = 4096;
/// Nesting of array values.
const MAX_ATTR_DEPTH: usize = 8;

fn attr_name(s: &str, what: &str) -> R<String> {
    if s.is_empty() || s.chars().count() > MAX_ATTR_NAME {
        return Err(format!("{what} must be 1-{MAX_ATTR_NAME} characters").into());
    }
    Ok(s.to_owned())
}

/// A script value as an attribute value: `()`, bool, int, float, string or an
/// array of these.
pub(crate) fn attr_value(d: &Dynamic, depth: usize) -> R<mt_skp::AttrValue> {
    use mt_skp::AttrValue as A;
    if depth > MAX_ATTR_DEPTH {
        return Err("attribute arrays are nested too deeply".into());
    }
    Ok(if d.is_unit() {
        A::Null
    } else if let Ok(b) = d.as_bool() {
        A::Boolean(b)
    } else if let Ok(i) = d.as_int() {
        A::Integer(i)
    } else if let Ok(f) = d.as_float() {
        A::Float(f)
    } else if let Ok(c) = d.as_char() {
        A::String(c.to_string())
    } else if d.is_string() {
        A::String(d.clone().into_immutable_string()?.to_string())
    } else if let Some(a) = d.clone().try_cast::<Array>() {
        A::Array(
            a.iter()
                .map(|x| attr_value(x, depth + 1))
                .collect::<R<_>>()?,
        )
    } else {
        return Err(format!(
            "an attribute value is (), a bool, a number, a string or an array of these, not {}",
            d.type_name()
        )
        .into());
    })
}

/// An attribute value as a script value (points and lengths as numbers).
pub(crate) fn attr_dynamic(v: &mt_skp::AttrValue) -> Dynamic {
    use mt_skp::AttrValue as A;
    match v {
        A::Null => Dynamic::UNIT,
        A::String(s) => s.clone().into(),
        A::Integer(i) => (*i).into(),
        A::Float(x) | A::Length(x) => (*x).into(),
        A::Vec(p) => Dynamic::from_array(p.iter().map(|x| (*x).into()).collect()),
        A::Array(items) => Dynamic::from_array(items.iter().map(attr_dynamic).collect()),
        A::Boolean(b) => (*b).into(),
    }
}

/// The dictionaries of a group or component of the active context.
fn dicts_of(ents: &Entities, x: Entity) -> R<&mt_skp::AttrDicts> {
    match x {
        Entity::Instance(_) => Ok(&ents.instances[instance(ents, x)?].attrs.dictionaries),
        _ => Err("attribute dictionaries are kept on groups and components only".into()),
    }
}

fn register_attributes(e: &mut Engine, host: &Rc<RefCell<Host>>) {
    let get = |hh: &Rc<RefCell<Host>>, x: Entity, dict: &str, key: &str| -> R<Dynamic> {
        let h = hh.borrow();
        let ents = h.active()?;
        Ok(dicts_of(ents, x)?
            .get(dict)
            .and_then(|d| d.get(key))
            .map_or(Dynamic::UNIT, attr_dynamic))
    };
    let hh = host.clone();
    e.register_fn(
        "get_attribute",
        move |x: Entity, dict: &str, key: &str| -> R<Dynamic> { get(&hh, x, dict, key) },
    );
    let hh = host.clone();
    e.register_fn(
        "get_attribute",
        move |x: Entity, dict: &str, key: &str, default: Dynamic| -> R<Dynamic> {
            let v = get(&hh, x, dict, key)?;
            Ok(if v.is_unit() { default } else { v })
        },
    );
    let hh = host.clone();
    e.register_fn("attribute_dictionaries", move |x: Entity| -> R<Array> {
        let h = hh.borrow();
        let ents = h.active()?;
        Ok(dicts_of(ents, x)?
            .keys()
            .map(|k| Dynamic::from(k.clone()))
            .collect())
    });
    let hh = host.clone();
    e.register_fn("attributes", move |x: Entity, dict: &str| -> R<Dynamic> {
        let h = hh.borrow();
        let ents = h.active()?;
        Ok(match dicts_of(ents, x)?.get(dict) {
            None => Dynamic::UNIT,
            Some(d) => Dynamic::from_map(
                d.iter()
                    .map(|(k, v)| (k.as_str().into(), attr_dynamic(v)))
                    .collect(),
            ),
        })
    });
    let hh = host.clone();
    e.register_fn(
        "set_attribute",
        move |x: Entity, dict: &str, key: &str, value: Dynamic| -> R<Dynamic> {
            let dict = attr_name(dict, "a dictionary name")?;
            let key = attr_name(key, "an attribute key")?;
            let v = attr_value(&value, 0)?;
            hh.borrow_mut().write(|_, ents| {
                dicts_of(ents, x)?;
                let i = instance(ents, x)?;
                let dicts = &mut ents.instances[i].attrs.dictionaries;
                if !dicts.contains_key(&dict) && dicts.len() >= MAX_DICTS {
                    return Err(format!("at most {MAX_DICTS} dictionaries per entity").into());
                }
                let d = dicts.entry(dict).or_default();
                if !d.contains_key(&key) && d.len() >= MAX_ENTRIES {
                    return Err(format!("at most {MAX_ENTRIES} attributes per dictionary").into());
                }
                d.insert(key, v);
                Ok(value)
            })
        },
    );
    let hh = host.clone();
    e.register_fn(
        "delete_attribute",
        move |x: Entity, dict: &str, key: &str| -> R<bool> {
            let mut h = hh.borrow_mut();
            // Nothing to delete: no change (no undo step).
            if !dicts_of(h.active()?, x)?
                .get(dict)
                .is_some_and(|d| d.contains_key(key))
            {
                return Ok(false);
            }
            h.write(|_, ents| {
                let i = instance(ents, x)?;
                let dicts = &mut ents.instances[i].attrs.dictionaries;
                if let Some(d) = dicts.get_mut(dict) {
                    d.remove(key);
                    if d.is_empty() {
                        dicts.remove(dict);
                    }
                }
                Ok(true)
            })
        },
    );
    let hh = host.clone();
    e.register_fn(
        "delete_attribute",
        move |x: Entity, dict: &str| -> R<bool> {
            let mut h = hh.borrow_mut();
            if !dicts_of(h.active()?, x)?.contains_key(dict) {
                return Ok(false);
            }
            h.write(|_, ents| {
                let i = instance(ents, x)?;
                ents.instances[i].attrs.dictionaries.remove(dict);
                Ok(true)
            })
        },
    );
}

fn register_ui(e: &mut Engine, host: &Rc<RefCell<Host>>) {
    let hh = host.clone();
    e.register_fn("message", move |s: Dynamic| {
        hh.borrow_mut()
            .requests
            .push(Request::Message(s.to_string()));
    });
    let hh = host.clone();
    e.register_fn("status", move |s: Dynamic| {
        hh.borrow_mut()
            .requests
            .push(Request::Status(s.to_string()));
    });
    let hh = host.clone();
    e.register_fn("log", move |s: Dynamic| {
        hh.borrow().log(&s.to_string(), false)
    });
    let hh = host.clone();
    e.register_fn(
        "input",
        move |title: &str, fields: Array, f: Dynamic| -> R<()> {
            let mut h = hh.borrow_mut();
            let fields = parse_fields(&fields, h.units, &h.unit_options)?;
            let callback = callback(f)?;
            h.requests.push(Request::Input {
                title: title.to_owned(),
                fields,
                callback,
            });
            Ok(())
        },
    );
    let hh = host.clone();
    e.register_fn("set_state", move |k: &str, v: Dynamic| {
        hh.borrow_mut().state.insert(k.into(), v);
    });
    let hh = host.clone();
    e.register_fn("get_state", move |k: &str| -> Dynamic {
        hh.borrow().state.get(k).cloned().unwrap_or(Dynamic::UNIT)
    });
    let hh = host.clone();
    e.register_fn("extension_id", move || -> String {
        hh.borrow().ext_id.clone()
    });
}

fn register_units(e: &mut Engine, host: &Rc<RefCell<Host>>) {
    for (name, inches) in [
        ("inch", 1.0),
        ("feet", 12.0),
        ("mm", 1.0 / 25.4),
        ("cm", 1.0 / 2.54),
        ("m", 1000.0 / 25.4),
    ] {
        e.register_fn(name, move |x: Dynamic| -> R<f64> { Ok(num(&x)? * inches) });
        let to = format!("to_{name}");
        e.register_fn(to.as_str(), move |x: Dynamic| -> R<f64> {
            Ok(num(&x)? / inches)
        });
    }
    let hh = host.clone();
    e.register_fn("format_length", move |x: Dynamic| -> R<String> {
        let h = hh.borrow();
        Ok(mt_infer::units::format_length(
            num(&x)?,
            h.units,
            &h.unit_options,
        ))
    });
    let hh = host.clone();
    e.register_fn("format_area", move |x: Dynamic| -> R<String> {
        let h = hh.borrow();
        Ok(mt_infer::units::format_area(num(&x)?, &h.unit_options))
    });
    let hh = host.clone();
    e.register_fn("parse_length", move |s: &str| -> Dynamic {
        let h = hh.borrow();
        mt_infer::units::parse_length(s, h.units).map_or(Dynamic::UNIT, Dynamic::from)
    });
    let hh = host.clone();
    e.register_fn("units", move || -> String {
        hh.borrow().units.suffix().trim().to_owned()
    });
    // Small vector helpers for points [x, y, z].
    e.register_fn("vadd", |a: Dynamic, b: Dynamic| -> R<Dynamic> {
        Ok(to_array(point(&a)? + point(&b)?))
    });
    e.register_fn("vsub", |a: Dynamic, b: Dynamic| -> R<Dynamic> {
        Ok(to_array(point(&a)? - point(&b)?))
    });
    e.register_fn("vscale", |a: Dynamic, s: Dynamic| -> R<Dynamic> {
        Ok(to_array(point(&a)? * num(&s)?))
    });
    e.register_fn("distance", |a: Dynamic, b: Dynamic| -> R<f64> {
        Ok(point(&a)?.distance(point(&b)?))
    });
}

fn register_io(e: &mut Engine, host: &Rc<RefCell<Host>>) {
    let hh = host.clone();
    e.register_fn("http_get", move |url: &str, f: Dynamic| -> R<()> {
        let mut h = hh.borrow_mut();
        h.need(Permission::Net)?;
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err("http_get: only http:// and https:// URLs".into());
        }
        let callback = callback(f)?;
        h.requests.push(Request::HttpGet {
            url: url.to_owned(),
            callback,
        });
        Ok(())
    });
    let hh = host.clone();
    e.register_fn("read_file", move |name: &str| -> R<String> {
        let path = data_file(&hh.borrow(), name)?;
        std::fs::read_to_string(&path).map_err(|e| format!("{name}: {e}").into())
    });
    let hh = host.clone();
    e.register_fn("write_file", move |name: &str, text: &str| -> R<()> {
        let path = data_file(&hh.borrow(), name)?;
        std::fs::write(&path, text).map_err(|e| format!("{name}: {e}").into())
    });
    let hh = host.clone();
    e.register_fn("list_files", move || -> R<Array> {
        let h = hh.borrow();
        h.need(Permission::File)?;
        let dir = super::package::extension_data_dir(&h.ext_id).ok_or("no data folder")?;
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        Ok(names.into_iter().map(Dynamic::from).collect())
    });
    // The extension's folder for its own icons (read-only, informational).
    let hh = host.clone();
    e.register_fn("extension_dir", move || -> String {
        hh.borrow().dir.display().to_string()
    });
}

/// The map an `on_click` / `on_move` gets with the point: the inference's kind and
/// the face normal under the cursor.
pub(crate) fn inference_map(inf: &mt_infer::Inference) -> Map {
    let mut m = Map::new();
    m.insert("kind".into(), format!("{:?}", inf.kind).into());
    m.insert("label".into(), inf.label().into());
    m.insert("normal".into(), inf.normal.map_or(Dynamic::UNIT, to_array));
    m.insert("locked".into(), inf.locked.into());
    m
}

pub(crate) fn point_dynamic(p: DVec3) -> Dynamic {
    to_array(p)
}

/// The polylines an `on_move` returned: an array of arrays of points.
pub(crate) fn polylines(v: &Dynamic) -> Vec<Vec<DVec3>> {
    let Some(a) = v.clone().try_cast::<Array>() else {
        return Vec::new();
    };
    a.iter()
        .filter_map(|l| l.clone().try_cast::<Array>())
        .map(|l| l.iter().filter_map(|p| point(p).ok()).collect::<Vec<_>>())
        .filter(|l| l.len() >= 2)
        .collect()
}

/// Ids for menu commands: `ext:<extension id>:<command id>`.
pub(crate) fn app_command_id(ext: &str, cmd: &str) -> String {
    format!("ext:{ext}:{cmd}")
}

pub(crate) fn split_command_id(id: &str) -> Option<(&str, &str)> {
    id.strip_prefix("ext:")?.split_once(':')
}

/// Builds the string a console shows for a value (`()` shows nothing).
pub(crate) fn show_value(v: &Dynamic) -> Option<String> {
    if v.is_unit() {
        None
    } else if let Some(s) = v.clone().try_cast::<ImmutableString>() {
        Some(format!("{s:?}"))
    } else {
        Some(format!("{v:?}"))
    }
}

/// The granted permissions for a manifest: declared and not revoked, and sensitive
/// ones only when approved.
pub(crate) fn granted(
    manifest: &Manifest,
    approved: &[Permission],
    revoked: &[Permission],
) -> BTreeSet<Permission> {
    manifest
        .permissions
        .iter()
        .copied()
        .filter(|p| !p.sensitive() || approved.contains(p))
        .filter(|p| !revoked.contains(p))
        .collect()
}
