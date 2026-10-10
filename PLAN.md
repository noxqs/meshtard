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
- [ ] Not yet read (Python oracle has them, C++ does not): edge layer and curve id, legacy face layers, legacy model-level dimensions, text positions.
- [ ] Loading into `mt-kernel` entities moves to M2, where the kernel is built.

Reading first means real SketchUp models are available for testing everything else.

### M2: Kernel
- [x] `mt-kernel`: vertices, edges, faces with holes; an edge can border any number of faces.
- [x] Add line: endpoint snapping (0.001"), crossing edges split, collinear edges reused, auto-face on closed coplanar loops, faces split by lines across them, holes from nested loops.
- [x] Erase edge: bounded faces deleted, coplanar neighbors merged, split lines healed.
- [x] Validity checker after every operation: 16 scenario tests plus random sequences (3,000+ cases per run).
- [x] `.skp` import into kernel entities (all 25 sample files pass topology checks), triangulation (exact area on every well-formed face).
- [x] Renderer: shaded two-sided faces with materials, edges drawn on top, translucency. The app draws `.skp` files with faces.
- [ ] Not yet: undo (transactions), spatial index for large models (lookups are linear today), Intersect Faces.

### M3: Renderer and GUI shell
- [x] Faces, edges, profiles (borders and silhouettes), front/back face colors.
- [x] `mt-ui`: menu bar, toolbars (Getting Started, Large Tool Set) with original icons, Default Tray with Entity Info, status bar, Measurements box (typing captured from the viewport), all built from `spec/`.
- [x] Selection: click, Ctrl/Shift/Ctrl+Shift, window vs crossing, double/triple click, highlight; Select All, Select None, Delete.
- [x] CPU BVH for picking.
- [ ] CUDA triangulation and BVH build: to be done on the user's Linux machine with an NVIDIA GPU (Remote Control session).
- [ ] Not yet: dockable toolbars, other tray panels' contents, component-aligned selection boxes.

### M4: Inference engine
- [x] `mt-infer`: endpoint, midpoint, origin, on edge, on axis, on face.
- [x] Directions from the start point: red/green/blue axes, parallel and perpendicular to the reference edge, "from point" tracking; arrow-key locks (right red, left green, up blue, down parallel) and Shift lock.
- [x] Typed lengths in the Measurements box (mm, cm, m, ", ', feet and inches; model units by default).
- [x] Line tool (brought forward from M5 to exercise inference): chained segments, ends when a face closes, Esc cancels.
- [ ] Not yet: center and intersection inferences, the hover delay SketchUp uses before acquiring points, inference on component geometry while editing inside it.

### M5: Drawing tools
- [x] Line, Freehand, Rectangle, Rotated Rectangle, Circle, Polygon, Arc (center), 2 Point Arc, 3 Point Arc, Pie, Eraser.
- [x] Curves in the kernel: arcs, circles and freehand curves select, erase and report (Entity Info) as one entity.
- [x] Shapes lie on the clicked face's plane or the ground; arrow keys lock the plane. Typed values: rectangle `w,h`, radius, `24s` sides/segments, arc bulge, arc angle.
- [x] Eraser: click or drag; Shift hides, Ctrl softens/smooths, Ctrl+Shift unsoftens.
- [ ] Not yet: square/golden-section and tangent/half-circle inferences, rectangle orientation on tilted faces as SketchUp does it, Eraser drag preview highlighting in SketchUp's style, undo.

### M6: Modeling tools
- [x] Undo/Redo (snapshot per edit, 100 steps).
- [x] Push/Pull (Ctrl keeps the start face; pushing through cuts a hole), Move and Ctrl copy with `5x` / `/5` arrays, Rotate (+copy, typed angle), Scale (simplified, three points or typed `s` / `x,y,z`), Offset (typed distance, arcs stay arcs), Follow Me (select path, click profile), Intersect Faces with Model / with Selection.
- [x] Tape Measure: parallel guide from an edge, guide point otherwise; guides imported from `.skp`, used by inference; Edit > Delete Guides.
- [ ] Not yet: Protractor, Dimensions, Text; Scale grips; Move/Rotate showing the moved geometry live (now an outline); Offset trimming partial collapses; Push/Pull double-click repeat; Ctrl toggling copy mid-operation.
- [ ] CUDA: intersection and solid tools (mt-cuda in progress on the Linux machine).

### M7: Scene structure
- [x] Groups and components: Create Group, Make Component (G), Explode, Make Unique; double-click edits in place (rest of the model faded, not selectable; Esc, click outside or Edit > Close Group/Component closes). Undo covers the whole model and edit context.
- [x] Tray panels: Tags (add, delete, rename, visibility, color; Entity Info Tag dropdown), Materials (In Model, Create Material, edit color and opacity; Paint Bucket), Outliner, Scenes (add, update, delete, activate; scene tabs), Styles (edges, profiles), Shadows (time, date, light, dark, on faces, on ground; View > Shadows).
- [x] Renderer: sun shadows (shadow map), section cut clipping and cut lines (renderer API), transparent materials sorted back to front.
- [ ] Not yet: Section Plane tool and section planes in the model; face styles (wireframe, hidden line, X-ray, monochrome) in the renderer; Components panel and component axes dialog; textures; scene transitions; tag folders and dashes; Entity Info editing beyond Tag.

### M8: `.skp` writing
- Legacy format (2013-2020) via the OpenSKP port.
- VFF (2021+) writing: not covered by OpenSKP. Needs own research, or skppy if its license allows.
- Test: files we write open in real SketchUp and in OpenSKP.

### M9: Windows and macOS
- Windows: Vulkan or DX12 via wgpu, CUDA works the same.
- macOS: Metal via wgpu. No CUDA on macOS, so the CPU fallback (or a Metal compute path) is used there.
- Native menu bar on macOS, matching the macOS SketchUp menus.

### Later
- Ruby plugin API (kept in view).

## Risks

1. **Kernel robustness.** Floating-point tolerance bugs are where most effort goes. Invariant checker and property tests from day one.
2. **Matching behavior exactly.** Inference priorities, tool edge cases and menu enable/disable rules need constant comparison with the real app. Everything observed goes into `spec/` so it is checked, not remembered.
3. **VFF writing.** No MIT source covers it yet.
4. **CUDA interop with wgpu.** May need raw Vulkan (`ash`) for external memory.

## Open questions

- Is a test corpus of real `.skp` files available, across versions 2013 to 2026? The trial covers 2026; older versions need files from elsewhere.
