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
| `MESHTARD_BENCH_STEPS=1` | Print the time of each part of the rebuilds run as the app does (and how much of the scene each kept). |

Frame times need a wgpu adapter; without one they are skipped. On a machine without a
GPU, Mesa's software rasterizer (lavapipe / llvmpipe) works.

## Models

| Model | Edges | Triangles | What it stands for |
|---|---:|---:|---|
| grid 1M edges | 999,941 | 665,858 | A 577 x 577 waved grid of triangles at the root: one huge flat context (terrain, a scanned mesh). |
| instances 10k | 120,000 | 120,000 | 10,000 instances of one box component (furniture, fasteners, trees). |
| groups 5k | 60,000 | 60,000 | 5,000 groups, each with its own definition (an imported model). |
| nested 4^6 | 49,152 | 49,152 | Components nested 6 deep, 4 per level: 4,096 boxes from 7 definitions. |
| windows 5k | 240,000 | 250,000 | 5,000 instances of a window component: a frame of four boxes around a translucent pane (glazing, railings). |
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
| GPU upload, frame | `Viewport::set_faces` + `set_edges` (and, since instancing, the instanced meshes and their instances: `instancing::upload_faces`, `Scene::flat_edges`), then one 1280 x 800 frame (mean of 5) | every rebuild; every repaint |
| first hover pick / first hover inference | the first mouse move after a rebuild (builds the culling indexes, see below) | first mouse move after an edit |
| hover pick, hover inference | `selection::pick` and `Engine::infer` at 48 cursor positions over the view (mean) | every mouse move (Select tool / drawing tools) |
| select all, selection highlight | Edit > Select All and the highlight it draws (`live_view::Highlight`) | selection changes |
| move preview (1/10) | one mouse move of Move with a tenth of everything selected: stretch of connected geometry, the kept and moved triangles, the highlight | every mouse move while Move, Rotate or Scale runs |
| undo snapshot (first), move commit, undo snapshot (after an edit) | `Document::snapshot` alone (the first undo step copies everything), the Move itself (`Document::edit`, which takes an undo step), and the undo step the next edit would take (it copies only what the Move changed) | every edit |
| rebuild after move, undo | `Document::scene` after the move (with the triangles kept from the first build), then Edit > Undo | after an edit |
| small edit, undo snapshot (after a small edit), undo (small edit) | Move of one of the moved entities, the undo step the next edit would take, then Edit > Undo | every small edit |
| first rebuild (kept), rebuild after … (full / kept), first hover after … | The model-dependent part of `App::rebuild` (scene, pick index, snap geometry, instance bounds, GPU upload, highlight without a selection) after the small edit, the move of a tenth and their undos, made in full as before the fifth round or kept as the app does now, and the first hover after it (see "Fifth round") | every edit, undo, redo |
| highlight one / all (full / kept) | The highlight of one entity and of everything, sent whole or as changes | selection changes |

Each model also prints a checksum of its scene (every vertex, index, edge, owner,
material, hidden edge and container; instanced triangles count as their flattened
vertices, see "GPU instancing" below) and of the pick and inference answers at the 48
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

## Opening and saving .skp

A second pass on the `.skp` reader and writer (`mt-skp`) and the kernel's import and
export (`mt-kernel/src/import.rs`, `export.rs`). Same machine and caveats as above
(release build, `MESHTARD_BENCH_NO_GPU=1`; the times move by up to about 1.5x between
runs). "Before" is the base commit, "after" this change.

| model | save | open: parse .skp | open: build model | open |
|---|---:|---:|---:|---:|
| grid 1M edges | 3.73 s → 2.44 s | 23.56 s → 3.91 s (6.0x) | 3.03 s → 2.04 s | 22.66 s → 6.00 s (3.8x) |
| instances 10k | 5.0 ms → 4.2 ms | 35 ms → 20 ms | 3.8 ms → 3.4 ms | 41 ms → 25 ms |
| groups 5k | 165 ms → 97 ms | 823 ms → 213 ms (3.9x) | 108 ms → 76 ms | 728 ms → 348 ms |
| coedge_orientation_regression.skp | 169 ms → 109 ms | 836 ms → 245 ms (3.4x) | 120 ms → 80 ms | 1.20 s → 311 ms (3.9x) |
| gondola_v20.skp | 15 ms → 6.1 ms | 57 ms → 15 ms | 7.1 ms → 4.9 ms | 68 ms → 21 ms |

"read file" (opening the original corpus file: SketchUp 2021+ format for the car):
coedge_orientation_regression.skp 1.19 s → 785 ms, gondola_v20.skp 68 ms → 21 ms. The
scene and hover checksums are unchanged for every model.

The focused benchmark, `cargo test --release -p mt-skp --test bench -- --ignored
--nocapture` (`crates/mt-skp/tests/bench.rs`), writes the million-edge grid with the
writer alone and parses it back: write 1.84 s → 1.37 s, parse 23.65 s → 3.86 s; the
car (original file) parses in 2.21 s → 0.62 s. It prints a checksum of the bytes
written and of each model read (the same before and after).
`MESHTARD_SKP_BENCH_GRID`, `_RUNS`, `_FILES` and `_CHECKSUM=0` tune it.

What changed (`crates/mt-kernel/tests/skp_checksums.rs` checks that every corpus file,
and a 90 x 90 grid written by the kernel, still parses to the same model, builds the
same kernel model and saves to the same bytes; the writer's byte-for-byte test against
OpenSKP and the round-trip tests are unchanged):

1. **Legacy reader memory** (`legacy.rs`). Every object read was a 600-byte value
   (every field any entity type has), so the grid's 4.7 million objects took gigabytes,
   and most of the time went to page faults and the allocator. The rare fields
   (names, transforms, textures, annotations) now sit in a box only the entities that
   have them allocate; edge uses (2 million in the grid) keep only their edge and
   sense in the slot table, and a loop keeps those pairs instead of the objects.
2. **Slot table as a list** (`Slots`). The archive numbers objects one after another,
   so the store map is a list indexed from the first slot (a hash map only for the few
   slots outside the walk) instead of a SipHash map of millions of entries.
3. **Faster hashing** (`mt-skp/src/fx.rs`) for the class-name tables, the writer's
   vertex and edge sharing maps and the reader's leftover maps; the kernel import's
   id maps use the kernel's `FxMap`. None is iterated in an order that matters.
4. **Vertices, edges and faces collected as lists** (`raw::Keyed`) and sorted once
   when the model is built, instead of a million map insertions; the legacy `fill`
   tracks the vertices and edges it has added with bit sets.
5. **VFF tags** (`tlv.rs`, `geometry.rs`): the tag literals were parsed from text on
   every comparison (a sixth of the time to read a SketchUp 2021+ file); they are
   constants now.
6. **Writer**: persistent ids encoded without an allocation.
7. **Kernel**: the import computes the root's bounding box only when the file has
   section planes (it is only used to place them); the export finds a free definition
   name with a set instead of comparing with every definition written so far
   (quadratic in the number of groups).
8. **Opening a file without a saved view builds the scene once** (`App::install`):
   Zoom Extents now uses the scene the app builds anyway instead of building one of its
   own first (a second build only for models with face-camera components, whose scene
   depends on the view). Not in the benchmark (it times `Document::open` alone); on the
   grid it saves one scene build, about 1 s.

## Second round: undo steps, scene rebuilds, Select All

A later change took on three items of the list below. The scene and hover checksums
are again the same before and after for every model.

### Results

Same machine and setup as above, without the GPU stages (`MESHTARD_BENCH_NO_GPU=1`);
"before" is the commit this change started from (where the undo step stage timed
`Model::clone`), "after" this change, run right after it. The machine was shared with
other builds, so differences under about 1.3x are noise (the grid's rebuild after move
measured 0.93 to 1.15 s on either side over several runs).

| stage | grid 1M edges | instances 10k | groups 5k | nested 4^6 | coedge_orientation_regression.skp | gondola_v20.skp | Sponza.gltf | bridge.ifc | landscaping.ifc |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| undo snapshot (after an edit) | 575 ms → 328 ms (1.8x) | 1.8 ms → 1.1 ms (1.6x) | 65 ms → 27 ms (2.4x) | 0.05 ms → 0.04 ms (~) | 50 ms → 8.2 ms (6.1x) | 2.9 ms → 0.84 ms (3.5x) | 380 ms → 30 ms (12.6x) | 12 ms → 1.1 ms (11.5x) | 11 ms → 2.2 ms (4.9x) |
| undo snapshot (first) | 575 ms → 253 ms (2.3x) | 1.8 ms → 1.1 ms (1.5x) | 65 ms → 55 ms (~) | 0.05 ms → 0.21 ms (~) | 50 ms → 28 ms (1.8x) | 2.9 ms → 3.4 ms (~) | 380 ms → 77 ms (4.9x) | 12 ms → 3.7 ms (3.3x) | 11 ms → 7.4 ms (1.4x) |
| move commit | 7.73 s → 7.55 s (~) | 1.0 ms → 2.6 ms (0.40x) | 50 ms → 66 ms (0.75x) | 0.02 ms → 0.06 ms (~) | 32 ms → 6.1 ms (5.2x) | 1.7 ms → 1.0 ms (1.6x) | 222 ms → 31 ms (7.0x) | 10 ms → 1.4 ms (7.8x) | 7.1 ms → 2.8 ms (2.5x) |
| undo | 0.03 ms → 0.24 ms (~) | 0.05 ms → 0.92 ms (~) | 0.06 ms → 19 ms (slower) | 0.05 ms → 0.19 ms (~) | 1.1 ms → 6.3 ms (slower) | 0.07 ms → 1.3 ms (slower) | 0.05 ms → 23 ms (slower) | 0.04 ms → 1.4 ms (slower) | 0.05 ms → 2.5 ms (slower) |
| scene build | 1.03 s → 917 ms (~) | 53 ms → 49 ms (~) | 66 ms → 78 ms (~) | 35 ms → 23 ms (1.5x) | 267 ms → 214 ms (~) | 37 ms → 29 ms (1.3x) | 642 ms → 350 ms (1.8x) | 25 ms → 18 ms (1.4x) | 23 ms → 26 ms (~) |
| rebuild after move | 929 ms → 1.15 s (~) | 50 ms → 48 ms (~) | 97 ms → 72 ms (1.4x) | 22 ms → 22 ms (~) | 214 ms → 199 ms (~) | 38 ms → 26 ms (1.5x) | 357 ms → 324 ms (~) | 17 ms → 8.9 ms (1.9x) | 21 ms → 20 ms (~) |
| select all | 684 ms → 66 ms (10.3x) | 5.9 ms → 1.2 ms (4.8x) | 2.6 ms → 0.64 ms (4.1x) | 1.6 ms → 0.37 ms (4.4x) | 4.5 ms → 0.90 ms (5.0x) | 1.2 ms → 0.41 ms (2.9x) | 6.4 ms → 1.2 ms (5.4x) | 0.56 ms → 0.10 ms (~) | 0.85 ms → 0.15 ms (~) |
| selection highlight | 1.10 s → 243 ms (4.5x) | 17 ms → 11 ms (1.6x) | 8.3 ms → 5.6 ms (1.5x) | 4.2 ms → 2.9 ms (1.4x) | 9.7 ms → 2.7 ms (3.6x) | 2.0 ms → 1.4 ms (1.4x) | 15 ms → 7.5 ms (1.9x) | 1.0 ms → 0.63 ms (1.7x) | 2.0 ms → 1.6 ms (1.3x) |

### What changed

1. **Undo steps share unchanged entities collections** (`mt-app/src/undo.rs`). An undo
   step holds the model's settings (everything but the geometry, small: copied each
   time) and one `Arc<Entities>` per collection (the root's, each definition's).
   `undo::Shared` keeps the copy each collection had in the last step; the next step
   reuses it after checking it still equals the model's collection (an exact
   comparison, `impl PartialEq for Entities`, on several threads for big ones), so only
   what changed is copied. The comparison, rather than trusting which context an edit
   names, is what keeps changes made without an undo step (Scenes applying their
   hidden geometry, Texture Position's preview, tests) from being lost. `Document::edit`
   drops the edited collection's copy (it is bound to differ). Undo and redo keep
   collections the two states share in place (with their spatial index) and move the
   others; they now compare the shared collections, which costs a few milliseconds to
   ~25 ms on big models (the "undo" row) instead of nothing. Old steps are freed on
   another thread. Kernel types are unchanged apart from new `PartialEq` derives, so no
   importer changed.
2. **Big collections are copied on three threads** (`impl Clone for Entities`: vertices,
   edges and faces at once from 50,000 entities): the grid's single million-edge root,
   which every edit there must copy, in about half the time.
3. **Faces' triangles are kept between scene builds** (`mt-app/src/tess_cache.rs`). Each
   collection with 16 faces or more keeps its faces' triangles in flat lists, in face
   order, with what they were computed from (loop vertices, positions, normal: all that
   `tessellate::triangulate` reads). The next build reuses an entry only when those
   are unchanged, so an edit re-tessellates only the faces it changed. Faces with fewer
   than 4 vertices and no holes are not kept: checking them costs about what
   tessellating them does (the grid's triangles), and per-collection bookkeeping made
   models of many tiny definitions slower (groups 5k), hence the 16-face floor. Faces
   with "cut opening" components glued to them are not kept either. The gain is
   modest (up to ~1.5x on rebuilds of real files): tessellation is about half of a
   scene build, and the rest (colors, texture coordinates, transforms) is still done
   for the whole model.
4. **Select All** uses a set of owners stored as tables by index (`selection::OwnerSet`:
   faces and edges by their key's slot, holding the key's version so a stale key never
   matches a live one; anything else in a hash set) for the selection and the
   de-duplication in `live_view::all_owners`, which also skips a face's repeated
   triangles before looking them up. Grid: 0.68 s to 0.07 s.
5. **The selection highlight** looks each selected face up once (its triangles are
   consecutive), sizes its vertex list from the count, checks locks only for groups and
   components (once per highlight, not once per edge) and skips the stretch lookups when
   nothing stretches. Grid: 1.1 s to 0.24 s. The selection's set lookups per edge got
   faster with item 4.

Tests: `undo_and_redo_restore_the_exact_model_after_many_edit_kinds` (16 kinds of edit
plus changes without an undo step in between, then undo all, redo all and a new edit
part way, comparing full `Debug` dumps of the model, context, units and state id, slot
maps' free lists included, and the scene against a new document's after every step),
`kept_triangles_follow_every_edit`, `undo::tests::steps_share_what_did_not_change`,
`selection::tests::owner_set_tells_key_versions_apart` and
`live_view::tests::all_owners_matches_a_hash_set`. The app was checked under Xvfb:
draw a rectangle, Push/Pull it, Edit > Undo, Edit > Redo.

## GPU instancing

Groups and components placed more than once are drawn from one mesh per definition,
once per instance with the instance's transform, instead of being copied into the
vertex and edge buffers once per instance (`mt-app/src/instancing.rs`,
`mt_render::Viewport::set_instance_meshes`). The scene and hover checksums are the same
before and after for every model.

### Results

Same machine and setup as above, GPU stages on llvmpipe. "Before" is the commit this
change started from (with only the memory line added to the benchmark), "after" this
change, run right after it. Memory is the size of the scene's per-triangle and
per-edge lists (vertices, indices, world triangles, owners, materials, edges, plus the
meshes) and of the buffers uploaded to the GPU (face vertices and indices, edges,
meshes, instance transforms). The machine was shared: a second "after" run measured
frame times up to 2x slower on every model, the small file included, so frame time
differences here are noise.

| stage | instances 10k | nested 4^6 | coedge_orientation_regression.skp | gondola_v20.skp | groups 5k | grid 1M edges |
|---|---:|---:|---:|---:|---:|---:|
| GPU buffers | 42.7 MB → 4.5 MB (9.5x) | 17.5 MB → 1.8 MB (9.7x) | 55.1 MB → 22.4 MB (2.5x) | 16.2 MB → 6.1 MB (2.7x) | 21.4 MB (same) | 264 MB (same) |
| scene memory | 59.0 MB → 27.8 MB (2.1x) | 24.2 MB → 11.4 MB (2.1x) | 77.4 MB → 64.3 MB (1.2x) | 22.8 MB → 13.6 MB (1.7x) | 29.5 MB (same) | 360 MB (same) |
| GPU upload | 41 ms → 2.7 ms (15x) | 8.1 ms → 1.2 ms (6.8x) | 42 ms → 19 ms (2.2x) | 7.5 ms → 3.2 ms (2.3x) | 9.5 ms → 12 ms (~) | 269 ms → 326 ms (slower, see below) |
| scene build | 33 ms → 22 ms (1.5x) | 15 ms → 13 ms (~) | 117 ms → 95 ms (1.2x) | 17 ms → 11 ms (1.6x) | 41 ms → 49 ms (~) | 697 ms → 663 ms (~) |
| rebuild after move | 34 ms → 19 ms (1.7x) | 22 ms → 9.1 ms (2.4x) | 140 ms → 78 ms (1.8x) | 16 ms → 9.9 ms (1.6x) | 39 ms → 50 ms (~) | 744 ms → 763 ms (~) |
| frame (1280 x 800) | 178 ms → 195 ms (~) | 96 ms → 93 ms (~) | 390 ms → 201 ms (noise) | 150 ms → 85 ms (noise) | 119 ms → 122 ms (~) | 1.04 s → 1.00 s (~) |
| move preview (1/10) | 10 ms → 6.8 ms (1.5x) | 11 ms → 13 ms (~) | 15 ms → 18 ms (~) | 2.4 ms → 2.4 ms (~) | 5.0 ms → 5.5 ms (~) | 1.33 s → 0.69 s (noise) |

The second run's scene builds were within 1.1x of "before" on every model (instances
10k 32 ms, gondola 17 ms): the gain in build time is small and within this machine's
noise; memory and uploads are what instancing saves. The benchmark's GPU upload stage
now also splits the edges into flattened and instanced ones (`Scene::flat_edges`, a
copy of the edge list), which the app does with the highlight anyway: that copy is
the grid's and groups 5k's extra upload time. Frames draw the same number of vertices
either way (one mesh draw per instance list instead of one big draw), so llvmpipe,
bound by vertex and fragment work, shows no change.

### How it works

1. **Meshes and placements** (`instancing.rs`). While the scene is collected, an
   instance of a definition placed twice or more (counted once per build,
   `instancing::Builder`) becomes a *placement* of a mesh: the definition's own faces
   and edges in its own coordinates (the instances it contains are placements of
   their own, so nested components instance at every level). A mesh is made per
   definition *and* per what its contents are drawn with: inherited material, tag
   (Color by Tag, dashes), fading while another context is open (rest of model or
   similar instance). Everything else that differs per instance is the transform:
   face-camera components' per-view transform, mirroring (the shader turns the front
   side around when the transform's determinant is negative) and Cut Opening (the cut
   holes are in the faces of the definition the openings are glued in, so they are in
   its mesh). The first placement of a mesh builds it (triangles kept with their
   double-precision positions and the texture coordinates per vertex); later
   placements only add their world triangles and edges to the lists used for picking
   (since the fourth round, not even those: see "Fourth round" below).
2. **Flattened instead**: definitions placed once, the instances on the path to the
   open context (their contents are edited in place and selectable), hidden instances
   drawn by View > Hidden Objects, "Shadows face sun" components and everything in
   them (their shadow casters are turned vertices of the flat buffer), and meshes with
   a face drawn by View > Hidden Geometry (the mesh of its triangles is drawn as
   world edges of each placement). Translucent meshes were flattened too until
   LEFTOVERS 26 (see "Translucent meshes and Jitter" below). Edges stay flattened
   when colored by axis (their color depends on the world direction), and for meshes
   with fewer than 8 edges (an instance of the edges costs 6 transforms).
3. **Exactness.** `Scene::tri_vertices` (and `flattened_vertices`, used by the
   checksum) gives any instanced triangle's vertices exactly as flattening made them,
   from the double-precision positions and the same normal and winding computation,
   so the scene checksum (and the picking lists, which are unchanged) prove the
   instanced scene is the flattened one. `instancing::tests` checks it per triangle,
   with tags, materials, mirrored and scaled instances, nested components, an open
   context, Hide Rest of Model, hidden geometry, Jitter and a hidden instance.
4. **Rendering** (`mt-render`). `set_instance_meshes` uploads each mesh's faces and
   edges; `set_face_instances` / `set_edge_instances` upload transforms (one buffer
   per kind, a range per mesh). Faces draw with a per-instance transform attribute
   (normals by the cofactor matrix, which is the inverse transpose up to the
   determinant's scale); edges draw the mesh's segments as instances with the
   transform repeated for their 6 quad vertices, so they go through the same line
   shader (profiles from the transformed normals, depth cue, fog, back edges, section
   clipping). Shadows, the section plane, ambient occlusion, X-ray (instanced faces
   after the model's opaque ones, then every translucent triangle sorted), fog
   and the face styles apply alike; bounds (shadow fit, depth cue and fog range) come
   from each mesh's distinct points transformed per instance (box corners for meshes
   over 256 points). The headless test `instanced_meshes_draw_like_flattened_ones`
   renders five instances (moved, turned, unevenly scaled, mirrored, tilted) both ways
   in nine styles: at most 13 of 256,000 pixels differ (rounding along edges).
5. **The app.** `App::rebuild` uploads the meshes and all placements; Move, Rotate and
   Scale (and the grips' transparent instance) leave the moving placements out of the
   instance list and draw them as preview faces (`live_view::kept_indices` returns the
   kept placements); the highlight draws selected and moving placements' edges
   flattened (with their selection color) and the rest instanced.

### Translucent meshes and Jitter (LEFTOVERS 26)

Meshes with translucent triangles (glass) are now instanced, and
so are edges with Jitter on.

**Translucent triangles.** With Nicer transparency and in X-ray the flattened
renderer sorts every translucent triangle back to front by its centroid, across the
whole model. The instanced ones now take part in that same sort
(`mt_render::viewport::Blend`): when a frame's view changed (or the faces or
instances did), the model's translucent triangles and each instance's (its mesh's
centroids placed by the instance's transform) are sorted together; each instance's
translucent triangles are written, in that order, into a stretch of one index buffer
of their own (three indices per triangle and instance, into the mesh's vertices), and
the sorted list is drawn as runs of consecutive triangles from the same source: the
model's buffer, or one instance (`draw_indexed` with that instance as the only one).
Instances that do not overlap in depth make one run each; interleaved ones are split
into as many runs as needed, so the order is exactly the flattened one. With Faster
transparency the instances' translucent triangles are drawn unsorted before the
model's, and with transparency off with the opaque ones. Shadows draw them as before.

**Jitter.** The instanced edges draw Jitter's second stroke with a second pipeline
over the same vertices (`inst_jitter_line_vs`, vertices 6..11 of the line shader), so
the instance buffer does not grow. The flattened edges now draw every edge's first
stroke, then every second one (as the instanced ones do), and the jitter's random
offsets are taken from the segment's ends snapped to an offset quarter-unit grid, so
an edge placed on the GPU (single precision) jitters like the same edge placed on the
CPU (whose ends differ in their last bits).

**Results.** The benchmark gained "windows 5k": 5,000 instances (turned seven ways) of
one window component, a frame of four boxes around a pane in a material at 35%
opacity (240,000 edges, 250,000 triangles). Same machine as above (llvmpipe), release
build, run right before and right after the change; scene and hover checksums equal.

| stage | windows 5k |
|---|---:|
| GPU buffers | 88.2 MB → 2.4 MB (37x; of which 0.12 MB of sorted instance indices) |
| scene memory | 122.0 MB → 17.2 MB (7.1x) |
| GPU upload | 124 ms → 4.7 ms (26x) |
| scene build | 103 ms → 20 ms (5.2x) |
| pick index (BVH) | 59 ms → 2.6 ms |
| rebuild after move | 121 ms → 10 ms (12x) |
| move preview (1/10) | 46 ms → 15 ms (3.1x) |
| rebuild after move (1/10), full / kept | 397 / 91 ms → 42 / 29 ms |
| frame (1280 x 800) | 493 ms → 407 ms (~, llvmpipe) |

The sort itself is as costly as the flattened one (every translucent triangle, when
the view changes) plus the per-instance index upload (12 bytes per translucent
triangle and instance).

Tests: `instanced_translucent_meshes_draw_like_flattened_ones` (headless: the
five instances of a box with translucent walls and an opaque floor, a translucent
wall of the model's own crossing them, both ways with Nicer transparency in
perspective and parallel, X-ray, X-ray with Faster, transparency off, Jitter with
shadows; and the instanced viewport drawn again after a frame from another view:
at most 6 of 256,000 pixels differ, 4 with Jitter), Jitter with endpoints added to
`instanced_meshes_draw_like_flattened_ones` (3 pixels), and
`instancing::tests::instanced_scene_flattens_to_the_flattened_one` (glass boxes
instanced, a hidden face, every edge equal to the flattened scene's, edges instanced
with Jitter).

## Third round: undo steps below the collection level

The second round shared unchanged collections between undo steps, but an edit of a
huge single context still copied all of it into the next step (grid: ~0.15 s per step
on a quiet machine, ~0.3 to 0.6 s on a loaded one), and undoing such a step copied it
back.

### Options

- **(a) Persistent chunked storage** for the vertex, edge and face slot maps: chunks
  behind `Arc`s, copied on the first write after a copy. Tried first (a `ChunkMap`
  with `slotmap`'s keys, versions and free list): undo steps were nearly free, but
  every read went through the chunk table and every write checked the chunk's kind.
  Random reads measured 10 to 30% slower and writes up to 60% slower in a
  micro-benchmark, and the grid's generation ~10% slower. Rejected: kernel operations
  must not get slower.
- **(b) An operation log per edit** (changed slots before and after, recorded inside
  `Document::edit`): every kernel write would have to go through a logging layer
  anyway, and writes made outside `Document::edit` (Scenes, Texture Position's
  preview, tests) would be missed unless logged too, which needs the same wrapper as
  (c) plus replaying in both directions.
- **(c) Dirty tracking** in the slot maps, with snapshots that share unchanged chunks:
  chosen. Reads are exactly `SlotMap`'s (flat storage), a write sets one flag, and no
  write can escape it, wherever it is made.

### What changed

1. **`mt_kernel::slot_store::SlotStore`** replaces `slotmap::SlotMap` for an
   `Entities`' vertices, edges, faces and curves. It is the same flat slot map (same
   keys, versions, free list and iteration order; its `Debug` output is
   `SlotMap`'s, so the `.skp` checksum test, which hashes model dumps, is unchanged)
   plus one flag per chunk of 256 slots, set by every `&mut` access (`get_mut`,
   `IndexMut`, `insert`, `remove`, `retain`, `iter_mut`, `values_mut`).
   `SlotStore::freeze` returns a `Snapshot` (the slots as shared, immutable chunks):
   clean chunks are taken from the previous snapshot as they are (a reference
   count), dirty ones are compared with it and copied only when they differ.
   `SlotStore::thaw` makes the store equal to a snapshot again, rewriting only the
   chunks that differ from its own last snapshot. Construction sites are unchanged
   (`Entities` is still built by `Default`; the API is `SlotMap`'s).
2. **`Entities::freeze` / `thaw` / `swap_to`** and `EntitiesSnapshot`: a collection's
   state for undo is its three map snapshots plus a copy of the rest (instances,
   curves, guides, annotations; small, copied whole when it changed). A thaw that
   changes anything drops the spatial index, freed on another thread (a million
   edges: ~0.2 s).
3. **Undo steps** (`mt-app/src/undo.rs`) hold one `Arc<EntitiesSnapshot>` per
   collection. Taking a step freezes each collection (a clean collection reuses its
   last snapshot: no comparison any more); undo and redo freeze the current state and
   thaw the target in place (`swap_to`), so they cost what differs between the two
   states. The flags replace the full comparisons of the second round as the
   guarantee that changes made without an undo step are never lost.
4. The benchmark has three more stages: a **small edit** (moving one entity), the undo
   step after it, and its undo.

### Results

`MESHTARD_BENCH_NO_GPU=1`, before (`bc0788b`) and after run back to back on a quieter
machine than the second round's (load ~1.5, 4 cores), same scene and hover checksums
for every model.

| stage | grid 1M edges | instances 10k | groups 5k | nested 4^6 | coedge_orientation_regression.skp | gondola_v20.skp | Sponza.gltf | bridge.ifc | landscaping.ifc |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| undo snapshot (after a small edit) | 139 ms → 1.1 ms (126x) | 0.79 ms → 0.92 ms (~) | 38 ms → 27 ms (1.4x) | 0.07 ms → 0.02 ms (~) | 7.9 ms → 1.3 ms (6.1x) | 0.67 ms → 0.10 ms (6.7x) | 20 ms → 0.21 ms (94x) | 1.4 ms → 0.24 ms (5.8x) | 2.7 ms → 0.53 ms (5.1x) |
| undo (small edit) | 132 ms → 0.74 ms (179x) | 0.44 ms → 1.2 ms (0.37x) | 13 ms → 9.9 ms (~) | 0.06 ms → 0.03 ms (~) | 3.8 ms → 0.75 ms (5.0x) | 0.44 ms → 0.08 ms (~) | 17 ms → 0.37 ms (47x) | 0.72 ms → 0.52 ms (1.4x) | 1.6 ms → 0.36 ms (4.5x) |
| small edit | 900 ms → 909 ms (~) | 1.5 ms → 3.7 ms (0.40x) | 14 ms → 11 ms (~) | 0.25 ms → 0.08 ms (~) | 3.7 ms → 1.1 ms (3.2x) | 1.1 ms → 0.31 ms (3.7x) | 20 ms → 0.39 ms (50x) | 1.4 ms → 0.41 ms (3.3x) | 2.3 ms → 0.58 ms (3.9x) |
| undo snapshot (after an edit) | 158 ms → 37 ms (4.2x) | 1.1 ms → 0.87 ms (~) | 20 ms → 11 ms (1.9x) | 0.04 ms → 0.02 ms (~) | 3.5 ms → 0.57 ms (6.2x) | 0.34 ms → 0.05 ms (~) | 23 ms → 0.34 ms (68x) | 0.65 ms → 0.09 ms (7.2x) | 1.6 ms → 0.24 ms (6.5x) |
| undo | 0.21 ms → 49 ms (slower) | 0.84 ms → 1.4 ms (0.60x) | 11 ms → 11 ms (~) | 0.16 ms → 0.06 ms (~) | 3.2 ms → 2.9 ms (~) | 0.79 ms → 0.16 ms (4.9x) | 18 ms → 0.81 ms (22x) | 0.95 ms → 0.21 ms (4.5x) | 7.9 ms → 0.33 ms (24x) |
| move commit | 4.85 s → 4.72 s (~) | 2.6 ms → 2.5 ms (~) | 50 ms → 9.8 ms (5.1x) | 0.05 ms → 0.03 ms (~) | 3.3 ms → 0.57 ms (5.8x) | 0.65 ms → 0.11 ms (5.9x) | 17 ms → 0.34 ms (51x) | 1.2 ms → 0.13 ms (9.0x) | 1.6 ms → 0.31 ms (5.1x) |
| undo snapshot (first) | 128 ms → 180 ms (~) | 1.2 ms → 0.83 ms (~) | 33 ms → 34 ms (~) | 0.16 ms → 0.07 ms (~) | 20 ms → 13 ms (1.6x) | 1.6 ms → 1.9 ms (~) | 60 ms → 42 ms (1.4x) | 2.9 ms → 2.6 ms (~) | 4.6 ms → 6.3 ms (~) |

Reading the rows:

- The grid's **small edit** step goes from a copy of the whole context to ~1 ms, and
  its undo from ~0.13 s to under a millisecond. The Move of a tenth of the grid
  touches chunks all over the context: its step costs 37 ms instead of ~0.15 s.
- **Undo after the big Move** got slower (0.2 ms → ~50 ms): before, the undo step
  had already paid for a full copy, which undo then swapped in; now undo rewrites the
  chunks the Move changed. Edit plus undo together went from ~0.16 s to ~0.09 s.
- **Move commit** includes the step taken before the edit, which is now a check of
  the flags instead of a comparison of every collection (5 to 50x on real files).
- **instances 10k** is ~1 to 2 ms slower in undo and small edits: its 10,000 instances
  are "the rest", copied whole into each step that changed one (as before) and now
  also back on undo (before, undo moved the step's collection in). Splitting the
  instance list the same way would remove it.
- **undo snapshot (first)** copies every chunk (no snapshot to share with yet): same
  as before within noise (repeated runs: 128 to 172 ms before, 130 to 187 ms after).

**Kernel operations.** Reads are `SlotMap`'s; writes set one flag. Repeated
alternating runs (three each, same binary inputs): scene build, `import::from_skp`
and the grid's Move commit are the same within noise (scene 316 to 327 ms vs 317 to
322 ms minimum on a 400 x 400 grid); generating the grid (pure insertion and
`get_mut`, the most write-heavy path) measured ~3 to 5% slower minimum to minimum,
and `callgrind` counts ~3% more instructions in it (the flag writes and an extra
occupancy check per lookup). `SlotStore`'s slots are exactly as big as `SlotMap`'s.

Tests: `slot_store::tests::behaves_as_slotmap` (20,000 random inserts, removes,
changes and retains against `SlotMap`: same keys, iteration and dump; every snapshot
keeps its state and thaws back to it exactly; a one-entry change shares all chunks but
one), `undo::tests::steps_share_untouched_chunks` (5,000 lines, one vertex moved and
one edge erased without telling the undo code: the step shares all but a few chunks;
undo and redo restore exact dumps), and
`undo_and_redo_restore_the_exact_model_after_many_edit_kinds` extended to 19 kinds of
edit, now including 700 lines (several chunks), an untracked change inside a chunk
shared with the previous step, erasing from the middle (free list) and drawing into
the freed slots. Breaking the flags (treating every chunk as clean) makes four of the
undo tests fail. The app was checked under Xvfb: rectangle, Push/Pull, Move, then
Edit > Undo / Redo many times; every state renders identically to the same sequence on
the commit before this change.

## Fourth round: picking from instanced meshes, undo after big edits

Two items of the list below. The scene and hover checksums are again the same before
and after for every model.

### Results

`MESHTARD_BENCH_NO_GPU=1`, the base commit (`238ac5a`) and this change built side by
side and run alternately, twice each, on a quiet machine (load ~1 to 2); the table
gives each side's faster run. Differences under about 1.5x are noise: the grid's
"undo" (the same code on both sides) measured 16.7 and 62 ms in the base commit's two
runs and 48 and 53 ms in this change's, and models without placements (grid, groups
5k), whose pick, snap and bounds code paths do the same work as before, moved by up to
1.6x either way between runs.

| stage | grid 1M edges | instances 10k | groups 5k | nested 4^6 | coedge_orientation_regression.skp | legacy_v2014_synthetic.skp | gondola_v20.skp |
|---|---:|---:|---:|---:|---:|---:|---:|
| scene build | 732 ms → 774 ms (~) | 19.9 ms → 6.3 ms (3.2x) | 49 ms → 53 ms (~) | 8.3 ms → 1.8 ms (4.6x) | 95 ms → 88 ms (~) | 0.06 ms → 0.06 ms (~) | 10.8 ms → 8.9 ms (~) |
| pick index (BVH) | 208 ms → 204 ms (~) | 27 ms → 4.3 ms (6.4x) | 11.4 ms → 15.7 ms (~) | 9.7 ms → 1.4 ms (7.0x) | 51 ms → 15.2 ms (3.3x) | <0.01 ms → <0.01 ms (~) | 10.7 ms → 4.2 ms (2.5x) |
| snap geometry | 144 ms → 136 ms (~) | 14.0 ms → 15.3 ms (~) | 6.3 ms → 6.9 ms (~) | 5.1 ms → 5.2 ms (~) | 12.4 ms → 13.7 ms (~) | 0.01 ms → 0.01 ms (~) | 2.1 ms → 2.4 ms (~) |
| instance bounds | 3.8 ms → 4.1 ms (~) | 11.9 ms → 9.3 ms (~) | 5.3 ms → 8.6 ms (0.61x) | 2.7 ms → 0.91 ms (3.0x) | 7.8 ms → 3.2 ms (2.4x) | 0.01 ms → 0.01 ms (~) | 2.1 ms → 1.9 ms (~) |
| first hover pick | 270 ms → 264 ms (~) | 22 ms → 4.9 ms (4.5x) | 9.4 ms → 11.4 ms (~) | 7.9 ms → 1.1 ms (6.9x) | 16.3 ms → 4.8 ms (3.4x) | 0.01 ms → 0.01 ms (~) | 2.8 ms → 2.8 ms (~) |
| hover pick | 0.02 ms → 0.02 ms (~) | 0.01 ms → 0.01 ms (~) | 0.01 ms → 0.01 ms (~) | <0.01 ms → <0.01 ms (~) | 0.02 ms → 0.02 ms (~) | <0.01 ms → <0.01 ms (~) | 0.01 ms → 0.01 ms (~) |
| first hover inference | 332 ms → 330 ms (~) | 34 ms → 39 ms (~) | 15.0 ms → 16.3 ms (~) | 12.2 ms → 12.2 ms (~) | 29 ms → 37 ms (~) | 0.01 ms → 0.01 ms (~) | 4.6 ms → 5.4 ms (~) |
| hover inference | 0.28 ms → 0.25 ms (~) | 0.13 ms → 0.15 ms (~) | 0.07 ms → 0.09 ms (~) | 0.01 ms → 0.01 ms (~) | 0.05 ms → 0.05 ms (~) | 0.01 ms → 0.01 ms (~) | 0.02 ms → 0.02 ms (~) |
| select all | 49 ms → 49 ms (~) | 0.96 ms → 0.93 ms (~) | 0.53 ms → 0.57 ms (~) | 0.29 ms → 0.25 ms (~) | 0.83 ms → 0.94 ms (~) | <0.01 ms → <0.01 ms (~) | 0.19 ms → 0.22 ms (~) |
| selection highlight | 197 ms → 194 ms (~) | 9.1 ms → 11.4 ms (~) | 4.2 ms → 3.6 ms (~) | 2.2 ms → 2.2 ms (~) | 2.7 ms → 3.4 ms (~) | 0.01 ms → 0.01 ms (~) | 0.86 ms → 0.94 ms (~) |
| move preview (1/10) | 739 ms → 684 ms (~) | 6.7 ms → 7.1 ms (~) | 5.5 ms → 5.2 ms (~) | 16.0 ms → 15.9 ms (~) | 20 ms → 23 ms (~) | 0.01 ms → 0.01 ms (~) | 1.7 ms → 1.8 ms (~) |
| undo snapshot (first) | 151 ms → 140 ms (~) | 0.83 ms → 0.90 ms (~) | 41 ms → 36 ms (~) | 0.08 ms → 0.07 ms (~) | 17.9 ms → 12.3 ms (~) | 0.03 ms → 0.03 ms (~) | 1.3 ms → 1.4 ms (~) |
| move commit | 4.40 s → 4.66 s (~) | 2.2 ms → 2.3 ms (~) | 10.7 ms → 9.7 ms (~) | 0.03 ms → 0.03 ms (~) | 0.63 ms → 0.70 ms (~) | 0.02 ms → 0.02 ms (~) | 0.06 ms → 0.06 ms (~) |
| undo snapshot (after an edit) | 40 ms → 37 ms (~) | 0.70 ms → 0.76 ms (~) | 12.2 ms → 10.6 ms (~) | 0.02 ms → 0.02 ms (~) | 0.49 ms → 0.46 ms (~) | 0.01 ms → 0.01 ms (~) | 0.04 ms → 0.05 ms (~) |
| rebuild after move | 704 ms → 732 ms (~) | 18.5 ms → 5.3 ms (3.5x) | 50 ms → 51 ms (~) | 8.7 ms → 1.9 ms (4.4x) | 75 ms → 67 ms (~) | 0.02 ms → 0.02 ms (~) | 9.9 ms → 9.2 ms (~) |
| undo | 16.7 ms → 48 ms (0.35x) | 1.3 ms → 1.2 ms (~) | 12.4 ms → 10.8 ms (~) | 0.06 ms → 0.05 ms (~) | 1.2 ms → 1.1 ms (~) | 0.01 ms → 0.01 ms (~) | 0.09 ms → 0.08 ms (~) |
| small edit | 783 ms → 780 ms (~) | 2.8 ms → 2.7 ms (~) | 12.5 ms → 11.9 ms (~) | 0.11 ms → 0.08 ms (~) | 0.96 ms → 0.97 ms (~) | 0.02 ms → 0.01 ms (~) | 0.17 ms → 0.17 ms (~) |
| undo snapshot (after a small edit) | 0.96 ms → 1.1 ms (~) | 0.77 ms → 0.85 ms (~) | 36 ms → 31 ms (~) | 0.02 ms → 0.02 ms (~) | 1.4 ms → 0.91 ms (1.5x) | 0.01 ms → 0.01 ms (~) | 0.05 ms → 0.05 ms (~) |
| undo (small edit) | 1.1 ms → 0.76 ms (~) | 1.2 ms → 1.2 ms (~) | 11.3 ms → 13.0 ms (~) | 0.04 ms → 0.02 ms (~) | 0.58 ms → 0.61 ms (~) | 0.01 ms → 0.01 ms (~) | 0.04 ms → 0.04 ms (~) |

Scene memory (the per-triangle and per-edge lists, `bench::scene_bytes`): instances 10k
27.8 MB → 10.4 MB, nested 4^6 11.4 MB → 4.3 MB, the car 64.3 MB → 49.2 MB, gondola
13.6 MB → 10.6 MB; models without placements unchanged.

### Picking and snapping from the meshes

The scene build used to transform every instanced triangle and edge into world space
for the picking, snapping and section lists, and the pick index (a BVH over every
world triangle) and the edge culling index were built over all of them. Now:

1. **Placements' triangles and edges are not stored** (`document.rs`, `instancing.rs`).
   They keep their numbers (triangle and edge indices, owners, materials, containers
   and hidden marks are stored for every one, as before), but `Scene` stores the world
   triangles and edges only outside placements. `Scene::tri(t)` / `Scene::edge(e)`
   compute a placement's from its mesh and transform with the arithmetic of the
   flattening (`LocalTri::world`, `Mesh::placed_edge`: the edge's ends and face normals
   are kept per mesh, each distinct end and normal placed once per placement);
   `for_each_tri` / `for_each_edge_in` walk them in order, and `triangles()` /
   `edges()` make (and keep) the full lists for the rare users (section cuts, Zoom
   Selection, thumbnails, tests). Edges colored by axis depend on the transform: their
   placements still store them (`Placement::lazy_edges` false).
2. **Two-level pick index** (`pick_index::PickIndex`, a `RayIndex`): a BVH over the
   stored triangles (on the GPU for large models with `cuda`, as before), a box index
   over the placements' world boxes (their mesh's box transformed, grown by 1e-7 of
   its size plus rounding), and one BVH per mesh in its own coordinates
   (`Bvh::ray_visit`, boxes grown likewise), queried with the ray taken into the
   placement's coordinates (the ray parameter is the same in both, the map being
   affine). Every candidate is tested on its world triangle, computed exactly as the
   flattening does, so the hit is the flattened scene's. Ties at the same distance now
   go to the lowest triangle number in `Bvh::ray` as well (`Hit::nearer`), so neither
   index's shape decides them.
3. **Two-level edge culling** (`pick_index::EdgeIndex`, behind `Scene::edges_near`): the
   stored edges in one `cull::BoxIndex`, the placements by their world box, then (for
   meshes of 32 edges or more) an index over the mesh's edges queried with the view
   pyramid taken into the placement's coordinates (`cull::Pyramid::local`: distances
   stay world distances, with a margin for rounding and the single-precision ends).
   The exact pixel test then runs on the candidates' world segments as before.
4. **Snap geometry, instance bounds, the highlight and the scene's bounds** walk the
   edges and triangles without storing them; instance bounds use each mesh's distinct
   points (8 transforms per box instead of 36 triangle corners and 24 edge ends).
5. The section plane size no longer walks the triangles when there is no section plane.

Inference still builds its world-space snap geometry (`mt_infer::Geometry`: every
edge's ends and the de-duplicated vertices, in edge order) from the computed segments:
its vertex de-duplication across instances decides the answers, so it is not
two-level (see "Left to do"). Its face hits use the two-level index.

Tests: `pick_index::tests::two_level_picking_matches_the_flattened_scene` (boxes and a
120-edge grid placed moved, turned, mirrored, unevenly scaled and tilted, nested two
levels, 1,500+ edges; plain, edges by axis, an open component and hidden geometry:
every world triangle and edge, one by one and all at once, owners' containers and
hidden marks, the checksum, ray hits (distance and triangle) at 3,600 cursors in
three views and 3,000 random rays, `selection::pick`, `Engine::infer`, the snap
geometry and the instances' bounds equal the flattened scene's), the existing
`instanced_scene_flattens_to_the_flattened_one`, `culled_pick_matches_a_full_scan`
and `culled_inference_matches_a_full_scan`.

### Undo after a big edit: measured again

The ~50 ms "undo" of the grid in the third round's table is one draw of a very noisy
stage: single runs of the base commit measured anywhere from 5 to 62 ms for that undo
(rewriting the chunks the Move changed); this change leaves the undo code as it was. A
focused measurement (move a tenth of the grid, then undo and redo 15 times; minimum
and median) gave on the base commit and on both variants tried below alike: undo 5 to
7 ms (median 7 to 10 ms), redo 7 to 9 ms. Single runs are slower when the rewrite is
the first in the process or other jobs share the machine. What an undo right after a
big edit costs is mostly the step it takes of the edited state first
(`Entities::swap_to` freezes before it thaws): 18 to 30 ms in all (about the "undo
snapshot (after an edit)" row plus the "undo" row).

Tried and not kept, as neither moved those numbers beyond the noise: rewriting the
changed slots in place with `Clone::clone_from` (the entities' lists keep their
allocations) instead of replacing them, and spreading `freeze` and `thaw` over several
threads per slot map (already three maps on three threads; medians got worse on the
shared machine). Swapping instead of copying (moving the current slots into the
new snapshot and the target snapshot's into the store when it holds the only
reference) does not apply as is: the store's last snapshot, the undo step and
`undo::Shared` all hold the target, and the store needs its last snapshot to know
which chunks are clean. A test was added for stores of many chunks with values that
own heap memory: `slot_store::tests::many_chunks_freeze_and_thaw_exactly`.

## Fifth round: incremental rebuilds

After every edit, undo and redo `App::rebuild` used to make everything again for the
whole model: the scene (colours, texture coordinates, world positions; only the
triangles of unchanged faces were kept), the pick index, the snap geometry and the
GPU buffers; the first mouse move after it built the culling indexes again; and
every selection change rebuilt and re-sent the whole edge list. Now each of them keeps
what the edit did not change and redoes the rest (LEFTOVERS 25). The scene, pick and
hover checksums are the same as before for every model, and the benchmark checks that
the kept pick index and snap geometry give the same hover answers.

### Results

Same machine as above, release build, GPU stages on llvmpipe. The benchmark now runs
the model-dependent part of `App::rebuild` itself (`bench::AppSide`: scene, pick index,
snap geometry, instance bounds, GPU upload and the highlight with nothing selected)
twice in the same run: "full" makes everything anew as the app did before this change
(`Document::scene`, `PickIndex::build`, `snap::geometry`, `instancing::upload_faces`,
`Highlight::build` and a new edge buffer), "kept" is what the app does now
(`Document::scene_from`, `PickIndex::build_kept`, `snap::geometry_kept`,
`edge_view::GpuScene`). "Small edit" moves one entity (the first face of the grid, the
first instance of the others), "move (1/10)" a tenth of everything; each is followed
by its undo. "Highlight one" selects one entity, "highlight all" everything. The first
mouse move after the small edit is compared with the base commit's first hover (pick
plus inference, which built both culling indexes from scratch).

| stage | grid 1M edges | instances 10k | groups 5k | nested 4^6 | car (.skp) | gondola (.skp) |
|---|---:|---:|---:|---:|---:|---:|
| rebuild after small edit | 1.52 s → 57 ms (27x) | 38 ms → 40 ms (~) | 86 ms → 26 ms (3.3x) | 10 ms → 16 ms (0.65x) | 110 ms → 34 ms (3.2x) | 23 ms → 13 ms (1.8x) |
| rebuild after undo of small edit | 2.24 s → 67 ms (33x) | 43 ms → 40 ms (~) | 79 ms → 23 ms (3.4x) | 11 ms → 16 ms (0.70x) | 128 ms → 39 ms (3.3x) | 22 ms → 13 ms (1.7x) |
| rebuild after move (1/10) | 2.48 s → 407 ms (6.1x) | 43 ms → 36 ms (~) | 96 ms → 41 ms (2.4x) | 11 ms → 19 ms (0.58x) | 122 ms → 35 ms (3.5x) | 23 ms → 13 ms (1.8x) |
| rebuild after undo of move (1/10) | 1.50 s → 309 ms (4.9x) | 44 ms → 45 ms (~) | 108 ms → 39 ms (2.8x) | 15 ms → 20 ms (0.76x) | 103 ms → 35 ms (3.0x) | 26 ms → 13 ms (2.0x) |
| highlight one | 131 ms → 0.17 ms (769x) | 5.3 ms → 4.0 ms (1.3x) | 6.3 ms → 0.15 ms (42x) | 4.5 ms → 5.4 ms (~) | 2.9 ms → 3.2 ms (~) | 1.8 ms → 0.30 ms (6.1x) |
| highlight all | 1.29 s → 430 ms (3.0x) | 25 ms → 24 ms (~) | 12 ms → 16 ms (0.75x) | 5.3 ms → 6.7 ms (0.79x) | 11 ms → 12 ms (~) | 2.5 ms → 3.5 ms (0.73x) |
| first mouse move after the small edit | 633 ms → 9.8 ms (65x) | 37 ms → 8.9 ms (4.2x) | 55 ms → 7.6 ms (7.2x) | 23 ms → 6.0 ms (3.9x) | 42 ms → 9.0 ms (4.7x) | 6.8 ms → 7.5 ms (~) |

Stages of the earlier rounds, base commit → this change (separate runs on the shared
machine, where other builds ran too: differences under about 2x on small models are
mostly noise; the full GPU upload of a new viewport varied from 12 to 33 ms for
groups 5k between runs, with the extra passes this change adds to `FaceBuffers::new`
measured at about 2 ms there):

| stage | grid 1M edges | instances 10k | groups 5k | nested 4^6 | car (.skp) | gondola (.skp) |
|---|---:|---:|---:|---:|---:|---:|
| scene build | 653 ms → 721 ms (~) | 3.9 ms → 5.2 ms (0.75x) | 69 ms → 42 ms (1.6x) | 1.9 ms → 3.1 ms (0.61x) | 81 ms → 97 ms (~) | 9.5 ms → 14 ms (0.68x) |
| pick index (BVH) | 227 ms → 244 ms (~) | 5.2 ms → 4.5 ms (~) | 14 ms → 11 ms (1.3x) | 1.6 ms → 1.7 ms (~) | 14 ms → 15 ms (~) | 4.4 ms → 4.4 ms (~) |
| snap geometry | 179 ms → 174 ms (~) | 16 ms → 15 ms (~) | 10 ms → 7.3 ms (1.4x) | 5.4 ms → 5.8 ms (~) | 13 ms → 16 ms (~) | 3.2 ms → 3.4 ms (~) |
| GPU upload | 319 ms → 374 ms (~) | 4.2 ms → 6.6 ms (0.64x) | 12 ms → 25 ms (0.49x) | 1.1 ms → 2.3 ms (0.49x) | 17 ms → 24 ms (0.70x) | 3.1 ms → 4.4 ms (0.71x) |
| first hover pick | 305 ms → 50 ms (6.1x) | 3.6 ms → 3.8 ms (~) | 23 ms → 5.5 ms (4.1x) | 1.4 ms → 1.8 ms (0.78x) | 4.4 ms → 4.9 ms (~) | 2.3 ms → 4.0 ms (0.57x) |
| first hover inference | 329 ms → 61 ms (5.4x) | 34 ms → 13 ms (2.7x) | 32 ms → 8.2 ms (4.0x) | 22 ms → 11 ms (1.9x) | 37 ms → 9.6 ms (3.9x) | 4.5 ms → 7.5 ms (0.60x) |
| hover pick | 0.03 ms → 0.04 ms (0.75x) | 0.01 ms → 0.01 ms (~) | 0.01 ms → 0.01 ms (~) | <0.01 ms → <0.01 ms | 0.10 ms → 0.02 ms (5.0x) | 0.01 ms → 0.01 ms (~) |
| hover inference | 0.23 ms → 0.23 ms (~) | 0.12 ms → 0.12 ms (~) | 0.05 ms → 0.07 ms (0.71x) | 0.02 ms → 0.02 ms (~) | 0.04 ms → 0.03 ms (1.3x) | 0.02 ms → 0.02 ms (~) |
| selection highlight | 1.77 s → 186 ms (9.5x) | 11 ms → 9.7 ms (~) | 3.6 ms → 4.8 ms (0.76x) | 2.7 ms → 3.3 ms (~) | 11 ms → 6.6 ms (1.7x) | 1.1 ms → 1.4 ms (~) |
| move commit | 5.04 s → 4.27 s (~) | 2.5 ms → 2.2 ms (~) | 9.4 ms → 12 ms (0.80x) | 0.05 ms → 0.05 ms (~) | 1.5 ms → 0.72 ms (2.1x) | 0.12 ms → 0.13 ms (~) |
| rebuild after move | 1.99 s → 555 ms (3.6x) | 5.0 ms → 5.0 ms (~) | 77 ms → 45 ms (1.7x) | 6.2 ms → 2.8 ms (2.2x) | 78 ms → 56 ms (1.4x) | 16 ms → 12 ms (1.4x) |
| undo | 65 ms → 69 ms (~) | 1.5 ms → 1.2 ms (1.3x) | 16 ms → 12 ms (1.3x) | 0.12 ms → 0.12 ms (~) | 6.4 ms → 1.4 ms (4.5x) | 0.16 ms → 0.28 ms (0.57x) |

Notes:

- The grid (one flat context of a million edges) gains most: a small edit draws again
  the few storage chunks it touched and writes a few thousand items into the scene's
  lists, the GPU buffers and the indexes. Moving a tenth of it redraws a quarter of
  the chunks (the moved faces' neighbours read the moved vertices).
- The car and the gondola (real files: groups and components, some instanced) gain 3x
  and 2x: groups not touched by the edit are taken whole.
- Instances 10k and nested 4^6 are all placements of a few meshes: the scene there was
  already cheap and stays about the same; moving the only top-level component of
  nested 4^6 moves everything, which makes the kept path a little slower than making it
  all anew (bookkeeping for nothing kept). Moving a group or component drawn flattened
  draws it again (its world positions change).
- Highlighting one entity no longer depends on the model's size. Select All still
  draws every edge, as before (from `Highlight::build`, sent as one buffer).
- The first mouse move after an edit builds the culling indexes in blocks on several
  threads and keeps the previous scene's blocks over unchanged edges and vertices; on
  the first scene (no blocks to keep) the grid's first hover pick and inference went
  from 305 and 329 ms to 50 and 61 ms.
- A build from scratch (`Document::scene`, "scene build") also walks by chunks and
  notes where the open context's entities go: within noise on the big models, maybe
  up to 1.3x slower on the small ones.

### How it works

1. **Chunk versions** (`mt_kernel::slot_store`). Each 256-slot chunk of a collection's
   vertices, edges and faces has a version that changes with every write to it (and
   with every rewrite by undo or redo) and never goes back; each store has an id of its
   own (copies get new ones). A chunk with the same id and version as when something
   was computed from it is exactly what it was. This costs a counter increment per
   write.
2. **Scene pieces** (`mt-app/src/scene_cache.rs`). The scene walk records, per visit of
   a collection (the root, a definition through an instance), where each chunk of its
   edges and of its faces put its edges, triangles and vertices, and the versions of
   every chunk the drawing read (the vertices, edges and faces they reach). The next
   build finds each visit again (by collection, level and order) and, when everything
   the visit depends on is the same (transform, inherited material, tag, fading, level,
   instancing role, cut openings), takes the chunks whose versions all match from the
   previous scene. A visit that makes an instanced mesh, and the placements of a kept
   mesh, are kept whole, mesh included; they do not depend on where they are placed,
   so moving a component keeps its mesh (and the GPU's copy of it). Anything else the
   walk reads (materials, tags, style, open context, face-camera view) is compared as a
   whole and rebuilds everything when it differs. The kept triangles of earlier rounds
   (`tess_cache`) follow along.
3. **Lists in place** (`scene_cache::List`). The scene's per-edge, per-triangle and
   per-vertex lists are put together from new items and ranges of the previous lists:
   when every range taken stays where it was (an edit that did not change how much a
   chunk draws), the new items are written into the previous lists in place, so a small
   edit of the grid writes a few thousand items instead of copying 360 MB.
4. **What changed.** The scene tells which ranges of each list are the previous
   scene's at the same place (`scene_cache::Changes`). The pick index keeps blocks of
   16,384 stored triangles with a BVH each (`pick_index::Blocks`, made on several
   threads) and the meshes' indexes; the snap geometry keeps blocks of 16,384 edges
   with their kept ends and de-duplicated points, and finds the vertices' first-seen
   order again only from the first changed block on and only in blocks holding a point
   a changed block gained or lost (`snap::geometry_kept`); the edge culling index
   (`pick_index::EdgeIndex`) and the inference culling index (`mt_infer::Geometry`) are
   in blocks too, and the next scene keeps the blocks over unchanged items.
5. **GPU** (`mt-app/src/edge_view.rs`, `mt_render::Viewport::update_faces`,
   `write_edges`, `set_extra_edges`). The face vertices that changed are written into
   the buffer in place (`queue.write_buffer`); the index buffer only when the indices or
   a triangle's opacity changed; the bounds per block of 65,536 vertices. The model's
   edge buffer holds the scene's edges as drawn without a selection (a copy is kept on
   the CPU: grid ~80 MB); after an edit only the changed edges are written. Move,
   Rotate and Scale send only new indices when they start and end (the vertices are on
   the GPU already).
6. **Highlight** (`live_view::Highlight::build_sparse`). The scene records where the
   open context's entities are drawn (per chunk, per instance: `ContextIndex`), so the
   highlight looks only at the edges and triangles of the selected, moving and
   stretching entities, writes the edges it draws otherwise into the edge buffer, puts
   back those it no longer does, and sends the rest (moved copies, boxes, selected
   placements' edges) as extra edges. Large selections (more than 4,096 entities and an
   eighth of the edges) are drawn as one buffer made in full, as before.
7. **Picking exactly.** `Bvh::ray` could leave out a triangle exactly as near as the
   best hit when a box's entry distance rounded above it, or when the ray ran in the
   plane of a box face (0 times infinity), so an index's answer could depend on its
   shape (seen with rays straight down on the grid's vertices). Box tests now let such
   rays in and allow a 1e-9 relative slack before giving up on a box
   (`bvh::ray_box`, `bvh::search_limit`), so blocks answer exactly as one index; the
   benchmark's hover checksums did not change.

Tests: `scene_cache::tests::incremental_scenes_equal_full_builds` (a grid of many
chunks, instanced and flattened components, groups, nested components, a translucent
instance and tiles with few edges: 27 kinds of edit and change without an undo step,
style changes, opened groups and components with Hide Rest of Model, undo all and redo
all; after each, every list of the kept scene equals a full build's, what it says it
kept is so, and building again with nothing changed keeps everything), the existing
`undo_and_redo_restore_the_exact_model_after_many_edit_kinds` and
`kept_triangles_follow_every_edit` (every step also checked kept against full),
`small_edits_keep_the_rest_in_place`, `lists_take_ranges_in_place_or_anew`,
`slot_store::tests::chunk_versions_follow_writes`,
`pick_index::tests::kept_blocks_pick_as_one_index` (blocks against one BVH on 4,000
rays a round, ties straight down on vertices, picking with the kept edge index against
a fresh scene's), `snap::tests::kept_snap_geometry_equals_a_full_build` and
`kept_snap_geometry_follows_random_edits` (60 random edits and undos; inference with
the kept culling index against a fresh one), `live_view::tests::sparse_highlight_draws_what_the_full_one_draws`,
`edge_view::tests::kept_edges_are_what_a_full_upload_holds` and the headless
`partial_updates_draw_like_full_ones` (partial writes render the same pixels).

## Left to do

Measured but not changed here, biggest first:

- **Opening and saving `.skp`, what remains** (grid: ~4 s to parse, ~2 s to build the
  kernel model, ~2.4 s to save). The legacy walk still allocates one object per
  vertex, edge, loop and face and keeps them all until the end (a per-kind arena, or
  filling the builder during the walk, would avoid most of it); building the public
  `mt_skp::Model` maps and then the kernel's slot maps copies every face twice more;
  the kernel import is bound by the kernel's own per-vertex and per-edge lists. VFF
  files copy every TLV leaf's payload (`tlv::parse_recursive`) and walk the tree
  several times (`geometry::collect_*`).
- **Move commit on huge flat contexts** (grid: 7-8 s for 100k faces). Spread over
  `transform_vertices`' welding: building the spatial index (`Entities::build_index`),
  `set_vertex_positions` and its `edges_in` / `faces_in` / `edges_near_segment` queries.
- **Undo right after a big edit in a huge flat context** (grid, after moving a tenth
  of it: ~20 to 25 ms, see "Fourth round"): the step of the edited state is taken
  first (comparing and copying every chunk the edit touched), then the changed chunks
  are rewritten. Taking that step in the background after the edit, or letting the
  store's base snapshot lend its chunks to the store, would hide most of it. Instances,
  curves, guides and annotations of a collection are still copied whole into a step
  when any of them changed (instances 10k: ~1 to 2 ms per step and per undo).
- **What rebuilds still redo** (see "Fifth round"). Groups and components drawn
  flattened (placed once, on the open context's path, hidden, "Shadows face sun",
  with a face shown as hidden geometry) are drawn again when their transform
  changes: the scene is in world space. Any change to materials, tags, the style, the open context or (with
  face-camera components) the camera draws everything again. Every rebuild still
  recomputes the instance bounds (instances 10k: ~6 ms), the section cut, the
  triangle containers of the snap geometry and the placements' top-level indexes
  (pick and edge culling), and walks every visit (models of many placements gain
  little: instances 10k and nested 4^6 stay at 15 to 45 ms). A visit is found again
  by its collection, level and order: inserting an instance before others of the same
  definition draws those again. The culling indexes keep blocks only from the scene
  just before (when a hover built them). The model's edge buffer is mirrored on the
  CPU (grid: ~80 MB).
- **What GPU instancing left flattened** (see "GPU instancing" and "Fourth round"):
  inference's snap geometry (`mt_infer::Geometry`) still holds every instanced edge's
  ends and vertices in world space (since the fifth round kept per block of edges and
  made again only where placements moved; ~15 ms and ~35 ms for its culling index
  when made in full for instances 10k). A two-level version must keep
  the vertex de-duplication across instances (its order decides ties). Section cuts
  make the full world triangle list. The owners, materials and containers are still
  stored per triangle and edge (instances 10k: ~5 MB). Selected and moving instances
  draw their edges flattened, rebuilt with the highlight. Translucent triangles of
  many interleaved instances sort into many small draws (one per run).
- **The highlight of large selections** (grid, Select All: ~0.4 s): more than 4,096
  selected or moving entities (and an eighth of the edges) are drawn as one new buffer
  made from every edge, as before; selecting few is now independent of the model's
  size.
- **Frame time** on llvmpipe grows with edges and triangles drawn, instanced or not
  (instancing saves memory and uploads, not vertex work); not measured on a
  real GPU here.
