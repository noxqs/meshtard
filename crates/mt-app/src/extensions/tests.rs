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
    let granted = runtime::granted(m, &[Permission::Net], &[]);
    assert!(granted.contains(&Permission::Net) && !granted.contains(&Permission::File));
    // Revoked: gone even when approved.
    let granted = runtime::granted(m, &[Permission::Net], &[Permission::Net]);
    assert!(!granted.contains(&Permission::Net));
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

/// The catalog and `.mtx` files committed under `extensions/` match the examples
/// (apart from a `signature.json`, which must then be valid, by any key).
#[test]
fn committed_dist_matches_examples() {
    let text = std::fs::read_to_string(repo().join("extensions/catalog.json")).unwrap();
    let cat = catalog::Catalog::parse(&text).unwrap();
    if let Ok(sig) = std::fs::read(repo().join("extensions/catalog.json.sig")) {
        let s = signing::verify_catalog(text.as_bytes(), Some(&sig), &[]);
        assert!(!matches!(s, signing::SigStatus::Invalid(_)), "{s:?}");
    }
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
        let s = signing::verify_package(&pkg.files, &[]);
        assert!(!matches!(s, signing::SigStatus::Invalid(_)), "{s:?}");
        let mut files = pkg.files.clone();
        let signed = files.remove(signing::SIGNATURE_FILE).is_some();
        let folder = e.id.strip_prefix("meshtard.").unwrap();
        let fresh = package::pack(&examples().join(folder)).unwrap();
        assert_eq!(
            if signed {
                package::pack_files(&files).unwrap()
            } else {
                bytes
            },
            fresh,
            "extensions/dist is stale: run `cargo test -p mt-app build_extension_dist -- --ignored`"
        );
    }
}

/// Writes `extensions/dist/*.mtx` and `extensions/catalog.json` from the examples.
/// With `MESHTARD_SIGNING_KEY=<key file>` (from `meshtard --extension-keygen`; the
/// release key is kept outside the repository) the packages get a `signature.json`
/// and the catalog a `catalog.json.sig`; without it they are unsigned and a stale
/// `catalog.json.sig` is removed.
#[test]
#[ignore = "writes into the repository; run after changing an example"]
fn build_extension_dist() {
    let key = std::env::var_os("MESHTARD_SIGNING_KEY")
        .map(|p| signing::read_signing_key(&std::fs::read_to_string(p).unwrap()).unwrap());
    let signer = std::env::var("MESHTARD_SIGNER").unwrap_or_else(|_| "Meshtard".into());
    let mut entries = Vec::new();
    std::fs::create_dir_all(repo().join("extensions/dist")).unwrap();
    let mut dirs: Vec<_> = std::fs::read_dir(examples())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    dirs.sort();
    for dir in dirs {
        let mut bytes = package::pack(&dir).unwrap();
        if let Some(k) = &key {
            let mut files = Package::from_zip(&bytes).unwrap().files;
            signing::sign_package(&mut files, k, &signer);
            bytes = package::pack_files(&files).unwrap();
        }
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
    std::fs::write(repo().join("extensions/catalog.json"), &text).unwrap();
    let sig = repo().join("extensions/catalog.json.sig");
    match &key {
        Some(k) => std::fs::write(sig, signing::sign_catalog(text.as_bytes(), k, &signer)).unwrap(),
        None => {
            let _ = std::fs::remove_file(sig);
        }
    }
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

#[test]
fn attribute_dictionaries_are_kept_and_saved_to_skp() {
    let script = format!(
        r#"{COMMAND}
fn run_it() {{
    let g = new_group("Box");
    edit_group(g);
    add_line([0, 0, 0], [10, 0, 0]);
    end_group();
    set_attribute(g, "meshtard", "count", 3);
    set_attribute(g, "meshtard", "label", "hello");
    set_attribute(g, "meshtard", "ratio", 0.5);
    set_attribute(g, "meshtard", "flag", true);
    set_attribute(g, "meshtard", "list", [1, "a", 2.5]);
    set_attribute(g, "other", "x", 1);
    g
}}
fn read_back() {{
    let g = groups()[0];
    [get_attribute(g, "meshtard", "count"), get_attribute(g, "meshtard", "label"),
     get_attribute(g, "meshtard", "missing", "dflt"), attribute_dictionaries(g).len(),
     attributes(g, "meshtard").len()]
}}
fn remove() {{
    let g = groups()[0];
    [delete_attribute(g, "meshtard", "count"), delete_attribute(g, "other"),
     delete_attribute(g, "nothing")]
}}
fn on_face() {{ set_attribute(faces()[0], "a", "b", 1); }}
fn bad_value() {{ set_attribute(groups()[0], "a", "b", #{{ x: 1 }}); }}
"#
    );
    let (mut ext, _) = ext_with("attrs", &["model.read", "model.write", "ui.menu"], &script);
    let mut doc = Document::untitled();
    let out = ext
        .run_command("ext:test.attrs:go", Some(&mut doc), &[])
        .unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    let dicts = &doc.model.root.instances[0].attrs.dictionaries;
    assert_eq!(dicts.len(), 2);
    assert_eq!(dicts["meshtard"]["count"], mt_skp::AttrValue::Integer(3));
    let call = |ext: &mut Extensions, doc: &mut Document, f: &str| {
        ext.call(
            "test.attrs",
            &rhai::FnPtr::new(f).unwrap(),
            Vec::new(),
            Some(doc),
            &[],
        )
    };
    let v = call(&mut ext, &mut doc, "read_back").value.unwrap();
    let v: Vec<String> = (v.cast::<rhai::Array>().iter())
        .map(|x| x.to_string())
        .collect();
    assert_eq!(v, ["3", "hello", "dflt", "2", "5"]);

    // Saved in .skp and read back.
    let bytes = doc.to_skp().unwrap();
    let back = mt_kernel::import::from_skp(&mt_skp::parse(&bytes).unwrap());
    let inst = &back.root.instances[0];
    let d = &inst.attrs.dictionaries["meshtard"];
    assert_eq!(d["label"], mt_skp::AttrValue::String("hello".into()));
    assert_eq!(d["ratio"], mt_skp::AttrValue::Float(0.5));
    assert_eq!(d["flag"], mt_skp::AttrValue::Boolean(true));
    assert_eq!(d["count"], mt_skp::AttrValue::Integer(3));
    assert!(inst.attrs.dictionaries.contains_key("other"));

    // Delete: one undo step; undo brings them back.
    let out = call(&mut ext, &mut doc, "remove");
    let v: Vec<String> = (out.value.unwrap().cast::<rhai::Array>().iter())
        .map(|x| x.to_string())
        .collect();
    assert_eq!(v, ["true", "true", "false"]);
    let dicts = &doc.model.root.instances[0].attrs.dictionaries;
    assert!(!dicts.contains_key("other") && !dicts["meshtard"].contains_key("count"));
    assert!(doc.undo());
    assert!(
        doc.model.root.instances[0]
            .attrs
            .dictionaries
            .contains_key("other")
    );

    // Faces have none; maps are not values. Both fail and change nothing.
    rectangle(&mut doc);
    let state = doc.state();
    let e = call(&mut ext, &mut doc, "on_face").error.unwrap();
    assert!(e.contains("groups and components"), "{e}");
    let e = call(&mut ext, &mut doc, "bad_value").error.unwrap();
    assert!(e.contains("attribute value"), "{e}");
    assert_eq!(doc.state(), state);
}

#[test]
fn attributes_need_model_permissions() {
    let script = format!(
        "{COMMAND}\nfn run_it() {{ set_attribute(groups()[0], \"a\", \"b\", 1); }}\nfn get() {{ get_attribute(groups()[0], \"a\", \"b\") }}"
    );
    let (mut ext, _) = ext_with("attrperm", &["model.read", "ui.menu"], &script);
    let mut doc = Document::untitled();
    let mut console = Extensions::default();
    console.console_eval("new_group(\"G\")", Some(&mut doc), &[]);
    assert_eq!(doc.model.root.instances.len(), 1);
    let out = ext
        .run_command("ext:test.attrperm:go", Some(&mut doc), &[])
        .unwrap();
    assert!(out.error.unwrap().contains("model.write"));
    let out = ext.call(
        "test.attrperm",
        &rhai::FnPtr::new("get").unwrap(),
        Vec::new(),
        Some(&mut doc),
        &[],
    );
    assert!(out.error.is_none());
    assert!(out.value.unwrap().is_unit());
}

#[test]
fn revoking_one_permission_without_reinstalling() {
    let script = format!("{COMMAND}\nfn run_it() {{ add_line([0,0,0], [10,0,0]); }}");
    let (mut ext, _) = ext_with(
        "revoke",
        &["model.read", "model.write", "ui.menu", "net"],
        &script,
    );
    let id = "test.revoke";
    assert!(ext.is_granted(id, Permission::ModelWrite));
    assert!(
        !ext.is_granted(id, Permission::Net),
        "sensitive: not approved"
    );
    ext.set_permission(id, Permission::ModelWrite, false);
    assert!(!ext.is_granted(id, Permission::ModelWrite));
    let mut doc = Document::untitled();
    let out = ext
        .run_command("ext:test.revoke:go", Some(&mut doc), &[])
        .unwrap();
    assert!(out.error.unwrap().contains("model.write"));
    assert!(doc.model.root.edges.is_empty());
    // Approving a sensitive one later, and taking it back.
    ext.set_permission(id, Permission::Net, true);
    assert!(ext.is_granted(id, Permission::Net));
    assert!(ext.scripts[0].host.borrow().has(Permission::Net));
    ext.set_permission(id, Permission::Net, false);
    assert!(!ext.scripts[0].host.borrow().has(Permission::Net));
    // Restored.
    ext.set_permission(id, Permission::ModelWrite, true);
    let out = ext
        .run_command("ext:test.revoke:go", Some(&mut doc), &[])
        .unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    // Taking back ui.menu: the extension cannot register its command and says so.
    ext.set_permission(id, Permission::UiMenu, false);
    assert!(ext.scripts.is_empty());
    assert!(ext.installed[0].error.as_ref().unwrap().contains("ui.menu"));
    // An update keeps what was revoked.
    let pkg = Package::from_folder(&ext.installed[0].dir.clone()).unwrap();
    ext.install(&pkg, Vec::new()).unwrap();
    assert!(!ext.is_granted(id, Permission::UiMenu));
}

#[test]
fn signed_packages_install_and_broken_ones_do_not() {
    let (mut ext, root) = ext_with("signer_base", &["model.read"], "let x = 1;");
    let src = package::tests::scratch("signer_src");
    std::fs::write(
        src.join("extension.toml"),
        "id = \"test.signed\"\nname = \"Signed\"\nversion = \"1.0\"\n",
    )
    .unwrap();
    std::fs::write(src.join("main.rhai"), "let x = 1;").unwrap();
    let mut pkg = Package::from_folder(&src).unwrap();
    // Unsigned: allowed (the UI warns), shown as unsigned.
    assert_eq!(
        ext.check_signature(&pkg).unwrap(),
        signing::SigStatus::Unsigned
    );
    let key = signing::test_key();
    signing::sign_package(&mut pkg.files, &key, "Tester");
    assert!(matches!(
        ext.check_signature(&pkg).unwrap(),
        signing::SigStatus::UnknownKey { .. }
    ));
    let pk = signing::public_key_hex(&key.verifying_key().to_bytes());
    ext.add_trusted_key("Test key", &pk).unwrap();
    assert!(ext.add_trusted_key("Again", &pk).is_err(), "no duplicates");
    assert!(ext.add_trusted_key("Bad", "1234").is_err());
    assert!(ext.check_signature(&pkg).unwrap().trusted());
    // Through a .mtx and installed: the manager shows who signed it.
    let bytes = package::pack_files(&pkg.files).unwrap();
    let pkg = Package::from_zip(&bytes).unwrap();
    ext.install(&pkg, Vec::new()).unwrap();
    assert!(
        root.join("test.signed")
            .join(signing::SIGNATURE_FILE)
            .exists()
    );
    let item = ext
        .installed
        .iter()
        .find(|i| i.manifest.as_ref().is_ok_and(|m| m.id == "test.signed"))
        .unwrap();
    assert_eq!(item.signature.describe(), "Signed by Test key");
    // A file changed after signing: refused.
    let mut bad = pkg.clone();
    bad.files.insert("main.rhai".into(), b"let x = 2;".to_vec());
    assert!(ext.check_signature(&bad).unwrap_err().contains("invalid"));
    assert!(
        ext.install(&bad, Vec::new())
            .unwrap_err()
            .contains("signature")
    );
    // Removing the key: signed by an untrusted key again.
    ext.remove_trusted_key("Test key");
    assert!(!ext.check_signature(&pkg).unwrap().trusted());
}

#[test]
fn tool_cursors_from_the_manifest_and_register_tool() {
    let script = r#"
register_tool(#{ id: "a", label: "A", menu: "Extensions > A", on_click: "c" });
register_tool(#{ id: "b", label: "B", menu: "Extensions > B", on_click: "c",
    cursor: "b.png", cursor_hotspot: [1, 2] });
register_tool(#{ id: "c", label: "C", menu: "Extensions > C", on_click: "c" });
fn c(p, i) {}
"#;
    let (mut ext, root) = ext_with("cursor", &["model.read", "ui.menu"], script);
    let dir = root.join("test.cursor");
    let toml = std::fs::read_to_string(dir.join("extension.toml")).unwrap();
    std::fs::write(
        dir.join("extension.toml"),
        format!("{toml}\n[cursors]\na = {{ image = \"a.png\", hotspot = [4, 28] }}\n"),
    )
    .unwrap();
    ext.load_all();
    let (path, c) = ext.tool_cursor("ext:test.cursor:a").unwrap();
    assert_eq!(c.hotspot, [4, 28]);
    assert_eq!(path, dir.join("a.png"));
    let (_, c) = ext.tool_cursor("ext:test.cursor:b").unwrap();
    assert_eq!((c.image.as_str(), c.hotspot), ("b.png", [1, 2]));
    assert!(ext.tool_cursor("ext:test.cursor:c").is_none());
    // A cursor image is checked when loaded: size and hotspot.
    let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 0, 255]));
    img.save(dir.join("a.png")).unwrap();
    let small = super::manifest::ToolCursor::new("a.png", [2, 2]).unwrap();
    assert!(ui::load_cursor(&dir.join("a.png"), &small).is_ok());
    let outside = super::manifest::ToolCursor::new("a.png", [9, 2]).unwrap();
    assert!(ui::load_cursor(&dir.join("a.png"), &outside).is_err());
    image::RgbaImage::new(65, 8)
        .save(dir.join("big.png"))
        .unwrap();
    assert!(ui::load_cursor(&dir.join("big.png"), &small).is_err());
    // A bad cursor in register_tool stops the load.
    let bad = r#"register_tool(#{ id: "a", label: "A", menu: "Extensions > A", on_click: "c",
        cursor: "../x.png" }); fn c(p, i) {}"#;
    let (ext, _) = ext_with("badcursor", &["model.read", "ui.menu"], bad);
    assert!(ext.installed[0].error.as_ref().unwrap().contains("cursor"));
}

/// Attribute dictionaries of the corpus files' groups and components are kept
/// through import, save and import again.
#[test]
fn corpus_attribute_dictionaries_survive_a_save() {
    fn all(m: &mt_kernel::Model) -> Vec<mt_skp::AttrDicts> {
        let mut out: Vec<_> = (m.root.instances.iter())
            .chain(m.definitions.values().flat_map(|d| &d.entities.instances))
            .map(|i| i.attrs.dictionaries.clone())
            .filter(|d| !d.is_empty())
            .collect();
        out.sort_by_key(|d| format!("{:?}", d.keys().collect::<Vec<_>>()));
        out
    }
    let dir = repo().join("tests/corpus/openskp");
    let mut seen = 0;
    for e in std::fs::read_dir(dir).unwrap() {
        let path = e.unwrap().path();
        if path.extension().is_none_or(|x| x != "skp") {
            continue;
        }
        let model = mt_kernel::import::from_skp(&mt_skp::read(&path).unwrap());
        let before = all(&model);
        if before.is_empty() {
            continue;
        }
        seen += 1;
        let bytes = mt_kernel::export::to_skp(&model).unwrap();
        let after = all(&mt_kernel::import::from_skp(
            &mt_skp::parse(&bytes).unwrap(),
        ));
        assert_eq!(after.len(), before.len(), "{}", path.display());
        for (a, b) in after.iter().zip(&before) {
            assert_eq!(
                a.keys().collect::<Vec<_>>(),
                b.keys().collect::<Vec<_>>(),
                "{}",
                path.display()
            );
            for (name, d) in a {
                assert_eq!(d.len(), b[name].len(), "{}: {name}", path.display());
            }
        }
    }
    assert!(seen > 0, "some corpus file has attribute dictionaries");
}
