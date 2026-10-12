# Unreleased

Changes since 0.1.0.

## Saving to .skp keeps more

- **The current view is saved.** A file opens where you left it (camera position, perspective or parallel projection, field of view), not in the Iso view. Two-point perspective is saved as plain perspective.
- **Arcs and circles stay arcs.** They reopen as arcs and circles you can edit by radius and segment count, not as plain curves. Convert to Polygon is not kept yet (the curve reopens as an arc).
- **Edge tags are saved.** Edges keep their own tag.
- **Guides and section planes inside groups and components are saved.**
- Writer library (`mt_skp::write`): `add_circle` / `add_arc` off the ground plane wrote a wrong second axis (inherited from OpenSKP's writer) that readers take for an ellipse; fixed.

Still not saved: scenes, shadow settings, fog, model axes, locks, the active section plane, two-point perspective, text and dimension tags, tag folders and dashes, environments, fonts and dimension expert settings. Where the SketchUp 2017 format keeps most of these is not known yet; files saved by SketchUp during the trial will show it (`spec/parity/TRIAL_CHECKLIST.md` 1b). Tag folders and environments came after SketchUp 2017 and need the newer file format.

## Shortcuts

- Default shortcuts checked against SketchUp 2026 for Windows (help page, 2026 Quick Reference Card, a Windows shortcut export; list with sources in `spec/parity/SHORTCUTS.md`). New defaults: K Back Edges, Ctrl+Q Exit, Ctrl+, Preferences.
- Preferences > Shortcuts lists every command by menu path (`Camera/Zoom Extents`), including right-click and extension commands, with the key under Assigned; the filter also matches a key.
- Pressing a key another command has asks Keep Original / Replace instead of taking it silently.
- Import... and Export... buttons on the Shortcuts page, and Show All....
- Help > Keyboard Shortcuts: every shortcut grouped by menu, with a filter, Copy and Save as text for printing.
- Menus and Search show punctuation keys as characters (`Ctrl+,`).
- A command can have several shortcuts. New second keys as in SketchUp for Windows: Ctrl+Insert Copy, Shift+Delete Cut, Shift+Insert Paste, Alt+Backspace Undo, Ctrl+Shift+E Zoom Extents; on the Mac, Forward Delete also deletes. Preferences > Shortcuts shows every key with Remove per key and Add...; menus show the first key. Saved shortcut files from earlier versions still load.
- Shift+F1 (Help > Context Help) opens the Instructor panel for the active tool.
- Tool modifier keys follow SketchUp 2026's Quick Reference Card: Orbit suspends gravity with Alt (was Ctrl); Eraser unsoftens with Alt; Alt + plus / minus change segments; Lasso adds with Alt, adds and subtracts with Shift, subtracts with Shift+Alt. New: Rectangle Alt draws from the center, Follow Me Alt uses a face's perimeter as the path (Alt-click the profile, then click the face), Protractor Ctrl turns guide creation off and on.

## macOS

- The menu bar follows SketchUp for Mac: a Meshtard application menu with About Meshtard, Settings... (Cmd+,), the system's Services/Hide/Show All, and Quit Meshtard (Cmd+Q); Preferences, Exit and About leave the Window, File and Help menus. The Window menu starts with Minimize and Zoom and lists the panels directly (Show Dialogs replaces the Default Tray submenu).
- Mac default shortcuts: Redo Cmd+Shift+Z, Select None Cmd+Shift+A, Delete with the delete (backspace) key, Entity Info Cmd+I, Materials Cmd+Shift+C. Tool keys are unchanged. Mostly inferred from forum posts and macOS conventions, not yet checked in SketchUp for Mac (`spec/parity/MAC.md`).
- Windows and Linux menus and shortcuts are unchanged.
