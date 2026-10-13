# Fuzzing the file readers

A damaged or hostile file must give an error: never a panic, a hang or a huge
allocation. This page says how the readers are fuzzed, what was found and fixed,
and the limits the readers now apply.

## How to run

The workspace pins a stable toolchain, so `cargo fuzz` (libFuzzer, nightly) is not
used. `crates/mt-fuzz` is a mutation fuzzer that runs on stable Rust:

```sh
# Every target, 20 000 mutated inputs each (findings in fuzz-out/):
cargo run --release -p mt-fuzz -- --iters 20000 --out fuzz-out
# Some targets, another seed, a tighter time limit:
cargo run --release -p mt-fuzz -- --iters 100000 --seed 7 --budget-ms 2000 dxf ifc
# The targets and their seed files; write the seeds out:
cargo run --release -p mt-fuzz -- --list
cargo run --release -p mt-fuzz -- --dump-seeds /tmp/seeds
# One input (exit 0 fine, 101 panic, 3 hang; an over-limit allocation aborts):
cargo run --release -p mt-fuzz -- --one dxf crash.bin
# Shrink a failing input while it fails the same way:
cargo run --release -p mt-fuzz -- --minimize dxf crash.bin small.dxf
```

- **Targets** (`mt_fuzz::targets`): `skp_legacy`, `skp_vff` (`mt_skp::parse`, then
  `mt_kernel::import::from_skp`), `step`, `ifc`, `dxf` (parse and `to_model`),
  `gltf`, `glb`, `obj`, `stl`, `stl_ascii`, `fbx`, `ply`, `ply_ascii`, `3mf`, `3ds`
  (`formats::read_scene`, then the faces built as File > Import does), `png`, `jpeg`
  (`mt_kernel::images::decode`, the texture decoder). Each runs the reader and the
  conversion the app does next and discards the result: `Err` is a pass.
- **Seeds**: the OpenSKP corpus (`tests/corpus/openskp`, files up to 250 kB),
  files meshtard's own exporters write from the small corpus models (OBJ, STL
  binary and ASCII, FBX, PLY binary and ASCII, 3MF, 3DS, IFC, glTF, GLB, legacy
  `.skp`), and hand-written files with what the exporters do not write
  (`crates/mt-fuzz/seeds/`: DXF entities, a STEP file with escapes and nested
  lists, an OBJ with materials, texture coordinates and negative indices, a PLY
  with colors and texture coordinates, an ASCII FBX), a PNG and a JPEG.
- **Mutations** (`mt_fuzz::Mutator`): bit flips; interesting bytes; 1, 2, 4 and 8
  byte integers set to boundary values or multiples of the file length (length and
  count field inflation), little and big endian; small additions to 32-bit fields;
  deletion, insertion and duplication of byte ranges; truncation; splicing from
  another seed of the format; for text formats, numbers replaced with boundary
  values (`-1`, `4294967296`, `1e308`, `NaN`, `inf`, ...) and lines deleted,
  duplicated or moved; for zip containers (VFF `.skp`, 3MF, `.style`, `.mtx`), one
  entry mutated and the archive written again so the reader gets past the zip.
- **Checks**: each target runs in a child process with a counting global allocator
  (`mt_fuzz::alloc`): a single allocation over **256 MiB** or more than **1 GiB**
  in use aborts the child. Inputs run on a worker thread with a 2 MiB stack (as the
  app's loading threads) and a time limit (`--budget-ms`, default 5 s): longer is a
  hang. Panics are caught and grouped by location (with the first `mt_kernel` /
  `mt_skp` function for panics inside std or a dependency). The input being run is
  written to `current-<target>.bin` first, so an abort leaves it behind; the
  campaign saves it to `crashes/` and restarts the child.
- **Regression tests**: `crates/mt-fuzz/regressions/<target>/` holds the
  minimized (or hand-reduced) inputs that crashed; `cargo test -p mt-fuzz` replays
  them with the same allocator limits, replays the seeds and fuzzes each target for
  200 inputs. `crates/mt-app/src/fuzz_tests.rs` fuzzes the app's own readers
  (`.style` zip and text files, `.mtx` extension packages): 300 inputs in
  `cargo test`, `FUZZ_ITERS` (default 20 000) with
  `cargo test -p mt-app --release fuzz_app_readers_long -- --ignored`.

The fuzzer is blind (no coverage feedback): it finds what mutations of the seeds
reach. Richer seeds find more.

## What was found and fixed

Runs (several campaigns with different seeds, fixes in between, the last one on the
final code): about 110 000 mutated inputs per target (DXF, IFC, STEP, glTF, GLB,
OBJ, STL, PLY, FBX, 3DS), 170 000 for legacy `.skp`, 210 000 for 3MF, 150 000 for
PNG and JPEG, 25 000 for VFF `.skp` (about 35 inputs a second), 200 000 each for
`.style` and `.mtx`; then hand-made inputs for the patterns the findings showed
(instancing, nesting, archives). Share of mutated inputs read without an error in
the last run, a hint of how deep they get: 3DS 79 %, legacy `.skp` 74 %, VFF `.skp` 58 %, 3MF
43 %, ASCII STL 42 %, OBJ 40 %, JPEG 33 %, FBX 31 %, STL 29 %, DXF 20 %, IFC and
STEP 13 %, PLY 9 to 11 %, glTF 6 to 9 %, PNG 1 % (checksums). Every finding is
fixed at its cause; valid files read as before (the corpus checksums and the
OpenSKP oracle tests are unchanged, the seeds must read without an error).

| Reader | Found | Fix |
|---|---|---|
| Mesh import (OBJ, STL, PLY, FBX, 3DS, 3MF, glTF, IFC) | A NaN or infinite vertex coordinate panicked in `earcutr` (`unwrap` on `None`: no leftmost point) | `mt_kernel::geom::earcut` refuses coordinates over 1e150 or not finite (all kernel callers; the section fill in `mt-render` checks the same); `mesh_import::to_entities` leaves out polygons with a missing vertex index or a vertex beyond 1e12 inches or not finite |
| Mesh healing | Huge coordinates overflowed the spatial grid's `i64` cell arithmetic: a near-endless loop pushing cells, then a 512 MB allocation | `validate::Grid` counts cells in saturating `i128` |
| FBX scenes | `scene::sub_mesh` indexed positions without a check | polygons naming a missing vertex are skipped |
| STEP / IFC | An unterminated `"binary"` value at the end of the file moved the lexer past the end: slice panic | the lexer stops with an error |
| glTF / GLB | The `gltf` crate's validation indexes the `POSITION` accessor before checking it exists (panic); its accessor readers hit `unreachable!` on index accessors of float type, and trust strides and sparse counts | the accessors a primitive names are checked before validation; accessors are read by meshtard's own reader (`gltf_read.rs`) with every offset, stride, type and sparse index checked, at most one value per buffer byte over the import (2²⁵ at least: a small file can name one large accessor many times) |
| glTF / GLB | Nodes listed as children of several nodes (or in a cycle) were built once per path: exponential | each node is built once |
| FBX (binary) | A compressed array property could claim 2³⁰ elements (an 8 GB `with_capacity`; a zlib bomb unpacked without limit) | arrays of at most 2²⁵ elements, decompression bounded by the claimed size; nesting at most 64 levels; offsets checked |
| FBX | A geometry is copied for every model using it: a 13 kB file (one compressed 400 000-vertex mesh, 40 models) asked for 16 million vertices | the coordinates and corners placed are at most max(2²⁴, 8 per file byte) |
| 3MF | Components nested 30 deep, each naming its child twice: 2³⁰ copies of a triangle from a 700-byte file | the vertices and triangles placed are at most max(2²², 8 per file byte) |
| DXF | `ARC` / `ELLIPSE` / hatch angles of 1e20: `while t <= s { t += TAU }` never ends; sweeps of 1e300: `usize::MAX` arc segments (capacity overflow, multi-GB allocations); sweeps of many turns: self-crossing curves that make face finding crawl | whole turns added in one step (`turns_above`), at most one turn of sweep, at most 24 segments per arc |
| DXF | Dimension blocks naming each other (`DIMENSION` without its measurement but with a block, exploded in place) nest 32 deep with any fan-out: 2³² shapes (hang) | at most twice the drawing's own shapes plus 200 000 are added over all expansions |
| DXF | `INSERT` arrays of 1000 × 1000 cells, any number of times: a 1 kB file asked for 40 million instances | at most 100 000 array cells per file (past it, an array keeps its first cell) |
| PLY | An element with no properties and a count of 2⁶⁴ − 1: a loop reading nothing, forever | elements without properties are not looped over |
| IFC | Boolean results and composite curves naming the same operand twice, nested: exponential (2³¹ joins) | representation items and curves read at most max(200 000, 8 per entity); Boolean operations at most 1000 plus twice the file's; vertices made at most max(10 million, 64 per entity) |
| IFCZIP | The `.ifc` entry was unpacked without limit | at most 100 bytes per archive byte (16 MiB at least) |
| 3MF | Parts were unpacked without limit (zip bomb) | at most 200 bytes per archive byte (64 MiB at least) |
| `.skp` (VFF) | A `model.dat` entry declaring 1 GB reserved 1 GB before reading a byte | at most 64 MiB reserved up front; the vector grows with the real content |
| Texture images, photos, thumbnails | A PNG header claiming 65 535 × 65 535 pixels allocated the picture (up to the `image` crate's default 512 MiB, then RGBA copies) | `mt_kernel::images` (the app) and `mt-render`'s texture decoder: at most 16 384 pixels a side, 8192 × 8192 pixels, 256 MiB of decoder memory |
| `.style` (zip) | Entries were read to their end whatever their declared size | at most 64 MiB an entry, 256 MiB in all, 10 000 entries |
| `.mtx` packages | The 20 MB limit summed declared sizes (which can lie, and overflow), then read each file up to 20 MB | the bytes really read count, and sizes add without overflow |

Clean in these runs: the legacy `.skp` reader (already bounds-checked with
OpenSKP's limits), the VFF reader beyond the reservation above, `from_skp`, the
STEP parser beyond the above, 3DS, STL, ASCII FBX, PNG and JPEG decoding.

## The app

Reading is the last line of defense too: `errors::guard` runs a reader under
`catch_unwind`, logs a panic (`log::error!`) and turns it into an error, which
the usual error dialog (`App::file_error`) shows. Guarded: File > Open
(`Document::open`), File > Import of every format (reading and building the faces,
which undoes any half-made definitions), opening `.skp` files as components
(component reload, the Components tray, the model library, thumbnails), `.style`
files and `.mtx` packages.

## Limits

| Limit | Value | Where |
|---|---|---|
| Fuzzer: one allocation | 256 MiB | `mt_fuzz::alloc::MAX_SINGLE` |
| Fuzzer: memory in use | 1 GiB | `mt_fuzz::alloc::MAX_LIVE` |
| Fuzzer: time per input | 5 s (`--budget-ms`) | `fuzz_readers` |
| Imported mesh coordinate | 1e12 inches | `mesh_import::MAX_COORD` |
| Triangulated coordinate | 1e150 | `geom::earcut` |
| glTF values read per import | one per buffer byte, at least 2²⁵ | `gltf_read::MIN_VALUES` |
| glTF accessor without a buffer view | 65 536 elements (sparse only) | `gltf_read` |
| FBX array property | 2²⁵ elements | `fbx::MAX_ARRAY` |
| FBX nesting | 64 levels | `fbx::MAX_DEPTH` |
| FBX coordinates and corners placed | 8 per file byte, at least 2²⁴ | `fbx::MIN_GEOMETRY` |
| 3MF vertices and triangles placed | 8 per file byte, at least 2²² | `threemf::MIN_GEOMETRY` |
| DXF array cells per file | 100 000 | `dxf::MAX_ARRAY_CELLS` |
| DXF expanded shapes | 2 × the drawing's + 200 000 | `dxf::EXTRA_BUILT_SHAPES` |
| DXF arc sweep | one turn | `dxf::arc_segments`, `turns_above` |
| IFC items and curves read | max(200 000, 8 per entity) | `ifc::MIN_WORK` |
| IFC Boolean operations | 1000 + 2 per Boolean result | `ifc::MIN_BOOLEANS` |
| IFC vertices made | max(10 million, 64 per entity) | `ifc::MIN_POINTS` |
| IFCZIP unpacked | 100 × archive, at least 16 MiB | `ifc::UNZIP_RATIO` |
| 3MF unpacked | 200 × archive, at least 64 MiB | `threemf::UNZIP_RATIO` |
| `.skp` entry reserved up front | 64 MiB | `vff::Zip::MAX_PREALLOC` |
| Image side / pixels / decoder memory | 16 384 / 8192² / 256 MiB | `mt_kernel::images` |
| `.style` entry / total / entries | 64 MiB / 256 MiB / 10 000 | `style_zip` |

Limits are set well above what real files need; a file over one gets an error or
loses only what is past the limit (named in the table).

## Known gaps

- No coverage guidance: deep paths (IFC's geometry, the VFF `model.dat` records)
  get few mutated inputs that still parse that far.
- Environment images (HDR, EXR) are decoded with the `image` crate's default
  limits (512 MiB): large equirectangular maps are normal there.
- Valid but huge or adversarial geometry (millions of crossing edges) is slow in
  the kernel's face finding; that is algorithmic cost, not a reader bug, and no
  reader limit stops it.
