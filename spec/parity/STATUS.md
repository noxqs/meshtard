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
| File | Open `.skp` (2013-2026) | unverified | edges only; faces come with the renderer work |
| File | Opens at the saved view | unverified | `Model::camera` |
| View | Edge style (color, width) | unverified | black, 1 px, soft and hidden edges skipped |
