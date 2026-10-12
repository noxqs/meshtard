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
| S10 | Help: Introducing Drawing Basics and Concepts (Last updated Sep 29, 2026): "press the COMMAND key on the Mac or ALT key on Windows" to cycle the Line tool's inferences; "Shift+Alt (Windows) or Shift + Command (MacOS)" face-plane lock; Alt / Command frees the Rotate and Protractor center | https://help.sketchup.com/en/sketchup/introducing-drawing-basics-and-concepts |
| F8 | Forum: No "close" option listed in "File" drop down (2021): on Windows "One instance per file >> no close, just exit"; the Mac version opens many files at once and a window closes with its red button or Command-W | https://forums.sketchup.com/t/no-close-option-listed-in-file-drop-down/157352 |
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

Single-key tool shortcuts are identical on both systems: the 2026 Mac and Windows Quick Reference Cards list the same keys (Space, B, E, L, R, C, A, M, P, Q, S, F, T, O, H, Z, Shift+S) [doc, S7]. S6 lists one set for both systems, adding Shift+Space (Lasso), Shift+Z (Zoom Extents), G (Make Component), K (Back Edges). Modifier keys inside tools: next section.

On Mac there are two kinds of shortcuts: menu key equivalents with Command, built into the app and not editable, and the Preferences > Shortcuts set (column "Key" on Mac, "Assigned" on Windows) [doc S5 for the column names; inf F2 for the two kinds]. Users cannot assign Command chords on Mac [inf, F2].

| Command | Windows (keymap.toml) | Mac (keymap_mac.toml) | Mark | Source |
|---|---|---|---|---|
| Redo | Ctrl+Y | Cmd+Shift+Z | [inf] | F7 and macOS convention (forum also reports Cmd+Y in the web app) |
| Select None | Ctrl+T | Cmd+Shift+A | [inf] | F3 (Cmd+T opens the Fonts panel on Mac, F1) |
| Delete | Delete | Backspace (the Mac "delete" key) and Delete (Forward Delete, fn+delete) | [inf] | F4; Forward Delete from macOS convention |
| Undo, Cut, Copy, Paste second keys | Alt+Backspace, Shift+Delete, Ctrl+Insert, Shift+Insert | none (Cmd+Z, X, C, V only) | [inf] | Mac keyboards have no Insert key; Option+delete and Shift+fn+delete edit text on macOS |
| Zoom Extents | Shift+Z, Ctrl+Shift+E | Shift+Z, Cmd+Shift+E (kept; the native menu shows Shift+Z, the app handles Cmd+Shift+E) | [inf] | |
| Preferences / Settings | Ctrl+, (SketchUp 2026 Windows, forum t/342241; SHORTCUTS.md) | Cmd+, | [inf] | macOS convention |
| Exit / Quit | Ctrl+Q (SketchUp 2026 Windows, F5; bound since 2026-10-12, SHORTCUTS.md) | Cmd+Q | [inf] | F5, macOS convention |
| Entity Info | none | Cmd+I | [inf] | B1 |
| Materials | none | Cmd+Shift+C | [inf] | F1 ("Colors palette") |
| Model Info | none | none | [inf] | B1 gives Shift+Cmd+I, which Invert Selection uses (kept as Cmd+Shift+I); unresolved |
| Invert Selection | Ctrl+Shift+I | Cmd+Shift+I (Ctrl read as Cmd) | [inf] | community posts only |
| Everything else with Ctrl | Ctrl+... | Cmd+... (same key) | [inf] | |

| Close (File menu, Mac only) | none (no such item: one model per app instance) | Cmd+W | [inf] | F8; macOS convention. `menus_mac.toml` `[[insert]]` puts File > Close after Open |

Not covered: Hide Dialogs shortcut (one forum post says Shift+A shows/hides all palettes, unconfirmed); Open Recent and the per-window Minimize; localized labels.

## Modifier keys inside tools

The two 2026 Quick Reference Cards (S7, both PDFs dated 2025-09-10) side by side, with S10 for the Line tool and the Rotate / Protractor center. The rule is Ctrl -> Option and Alt -> Command, except in the tools marked "Option", where Windows Alt is Option on the Mac. meshtard: `crates/mt-app/src/tool_mods.rs` (roles Copy = Ctrl / Option, Alternate = Alt / Command, AltOption = Alt / Option; `Platform::Pc` for Windows and Linux, `Platform::Mac`); tests run on every system with the platform passed in. Not checked in SketchUp for Mac.

| Tool | Windows (S7) | Mac (S7) | Role in meshtard |
|---|---|---|---|
| 3 Point Arc segments | Alt + / - | Option + / - | AltOption (Ctrl / Copy + / - kept, Meshtard's earlier key) |
| Eraser soften / hide / unsoften | Ctrl / Shift / Alt | Option / Shift / Option+Shift | Copy / Shift / Alternate (Ctrl+Shift = Option+Shift also unsoftens; Command unsoftens too [inf]) |
| Follow Me face perimeter | Alt | Command | Alternate |
| Flip copy | Ctrl | Option | Copy |
| Lasso add / subtract | Alt / Shift+Alt | Option / Shift+Option | AltOption |
| Line inference cycle | Alt (S7 says "lock in current inference direction"; S10 says cycle) | Shift (S7, lock) / Command (S10, cycle) | Alternate: meshtard follows S10, the newer page (see STATUS.md) |
| Move copy / auto-fold | Ctrl / Alt | Option / Command | Copy / Alternate |
| Offset overlap | Alt | Command | Alternate |
| Orbit without gravity | Alt | Option | AltOption |
| Paint Bucket fill / replace on object / sample | Ctrl / Ctrl+Shift / Alt | Option / Option+Shift / Command | Copy / Copy+Shift / Alternate |
| Protractor guide toggle | Ctrl | Option | Copy |
| Push/Pull copy of the face | Ctrl | Option | Copy |
| Rectangle from the center | Alt | Option | AltOption |
| Rotated Rectangle plane lock | Alt | Command | Alternate |
| Rotate copy | Ctrl | Option | Copy |
| Scale about the center | Ctrl | Option | Copy |
| Select add / subtract | Ctrl / Ctrl+Shift | Option / Option+Shift | Copy |
| Tape Measure guide toggle | Ctrl | Option | Copy |
| Shift+Alt face-plane lock; Rotate / Protractor free center | Shift+Alt; Alt (S10) | Shift+Command; Command (S10) | Alternate |
| Tag tool all instances / sample, Walk run, Move grip cycling | Ctrl / Alt, Ctrl, Alt | not listed | Copy / Alternate, Copy, Alternate [inf] |

The Mac's Control key does nothing in tools in meshtard (Control-click is the right click on macOS) [inf]. Status-bar hints, written with Ctrl and Alt, are shown with the Mac names; the Instructor texts are not yet.

## How it is built

- `spec/menus_mac.toml`: `app_menu` (label and items; its commands leave their Windows places), `flatten` (submenus replaced by their items), `relabel` (labels by command id). Separators left at a menu's start or end, or doubled, are dropped.
- `spec/keymap_mac.toml`: bindings that replace all of a command's Windows keys (a key or a list of keys; empty text unbinds); "Cmd" is accepted and stored as "Ctrl", which macOS reads as Command.
- `Spec::mac_layout()` builds the Mac variant; loading the spec builds and validates it on every system (each command exactly once, no shortcut bound twice, toolbar and tray commands present), and `mt-spec` tests compare both layouts. The app applies it only on macOS (`main.rs`), where the native menu bar puts the app menu first with the system items.
