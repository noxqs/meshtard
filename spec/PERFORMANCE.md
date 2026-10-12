# Performance on large models

How fast meshtard is with big models, measured stage by stage, and what was changed
to make it faster. The numbers come from the benchmark in `crates/mt-app/src/bench.rs`.

## Running the benchmark

```text
cargo test --release -p mt-app bench_large_models -- --ignored --nocapture
```

It prints one line per stage and, at the end, a Markdown table of all of them.

| Variable | Effect |
|---|---|
| `MESHTARD_BENCH=grid,instances` | Run only the models whose names contain one of the words. |
| `MESHTARD_BENCH_FILES=a.gltf:b.ifc` | Add `.skp`, glTF/GLB or IFC files (glTF and IFC are placed as one component, as File > Import does). |
| `MESHTARD_BENCH_GRID=300` | Size of the synthetic grid (default 577: a million edges). |
| `MESHTARD_BENCH_NO_GPU=1` | Skip the GPU stages (for profiling the CPU ones). |

Frame times need a wgpu adapter; without one they are skipped. On a machine without a
GPU, Mesa's software rasterizer (lavapipe / llvmpipe) works.

## Models

| Model | Edges | Triangles | What it stands for |
|---|---:|---:|---|
| grid 1M edges | 999,941 | 665,858 | A 577 x 577 waved grid of triangles at the root: one huge flat context (terrain, a scanned mesh). |
| instances 10k | 120,000 | 120,000 | 10,000 instances of one box component (furniture, fasteners, trees). |
| groups 5k | 60,000 | 60,000 | 5,000 groups, each with its own definition (an imported model). |
| nested 4^6 | 49,152 | 49,152 | Components nested 6 deep, 4 per level: 4,096 boxes from 7 definitions. |
| coedge_orientation_regression.skp | 86,161 | 174,634 | Real SketchUp file (FIAT 850 car), `tests/corpus/openskp`. |
| legacy_v2014_synthetic.skp | 44 | 22 | Small file: the fixed cost of each stage. |
| gondola_v20.skp | 18,275 | 53,269 | Real SketchUp file, `tests/corpus/openskp`. |
| Sponza.gltf | 139,162 | 258,819 | Khronos glTF sample "Sponza" (geometry only, textures not downloaded). |
| bridge.ifc | 10,859 | 17,312 | buildingSMART sample `IFC 4.0.2.1/Simple-Scene/Infra-Bridge.ifc`. |
| landscaping.ifc | 24,655 | 20,190 | buildingSMART sample `IFC 2.3.0.1/Simple-Scene/Infra-Landscaping.ifc`. |

The public samples are downloaded by hand (not in the repository) and passed with
`MESHTARD_BENCH_FILES`.

## Stages

Each model is saved to `.skp` and opened again, then:

| Stage | What it times | When the app does it |
|---|---|---|
| save, open | `Document::to_skp`, `Document::open` (split into parsing the file, `mt_skp::read`, and building the model, `import::from_skp`) | File > Save, File > Open |
| scene build | `Document::scene`: everything drawn, with owners for picking | every edit, undo, redo, context change (`App::rebuild`) |
| pick index (BVH), snap geometry, instance bounds | the rest of `App::rebuild` | every rebuild |
| GPU upload, frame | `Viewport::set_faces` + `set_edges`, then one 1280 x 800 frame (mean of 5) | every rebuild; every repaint |
| first hover pick / first hover inference | the first mouse move after a rebuild (builds the culling indexes, see below) | first mouse move after an edit |
| hover pick, hover inference | `selection::pick` and `Engine::infer` at 48 cursor positions over the view (mean) | every mouse move (Select tool / drawing tools) |
| select all, selection highlight | Edit > Select All and the highlight it draws (`live_view::Highlight`) | selection changes |
| move preview (1/10) | one mouse move of Move with a tenth of everything selected: stretch of connected geometry, the kept and moved triangles, the highlight | every mouse move while Move, Rotate or Scale runs |
| undo snapshot, move commit | `Model::clone` alone, and the Move itself (`Document::edit`, which takes the undo snapshot) | every edit |
| rebuild after move, undo | `Document::scene` after the move, then Edit > Undo | after an edit |

Each model also prints a checksum of its scene (every vertex, index, edge, owner,
material, hidden edge and container) and of the pick and inference answers at the 48
cursor positions. They are the same before and after every change below, for every
model, including the million-edge grid.

## Results

Measured on a shared 4-core VM (Intel Xeon @ 2.8 GHz, no GPU: frames on llvmpipe),
release build. "Before" is the base commit with only the benchmark added; "after" is
this change, run right after it. Other jobs shared the machine, so times vary by up
to about 1.5x between runs: stages nothing here touches (save, open, GPU upload,
frame) show that noise. A re-run of the grid gave 0.93 s for scene build and 0.92 s
for rebuild after move (the 2.10 s below is noise).

### What a user feels

| Model | hover pick | hover inference | move preview (1/10) | scene build | rebuild after move |
|---|---:|---:|---:|---:|---:|
| grid 1M edges | 274 ms → 0.030 ms | 696 ms → 0.32 ms | 136 s → 1.20 s | 1.43 s → 1.05 s | 1.30 s → 2.10 s (noise; 0.92 s on re-run) |
| instances 10k | 38 ms → 0.020 ms | 96 ms → 0.22 ms | 38 ms → 19 ms | 251 ms → 50 ms | 260 ms → 62 ms |
| groups 5k | 16 ms → 0.010 ms | 43 ms → 0.19 ms | 14 ms → 12 ms | 140 ms → 65 ms | 114 ms → 64 ms |
| nested 4^6 | 14 ms → 0.010 ms | 39 ms → 0.020 ms | 30 ms → 22 ms | 92 ms → 22 ms | 72 ms → 31 ms |
| coedge_orientation_regression.skp | 28 ms → 0.020 ms | 72 ms → 0.080 ms | 42 ms → 28 ms | 335 ms → 241 ms | 351 ms → 199 ms |
| gondola_v20.skp | 7.0 ms → 0.010 ms | 13 ms → 0.020 ms | 7.1 ms → 4.0 ms | 104 ms → 35 ms | 77 ms → 33 ms |
| Sponza.gltf | 42 ms → 0.050 ms | 125 ms → 0.82 ms | 152 ms → 134 ms | 626 ms → 420 ms | 668 ms → 390 ms |
| bridge.ifc | 3.4 ms → <0.01 ms | 9.3 ms → 0.010 ms | 13 ms → 5.6 ms | 40 ms → 20 ms | 47 ms → 23 ms |
| landscaping.ifc | 7.5 ms → 0.020 ms | 17 ms → 0.030 ms | 13 ms → 8.2 ms | 48 ms → 26 ms | 42 ms → 26 ms |

The first mouse move after an edit builds the culling indexes ("first hover" rows
below): about as long as one hover took before, once per edit instead of on every
move.

### Every stage

Before → after (speed-up; "~" is within noise or under a millisecond; times are printed to 0.01 ms, so the hover speed-ups are rough).

| stage | grid 1M edges | instances 10k | groups 5k | nested 4^6 | coedge_orientation_regression.skp | legacy_v2014_synthetic.skp | gondola_v20.skp | Sponza.gltf | bridge.ifc | landscaping.ifc |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| generate | 767 ms → 593 ms (1.3x) | 90 ms → 84 ms (~) | 48 ms → 29 ms (1.7x) | 1.0 ms → 0.93 ms (~) |  |  |  |  |  |  |
| save (.skp bytes) | 3.84 s → 3.51 s (~) | 4.7 ms → 4.8 ms (~) | 211 ms → 206 ms (~) | 0.37 ms → 0.42 ms (~) | 150 ms → 135 ms (~) | 2.6 ms → 2.2 ms (~) | 11 ms → 17 ms (0.63x) | 990 ms → 1.05 s (~) | 17 ms → 18 ms (~) | 38 ms → 34 ms (~) |
| open: parse .skp | 26.26 s → 20.84 s (1.3x) | 39 ms → 44 ms (~) | 848 ms → 730 ms (1.2x) | 0.50 ms → 0.55 ms (~) | 915 ms → 1.05 s (~) | 5.7 ms → 3.3 ms (1.7x) | 56 ms → 61 ms (~) | 5.09 s → 4.48 s (~) | 108 ms → 97 ms (~) | 243 ms → 203 ms (1.2x) |
| open: build model | 2.21 s → 2.77 s (0.80x) | 5.3 ms → 4.3 ms (1.2x) | 148 ms → 94 ms (1.6x) | 0.05 ms → 0.05 ms (~) | 118 ms → 156 ms (0.75x) | 0.86 ms → 0.52 ms (~) | 6.4 ms → 6.4 ms (~) | 610 ms → 616 ms (~) | 13 ms → 11 ms (1.2x) | 27 ms → 24 ms (~) |
| open | 20.74 s → 20.90 s (~) | 45 ms → 52 ms (~) | 874 ms → 728 ms (1.2x) | 0.44 ms → 0.40 ms (~) | 1.11 s → 1.12 s (~) | 6.0 ms → 3.9 ms (1.5x) | 95 ms → 55 ms (1.7x) | 6.72 s → 6.32 s (~) | 166 ms → 106 ms (1.6x) | 269 ms → 228 ms (1.2x) |
| scene build | 1.43 s → 1.05 s (1.4x) | 251 ms → 50 ms (5.0x) | 140 ms → 65 ms (2.2x) | 92 ms → 22 ms (4.1x) | 335 ms → 241 ms (1.4x) | 0.33 ms → 0.13 ms (~) | 104 ms → 35 ms (3.0x) | 626 ms → 420 ms (1.5x) | 40 ms → 20 ms (2.0x) | 48 ms → 26 ms (1.9x) |
| pick index (BVH) | 468 ms → 306 ms (1.5x) | 59 ms → 47 ms (1.3x) | 28 ms → 16 ms (1.8x) | 34 ms → 13 ms (2.7x) | 95 ms → 58 ms (1.6x) | 0.01 ms → 0.01 ms (~) | 66 ms → 14 ms (4.6x) | 205 ms → 86 ms (2.4x) | 6.9 ms → 4.2 ms (1.6x) | 11 ms → 5.9 ms (1.8x) |
| snap geometry | 258 ms → 171 ms (1.5x) | 77 ms → 27 ms (2.9x) | 23 ms → 9.2 ms (2.5x) | 34 ms → 6.3 ms (5.4x) | 56 ms → 20 ms (2.8x) | 0.03 ms → 0.02 ms (~) | 27 ms → 2.9 ms (9.3x) | 159 ms → 28 ms (5.8x) | 8.2 ms → 1.7 ms (4.8x) | 15 ms → 3.3 ms (4.4x) |
| instance bounds | 3.5 ms → 3.8 ms (~) | 66 ms → 18 ms (3.8x) | 21 ms → 9.9 ms (2.2x) | 24 ms → 3.1 ms (7.8x) | 40 ms → 13 ms (3.1x) | 0.02 ms → 0.01 ms (~) | 26 ms → 2.9 ms (8.9x) | 61 ms → 14 ms (4.4x) | 3.5 ms → 0.95 ms (3.7x) | 4.9 ms → 1.5 ms (3.3x) |
| GPU upload | 486 ms → 351 ms (1.4x) | 61 ms → 72 ms (0.84x) | 18 ms → 19 ms (~) | 13 ms → 13 ms (~) | 62 ms → 69 ms (~) | 0.04 ms → 0.04 ms (~) | 18 ms → 14 ms (1.3x) | 106 ms → 93 ms (~) | 4.7 ms → 4.6 ms (~) | 5.1 ms → 5.5 ms (~) |
| frame (1280x800) | 4.42 s → 2.67 s (1.7x) | 939 ms → 543 ms (1.7x) | 614 ms → 375 ms (1.6x) | 456 ms → 336 ms (1.4x) | 1.07 s → 724 ms (1.5x) | 140 ms → 78 ms (1.8x) | 375 ms → 269 ms (1.4x) | 1.46 s → 686 ms (2.1x) | 272 ms → 113 ms (2.4x) | 319 ms → 138 ms (2.3x) |
| first hover pick | 318 ms → 317 ms (~) | 36 ms → 29 ms (1.2x) | 22 ms → 25 ms (~) | 12 ms → 48 ms (0.25x) | 63 ms → 28 ms (2.3x) | 0.02 ms → 0.02 ms (~) | 16 ms → 4.6 ms (3.5x) | 80 ms → 39 ms (2.1x) | 2.8 ms → 3.5 ms (0.79x) | 21 ms → 8.8 ms (2.4x) |
| hover pick | 274 ms → 0.03 ms (9150x) | 38 ms → 0.02 ms (1879x) | 16 ms → 0.01 ms (1576x) | 14 ms → 0.01 ms (1416x) | 28 ms → 0.02 ms (1410x) | 0.01 ms → <0.01 ms (~) | 7.0 ms → 0.01 ms (703x) | 42 ms → 0.05 ms (848x) | 3.4 ms → <0.01 ms (678x) | 7.5 ms → 0.02 ms (378x) |
| first hover inference | 651 ms → 411 ms (1.6x) | 94 ms → 49 ms (1.9x) | 42 ms → 43 ms (~) | 41 ms → 26 ms (1.6x) | 62 ms → 41 ms (1.5x) | 0.06 ms → 0.03 ms (~) | 13 ms → 6.4 ms (2.0x) | 104 ms → 61 ms (1.7x) | 7.2 ms → 3.8 ms (1.9x) | 16 ms → 8.1 ms (2.0x) |
| hover inference | 696 ms → 0.32 ms (2176x) | 96 ms → 0.22 ms (435x) | 43 ms → 0.19 ms (229x) | 39 ms → 0.02 ms (1943x) | 72 ms → 0.08 ms (894x) | 0.04 ms → 0.02 ms (~) | 13 ms → 0.02 ms (654x) | 125 ms → 0.82 ms (153x) | 9.3 ms → 0.01 ms (935x) | 17 ms → 0.03 ms (567x) |
| select all | 806 ms → 596 ms (1.4x) | 15 ms → 6.8 ms (2.2x) | 5.6 ms → 11 ms (0.51x) | 2.8 ms → 1.8 ms (1.6x) | 8.6 ms → 4.9 ms (1.7x) | 0.01 ms → 0.01 ms (~) | 2.0 ms → 1.4 ms (1.5x) | 14 ms → 6.4 ms (2.2x) | 1.2 ms → 0.55 ms (2.3x) | 1.3 ms → 0.97 ms (1.3x) |
| selection highlight | 826 ms → 944 ms (~) | 29 ms → 21 ms (1.4x) | 16 ms → 13 ms (1.2x) | 11 ms → 5.0 ms (2.1x) | 14 ms → 10 ms (1.4x) | 0.65 ms → 0.86 ms (~) | 3.1 ms → 2.1 ms (1.5x) | 21 ms → 14 ms (1.6x) | 4.2 ms → 0.97 ms (4.3x) | 4.5 ms → 2.2 ms (2.0x) |
| move preview (1/10) | 136.17 s → 1.20 s (114x) | 38 ms → 19 ms (2.0x) | 14 ms → 12 ms (~) | 30 ms → 22 ms (1.3x) | 42 ms → 28 ms (1.5x) | 0.03 ms → 0.02 ms (~) | 7.1 ms → 4.0 ms (1.8x) | 152 ms → 134 ms (~) | 13 ms → 5.6 ms (2.3x) | 13 ms → 8.2 ms (1.5x) |
| undo snapshot (clone) | 703 ms → 642 ms (~) | 3.2 ms → 2.5 ms (1.3x) | 76 ms → 61 ms (1.2x) | 0.16 ms → 0.15 ms (~) | 45 ms → 43 ms (~) | 0.04 ms → 0.05 ms (~) | 4.5 ms → 4.0 ms (~) | 387 ms → 289 ms (1.3x) | 12 ms → 13 ms (~) | 13 ms → 17 ms (0.75x) |
| move commit | 7.79 s → 7.56 s (~) | 1.6 ms → 1.2 ms (1.4x) | 48 ms → 30 ms (1.6x) | 0.06 ms → 0.12 ms (~) | 33 ms → 29 ms (~) | 0.02 ms → 0.04 ms (~) | 2.5 ms → 2.6 ms (~) | 250 ms → 188 ms (1.3x) | 8.7 ms → 6.2 ms (1.4x) | 14 ms → 12 ms (1.2x) |
| rebuild after move | 1.30 s → 2.10 s (0.62x) | 260 ms → 62 ms (4.2x) | 114 ms → 64 ms (1.8x) | 72 ms → 31 ms (2.3x) | 351 ms → 199 ms (1.8x) | 0.11 ms → 0.08 ms (~) | 77 ms → 33 ms (2.3x) | 668 ms → 390 ms (1.7x) | 47 ms → 23 ms (2.0x) | 42 ms → 26 ms (1.6x) |
| undo | 0.02 ms → 0.02 ms (~) | <0.01 ms → 0.04 ms (~) | <0.01 ms → <0.01 ms (~) | <0.01 ms → 0.08 ms (~) | 0.02 ms → 0.36 ms (~) | <0.01 ms → 0.01 ms (~) | 0.04 ms → 0.01 ms (~) | 0.02 ms → 0.03 ms (~) | 0.08 ms → 0.11 ms (~) | 0.06 ms → 0.06 ms (~) |
| read file |  |  |  |  | 1.25 s → 984 ms (1.3x) | 9.7 ms → 6.9 ms (1.4x) | 64 ms → 86 ms (0.74x) | 7.93 s → 7.65 s (~) | 462 ms → 442 ms (~) | 264 ms → 217 ms (1.2x) |

## What changed

Every change keeps the output identical: the scene and hover checksums above, plus
tests that compare each fast path with the code it replaces.

1. **Hover picking and inference cull by screen position** (`mt-infer/src/cull.rs`,
   `Scene::edges_near`, `Geometry::edges_near` / `vertices_near`). Both used to project
   every edge (and every vertex) of the model on every mouse move. A bounding volume
   hierarchy over the edges' and points' boxes is now built on the first hover after a
   rebuild, and each hover keeps only the items whose box meets the pyramid of view
   rays through a square of pixels around the cursor (the aperture plus 2 px). The
   exact pixel tests then run over those, in index order, so ties resolve as before.
   Under 1,024 items every item is tested as before. Tests:
   `cull::tests::near_keeps_every_segment_within_the_radius`,
   `tests::culled_inference_matches_a_full_scan` (mt-infer) and
   `selection::tests::culled_pick_matches_a_full_scan` (mt-app).
2. **One projection matrix per hover** (`Camera::project_with`). `Camera::project`
   rebuilt the view and projection matrices for every point; pick and inference now
   compute them once (same arithmetic, same results).
3. **Move / Rotate / Scale preview: stretch no longer quadratic** (`move_extra::stretch`).
   It looked each neighboring edge and face up in the list of targets with a linear
   search; a hash set now. With a tenth of the million-edge grid selected, one mouse
   move took over two minutes.
4. **Components are tessellated once per scene** (`Document::collect`). The faces of an
   entities collection reached a second time (another instance of the same definition)
   reuse the triangles computed for it, in the definition's coordinates.
5. **Simple faces skip the general tessellation work** (`tessellate::triangulate`). A face
   with one loop that never touches itself goes straight to earcut with the same input
   the general case would give it (no loop splitting, region/hole sorting or second copy
   of the loop). Test: `simple_face_shortcut_matches_the_general_case` compares both on
   every face of the corpus.
6. **Faster hashing** for the hot sets and maps: the selection, the highlight and preview
   target sets, the stretch, the scene's hidden-edge set, the snap vertex de-duplication
   and the instance bounds now use the kernel's FxHash (`spatial::FxMap` / `FxSet`, now
   public) instead of SipHash. None of them is iterated in an order that matters.
7. **Containers as lists** (`Scene::edge_container` / `tri_container`): the group or
   component of each edge and triangle was a hash map keyed by index; it is now a list by
   index (shorter when the last entries are in none), which also makes building the snap
   geometry cheaper.
8. **Pick index build** (`Bvh::build`): the three-times-centroid of each triangle is
   computed once instead of in every comparison of every split (same values, same tree).
9. **The selection highlight and live preview code moved out of `App`**
   (`mt-app/src/live_view.rs`), unchanged, so the benchmark runs exactly what the app runs.

The app was also checked by hand under Xvfb with the car model: hover, Select All with
its highlight, and a Move with its live preview and inference tooltip draw as before.

## Left to do

Measured but not changed here, biggest first:

- **Opening and saving `.skp`** (grid: ~21 s to parse, ~3.5 s to write; groups 5k:
  0.7 s to parse). The time is in `mt-skp` (`legacy::fill`, `Archive::read`, its hash
  maps and many small allocations), which this change did not touch.
- **Move commit on huge flat contexts** (grid: 7-8 s for 100k faces). Spread over
  `transform_vertices`' welding: building the spatial index (`Entities::build_index`),
  `set_vertex_positions` and its `edges_in` / `faces_in` / `edges_near_segment` queries.
- **Undo snapshots copy the whole model** (grid: 0.6-0.7 s, Sponza 0.3 s per edit).
  Structural sharing needs shared ownership inside `Model` / `Definition`, whose
  struct literals are also built by the importers (`formats/`, `gltf_io.rs`); and the
  app changes some model settings directly without an undo step (view toggles, style
  and shadow settings), so a snapshot cannot assume only the edited context changed.
- **Whole-scene rebuilds after small edits.** `App::rebuild` rebuilds the scene, pick
  index, snap geometry and GPU buffers for the whole model after every edit. An
  incremental rebuild (only the edited context's part) would need the scene split per
  context, with owners and indices kept stable.
- **No GPU instancing.** Each instance's triangles are copied into the vertex buffer
  (instances 10k: 120k triangles where 12 would do with per-instance transforms).
- **Select All and the highlight on huge models** (grid: ~0.6 s and ~0.9 s): the
  highlight rebuilds and re-uploads the whole edge list on every selection change.
- **Frame time** on llvmpipe grows with edges (grid ~3 s per frame); not measured on a
  real GPU here.
- Opening a file without a saved view builds the scene twice (once for Zoom Extents in
  `Document::extents_camera`, once in `App::rebuild`).
