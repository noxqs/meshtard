# Meshtard extensions

FEATURES.md item 69. SketchUp extensions are Ruby scripts against SketchUp's Ruby API;
Meshtard does not run them (a compatibility layer for a subset of that API is a later,
optional step). Meshtard has its own extension system instead: sandboxed scripts in
[Rhai](https://rhai.rs), a manifest with permissions, an Extension Manager, an
Extension Warehouse catalog and an Extension Console. All of this is a Meshtard design
(`differs` in STATUS.md), not a copy of SketchUp's.

Code: `crates/mt-app/src/extensions/` (`manifest.rs`, `package.rs`, `runtime.rs`,
`catalog.rs`, `ui.rs`, tests in `tests.rs`). Examples: `extensions/examples/`.

## Why Rhai

| Option | For | Against |
|---|---|---|
| **Rhai** (chosen) | Pure Rust, MIT/Apache-2.0, no C toolchain; sandboxed by default (no file, network or process access: a script can only call what the app registers); built-in operation limit, call depth, string/array/map size limits and a progress callback for a time limit; Rust-like syntax, easy to embed | Slower than Lua or WASM; a smaller ecosystem |
| Lua (mlua) | Fast, well known | C library to build on every platform; the sandbox must be built by removing `io`, `os`, `load`... |
| WASM (wasmtime) | Strong isolation, any source language | Heavy dependency; extension authors need a compiler toolchain; host API through an ABI |
| Ruby | SketchUp's language | Embedding MRI is large and not sandboxable; the SketchUp API is not ours to copy |

Rhai 1.x (`rhai` crate, default features). The engine is created per extension with:

- `DummyModuleResolver`: `import` is not available (no reading files); an extension
  with several script files lists them in `scripts` and they are compiled together.
- `set_max_operations(5_000_000)` per call and a 3-second wall-clock limit (progress
  callback), `max_call_levels(64)`, expression depth 64, strings up to 1 MB, arrays
  up to 1M items, maps up to 65,536 keys.
- `print` and `debug` go to the Extension Console.

## Package format

An extension is a folder, or the same folder zipped into a `.mtx` file (the folder's
contents at the zip's root, or inside one top-level folder):

```
stair_maker/
  extension.toml
  main.rhai
  icon.png          (optional, PNG)
```

Only `.rhai`, `.toml`, `.png`, `.md`, `.txt` and `.json` files are installed (others
are skipped); paths must stay inside the package (no `..`, no absolute paths: zip-slip
is refused); at most 500 files and 20 MB unpacked.

### `extension.toml`

```toml
id = "meshtard.stair_maker"     # a-z 0-9 _ - . (3-64 chars); also the install folder
name = "Stair Maker"
version = "1.0.0"               # major.minor.patch
author = "Meshtard examples"
description = "Makes a straight stair as a group."
license = "MIT"
min_meshtard = "0.1.0"          # refused by older Meshtard versions
permissions = ["model.read", "model.write", "ui.menu", "ui.toolbar"]
scripts = ["main.rhai"]         # optional; default ["main.rhai"]; run in this order
icon = "icon.png"               # optional
homepage = "https://..."        # optional
```

Unknown keys are ignored (newer manifests stay readable); unknown permissions,
bad ids and versions are errors.

### Permissions

| Permission | Allows | Asked at install |
|---|---|---|
| `model.read` | `edges()`, `faces()`, `groups()`, entity properties, `materials()`, `tags()`, `selection()`, `select()`, observers | no (listed) |
| `model.write` | `add_line`, `add_face`, `push_pull`, `erase`, `make_group`, `new_group`, `set_material`, `add_tag`... | no (listed) |
| `ui.menu` | `register_command` / `register_tool` with a `menu` path | no (listed) |
| `ui.toolbar` | `toolbar: true` on a command | no (listed) |
| `net` | `http_get(url, callback)` | **yes**: a check box the user must tick |
| `file` | `read_file`, `write_file`, `list_files` in `<data>/meshtard/extension_data/<id>/` only | **yes** |

A function whose permission was not declared (or, for `net` and `file`, not approved)
fails with "permission denied: `model.write` was not granted to ..." and the call is
rolled back. Messages, input dialogs, status text, units formatting and the
extension's own state need no permission.

## Where things live

- Installed extensions: `<data dir>/meshtard/extensions/<id>/` (Linux
  `~/.local/share/meshtard/extensions`, Windows `%LOCALAPPDATA%\meshtard\extensions`,
  macOS `~/Library/Application Support/meshtard/extensions`).
- Enabled state, approved permissions and the catalog URL override:
  `<config dir>/meshtard/extensions.toml`.
- Files of extensions with the `file` permission: `<data dir>/meshtard/extension_data/<id>/`.

Enabled extensions are loaded at startup after the main window (shell) exists; an
extension that fails to load (syntax error, bad manifest, missing permission for what
its top level does) is marked with its error in the Extension Manager and logged in
the Extension Console; startup continues.

## Script lifecycle

1. **Load**: the scripts' top level runs once. It registers commands, tools and
   observers. The model is not available during loading.
2. **Callbacks**: a command's `callback`, a tool's `on_click` / `on_move`, an
   observer, an input dialog's or an HTTP request's callback. Each runs with the
   active model and the selection.
3. **Budget**: each load or callback may run 5,000,000 operations and 3 seconds; then
   it stops with "stopped: the script ran more than ... operations (an endless loop?)".
4. **Undo and rollback**: all changes of one callback are one undo step (Edit > Undo
   reverts the whole callback). If the callback fails (error, `throw`, budget,
   permission) or the changed context or new groups fail `mt_kernel::validate`
   (problems that were already there do not count), the model is put back exactly as
   before the call: no undo step, nothing changed.
5. **Errors** show in an error dialog ("The extension ... stopped with an error.") and
   in the Extension Console log.

Functions defined in the script cannot see the top level's variables (a Rhai rule);
keep state with `set_state` / `get_state`. Entity handles are valid during the call
that returned them (instance handles are indices and change when instances are
erased).

Callbacks are given as a function name (`"ask"`) or a function pointer (`Fn("ask")`,
or a closure `|v| ...`).

## API reference

Lengths are in **inches** (the model's internal unit); use `mm(x)`, `cm(x)`, `m(x)`,
`feet(x)`, `inch(x)` to convert into inches and `to_mm(x)` ... to convert back, and
`format_length` to show a length in the model's units. Points and vectors are arrays
`[x, y, z]` (red, green, blue). All model functions work in the **active context**
(the open group or component, else the model root) or, after `edit_group(g)`, inside
that group.

### Registration (top level)

```rhai
register_command(#{
    id: "make",                          // letters, digits, _ - .
    label: "Make Stair...",
    menu: "Extensions > Stair Maker",    // top menu (existing label, else Extensions) > submenus
    toolbar: true,                       // a button on the "Extensions" toolbar (ui.toolbar)
    tooltip: "Stair Maker",
    icon: "icon.png",                    // PNG in the extension (toolbar button)
    shortcut: "Ctrl+Shift+K",            // default; changeable in Preferences > Shortcuts
    callback: "ask",
});

register_tool(#{                         // a click tool (becomes the active tool)
    id: "marker", label: "Marker", menu: "Extensions > Marker",
    status: "Click to place a marker.",
    on_click: "clicked",                 // fn clicked(point, info)
    on_move: "moved",                    // fn moved(point, info) -> [[p1, p2, ...], ...] preview polylines
});

on_selection_changed("update", 300);     // callback, throttle in ms (default 250, min 50)
on_model_changed("changed");
```

The command appears in the menu as `ext:<extension id>:<command id>` (that id is what
Preferences > Shortcuts lists; a declared shortcut is used only when no other command
has it). If the last part of `menu` equals the label it is the item itself, otherwise
every part names a submenu. `info` for tools is `#{ kind, label, normal, locked }`
(the inference under the cursor: `"Endpoint"`, `"OnFace"`, ...; `normal` is the face
normal or `()`). The point is in the active context's coordinates.

### Model (model.read)

| Function | Returns |
|---|---|
| `edges()`, `faces()`, `groups()` | arrays of entities of the active context (`groups()` = groups and components) |
| `materials()`, `tags()` | arrays of names |
| `selection()` | the selected entities |
| `select(entities)`, `clear_selection()` | sets the selection after the call |
| `e.kind` | `"edge"`, `"face"`, `"group"`, `"component"`, `"section_plane"`, `"dimension"`, `"text"` |
| `e.valid` | still exists |
| edge: `start`, `end`, `length`, `faces`, `vertices`, `tag` | |
| face: `area` (in²), `normal`, `vertices` (outer loop), `edges`, `material`, `tag` | |
| group/component: `name`, `definition`, `origin`, `bounds` (`[min, max]` in the context), `material`, `tag`, `entity_count` | |

### Model (model.write)

| Function | Does |
|---|---|
| `add_line(a, b)` | an edge (split and merged like the Line tool; closed loops get faces); returns the edges |
| `add_face(points)` | the edges of a closed loop; returns the face or `()` |
| `push_pull(face, distance)` | along the face normal; returns the moved face or `()` |
| `erase(entity_or_array)` | locked groups are kept |
| `new_group(name)` | an empty group; returns it |
| `edit_group(g)` / `end_group()` | later calls work inside `g` / back out |
| `make_group(entities [, name])`, `make_component(entities [, name])` | moves the entities (faces bring their edges) into a new group / component |
| `set_name(g, name)`, `move_by(g, vector)` | instance name; translate |
| `add_material(name, [r, g, b])` | adds a color material if there is none by that name |
| `set_material(entity_or_array, name_or_())` | faces (front) and groups |
| `add_tag(name)`, `set_tag(entity_or_array, name_or_())` | |

### UI

| Function | Does |
|---|---|
| `message(text)` | a dialog with OK |
| `status(text)` | status bar text (and the active tool's status) |
| `log(value)`, `print(value)` | a line in the Extension Console |
| `input(title, fields, callback)` | a dialog; OK calls `callback(values)` with a map `key -> value` |

Input fields: `#{ key, label, type, value }` with `type` = `"text"`, `"number"`,
`"length"` (inches; shown and typed in the model's units) or `"choice"` with
`options: [...]`.

### Units and helpers

`units()` (e.g. `"mm"`), `format_length(in)`, `format_area(in²)`,
`parse_length(text)` (in the model's units, `()` if not a length), `mm`, `cm`, `m`,
`feet`, `inch`, `to_mm`... , `vadd(a, b)`, `vsub(a, b)`, `vscale(a, s)`,
`distance(a, b)`, `set_state(key, value)`, `get_state(key)`, `extension_id()`.

### Network and files (asked at install)

```rhai
http_get("https://example.com/data.json", |r| {   // net
    if r.ok { log(r.body.len()) } else { log(r.error) }
});
write_file("notes.txt", "hello");                  // file: own data folder only
let t = read_file("notes.txt");
let names = list_files();
```

`http_get` runs on a background thread; the callback gets `#{ ok, body, error }`
(bodies over 8 MB are refused).

## Examples

### Stair Maker (`extensions/examples/stair_maker`)

```rhai
register_command(#{ id: "make", label: "Make Stair...", menu: "Extensions > Stair Maker",
    toolbar: true, icon: "icon.png", callback: "ask" });

fn ask() {
    input("Stair Maker", [
        #{ key: "width", label: "Width", type: "length", value: mm(900.0) },
        #{ key: "rise", label: "Rise per step", type: "length", value: mm(175.0) },
        #{ key: "run", label: "Run (tread depth)", type: "length", value: mm(280.0) },
        #{ key: "count", label: "Steps", type: "number", value: 10 },
    ], "make_stair");
}

fn make_stair(v) {
    let g = new_group("Stair");
    edit_group(g);
    let face = add_face(profile(v.rise, v.run, v.count.to_int()));
    push_pull(face, if face.normal[1] > 0.0 { v.width } else { -v.width });
    end_group();
    select([g]);
}
```

### Selection Info (`extensions/examples/selection_info`)

```rhai
on_selection_changed("update_status", 300);

fn update_status() {
    let area = 0.0;
    let faces = 0;
    for e in selection() {
        if e.kind == "face" { faces += 1; area += e.area; }
    }
    status(`${selection().len()} selected: ${faces} faces (${format_area(area)})`);
}
```

## Extension Manager (Extensions > Extension Manager)

A modeless window: Install from File... (`.mtx`), Install from Folder..., the
installed extensions with an Enabled check box (kept in `extensions.toml`), details
(id, version, author, license, folder, permissions and whether each is granted, load
error), Uninstall, Reload All, a Console button, and the Extension Warehouse catalog
URL (empty = the default; `file://` URLs and plain paths work, for testing).
Installing shows the manifest and its permissions; `net` and `file` have check boxes
that are off by default (unticked = the extension runs without them).

## Extension Warehouse (Extensions > Extension Warehouse)

A catalog JSON, by default
`https://raw.githubusercontent.com/noxqs/meshtard/HEAD/extensions/catalog.json`
(the repository's `extensions/catalog.json` on its default branch):

```json
{
  "format": 1,
  "name": "Meshtard Extension Warehouse",
  "extensions": [
    {
      "id": "meshtard.stair_maker", "name": "Stair Maker", "version": "1.0.0",
      "author": "...", "description": "...", "license": "MIT",
      "permissions": ["model.read", "model.write", "ui.menu", "ui.toolbar"],
      "min_meshtard": "0.1.0",
      "url": "dist/stair_maker-1.0.0.mtx",
      "sha256": "e1e0...", "size": 1698, "homepage": "..."
    }
  ]
}
```

`url` is absolute or relative to the catalog's URL. The window loads the catalog on a
background thread, searches name/id/author/description, shows details, and Install...
downloads the `.mtx` (through the Model Library's HTTP code, `library/net.rs`),
**checks its SHA-256 against the catalog** and that the package's id and version match
the entry, then shows the permission prompt. A mismatch is reported and nothing is
installed. The checksum protects against a corrupted or swapped download, not against
a malicious catalog (signatures are a later step).

`extensions/dist/*.mtx` and `extensions/catalog.json` are built from
`extensions/examples/` by `cargo test -p mt-app build_extension_dist -- --ignored`
(deterministic zips); the test `committed_dist_matches_examples` fails when they are
stale.

## Extension Console (Window > Extension Console)

A Meshtard addition, unverified against SketchUp's Ruby Console: the log of every
extension (`print`, `log`, load messages, errors in red) and a one-line Rhai REPL
against the active model with `model.read` and `model.write` (no `net` / `file`).
Variables persist between lines; Up/Down recall earlier lines; each line that changes
the model is one undo step; errors stay in the console.

## Security model (summary)

- Scripts can only call registered functions: no file system, process or network
  access except the `file` (own data folder) and `net` (HTTP GET) functions, which
  need a declared and user-approved permission.
- Model access goes through the facade (no raw kernel pointers); writes are
  validated and rolled back on error.
- Runaway scripts stop after 5M operations or 3 s; recursion, string, array and map
  sizes are capped.
- Packages: paths checked (zip slip), file types filtered, size capped; Warehouse
  downloads checked by SHA-256.
- Not done: code signing of packages and catalogs, a per-extension memory limit
  beyond Rhai's size caps, revoking a permission without reinstalling.

## Left (later)

- A compatibility layer for a subset of SketchUp's Ruby API (FEATURES 69 (4)).
- Signed catalogs and packages; update notifications (the Warehouse shows versions,
  installing a newer one replaces the old).
- macOS native menu bar: extension commands are not added to it (drawn menus only).
- Web dialogs (`UI::HtmlDialog` equivalent), custom tool cursors, drawing in the
  viewport beyond preview polylines, entity attributes (attribute dictionaries).
