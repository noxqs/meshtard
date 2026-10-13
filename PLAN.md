# meshtard: a SketchUp clone in Rust

## Goal

A modeler that behaves exactly like SketchUp: same GUI layout, same menus, same hotkeys, same tools, and reads/writes real `.skp` files.

- **Platform:** Linux first (X11 and Wayland). Windows/macOS later.
- **GPU:** Vulkan via `wgpu` for drawing. CUDA for heavy compute, with a CPU fallback for machines without an NVIDIA GPU.
- **Plugins (Ruby API):** kept in view, not in scope yet. The data model is designed so a Ruby bridge can be added later.

## Approach

Rebuild the application from scratch by matching observable behavior (clean room). No disassembly of SketchUp binaries; its EULA forbids it.

- **File format:** port OpenSKP (MIT) to Rust. It already reverse-engineered both containers, so we reuse their work instead of redoing it.
- **GUI, menus, hotkeys:** replicate from the running application. SketchUp has no Linux build, so the Windows version is the reference.
- **Icons and name:** Trimble's icons and the "SketchUp" name are protected. We draw our own icons in the same style, same size and position. Menu structure, layout and shortcuts can be matched exactly.

## Reference sources

| Source | License | Use |
|---|---|---|
| [OpenSKP](https://github.com/iamahsanmehmood/openskp) | MIT | Primary. Port its C++ implementation (`core.cpp`, `legacy.cpp`, `create.cpp`, `edit.cpp`, about 5,200 lines) and `docs/BINARY_FORMAT.md` to Rust. Keep its copyright notice. Use its Python package as a test oracle. |
| skppy (Blender Artists forum) | Not checked | Writes 2026-format (VFF) files, which OpenSKP cannot. Check its license before reading its code. Otherwise use it only as a black box to produce test files. |
| Real SketchUp 2026 (Windows) | n/a | Ground truth for GUI, menus, hotkeys, tool behavior. Screenshots and recorded behavior go in `spec/`. |

## Architecture

### Data model mirrors `.skp`

The in-memory model matches SketchUp's own entity model 1:1 so files round-trip without loss:
Vertex, Edge, EdgeUse, Loop, Face, Curve/ArcCurve, ComponentDefinition, ComponentInstance, Group, Layer (Tag), Material, Style, Page (Scene), AttributeDictionary, guides, section planes, dimensions, text.

SketchUp's EdgeUse is a radial-edge structure: one edge can border any number of faces. So the kernel is a non-manifold boundary representation, not a plain triangle mesh.

### Workspace layout

```
crates/
  mt-kernel/   entities, topology, invariant checker (no graphics deps)
  mt-ops/      split/merge edges, auto-face, push/pull, follow me, offset, intersect
  mt-infer/    inference (snapping) engine
  mt-model/    model, components, tags, materials, styles, scenes, undo stack
  mt-skp/      .skp read/write: port of OpenSKP (VFF 2021+ and legacy 2013-2020)
  mt-cuda/     CUDA kernels behind a cargo feature, CPU fallback for each
  mt-render/   wgpu (Vulkan) renderer: faces, edges, profiles, styles, shadows
  mt-tools/    interactive tools as state machines
  mt-ui/       menus, toolbars, trays/panels, status bar, Measurements box
  mt-app/      winit shell, keymap, preferences
spec/
  menus.toml   every menu and item, in order, with its default shortcut
  keymap.toml  default hotkeys
  tools/       per-tool behavior notes and screenshots from real SketchUp
  ui/          layout screenshots and measurements
```

Menus, toolbars and keymap are data files read at startup, so matching SketchUp is a matter of editing `spec/`, not code.

### Key libraries

| Need | Crate |
|---|---|
| Window, input (X11/Wayland) | `winit` |
| Rendering | `wgpu` (Vulkan backend on Linux) |
| GUI | `egui` with a custom theme to match SketchUp's look |
| Math | `glam` (f64) |
| Entity storage | `slotmap` |
| CUDA | `cudarc` (driver API + NVRTC), feature flag `cuda` |
| Face triangulation | `spade` (constrained Delaunay, handles holes) |
| ZIP (VFF container) | `zip` |
| XML (materials, styles) | `quick-xml` |
| Tests | `proptest`, `insta` (snapshot tests) |

### Where CUDA is used

Only where the work is large and parallel. Each kernel has a CPU version with the same result, tested against each other.

| Task | Why it is heavy |
|---|---|
| Triangulating all faces on load | Large files have hundreds of thousands of faces |
| BVH build for picking and inference | Rebuilt after every edit on big models |
| Intersect Faces / solid tools (union, subtract) | Pairwise face-face tests |
| Smooth normals, soften edges | Per-vertex work over the whole model |
| Shadow and ambient occlusion precompute | Per-pixel/per-sample work |

The renderer itself stays on Vulkan. CUDA results are shared with Vulkan through external memory interop (`VK_KHR_external_memory_fd`) to avoid copies, if `wgpu` exposes it well enough; otherwise copy through host memory first and optimize later.

## Milestones

### Comparison with real SketchUp (runs alongside every milestone)

SketchUp runs only on Windows and macOS, so the trial runs on one of those machines, not on the Linux dev box.

1. **Before starting the trial**, write `spec/parity/*.md`: one scripted scenario per feature (e.g. "draw 1000 mm rectangle, push/pull 500 mm, erase top edge, expected result"). The trial clock is short, so the scripts are ready before it starts.
2. **During the trial**, run every script in SketchUp and record:
   - screenshots and short screen recordings into `spec/tools/` and `spec/ui/`
   - the `.skp` file saved after each scenario into `tests/corpus/` (this is also the test corpus for the file format)
   - menu tree, shortcut export, preferences into `spec/`
3. **After**, run the same scripts in meshtard. `spec/parity/STATUS.md` tracks each feature as missing / differs / matches.
4. Automated part: the saved `.skp` files are loaded by meshtard, and the scripted scenarios are replayed through our tool API; the resulting geometry is compared with what SketchUp saved.

### M0: Skeleton and spec capture
- [x] Cargo workspace, CI (fmt, clippy, test) on Linux. CUDA CI job added together with `mt-cuda`.
- [x] Draft `spec/` from memory: menus, keymap, default model, trial checklist, scenario 00.
- [x] Window with menu bar from `spec/`, status bar, Vulkan viewport: sky, ground, axes (dotted negative halves).
- [x] Navigation: middle drag orbit, shift + middle drag pan, wheel zoom at cursor, Orbit/Pan/Zoom tools (O/H/Z), Space for Select.
- [ ] Capture from real SketchUp 2026 during the trial (`spec/parity/TRIAL_CHECKLIST.md`) and correct the drafts.

### M1: `.skp` reading (port of OpenSKP)
- [x] `mt-skp`: VFF container (2021+) and legacy MFC CArchive (SketchUp 3 to 2020), ported from OpenSKP C++ at commit `359d183`. Geometry, components, tags, materials with textures, styles with watermarks, scenes, saved view, dimensions, guides, attribute dictionaries.
- [x] Test: all 25 files in `tests/corpus/openskp` match OpenSKP Python's output (same commit), field by field. Known gaps both ways are listed in `crates/mt-skp/tests/oracle.rs`.
- [x] App: File > Open and `meshtard FILE.skp` load a model and draw its edges, at the view it was saved with.
- [x] Edge curve ids, legacy face layers, legacy model-level dimensions, text positions, legacy styles now read.
- [ ] Not yet read: edge layers; VFF texts (none in the corpus); tag folders and dashes (no sample).
- [ ] Loading into `mt-kernel` entities moves to M2, where the kernel is built.

Reading first means real SketchUp models are available for testing everything else.

### M2: Kernel
- [x] `mt-kernel`: vertices, edges, faces with holes; an edge can border any number of faces.
- [x] Add line: endpoint snapping (0.001"), crossing edges split, collinear edges reused, auto-face on closed coplanar loops, faces split by lines across them, holes from nested loops.
- [x] Erase edge: bounded faces deleted, coplanar neighbors merged, split lines healed.
- [x] Validity checker after every operation: 16 scenario tests plus random sequences (3,000+ cases per run).
- [x] `.skp` import into kernel entities (all 25 sample files pass topology checks), triangulation (exact area on every well-formed face).
- [x] Renderer: shaded two-sided faces with materials, edges drawn on top, translucency. The app draws `.skp` files with faces.
- [x] Spatial index (multi-level grid) for kernel lookups: Push/Pull 51 -> 3 ms, Move 77 -> 4 ms on a 51k-edge model.

### M3: Renderer and GUI shell
- [x] Faces, edges, profiles (borders and silhouettes), front/back face colors.
- [x] `mt-ui`: menu bar, toolbars (Getting Started, Large Tool Set) with original icons, Default Tray with Entity Info, status bar, Measurements box (typing captured from the viewport), all built from `spec/`.
- [x] Selection: click, Ctrl/Shift/Ctrl+Shift, window vs crossing, double/triple click, highlight; Select All, Select None, Delete.
- [x] CPU BVH for picking.
- [ ] CUDA triangulation and BVH build: to be done on the user's Linux machine with an NVIDIA GPU (Remote Control session).
- [x] Dockable toolbars: drag by the grip to any edge or float; View > Toolbars... and right-click show or hide them; the layout is saved between runs.
- [x] Environments and Instructor panels (M18).

### M4: Inference engine
- [x] `mt-infer`: endpoint, midpoint, origin, on edge, on axis, on face.
- [x] Directions from the start point: red/green/blue axes, parallel and perpendicular to the reference edge, "from point" tracking; arrow-key locks (right red, left green, up blue, down parallel) and Shift lock.
- [x] Typed lengths in the Measurements box (mm, cm, m, ", ', feet and inches; model units by default).
- [x] Line tool (brought forward from M5 to exercise inference): chained segments, ends when a face closes, Esc cancels.
- [x] Center and intersection inferences, 1 s hover delay before acquiring points, square / golden section / half circle / tangent hints.
- [x] Inference on component geometry while editing inside it (FEATURES 49).

### M5: Drawing tools
- [x] Line, Freehand, Rectangle, Rotated Rectangle, Circle, Polygon, Arc (center), 2 Point Arc, 3 Point Arc, Pie, Eraser.
- [x] Curves in the kernel: arcs, circles and freehand curves select, erase and report (Entity Info) as one entity.
- [x] Shapes lie on the clicked face's plane or the ground; arrow keys lock the plane. Typed values: rectangle `w,h`, radius, `24s` sides/segments, arc bulge, arc angle.
- [x] Eraser: click or drag; Shift hides, Ctrl softens/smooths, Ctrl+Shift unsoftens.
- [x] Shapes on tilted faces: sides along the level line and up the slope (unverified against SketchUp on diagonal slopes); arrow keys re-orient after the first point.

### M6: Modeling tools
- [x] Undo/Redo (snapshot per edit, 100 steps).
- [x] Push/Pull (Ctrl keeps the start face; pushing through cuts a hole), Move and Ctrl copy with `5x` / `/5` arrays, Rotate (+copy, typed angle), Scale (simplified, three points or typed `s` / `x,y,z`), Offset (typed distance, arcs stay arcs), Follow Me (select path, click profile), Intersect Faces with Model / with Selection.
- [x] Tape Measure: parallel guide from an edge, guide point otherwise; guides imported from `.skp`, used by inference; Edit > Delete Guides.
- [x] Protractor (angle or slope guides), Dimensions (linear, radius, diameter), Text (leader and screen text), saved to .skp.
- [x] Push/Pull double-click repeats the last distance; Ctrl toggles copy / new faces.
- [x] Scale grip box (corner, edge, face grips; typed factors and sizes); Move/Rotate/Scale show the geometry live; Offset trims partial collapses.
- [x] Make Component dialog (name, description, Glue to, Set Component Axes); gluing components lie on matching faces.
- [ ] CUDA: intersection and solid tools (mt-cuda in progress on the Linux machine).

### M7: Scene structure
- [x] Groups and components: Create Group, Make Component (G), Explode, Make Unique; double-click edits in place (rest of the model faded, not selectable; Esc, click outside or Edit > Close Group/Component closes). Undo covers the whole model and edit context.
- [x] Tray panels: Tags (add, delete, rename, visibility, color; Entity Info Tag dropdown), Materials (In Model, Create Material, edit color and opacity; Paint Bucket), Outliner, Scenes (add, update, delete, activate; scene tabs), Styles (edges, profiles), Shadows (time, date, light, dark, on faces, on ground; View > Shadows).
- [x] Renderer: sun shadows (shadow map), section cut clipping and cut lines (renderer API), transparent materials sorted back to front.
- [x] Section Plane tool: planes per context, drawn, selectable, movable, deletable; double-click toggles the active cut; View > Section Planes / Section Cuts; cut lines.
- [x] Face styles (Wireframe, Hidden Line, Shaded, Shaded With Textures, Monochrome) and X-ray.
- [x] Components panel: In Model list with instance counts, place instances, edit name and description, Purge Unused.
- [x] Large models build their pick index on the GPU (`--features cuda`, from 20k triangles; threshold being measured).
- [x] Textures: imported with their UV mapping (checked against OpenSKP), drawn in Shaded With Textures, saved back.
- [x] Section fill, animated scene transitions, Eraser drag preview, selection boxes in component axes.
- [ ] Left for later: tag folders and dashes (no sample file), Components Statistics tab.

### M8: `.skp` writing
- [x] Writer: Rust port of OpenSKP's legacy v17 (SketchUp 2017) writer, byte-identical to OpenSKP on 17 models; extended with plain edges (per-edge soft/smooth/hidden), root group instances (shared group definitions), a plane tolerance for real models and empty-root files.
- [x] File > New, Save (Ctrl+S), Save As, Save A Copy As; unsaved changes marked in the title.
- [x] Every corpus model saves and reopens with identical counts in both our reader and OpenSKP's.
- [x] Saved since: the camera view, edge tags, guides and section planes inside components, arcs and circles as radius-editable arcs (M14); styles and units (M14).
- [ ] Not saved yet: scenes, shadow settings, active section (formats need SketchUp-saved samples, M14).
- [x] "Save changes?" prompt on New, Open, dropped files and closing.
- [ ] Check in real SketchUp 2026 that saved files open correctly (during the trial).
- [ ] VFF (2021+) writing: no reference implementation. Container survey in `spec/VFF_FORMAT.md` (header, ZIP layout, meta.dat); writer not started (parked, low priority; LEFTOVERS 21).

### M9: Windows and macOS
- [x] Builds and tests on Linux, Windows and macOS in CI (GitHub Actions); release binaries for Windows (with the CUDA option) and macOS are attached to every run. Rust pinned (`rust-toolchain.toml`).
- [x] Linux release build tested on the RTX 4070 SUPER machine (Vulkan, CUDA on).
- [x] CUDA falls back to the CPU when the NVIDIA driver or NVRTC is missing (Windows needs the CUDA Toolkit's NVRTC for the GPU path).
- [x] macOS: native menu bar from `menus.toml` (Cmd shortcuts, enabled and checked states), Cmd for Ctrl shortcuts, Mac symbols in menus.
- [x] A `.skp` dropped on the window opens; a `.skp` path on the command line opens.
- [x] SketchUp for Mac's menu layout and default shortcuts: `spec/menus_mac.toml`, `spec/keymap_mac.toml` (sources `spec/parity/MAC.md`; mostly inferred, unverified), applied on macOS only, both layouts tested on every system.
- [ ] Not yet: tried by hand on Windows and macOS hardware; installers and app bundles (on hold); HiDPI check on Windows/macOS.

### Later
- Ruby plugin API (kept in view).

### M10: Everyday commands (FEATURES.md Tier 1)
- [x] Items 1-12 of `spec/parity/FEATURES.md`: camera commands, Hide/Lock, Cut/Copy/Paste, Invert Selection, View and Edge Style toggles, Window menu panels, Zoom Window (plus Fog, item 26).
- Model-only for now (not in the .skp): lock flag, Axes/Guides/Hidden Geometry, edge style options, fog.

### M11: File exchange (FEATURES.md items 14-17)
- [x] Export 2D, Export 3D, Import, View > Animation commands.
- Not done: glTF export, OBJ texture import, animation settings saved in the file.

### M12: Solid Tools and modeling extras (FEATURES.md items 13, 18-28)
- [x] Solid Tools (kernel booleans, about 0.7% of near-degenerate random cases refused), Tag and Axes tools, camera tools, Two-Point Perspective, Model Info, Preferences, Revert, Component Edit hiding, Intersect With Context, Soften Edges.
- Not saved in the .skp yet: model axes, two-point (units are saved since M14).

### M13: Presentation and the rest (FEATURES.md Tier 3)
- [x] Sandbox From Scratch and From Contours (Delaunay TIN), 3D Text (ttf-parser outlines, bundled Ubuntu Light and Hack), image entities from File > Import (PNG/JPEG) with Explode to a textured face (FEATURES.md items 29, 31, 30 part 1).
- Not done: Smoove/Stamp/Drape, images saved as image entities (they are saved as groups).
- [x] Items 32-38: Export Section Slice (R12 DXF), Export Animation (numbered PNG/JPEG frames), Print Setup / Print Preview / Print (one-page PDF written directly, opened in the system viewer), templates (New from Template, Save As Template, default template for File > New), Generate Report (CSV/HTML), Manage Trays (named trays, one docked at a time), About; Help links open the project page.
- Not done: DWG, MP4 (no small pure-Rust encoder), OS print dialog, tiled printing, several trays shown at once, report templates over attributes.

### M14: Writer completeness (FEATURES.md item 39)
- [x] Section planes (reopen in their plane around the model), style settings with known style items, units and snapping (`UnitsOptions`), image entities (`CImage`) survive save and reopen; round-trip tests in `crates/mt-kernel/tests/export.rs`.
- [x] Formats read from the corpus (reader code plus SketchUp-saved files) and written, round-tripped through our reader and OpenSKP's: the saved view (`CCamera`), arcs and circles (`CArcCurve` through the model's own vertices; the y-axis field fixed, OpenSKP's writer wrote `(0, r, 0)`), edge tags (drawbase layer, as in `gondola_v20.skp`), guides and section planes inside components.
- [ ] Scenes, Monochrome, fog, Component Edit hiding, model axes, locks, two-point, active section, shadow settings, polygon marks: format unknown without SketchUp-saved samples (the corpus has no v17 scene, only one SketchUp 2020 scene with every property captured); captures listed in `spec/parity/TRIAL_CHECKLIST.md` section 1b. Tag folders (2021) and environments (2025) do not exist in the 2017 format.

### M15: Toolbars and cursors (FEATURES.md items 40, 41)
- [x] Item 40: the remaining View > Toolbars toolbars (Standard, Views, Camera, Styles, Edit, Drawing, Principal, Construction, Section, Solid Tools, Tags, Shadows, Sandbox, Measurements), hidden by default, dockable and floating like the others, with own icons for every button; widget items (tag dropdown, shadow sliders, Measurements box) in `toolbars.toml`.
- [x] Item 41: mouse cursors per tool and modifier (drawn by meshtard over the viewport); left: Paint Bucket fill-mode badges, Follow Me path, Scale grip resize, busy cursor.
- [x] Dialogs: one Windows-11-style dialog (draggable title bar, close button, border, shadow, dimmed backdrop) for every dialog in mt-app; Toolbars and Manage Trays windows still to convert.
- [x] Splash at the image's size; "Meshtard" in UI text.

### M16: everyday gaps from the help comparison (FEATURES.md 44-54)
- [x] Items 44, 45, 46, 47, 51: viewport right-click menu with the Select submenu, Reverse / Orient Faces, Flip Along, Reset Scale / Skew, Scale Definition, Divide, Weld Edges, Explode Curve, Zoom Selection (`context_menu.rs`, kernel `orient`, `instance_ops`, `curves`).
### M17: regular gaps (FEATURES.md 55-64)
### M18: occasional gaps (FEATURES.md 65-67)

### M19: Libraries and extensions (FEATURES.md 68-69)
- [x] Model library browser with pluggable sources (local folders, Poly Haven, Sketchfab token), download cache with a size limit and recent downloads; Rhai extension system, Extension Manager, Meshtard Extension Warehouse, Extension Console. Also Match Photo (item 30). Left: Sketchfab OAuth, Ruby API compatibility, signed packages.

### M20: Leftovers round (LEFTOVERS.md 2-15)
- [x] Kernel grazing-edge fix, f64 glTF positions, Divide by length, Shift plane lock in 3-point tools (2-5).
- [x] Component Cut Opening / Always Face Camera / Shadows Face Sun (6); Sandbox Smoove, Stamp, Drape, Add Detail, Flip Edge (7).
- [x] Import: DXF blocks/text/dimensions/hatches, glTF and FBX hierarchy; export: 3DS, USDZ, selection only, ASCII STL/PLY, PLY UVs, FBX separate parts (8, 9).
- [x] Tray panel collections, Statistics, Mix, .style files, tag multi-select, scene thumbnails (10); Model Info fonts, dimension expert settings, OpenStreetMap location snapshot (11); watermark masks and order, Modeling colors (13); Search button (14); Model Library cache and recent downloads (12).
- [x] IFC import/export, openings and boolean differences cut (15); Preferences > Files folders used (FEATURES 43).
- [x] `validate` geometry checks on a grid (about 80x faster).
- [x] Mesh import healing (`mesh_heal.rs`): invalid imports 16 -> 3 of 59 sample files, all Khronos glTF samples valid. Left (LEFTOVERS 17): snap rounding for near-collinear edges and paper-thin solids; slivers lost on glTF re-export of healed faces.
- Waiting on the trial: LEFTOVERS 1 (model-only data in .skp, needs sample files) and 5 (Arc Side and Center, Intersection with Hidden Section).

## Risks

1. **Kernel robustness.** Floating-point tolerance bugs are where most effort goes. Invariant checker and property tests from day one.
2. **Matching behavior exactly.** Inference priorities, tool edge cases and menu enable/disable rules need constant comparison with the real app. Everything observed goes into `spec/` so it is checked, not remembered.
3. **VFF writing.** No MIT source covers it yet.
4. **CUDA interop with wgpu.** May need raw Vulkan (`ash`) for external memory.

## Open questions

- Is a test corpus of real `.skp` files available, across versions 2013 to 2026? The trial covers 2026; older versions need files from elsewhere.
