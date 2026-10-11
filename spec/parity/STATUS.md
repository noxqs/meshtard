# Parity status

`missing` = not implemented, `differs` = implemented but not matching SketchUp, `matches` = verified against SketchUp 2026, `unverified` = implemented from memory, not yet compared.

| Area | Item | Status | Notes |
|---|---|---|---|
| UI | Menu bar structure | unverified | `menus.toml` draft |
| UI | Default shortcuts | unverified | `keymap.toml` draft; only camera keys active |
| UI | Toolbars, tray, status bar, Measurements box | unverified | `mt-ui` crate; screenshot `meshtard/m3_shell.png` |
| UI | Default toolbars (Getting Started top, Large Tool Set left) | unverified | `toolbars.toml` draft; which toolbars are visible on first start is unsure |
| UI | Toolbar flyouts (Arcs, Shapes on Getting Started) | differs | opened by clicking the small corner triangle; SketchUp behavior not checked |
| UI | Toolbar tooltips | unverified | shows "Name (Shortcut)"; SketchUp's exact tooltip text not checked |
| UI | Default Tray panels, order, expanded state, width | unverified | `tray.toml` draft; panel contents other than Entity Info not implemented |
| UI | Entity Info panel | unverified | shows caller-provided title and rows |
| UI | Measurements box typing from the viewport | unverified | digit or `.,;-+[<'"/*@=` starts typing, then all text until Enter/Escape |
| UI | Window theme (colors, sizes) | unverified | `ui_theme.toml` draft (Windows 11 light) |
| UI | Toolbar icons | differs | own procedural drawings by design (Trimble icons are copyrighted) |
| View | Sky and ground | unverified | colors are placeholders |
| View | Axes (solid positive, dotted negative) | unverified | |
| Camera | Default eye, target, FOV | unverified | |
| Camera | Orbit (middle drag, O) | unverified | scenario 00 |
| Camera | Pan (shift+middle, H) | unverified | scenario 00 |
| Camera | Zoom (wheel, Z) | unverified | scenario 00 |
| Camera | Zoom Extents, Standard Views, Parallel Projection | missing | |
| File | Open `.skp` (2013-2026) | unverified | faces and edges; no textures yet |
| File | Opens at the saved view | unverified | `Model::camera` |
| View | Edge style (color, width) | unverified | black, 1 px, soft and hidden edges skipped |
| View | Face shading | unverified | `[shading]` draft: flat diffuse, camera light, two-sided |
| Geometry | Merge tolerance 0.001" | unverified | `mt_kernel::TOL` |
| Geometry | Lines split each other where they cross | unverified | |
| Geometry | Closed planar loop creates a face | unverified | |
| Geometry | Face on the ground plane faces down | unverified | other new faces: match a neighbor, else largest normal component positive |
| Geometry | Line across a face splits it | unverified | |
| Geometry | Loop inside a face makes a hole and an inner face | unverified | |
| Geometry | Erasing a shared edge merges coplanar faces | unverified | |
| Geometry | Erasing a splitting edge heals the split line | unverified | |
| Select | Click rules (Ctrl add, Shift toggle, Ctrl+Shift remove) | unverified | |
| Select | Double click face/edge, triple click connected | unverified | |
| Select | Window (left to right) vs crossing (right to left) | unverified | solid vs dashed box |
| Select | Edge pick aperture 5 px | unverified | |
| View | Profiles (width 2, borders and silhouettes) | unverified | |
| View | Selection highlight (blue edges, dotted faces, component box) | unverified | box is world-aligned, SketchUp's follows component axes |
| Inference | Point kinds, priorities, colors, tooltip names | unverified | endpoint green, midpoint cyan, on edge red, on face blue |
| Inference | Pick distances (10 px points, 6 px edges, 10 px directions) | unverified | |
| Inference | Arrow keys and Shift locking | unverified | |
| Line tool | Chain continues until a face closes; Esc cancels | unverified | |
| Measurements | Length entry formats | unverified | |
| Draw | Default segments: circle 24, arc 12, polygon 6 sides | unverified | |
| Draw | Shape plane: clicked face, else ground; arrow keys lock | unverified | |
| Draw | Typed values per tool (`w,h`, radius, `24s`, bulge, angle) | unverified | |
| Eraser | Whole curve erased from one segment; Shift hide, Ctrl soften | unverified | |
| Select | One click on a curve segment selects the curve | unverified | |
| Edit | Undo/Redo | unverified | snapshot per edit, limit 100 (SketchUp's limit not checked) |
| Tape Measure | Parallel guide from edge, guide point from point | unverified | guide lines drawn dashed gray, infinite |
| Push/Pull | Extrude, Ctrl new faces, cut through | unverified | |
| Move | Stretch connected geometry, Ctrl copy, `5x` and `/5` arrays | unverified | preview is an outline; SketchUp moves the geometry live |
| Rotate | Center, start, end; typed angle; Ctrl copy | unverified | protractor plane from the three points, not SketchUp's protractor orientation |
| Scale | Three points or typed factor | differs | SketchUp uses bounding-box grips |
| Offset | Inward/outward, arcs stay arcs, partial collapses trimmed (vanishing arms, split loops, holes merging) | unverified | corners always mitered (no limit); where trimming cuts an arc, its end piece is a chord piece |
| Follow Me | Select path, click profile; soft joints under 30° | unverified | |
| Intersect Faces | With Model, With Selection | unverified | With Context missing |
| Groups | Make Group / Component, origin at selection's bounding-box corner | unverified | component axes dialog missing |
| Groups | Edit in place: fade rest, Esc / click outside closes | unverified | fade amount a guess |
| Groups | Explode merges with surroundings | differs | welds vertices and overlapping edges only; no splitting where edges cross |
| Tags | Panel, Untagged first, visibility, color, Entity Info Tag | unverified | no folders, no dashes |
| Materials | In Model grid, Create Material, edit color/opacity | differs | egui color picker; no textures; new material color a guess |
| Paint Bucket | Paints clicked side, or selection when clicking a selected entity | unverified | no Alt sample, Ctrl/Shift fills |
| Outliner | Tree, selection, filter | unverified | |
| Scenes | Add/Update/Delete/Activate, tabs | differs | no transition animation, no per-scene property toggles |
| Shadows | Sun position (NOAA), shadow map, light/dark | unverified | default location and time from memory |
| Styles | Edges and profiles toggles | unverified | face styles not rendered yet |
| Section Plane | Place on face, active cut, cut lines, arrows | differs | no name/symbol dialog; arrows drawn on the kept side; section fill missing |
| Face styles | Wireframe, Hidden Line, Shaded, Monochrome, X-ray 50% | unverified | textures missing |
| Components | In Model list, place, edit, Purge Unused | differs | list with icons instead of thumbnails; no gluing or cursor rotation when placing |
| File | Save / Save As writes .skp (SketchUp 2017 format) | differs | SketchUp 2026 saves its own (VFF) format; view, scenes, styles not saved |
| File | Unsaved-changes asterisk in title | unverified | |
| Platform | Windows build | unverified | CI-built, not yet run by hand |
| Platform | macOS menu bar (native), Cmd shortcuts | unverified | Windows menu layout used; SketchUp for Mac's differs |
