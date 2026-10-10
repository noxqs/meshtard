# Parity status

`missing` = not implemented, `differs` = implemented but not matching SketchUp, `matches` = verified against SketchUp 2026, `unverified` = implemented from memory, not yet compared.

| Area | Item | Status | Notes |
|---|---|---|---|
| UI | Menu bar structure | unverified | `menus.toml` draft |
| UI | Default shortcuts | unverified | `keymap.toml` draft; only camera keys active |
| UI | Toolbars, tray, status bar, Measurements box | missing | M3 |
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
