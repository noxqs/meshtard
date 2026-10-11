# 3D exchange formats

What SketchUp 2026 (desktop) imports and exports, and what Meshtard does for each
format. `[doc]` = from help.sketchup.com (read 2026-10-11), `[inf]` = from memory or
inferred, unverified. Code: `crates/mt-kernel/src/formats/` (new formats),
`mesh_import.rs` / `mesh_export.rs` (OBJ, STL, COLLADA); app dialogs in
`crates/mt-app/src/import.rs` and `export.rs`. glTF/GLB and DXF/DWG are covered by a
separate work item and are not described here.

## SketchUp's list

| Format | SketchUp import | SketchUp export | SketchUp options (export unless said) |
|---|---|---|---|
| 3DS | yes [doc] | yes [doc] | Import: Merge Coplanar Faces, Units [doc]. Export: Full Hierarchy / By Tag / By Material / Single Object, current selection, two-sided faces, stand-alone edges, texture maps (preserve texture coordinates or weld vertices), cameras from pages, units [doc]. 65,536 vertices per mesh; 8.3 texture names [doc] |
| COLLADA (.dae) | yes [doc] | yes [doc] | two-sided faces, triangulate, hidden geometry, texture maps, component hierarchy [inf] |
| DWG / DXF | yes [doc] | yes (2D and 3D) [doc] | separate work item |
| FBX | no [inf] (the help only has an export page [doc]) | yes [doc] | Export Only Current Selection, Triangulate All Faces, Export Two-Sided Faces, Separate Disconnected Faces, Export Texture Maps, Swap YZ Coordinates (Y Is Up), Units (default Model Units) [doc] |
| glTF / GLB | yes (2025+) [inf] | yes [doc] | separate work item |
| IFC / IFCZIP | yes [inf] | yes [inf] | not planned (BIM schema) |
| KMZ (Google Earth) | yes [inf] | yes [inf] | geo-located model as COLLADA in a ZIP [inf] |
| OBJ | yes (2024+) [inf] | yes [doc] | Export: current selection, triangulate, two-sided faces [doc], texture maps, swap YZ, units [inf] |
| PLY | no [inf] | no [inf] | (not a SketchUp format; common for scans and point clouds) |
| 3MF | no [inf] (extension only) | no [inf] | (common for 3D printing; Meshtard extra) |
| STL | yes [doc] | yes [doc] | Import: units, merge coplanar, preserve drawing origin [inf]. Export: selection only, ASCII/binary, units [inf] |
| USDZ | no [inf] | yes [doc] | not done |
| VRML (.wrl) | no [inf] | yes [doc] | Output Texture Maps, Ignore Back of Face Material, Output Edges, Use VRML Standard Orientation, Generate Cameras, Allow Mirrored Components, Check for Material Overrides [doc] |
| XSI | no [inf] | yes [doc] | not done (Softimage, discontinued) |

## Meshtard

| Format | Import | Export | Implementation | License | Limitations |
|---|---|---|---|---|---|
| OBJ | yes | yes | own (`mesh_import::parse_obj` + `formats::obj` for `vt`/`map_Kd`; `mesh_export::to_obj`) | own | groups/objects, normals and smoothing ignored on import; MTL options before the `map_Kd` file name ignored; export triangulates and flattens |
| STL | yes | yes | own | own | binary export only; no colors |
| COLLADA | no | yes | own (`mesh_export::to_dae`) | own | flattened, no instancing |
| FBX | yes | yes | own reader for binary (7.x, 64-bit records from 7500, zlib arrays) and ASCII (6.x/7.x); own binary 7.4 writer | own; layout from Blender's `io_scene_fbx` (`encode_bin.py`, GPL: read for the format only, no code copied) and the 2013 "FBX binary file format specification" notes. `fbxcel`/`fbxcel-dom` (MIT/Apache-2.0) read binary only and are not used | Import: meshes with full model transforms (pivots, offsets, pre/post rotation, rotation order, geometric transform), axes and `UnitScaleFactor` from GlobalSettings, materials (diffuse, opacity), diffuse textures (embedded or file, PNG/JPEG), first UV layer; hierarchy flattened into one group; normals, skinning, animation, cameras, lights ignored. Export: one mesh model, one geometry, per-polygon materials, normals, one UV layer, texture files next to the `.fbx`; no hierarchy, no ASCII FBX (Blender cannot read ASCII FBX) |
| PLY | yes | yes | own | own; format after Paul Bourke's PLY description | Import: ASCII and binary (LE/BE), all scalar types, vertex/face colors as `Color_RRGGBB` materials (optional; >256 colors rounded to 4 bits), UVs with a `TextureFile` comment. Export: binary LE with per-vertex colors (vertices split per material); no UVs; point clouds (no faces) give nothing |
| 3MF | yes | yes | own (ZIP via the `zip` crate, MIT, already a dependency; small own XML reader) | own; 3MF Core Spec 1.3 and Materials Extension (BSD-2-Clause specs) | Import: build items, nested components, Production Extension `p:path` parts, base materials, color groups, 2D texture groups; units attribute. Export: one welded mesh object, base materials (alpha for translucency), textures as texture groups. Beam lattice, slices, multi-properties ignored |
| 3DS | yes | no | own chunk reader | own; chunk ids from the public 3DS notes / lib3ds chunk list | world-space vertices, materials (diffuse, transparency, first texture map), UVs, master scale; keyframer hierarchy, cameras, lights, smoothing groups ignored; 3DS has no unit (dialog default inches, unverified) |
| VRML 2.0 | no | yes | own | own; ISO/IEC 14772 node layout | one `Shape` per material, `IndexedFaceSet` triangles, Material diffuse/transparency, ImageTexture with coordinates; options texture maps, ignore back of face material (`solid FALSE`), standard orientation (Y up); no edges, cameras, mirrored-component or material-override options |
| KMZ | no | yes | own (KML 2.2 + COLLADA from `to_dae` in a ZIP) | own | placed at the model's latitude/longitude, heading = north angle, relative to ground; COLLADA in inches |

## Test files used (downloaded to a scratch folder, not in the repository)

- assimp `test/models` (repository BSD-3-Clause; per-file provenance in its README files):
  OBJ (box, spider, cube_usemtl, cube_mtllib_after_g, concave_polygon, WusonOBJ,
  regr01, space_in_material_name), STL (Spider ascii/binary, Wuson, sphereWithHole,
  3DSMaxExport, triangle_with_two_solids), FBX (box, spider, phong_cube, cubes_*,
  global_settings, huesitos, embedded_ascii/box, box_orphant_embedded_texture,
  boxWithCompressedCTypeArray, maxPbrMaterial_metalRough, embedded_double_uv,
  animation_with_skeleton, transparentTest), PLY (cube, cube_binary, cube_uv,
  float-color, Wuson, pond.0, cube_binary_header_with_RN_newline), 3MF (box,
  cube_gears_prod_req, production_components), 3DS (fels, test1,
  cube_with_diffuse_texture, cubes_with_alpha, RotatingCube,
  CameraRollAnimWithChildObject).
- tinyobjloader `models/` (MIT): cornell_box, cube, catmark_torus_creases0.
- Stanford bunny (Stanford 3D Scanning Repository, free for research use):
  bun_zipper.ply, bun_zipper_res2.ply.
- 3MF Consortium `3mf-samples` (BSD-2-Clause): core box, cylinder, cube_gears,
  heartgears, torus, multiple_cylinders; material rhombicuboctahedron_color,
  pyramid_vertexcolor, dodeca_chain_loop_color.

Round trips: `cargo run --release -p mt-kernel --example formats_check -- OUT FILE...`
imports each file (its units, coplanar faces merged), exports OBJ, STL, COLLADA, FBX,
PLY, 3MF, VRML and KMZ, re-imports OBJ/STL/FBX/PLY/3MF and compares area, bounding box,
materials and textures (`MERGE=0` keeps the triangles). Exports were also read with the
assimp 5.3 command line tool (`assimp info`) as an independent reader.
