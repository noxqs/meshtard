//! Headless tests of the extension runtime: permissions, rollback, budgets, the
//! examples against a test model, menus, and the committed Warehouse files.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use glam::DVec3;

use super::manifest::{Manifest, Permission};
use super::package::{self, Config, Package};
use super::runtime;
use super::*;
use crate::document::{Document, Owner};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn examples() -> PathBuf {
    repo().join("extensions/examples")
}

/// An extension in a fresh folder with `script` as main.rhai.
fn ext_with(name: &str, permissions: &[&str], script: &str) -> (Extensions, PathBuf) {
    let root = package::tests::scratch(name);
    let dir = root.join(format!("test.{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let perms: Vec<String> = permissions.iter().map(|p| format!("\"{p}\"")).collect();
    std::fs::write(
        dir.join("extension.toml"),
        format!(
            "id = \"test.{name}\"\nname = \"{name}\"\nversion = \"1.0\"\npermissions = [{}]\n",
            perms.join(", ")
        ),
    )
    .unwrap();
    std::fs::write(dir.join("main.rhai"), script).unwrap();
    let mut ext = Extensions::new(Some(root.clone()), Config::default());
    ext.load_all();
    for i in &ext.installed {
        if let Some(e) = &i.error {
            eprintln!("{}: {e}", i.dir.display());
        }
    }
    (ext, root)
}

fn rectangle(doc: &mut Document) {
    doc.edit(|e| {
        let p = [
            DVec3::ZERO,
            DVec3::new(10.0, 0.0, 0.0),
            DVec3::new(10.0, 5.0, 0.0),
            DVec3::new(0.0, 5.0, 0.0),
        ];
        for i in 0..4 {
            mt_kernel::add_line(e, p[i], p[(i + 1) % 4]);
        }
        true
    });
}

const COMMAND: &str = r#"
register_command(#{ id: "go", label: "Go", menu: "Extensions > Test", callback: "run_it" });
"#;

#[test]
fn script_without_model_write_cannot_modify() {
    let script = format!("{COMMAND}\nfn run_it() {{ add_line([0,0,0], [10,0,0]); }}");
    let (mut ext, _) = ext_with("readonly", &["model.read", "ui.menu"], &script);
    let mut doc = Document::untitled();
    let out = ext
        .run_command("ext:test.readonly:go", Some(&mut doc), &[])
        .unwrap();
    let err = out.error.unwrap();
    assert!(
        err.contains("permission denied") && err.contains("model.write"),
        "{err}"
    );
    assert!(!out.changed);
    assert!(doc.model.root.edges.is_empty());
    assert!(!doc.can_undo());
    assert_eq!(ext.errors.len(), 1, "shown in an error dialog");
}

#[test]
fn menu_permission_is_needed_to_register() {
    let (ext, _) = ext_with("nomenu", &["model.read"], COMMAND);
    assert!(ext.scripts.is_empty());
    let err = ext.installed[0].error.as_ref().unwrap();
    assert!(err.contains("ui.menu"), "{err}");
}

#[test]
fn net_and_file_need_approval() {
    let script = format!(
        "{COMMAND}\nfn run_it() {{ write_file(\"a.txt\", \"x\"); }}\nfn net() {{ http_get(\"https://example.com\", \"cb\"); }}"
    );
    let (mut ext, _) = ext_with("sensitive", &["ui.menu", "file", "net"], &script);
    // Declared but not approved at install: refused.
    let out = ext.run_command("ext:test.sensitive:go", None, &[]).unwrap();
    assert!(out.error.unwrap().contains("`file` was not granted"));
    let m = &ext.scripts[0].manifest;
    let granted = runtime::granted(m, &[Permission::Net]);
    assert!(granted.contains(&Permission::Net) && !granted.contains(&Permission::File));
}

#[test]
fn error_rolls_back_the_whole_call() {
    let script = format!(
        "{COMMAND}\nfn run_it() {{ add_line([0,0,0], [10,0,0]); add_line([10,0,0], [10,10,0]); throw \"oops\"; }}"
    );
    let (mut ext, _) = ext_with(
        "rollback",
        &["model.read", "model.write", "ui.menu"],
        &script,
    );
    let mut doc = Document::untitled();
    rectangle(&mut doc);
    let before = doc.model.clone();
    let undo_before = doc.can_undo();
    let state = doc.state();
    let out = ext
        .run_command("ext:test.rollback:go", Some(&mut doc), &[])
        .unwrap();
    assert!(out.error.unwrap().contains("oops"));
    assert!(!out.changed);
    assert_eq!(doc.model.root.edges.len(), before.root.edges.len());
    assert_eq!(doc.model.root.faces.len(), before.root.faces.len());
    assert_eq!(doc.state(), state);
    assert_eq!(doc.can_undo(), undo_before);
}

#[test]
fn a_change_is_one_undo_step() {
    let script = format!(
        "{COMMAND}\nfn run_it() {{ add_face([[0,0,0], [10,0,0], [10,10,0], [0,10,0]]); add_line([20,0,0], [30,0,0]); }}"
    );
    let (mut ext, _) = ext_with("undo", &["model.read", "model.write", "ui.menu"], &script);
    let mut doc = Document::untitled();
    let out = ext
        .run_command("ext:test.undo:go", Some(&mut doc), &[])
        .unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    assert!(out.changed);
    assert_eq!(doc.model.root.faces.len(), 1);
    assert_eq!(doc.model.root.edges.len(), 5);
    assert!(doc.undo());
    assert!(doc.model.root.edges.is_empty());
    assert!(!doc.can_undo());
}

#[test]
fn operation_budget_stops_an_endless_loop() {
    let script = format!("{COMMAND}\nfn run_it() {{ let i = 0; loop {{ i += 1; }} }}");
    let (mut ext, _) = ext_with("loop", &["ui.menu"], &script);
    let t = Instant::now();
    let out = ext.run_command("ext:test.loop:go", None, &[]).unwrap();
    let err = out.error.unwrap();
    assert!(err.starts_with("stopped"), "{err}");
    assert!(t.elapsed() < runtime::MAX_TIME + Duration::from_secs(2));
    // The extension still works afterwards.
    assert!(ext.command("ext:test.loop:go").is_some());
}

#[test]
fn broken_extensions_do_not_stop_the_others() {
    let (mut ext, root) = ext_with(
        "good",
        &["ui.menu"],
        &format!("{COMMAND}\nfn run_it() {{}}"),
    );
    let bad = root.join("test.bad");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(
        bad.join("extension.toml"),
        "id = \"test.bad\"\nname = \"Bad\"\nversion = \"1\"\n",
    )
    .unwrap();
    std::fs::write(bad.join("main.rhai"), "fn (").unwrap();
    std::fs::create_dir_all(root.join("garbage")).unwrap();
    ext.load_all();
    assert_eq!(ext.installed.len(), 3);
    assert_eq!(ext.scripts.len(), 1);
    assert!(ext.installed.iter().filter(|i| i.error.is_some()).count() == 2);
    assert!(
        ext.log
            .borrow()
            .iter()
            .any(|l| l.error && l.text.contains("Bad"))
    );
}

#[test]
fn disabled_extensions_are_not_loaded() {
    let (mut ext, _) = ext_with(
        "toggle",
        &["ui.menu"],
        &format!("{COMMAND}\nfn run_it() {{}}"),
    );
    assert_eq!(ext.scripts.len(), 1);
    ext.set_enabled("test.toggle", false);
    assert!(ext.scripts.is_empty());
    assert!(!ext.installed[0].enabled);
    ext.set_enabled("test.toggle", true);
    assert_eq!(ext.scripts.len(), 1);
}

#[test]
fn menus_toolbar_and_shortcuts_from_commands() {
    let script = r#"
register_command(#{ id: "a", label: "Do A", menu: "Extensions > Tools > Do A", toolbar: true,
    shortcut: "Ctrl+Shift+J", callback: "run_it" });
register_command(#{ id: "b", label: "Do B", menu: "Draw", callback: "run_it" });
fn run_it() {}
"#;
    let (mut ext, _) = ext_with("menus", &["ui.menu", "ui.toolbar"], script);
    let mut spec = mt_spec::Spec::builtin().unwrap();
    assert!(ext.apply_to_spec(&mut spec));
    assert!(!ext.apply_to_spec(&mut spec), "unchanged");
    let commands = spec.commands();
    assert_eq!(commands["ext:test.menus:a"], "Do A");
    assert_eq!(commands["ext:test.menus:b"], "Do B");
    let tb = spec
        .toolbars
        .toolbar
        .iter()
        .find(|t| t.id == TOOLBAR_ID)
        .unwrap();
    assert_eq!(tb.buttons().next().unwrap().id, "ext:test.menus:a");
    assert_eq!(
        ext.default_shortcuts(),
        [("ext:test.menus:a".to_owned(), "Ctrl+Shift+J".to_owned())]
    );
    // Unloading takes them out again.
    ext.set_enabled("test.menus", false);
    assert!(ext.apply_to_spec(&mut spec));
    assert!(!spec.commands().contains_key("ext:test.menus:a"));
    assert!(spec.toolbars.toolbar.iter().all(|t| t.id != TOOLBAR_ID));
}

#[test]
fn observers_are_throttled() {
    let script = r#"
on_selection_changed("seen", 200);
fn seen() { let n = get_state("n"); if n == () { n = 0; } set_state("n", n + 1); }
fn count() { get_state("n") }
"#;
    let (mut ext, _) = ext_with("observer", &["model.read"], script);
    let mut doc = Document::untitled();
    rectangle(&mut doc);
    let f = doc.model.root.faces.keys().next().unwrap();
    let t0 = Instant::now();
    let sel = [Owner::Face(f)];
    assert_eq!(ext.observe(Some(&mut doc), &[], t0).0.len(), 0);
    assert_eq!(ext.observe(Some(&mut doc), &sel, t0).0.len(), 1);
    // Changed again within 200 ms: waits.
    let (outs, wait) = ext.observe(Some(&mut doc), &[], t0 + Duration::from_millis(50));
    assert!(outs.is_empty());
    assert!(wait.unwrap() <= Duration::from_millis(150));
    let (outs, _) = ext.observe(Some(&mut doc), &[], t0 + Duration::from_millis(250));
    assert_eq!(outs.len(), 1);
    let f = rhai::FnPtr::new("count").unwrap();
    let n = ext.call("test.observer", &f, Vec::new(), None, &[]);
    assert_eq!(n.value.unwrap().as_int().unwrap(), 2);
}

#[test]
fn click_tools_get_points() {
    let script = r#"
register_tool(#{ id: "pt", label: "Point Tool", menu: "Extensions > Point Tool",
    status: "Click a point.", on_click: "clicked", on_move: "moved" });
fn clicked(p, info) { add_line(p, vadd(p, [0, 0, 10])); status(`clicked ${info.kind}`); }
fn moved(p, info) { [[p, vadd(p, [0, 0, 5])]] }
"#;
    let (mut ext, _) = ext_with("tool", &["model.read", "model.write", "ui.menu"], script);
    let mut doc = Document::untitled();
    let id = "ext:test.tool:pt";
    assert!(
        ext.run_command(id, Some(&mut doc), &[]).is_none(),
        "tools start in the app"
    );
    let mut tool = ext.start_tool(id).unwrap();
    use crate::tools::DrawTool;
    assert_eq!(tool.status(), "Click a point.");
    let inf = mt_infer::Inference {
        point: DVec3::new(1.0, 2.0, 0.0),
        kind: mt_infer::Kind::Free,
        edge: None,
        guide: None,
        locked: false,
        normal: None,
        segment: None,
        container: None,
    };
    tool.preview(&inf);
    let outs = ext.tool_events(Some(&mut doc), &[]);
    assert_eq!(outs.len(), 1);
    assert_eq!(tool.preview(&inf).len(), 1, "on_move's preview line");
    let mut ents = mt_kernel::Entities::new();
    assert!(!tool.click(&mut ents, &inf));
    let outs = ext.tool_events(Some(&mut doc), &[]);
    assert!(outs.iter().any(|o| o.changed));
    assert_eq!(doc.model.root.edges.len(), 1);
    assert_eq!(tool.status(), "clicked Free");
}

#[test]
fn console_evaluates_against_the_model() {
    let mut ext = Extensions::default();
    let mut doc = Document::untitled();
    rectangle(&mut doc);
    let out = ext.console_eval("let n = faces().len(); n", Some(&mut doc), &[]);
    assert_eq!(out.value.unwrap().as_int().unwrap(), 1);
    // Variables persist between lines.
    let out = ext.console_eval("n + 1", Some(&mut doc), &[]);
    assert_eq!(out.value.unwrap().as_int().unwrap(), 2);
    let out = ext.console_eval("add_line([0,0,0],[0,0,10]).len()", Some(&mut doc), &[]);
    assert!(out.changed);
    let out = ext.console_eval("read_file(\"x\")", Some(&mut doc), &[]);
    assert!(!out.changed);
    assert!(
        ext.log
            .borrow()
            .iter()
            .any(|l| l.error && l.text.contains("`file`"))
    );
    assert!(ext.errors.is_empty(), "console errors stay in the console");
}

#[test]
fn stair_maker_example_runs_headless() {
    let pkg = Package::from_folder(&examples().join("stair_maker")).unwrap();
    let root = package::tests::scratch("stairs");
    let mut ext = Extensions::new(Some(root), Config::default());
    ext.install(&pkg, Vec::new()).unwrap();
    assert_eq!(ext.scripts.len(), 1);
    let mut doc = Document::untitled();
    let out = ext
        .run_command("ext:meshtard.stair_maker:make", Some(&mut doc), &[])
        .unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    assert_eq!(ext.inputs.len(), 1);
    assert_eq!(ext.inputs[0].fields.len(), 4);
    // OK with the defaults: 10 steps.
    let out = ext.answer_input(true, Some(&mut doc), &[]).unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    assert!(out.changed);
    assert_eq!(out.selection, Some(vec![Owner::Instance(0)]));
    let root = &doc.model.root;
    assert!(root.faces.is_empty());
    assert_eq!(root.instances.len(), 1);
    let def = &doc.model.definitions[root.instances[0].definition];
    assert!(def.is_group);
    // Two sides, a tread and a riser per step, the back and the bottom.
    assert_eq!(def.entities.faces.len(), 2 + 2 * 10 + 2);
    assert!(mt_kernel::validate(&def.entities, mt_kernel::Checks::ALL).is_empty());
    assert!(mt_kernel::is_solid(&def.entities));
    assert!(doc.undo());
    assert!(doc.model.root.instances.is_empty());
    // Bad values: an error, nothing changes.
    ext.inputs.clear();
    let f = rhai::FnPtr::new("make_stair").unwrap();
    let mut v = rhai::Map::new();
    for (k, x) in [
        ("width", 36.0),
        ("rise", 7.0),
        ("run", 11.0),
        ("count", 0.0),
    ] {
        v.insert(k.into(), x.into());
    }
    let out = ext.call(
        "meshtard.stair_maker",
        &f,
        vec![rhai::Dynamic::from_map(v)],
        Some(&mut doc),
        &[],
    );
    assert!(out.error.unwrap().contains("Steps"));
    assert!(doc.model.root.instances.is_empty());
}

#[test]
fn selection_info_example_runs_headless() {
    let pkg = Package::from_folder(&examples().join("selection_info")).unwrap();
    let root = package::tests::scratch("selinfo");
    let mut ext = Extensions::new(Some(root), Config::default());
    ext.install(&pkg, Vec::new()).unwrap();
    let mut doc = Document::untitled();
    doc.units = mt_infer::units::LengthUnit::Inch;
    rectangle(&mut doc);
    let f = doc.model.root.faces.keys().next().unwrap();
    let e = doc.model.root.edges.keys().next().unwrap();
    let sel = [Owner::Face(f), Owner::Edge(e)];
    let now = Instant::now();
    ext.observe(Some(&mut doc), &[], now);
    let (outs, _) = ext.observe(Some(&mut doc), &sel, now);
    assert_eq!(outs.len(), 1);
    let status = ext.ui.status.take().unwrap();
    assert!(
        status.starts_with("2 selected: 1 face (") && status.contains("1 edge,"),
        "{status}"
    );
    assert!(status.contains("50"), "area 50 sq in: {status}");
    let out = ext
        .run_command("ext:meshtard.selection_info:show", Some(&mut doc), &sel)
        .unwrap();
    assert!(out.error.is_none());
    assert_eq!(ext.messages.len(), 1);
    assert!(ext.messages[0].1.contains("1 face"));
    // Read-only: no write permission.
    assert!(
        !ext.scripts[0]
            .manifest
            .permissions
            .contains(&Permission::ModelWrite)
    );
}

/// The catalog and `.mtx` files committed under `extensions/` match the examples.
#[test]
fn committed_dist_matches_examples() {
    let text = std::fs::read_to_string(repo().join("extensions/catalog.json")).unwrap();
    let cat = catalog::Catalog::parse(&text).unwrap();
    assert_eq!(cat.extensions.len(), 2);
    for e in &cat.extensions {
        let bytes = std::fs::read(repo().join("extensions").join(&e.url)).unwrap();
        catalog::verify(e, &bytes).unwrap();
        assert_eq!(bytes.len() as u64, e.size);
        let pkg = Package::from_zip(&bytes).unwrap();
        assert_eq!(
            (pkg.manifest.id.as_str(), pkg.manifest.version.as_str()),
            (e.id.as_str(), e.version.as_str())
        );
        let keys: Vec<&str> = pkg.manifest.permissions.iter().map(|p| p.key()).collect();
        assert_eq!(keys, e.permissions);
        let folder = e.id.strip_prefix("meshtard.").unwrap();
        assert_eq!(
            bytes,
            package::pack(&examples().join(folder)).unwrap(),
            "extensions/dist is stale: run `cargo test -p mt-app build_extension_dist -- --ignored`"
        );
    }
}

/// Writes `extensions/dist/*.mtx` and `extensions/catalog.json` from the examples.
#[test]
#[ignore = "writes into the repository; run after changing an example"]
fn build_extension_dist() {
    let mut entries = Vec::new();
    std::fs::create_dir_all(repo().join("extensions/dist")).unwrap();
    let mut dirs: Vec<_> = std::fs::read_dir(examples())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    dirs.sort();
    for dir in dirs {
        let bytes = package::pack(&dir).unwrap();
        let m: Manifest = Package::from_zip(&bytes).unwrap().manifest;
        let folder = dir.file_name().unwrap().to_string_lossy().into_owned();
        let file = format!("dist/{folder}-{}.mtx", m.version);
        std::fs::write(repo().join("extensions").join(&file), &bytes).unwrap();
        let perms: Vec<serde_json::Value> = m.permissions.iter().map(|p| p.key().into()).collect();
        entries.push(serde_json::json!({
            "id": m.id,
            "name": m.name,
            "version": m.version,
            "author": m.author,
            "description": m.description,
            "license": m.license,
            "permissions": perms,
            "min_meshtard": m.min_meshtard,
            "url": file,
            "sha256": catalog::sha256_hex(&bytes),
            "size": bytes.len(),
            "homepage": m.homepage,
        }));
    }
    let cat = serde_json::json!({
        "format": 1,
        "name": "Meshtard Extension Warehouse",
        "extensions": entries,
    });
    let text = serde_json::to_string_pretty(&cat).unwrap() + "\n";
    std::fs::write(repo().join("extensions/catalog.json"), text).unwrap();
}

#[test]
fn warehouse_install_checks_the_download() {
    let bytes = package::pack(&examples().join("stair_maker")).unwrap();
    let entry = catalog::Entry {
        id: "meshtard.stair_maker".into(),
        name: "Stair Maker".into(),
        version: "1.0.0".into(),
        author: String::new(),
        description: String::new(),
        license: String::new(),
        permissions: Vec::new(),
        min_meshtard: None,
        url: "x.mtx".into(),
        sha256: catalog::sha256_hex(&bytes),
        size: 0,
        homepage: String::new(),
    };
    catalog::verify(&entry, &bytes).unwrap();
    let mut bad = bytes.clone();
    let n = bad.len() / 2;
    bad[n] ^= 1;
    assert!(catalog::verify(&entry, &bad).is_err());
    // A local catalog (file path) resolves package URLs next to it.
    let dir = package::tests::scratch("localcat");
    std::fs::write(dir.join("x.mtx"), &bytes).unwrap();
    let url = catalog::resolve(&format!("file://{}/catalog.json", dir.display()), "x.mtx");
    assert_eq!(catalog::fetch(&url).unwrap(), bytes);
}

#[test]
fn input_fields_read_back() {
    use super::runtime::{Field, FieldKind};
    let field = |kind| Field {
        key: "k".into(),
        label: "K".into(),
        kind,
    };
    let mut fields = vec![
        field(FieldKind::Length(900.0 / 25.4)),
        field(FieldKind::Length(1.0)),
        field(FieldKind::Number(3.0)),
    ];
    let shown = ["~ 35.43\"", "~ 1\"", "3"].map(String::from).to_vec();
    let typed = ["~ 35.43\"", "~ 2\"", "4,5"].map(String::from).to_vec();
    let unit = mt_infer::units::LengthUnit::Inch;
    ui::read_fields(&mut fields, &typed, &shown, unit).unwrap();
    // Unchanged: the exact value; edited: parsed (an approximate mark is allowed).
    assert_eq!(fields[0].kind, FieldKind::Length(900.0 / 25.4));
    assert_eq!(fields[1].kind, FieldKind::Length(2.0));
    assert_eq!(fields[2].kind, FieldKind::Number(4.5));
    let bad = ["x"].map(String::from).to_vec();
    assert!(ui::read_fields(&mut fields[..1], &bad, &[], unit).is_err());
}
