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
