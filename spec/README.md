# spec

Everything meshtard copies from SketchUp 2026 (Windows) lives here as data. The app reads these files at startup, so matching SketchUp means editing a file, not code.

| File | Content |
|---|---|
| `menus.toml` | Menu bar: every menu, item, submenu and separator, in order |
| `keymap.toml` | Default keyboard shortcuts, by command id |
| `default_model.toml` | What a new model looks like: camera, sky, ground, axes |
| `toolbars.toml` | Toolbars of the default workspace: dock side, row, columns, buttons (command id + tooltip), separators, flyouts |
| `tray.toml` | Default Tray: name, width, panels in order (id = Window menu command) and which start expanded |
| `ui_theme.toml` | Window chrome colors (Windows light theme), icon palette, sizes (font, toolbar button, status bar, Measurements box) |
| `parity/` | Scripted scenarios to run in SketchUp and in meshtard, and the status of each |

## Verification

Every file has `verified = false` until it has been checked line by line against SketchUp 2026 on Windows during the trial. The current content is a draft written from memory of SketchUp and **will contain mistakes**. `parity/TRIAL_CHECKLIST.md` lists what to capture.

Toolbar icons are meshtard's own drawings (`crates/mt-ui/src/icons.rs`), not SketchUp's; only their size and position match.

Command ids (`tool.line`, `file.save`, ...) are meshtard's own and stay stable even if labels change.
