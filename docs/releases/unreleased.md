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

## macOS

- The menu bar follows SketchUp for Mac: a Meshtard application menu with About Meshtard, Settings... (Cmd+,), the system's Services/Hide/Show All, and Quit Meshtard (Cmd+Q); Preferences, Exit and About leave the Window, File and Help menus. The Window menu starts with Minimize and Zoom and lists the panels directly (Show Dialogs replaces the Default Tray submenu).
- Mac default shortcuts: Redo Cmd+Shift+Z, Select None Cmd+Shift+A, Delete with the delete (backspace) key, Entity Info Cmd+I, Materials Cmd+Shift+C. Tool keys are unchanged. Mostly inferred from forum posts and macOS conventions, not yet checked in SketchUp for Mac (`spec/parity/MAC.md`).
- Windows and Linux menus and shortcuts are unchanged.

Changes since 0.1.0, grouped by area.

## IFC

- IFC import reads more solids: revolved solids, profiles swept along curves (surface curve and fixed reference sweeps), hollow and closed pipes, CSG primitives (blocks, cylinders, cones, spheres, pyramids) with unions and intersections done by the Solid Tools, and advanced breps with flat and curved (B-spline, cylinder, cone, sphere, torus) faces.
- IFC import brings in textures (embedded, pixel and image-file textures on triangulated geometry) and the site's location: latitude and longitude, north and map coordinates become the model's geo-location when it has none (Model Info > Geo-location shows the map origin). Sites far from the origin are moved near it, the offset kept in the geo-location.
- Furniture and other objects placed in rooms no longer come in hidden with the room.
- IFC export writes real element classes (walls, slabs, doors, windows, columns and more) from the Type an IFC import set, keeps imported names and GlobalIds, writes one storey per storey group, a `Reference` property and net volumes, and the site's location and map conversion.
