# meshtard: a SketchUp-style modeler in Rust

## Approach

Clean-room reimplementation of SketchUp's *behavior*, not disassembly of its binaries.

- SketchUp's EULA prohibits decompiling, and copying its code would taint the project.
- What makes SketchUp useful is observable from the outside: the push/pull workflow, "sticky" edges that auto-form faces, and the inference (snapping) engine. All of it can be studied by using the app and rebuilt from scratch.
- The `.skp` format is proprietary. Trimble ships an official C SDK for reading/writing it. Whether its license permits use in this project is unverified; check before relying on it. Until then, use open formats (glTF, OBJ, STL).

## Core design decisions

| Area | Choice | Why |
|---|---|---|
| Geometry kernel | Custom non-manifold boundary representation (radial-edge style): vertices, edges, loops, faces | SketchUp allows an edge shared by 3+ faces, dangling edges and faces with holes. A plain half-edge mesh cannot represent that. |
| Numbers | `f64` + a global merge tolerance | Simple and fast. Exact arithmetic is not worth the cost for this kind of modeler. |
| Math | `glam` (DVec3, DMat4) | Fast, f64 support, widely used. |
| Storage | Generational arenas (`slotmap`) for entities | Stable IDs, cheap deletes, easy undo diffs. |
| Undo/redo | Every tool emits a transaction (list of entity inserts/removes/changes); undo replays it in reverse | Matches SketchUp's one-step-per-operation undo. |
| Windowing | `winit` | Standard. |
| Rendering | `wgpu` | Cross-platform (Vulkan/Metal/DX12/WebGPU). |
| UI | `egui` (toolbar, entity info, outliner) | Fast to build, integrates with wgpu. |
| Face triangulation | `earcutr` or `spade` (constrained Delaunay) | Faces with holes need constrained triangulation. |
| Picking | CPU ray cast against a BVH (`parry3d` or own) | Needed for inference anyway; GPU ID buffer can come later. |
| File format | Own format via `serde` (RON for debugging, binary for release) | Versionable, easy to test. |

## Workspace layout

```
crates/
  mt-kernel/     geometry + topology, no graphics deps (most tests live here)
  mt-ops/        modeling operations: split, merge, push/pull, follow me, offset, intersect
  mt-infer/      inference/snapping engine
  mt-scene/      model, groups, components, layers/tags, materials, undo stack
  mt-render/     wgpu renderer: faces, edges, profiles, selection highlight
  mt-tools/      interactive tools (state machines driven by input events)
  mt-io/         native format, glTF/OBJ/STL import/export
  mt-app/        winit + egui shell, camera, tool palette
```

## Milestones

### M0: Skeleton
- Cargo workspace, CI (fmt, clippy, test).
- Window with orbit/pan/zoom camera, ground grid, red/green/blue axes.

### M1: Kernel
- Entities: Vertex, Edge, Loop, Face (outer loop + inner loops), with radial edge links.
- Invariants checker (`kernel.validate()`), run after every op in tests.
- Core ops:
  - Add edge: merge with existing vertices within tolerance, split crossing edges.
  - Auto-face: when a new edge closes a coplanar loop, create a face. If the loop lies inside an existing face, split that face or create a hole.
  - Erase edge: remove faces that depend on it, heal coplanar neighbours.
- Property-based tests (`proptest`) for random edge soups.

This is the hardest milestone. Everything else depends on it being robust.

### M2: Rendering and selection
- Triangulate faces, draw front/back face colors (SketchUp's white/blue-grey convention).
- Draw edges as screen-space thick lines; profile edges thicker.
- Click/shift/ctrl selection, window vs crossing box select.

### M3: Inference engine
- Snap targets: endpoint, midpoint, on-edge, on-face, center, intersection.
- Directional inferences: parallel to axes, parallel/perpendicular to an edge, "from point" (hover to acquire, track along axis).
- Axis locking with arrow keys and shift-lock.
- Typed input (Value Control Box): enter exact lengths after starting a tool.

### M4: Drawing tools
- Line, Rectangle, Circle, Polygon, Arc (2-point), Eraser.
- Curves are edge chains with a "curve" tag so they select and edit as one unit.

### M5: Modeling tools
- Push/Pull: extrude a face along its normal, merge with coplanar geometry, cut through when pushing to the opposite face.
- Move/Copy (with array copies like `5x`), Rotate, Scale.
- Offset, Follow Me, Tape Measure + guide lines.
- Intersect Faces.

### M6: Scene structure
- Groups and Components (definition + instances, edit in place, make unique).
- Tags (layers), visibility, outliner panel.
- Materials and colors, Paint Bucket.

### M7: I/O and polish
- Native save/load with versioning.
- glTF, OBJ, STL export; OBJ/STL import.
- Scenes (saved camera views), section planes, X-ray/hidden-line styles.
- `.skp` import if the SDK license allows.

## Risks

1. **Kernel robustness.** Floating-point tolerance bugs (near-coplanar faces, tiny edges) are where most effort will go. Mitigate with the invariant checker and property-based tests from day one.
2. **Inference feel.** SketchUp's usability comes from snapping priorities and timing. Expect lots of iteration; build a debug overlay showing candidate snaps and their scores.
3. **Scope.** M0 to M5 already gives a usable modeler. M6 and M7 are optional extensions.

## First steps

1. Create the workspace and `mt-kernel` with entity types and the invariant checker.
2. Implement "add edge" with vertex merging and edge splitting, fully tested.
3. Implement auto-face for planar closed loops.
4. Only then start the renderer.
