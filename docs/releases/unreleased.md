# Unreleased

## Shortcuts

- Default shortcuts checked against SketchUp 2026 for Windows (help page, 2026 Quick Reference Card, a Windows shortcut export; list with sources in `spec/parity/SHORTCUTS.md`). New defaults: K Back Edges, Ctrl+Q Exit, Ctrl+, Preferences.
- Preferences > Shortcuts lists every command by menu path (`Camera/Zoom Extents`), including right-click and extension commands, with the key under Assigned; the filter also matches a key.
- Pressing a key another command has asks Keep Original / Replace instead of taking it silently.
- Import... and Export... buttons on the Shortcuts page, and Show All....
- Help > Keyboard Shortcuts: every shortcut grouped by menu, with a filter, Copy and Save as text for printing.
- Menus and Search show punctuation keys as characters (`Ctrl+,`).

## macOS

- The menu bar follows SketchUp for Mac: a Meshtard application menu with About Meshtard, Settings... (Cmd+,), the system's Services/Hide/Show All, and Quit Meshtard (Cmd+Q); Preferences, Exit and About leave the Window, File and Help menus. The Window menu starts with Minimize and Zoom and lists the panels directly (Show Dialogs replaces the Default Tray submenu).
- Mac default shortcuts: Redo Cmd+Shift+Z, Select None Cmd+Shift+A, Delete with the delete (backspace) key, Entity Info Cmd+I, Materials Cmd+Shift+C. Tool keys are unchanged. Mostly inferred from forum posts and macOS conventions, not yet checked in SketchUp for Mac (`spec/parity/MAC.md`).
- Windows and Linux menus and shortcuts are unchanged.
