# SketchUp 2026 for Mac: menus and shortcuts

What SketchUp for Mac does differently from Windows in the menu bar and default shortcuts, and what meshtard does about it (`spec/menus_mac.toml`, `spec/keymap_mac.toml`, applied by `Spec::mac_layout` on macOS only).

Marks: **[doc]** stated by Trimble (help.sketchup.com, Quick Reference Card, release notes); **[inf]** inferred (forum posts, older books, macOS conventions) and to be checked in SketchUp 2026 for Mac. Pages read on 2026-10-12 (help pages "Last updated: Sep 29, 2026" unless noted). Nothing here has been checked in the app itself.

## Sources

| # | Source | URL |
|---|---|---|
| S1 | Help: Navigating the SketchUp Interface | https://help.sketchup.com/en/sketchup/user-interface |
| S2 | Help: Preferences | https://help.sketchup.com/en/sketchup/preferences |
| S3 | Help: Dialog Boxes and Trays | https://help.sketchup.com/en/sketchup/dialog-boxes-and-trays |
| S4 | Help: Managing Dialog Boxes in MacOS | https://help.sketchup.com/en/sketchup/managing-dialog-boxes-mac-os |
| S5 | Help: Creating and Editing Shortcuts | https://help.sketchup.com/en/creating-and-editing-shortcuts |
| S6 | Help: Default keyboard shortcuts | https://help.sketchup.com/en/default-keyboard-shortcuts |
| S7 | Quick Reference Card 2026, SketchUp, Mac / Windows (PDF) | https://download.sketchup.com/QRC2026/HC-QRC2026-en-SU-mac.pdf, https://download.sketchup.com/QRC2026/HC-QRC2026-en-SU-windows.pdf (listed on https://help.sketchup.com/en/quick-reference-cards) |
| S8 | Help: MacOS - Spacebar Opens Model Info (Aug 21, 2026) | https://help.sketchup.com/en/troubleshooting/space-bar-opens-model-info-dialog-box |
| S9 | Release notes SketchUp Desktop 2026.0 | https://help.sketchup.com/en/release-notes/sketchup-desktop-20260 |
| F1 | Forum: Switched from Windows 10 to macOS (2021) | https://forums.sketchup.com/t/switched-from-windows-10-to-macos/170097 |
| F2 | Forum: Default Keystrokes/Shortcuts List? (2021) | https://forums.sketchup.com/t/default-keystrokes-shortcuts-list/152384 |
| F3 | Forum: Unselect all | https://forums.sketchup.com/t/unselect-all/194758 |
| F4 | Forum: Delete with Backspace Hotkey | https://forums.sketchup.com/t/delete-with-backspace-hotkey/117661 |
| F5 | Forum: SketchUp 2026: Can I change the 'Exit' shortcut key (Ctrl+Q)? | https://forums.sketchup.com/t/sketchup-2026-can-i-change-the-exit-shortcut-key-ctrl-q/341060 |
| F6 | Forum: Model Info and Colors Window placement (2021) | https://forums.sketchup.com/t/model-info-and-colors-window-placement/155098 |
| F7 | Forum: Instant crash on command-shift-Z (Redo) | https://forums.sketchup.com/t/instant-crash-on-command-shift-z-the-keyboard-shortcut-for-redo/213916 |
| B1 | Google SketchUp: The Missing Manual, Mac shortcuts appendix (old, SketchUp 7 era) | https://www.oreilly.com/library/view/google-sketchup-the/9780596804060/apbs08.html |

## Menu bar

| Finding | Mark | Source | meshtard |
|---|---|---|---|
| Menus: SketchUp, File, Edit, View, Camera, Draw, Tools, Window, Extensions, Help. S1 lists "SketchUp (iOS Only)" first, which reads as a typo for macOS (the iPad app has no menu bar) | [doc] | S1 | app menu "Meshtard" first, then the Windows menus in the same order |
| Preferences open from **Settings** in the SketchUp menu on macOS (Window > Preferences on Windows) | [doc] | S2; F1, F6 confirm for older versions | `window.preferences` moved to the app menu as "Settings..." |
| The application menu also has About and Quit (and the system's Services, Hide, Hide Others, Show All) | [inf] | macOS convention; not listed by Trimble | About (`help.about`) and Quit (`file.exit`) moved there; File has no Exit, Help has no About; system items added by the native menu |
| Panels ("dialog boxes") are independent windows listed in the Window menu; no trays on Mac | [doc] | S1, S3, S4; F1 | "Default Tray" submenu flattened into Window; the panels stay in meshtard's tray (separate windows not built) |
| Window menu has Hide Dialogs / Show Dialogs (hide all open dialogs; show those hidden last time) | [doc] | S4 | `window.tray_show` relabelled "Show Dialogs" (a checked toggle, not SketchUp's two-name item) [inf] |
| Mac dialog list: Entity Info, Materials, Components, Styles, Tags, Scenes, Shadows, Fog, Match Photo, Soften Edges, Overlays, Instructor, Outliner | [doc] | S3 | unchanged (Windows list); Overlays and Match Photo panels are not separate items in meshtard |
| Model Info stays in the Window menu (and the status bar button); it never docks | [doc]/[inf] | S1 (status bar), F1, F6 | unchanged |
| Extensions menu exists on Mac | [doc] | S1 | unchanged |
| Preferences > Workspace on macOS: Cascade Main Windows, Save Current Window Size | [doc] | S2 | not done |
| Materials and Fonts are the system Colors / Fonts panels on Mac and cannot be stacked with the others | [inf] | F1 and a 2025 forum thread | not done (meshtard's own Materials panel) |
| Window menu: system Minimize (Cmd+M) and Zoom | [inf] | macOS convention | added by the native menu at the top of Window |
| Manage Trays... | - | meshtard addition (no Mac counterpart) | kept in Window |

## Shortcuts

Single-key tool shortcuts are identical on both systems: the 2026 Mac and Windows Quick Reference Cards list the same keys (Space, B, E, L, R, C, A, M, P, Q, S, F, T, O, H, Z, Shift+S) [doc, S7]. S6 lists one set for both systems, adding Shift+Space (Lasso), Shift+Z (Zoom Extents), G (Make Component), K (Back Edges). Modifier keys inside tools are Option/Command on Mac where Windows has Ctrl/Alt (S7, Mac card: e.g. Move Option = copy, Command = auto-fold); meshtard's tools still read Ctrl/Alt (not changed here).

On Mac there are two kinds of shortcuts: menu key equivalents with Command, built into the app and not editable, and the Preferences > Shortcuts set (column "Key" on Mac, "Assigned" on Windows) [doc S5 for the column names; inf F2 for the two kinds]. Users cannot assign Command chords on Mac [inf, F2].

| Command | Windows (keymap.toml) | Mac (keymap_mac.toml) | Mark | Source |
|---|---|---|---|---|
| Redo | Ctrl+Y | Cmd+Shift+Z | [inf] | F7 and macOS convention (forum also reports Cmd+Y in the web app) |
| Select None | Ctrl+T | Cmd+Shift+A | [inf] | F3 (Cmd+T opens the Fonts panel on Mac, F1) |
| Delete | Delete | Backspace (the Mac "delete" key) | [inf] | F4 |
| Preferences / Settings | none | Cmd+, | [inf] | macOS convention |
| Exit / Quit | none (SketchUp 2026 Windows reportedly has Ctrl+Q) | Cmd+Q | [inf] | F5, macOS convention |
| Entity Info | none | Cmd+I | [inf] | B1 |
| Materials | none | Cmd+Shift+C | [inf] | F1 ("Colors palette") |
| Model Info | none | none | [inf] | B1 gives Shift+Cmd+I, which Invert Selection uses (kept as Cmd+Shift+I); unresolved |
| Invert Selection | Ctrl+Shift+I | Cmd+Shift+I (Ctrl read as Cmd) | [inf] | community posts only |
| Everything else with Ctrl | Ctrl+... | Cmd+... (same key) | [inf] | |

Not covered: forward Delete (fn+delete) also erasing on Mac (one key per command in the keymap); Hide Dialogs shortcut (one forum post says Shift+A shows/hides all palettes, unconfirmed); File > Close (Cmd+W) and other standard Mac menu items SketchUp has (Close, Minimize per document window); localized labels.

## How it is built

- `spec/menus_mac.toml`: `app_menu` (label and items; its commands leave their Windows places), `flatten` (submenus replaced by their items), `relabel` (labels by command id). Separators left at a menu's start or end, or doubled, are dropped.
- `spec/keymap_mac.toml`: bindings that replace the Windows ones (empty text unbinds); "Cmd" is accepted and stored as "Ctrl", which macOS reads as Command.
- `Spec::mac_layout()` builds the Mac variant; loading the spec builds and validates it on every system (each command exactly once, no shortcut bound twice, toolbar and tray commands present), and `mt-spec` tests compare both layouts. The app applies it only on macOS (`main.rs`), where the native menu bar puts the app menu first with the system items.
