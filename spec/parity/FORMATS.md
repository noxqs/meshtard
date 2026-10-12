# 3D exchange formats

What SketchUp 2026 (desktop) imports and exports, and what Meshtard does for each
format. `[doc]` = from help.sketchup.com (read 2026-10-11), `[inf]` = from memory or
inferred, unverified. Code: `crates/mt-kernel/src/formats/` (new formats),
`mesh_import.rs` / `mesh_export.rs` (OBJ, STL, COLLADA); app dialogs in
`crates/mt-app/src/import.rs` and `export.rs`. glTF/GLB (`gltf_io.rs`) and DXF
(`dxf.rs`) have their own modules; their import is summed up at the end.

## SketchUp's list

| Format | SketchUp import | SketchUp export | SketchUp options (export unless said) |
|---|---|---|---|
| 3DS | yes [doc] | yes [doc] | Import: Merge Coplanar Faces, Units [doc]. Export: Full Hierarchy / By Tag / By Material / Single Object, current selection, two-sided faces, stand-alone edges, texture maps (preserve texture coordinates or weld vertices), cameras from pages, units [doc]. 65,536 vertices per mesh; 8.3 texture names [doc] |
| COLLADA (.dae) | yes [doc] | yes [doc] | two-sided faces, triangulate, hidden geometry, texture maps, component hierarchy [inf] |
| DWG / DXF | yes [doc] | yes (2D and 3D) [doc] | separate work item |
| FBX | no [inf] (the help only has an export page [doc]) | yes [doc] | Export Only Current Selection, Triangulate All Faces, Export Two-Sided Faces, Separate Disconnected Faces, Export Texture Maps, Swap YZ Coordinates (Y Is Up), Units (default Model Units) [doc] |
| glTF / GLB | yes (2025+) [inf] | yes [doc] | separate work item |
| IFC / IFCZIP | yes [inf] | yes [inf] | Classifier not planned; import of geometry, structure, location and textures, export of element classes, storeys and basic properties done (see below) |
| KMZ (Google Earth) | yes [inf] | yes [inf] | geo-located model as COLLADA in a ZIP [inf] |
| OBJ | yes (2024+) [inf] | yes [doc] | Export: current selection, triangulate, two-sided faces [doc], texture maps, swap YZ, units [inf] |
| PLY | no [inf] | no [inf] | (not a SketchUp format; common for scans and point clouds) |
| 3MF | no [inf] (extension only) | no [inf] | (common for 3D printing; Meshtard extra) |
| STL | yes [doc] | yes [doc] | Import: units, merge coplanar, preserve drawing origin [inf]. Export: selection only, ASCII/binary, units [inf] |
| USDZ | no [inf] | yes [doc] | done (see below) |
| VRML (.wrl) | no [inf] | yes [doc] | Output Texture Maps, Ignore Back of Face Material, Output Edges, Use VRML Standard Orientation, Generate Cameras, Allow Mirrored Components, Check for Material Overrides [doc] |
| XSI | no [inf] | yes [doc] | not done (Softimage, discontinued) |

## Meshtard

| Format | Import | Export | Implementation | License | Limitations |
|---|---|---|---|---|---|
| OBJ | yes | yes | own (`mesh_import::parse_obj` + `formats::obj` for `vt`/`map_Kd`; `mesh_export::to_obj`) | own | groups/objects, normals and smoothing ignored on import; MTL options before the `map_Kd` file name ignored; export triangulates and flattens |
| STL | yes | yes | own | own | export binary or ASCII (File format option); no colors |
| COLLADA | no | yes | own (`mesh_export::to_dae`) | own | flattened, no instancing |
| FBX | yes | yes | own reader for binary (7.x, 64-bit records from 7500, zlib arrays) and ASCII (6.x/7.x); own binary 7.4 writer | own; layout from Blender's `io_scene_fbx` (`encode_bin.py`, GPL: read for the format only, no code copied) and the 2013 "FBX binary file format specification" notes. `fbxcel`/`fbxcel-dom` (MIT/Apache-2.0) read binary only and are not used | Import: meshes with full model transforms (pivots, offsets, pre/post rotation, rotation order, geometric transform), axes and `UnitScaleFactor` from GlobalSettings, materials (diffuse, opacity), diffuse textures (embedded or file, PNG/JPEG), first UV layer; the model hierarchy becomes nested groups inside the import's group (named after the models; group transforms keep translation and rotation, scale and mirroring stay in the faces; a file with one mesh model stays flat; `formats::scene`); normals, skinning, animation, cameras, lights ignored. Export: one mesh model, one geometry, per-polygon materials, normals, one UV layer, texture files next to the `.fbx`; with "Separate disconnected faces" one model and geometry per connected part (triangles sharing a corner, named `<file>_1`, `<file>_2`, ... in model order, each connected only to the materials it uses); no hierarchy, no ASCII FBX (Blender cannot read ASCII FBX) |
| PLY | yes | yes | own | own; format after Paul Bourke's PLY description | Import: ASCII and binary (LE/BE), all scalar types, vertex/face colors as `Color_RRGGBB` materials (optional; >256 colors rounded to 4 bits), UVs with a `TextureFile` comment. Several `TextureFile` comments with a face `texnumber`. Export: binary LE or ASCII with per-vertex colors (vertices split per material); with texture maps, MeshLab's convention: `comment TextureFile` per image (written next to the file), a face `texcoord` list and, with several textures, `texnumber`; point clouds (no faces) give nothing |
| 3MF | yes | yes | own (ZIP via the `zip` crate, MIT, already a dependency; small own XML reader) | own; 3MF Core Spec 1.3 and Materials Extension (BSD-2-Clause specs) | Import: build items, nested components, Production Extension `p:path` parts, base materials, color groups, 2D texture groups; units attribute. Export: one welded mesh object, base materials (alpha for translucency), textures as texture groups. Beam lattice, slices, multi-properties ignored |
| 3DS | yes | yes | own chunk reader and writer | own; chunk ids from the public 3DS notes / lib3ds chunk list | Import: world-space vertices, materials (diffuse, transparency, first texture map), UVs, master scale; keyframer hierarchy, cameras, lights, smoothing groups ignored; 3DS has no unit (dialog default inches, unverified). Export (`to_3ds`): one object per material ("By Material"), split at 65,535 vertices or faces, corners welded on position and UV, materials with diffuse/ambient color, transparency and texture map (images next to the file), units, two-sided faces; names cut to 16 (materials) / 10 (objects) characters; texture names are not cut to 8.3. No Full Hierarchy / By Tag / Single Object choice, stand-alone edges, cameras from pages, smoothing groups or keyframer section |
| USDZ | no | yes | own `usda` writer; ZIP via `zip` (stored, 64-byte aligned) | own; Pixar's USDZ spec and `UsdPreviewSurface` spec | flattened; one `Mesh` per material with face-varying `st`, `UsdPreviewSurface` (linear diffuse color or `UsdUVTexture`, opacity, roughness 1, metallic 0), `doubleSided` unless two-sided faces are written; `metersPerUnit` and `upAxis` from the units and Y up options (default Y up). The layer is text (`usda`, allowed by the spec); Apple Quick Look versions that need binary `usdc` may not open it (writing Pixar's crate format is not done). Checked with `usd-core` 26.8: opens, materials bound, textures resolve, all 28 built-in validators (incl. the USDZ package rules) pass |
| VRML 2.0 | no | yes | own | own; ISO/IEC 14772 node layout | one `Shape` per material, `IndexedFaceSet` triangles, Material diffuse/transparency, ImageTexture with coordinates; options texture maps, ignore back of face material (`solid FALSE`), standard orientation (Y up); no edges, cameras, mirrored-component or material-override options |
| KMZ | no | yes | own (KML 2.2 + COLLADA from `to_dae` in a ZIP) | own | placed at the model's latitude/longitude, heading = north angle, relative to ground; COLLADA in inches |
| IFC | yes (`.ifc`, `.ifczip`) | yes (IFC4) | own STEP Part 21 reader (`formats::step`), importer (`formats::ifc`) and writer (`formats::ifc_export`) | own; ISO 10303-21 and the buildingSMART IFC2x3 / IFC4 / IFC4.3 schema docs. No crate fitted: `ruststep` (Apache-2.0) generates code per schema and has none for IFC, `ifc-lite-*` is MPL-2.0, `oxideav-ifc` (MIT) is a 0.0.x preview tied to its own framework, `openbim-step` is AGPL | Import (geometry only, IFC2X3 / IFC4 / IFC4X3): spatial structure and elements (aggregation, containment, nesting) as nested groups (instance named after the entity Name or its class, definition Type = IFC class, description = GlobalId; spaces hidden; openings left out), `IfcLocalPlacement` chains, body items `IfcFacetedBrep` (with voids), shell / face based surface models, `IfcTriangulatedFaceSet`, `IfcPolygonalFaceSet` (with voids), `IfcExtrudedAreaSolid` (rectangle, rounded / hollow rectangle, circle, hollow circle, ellipse, I / L / T / U, arbitrary closed profiles with voids from polylines, indexed poly curves with arcs, composite curves of polylines and trimmed circles / lines, derived profiles), `IfcSweptDiskSolid` / `IfcSweptDiskSolidPolygonal` (mitered tube, hollow with an inner radius, rings for closed directrices; start / end parameters not read, as IfcOpenShell), `IfcRevolvedAreaSolid` (24 steps per turn), `IfcSurfaceCurveSweptAreaSolid` / `IfcFixedReferenceSweptAreaSolid` (profile carried along the directrix without twist, mitered; x axis along the reference plane's normal or extrusion surface's normal, as IfcOpenShell, or along the fixed reference), CSG primitives (`IfcBlock`, `IfcRightCircularCylinder`, `IfcRightCircularCone`, `IfcSphere`, `IfcRectangularPyramid`, `IfcCsgSolid`), `IfcAdvancedBrep` (planar faces from edge loops of lines, circles, ellipses, polylines and B-spline curves; B-spline, cylindrical, conical, spherical and toroidal faces tessellated inside their edges: parameters of the edge points, unwrapped across seams, Delaunay with interior points where the surface curves, the edge points kept so faces stay watertight; other surfaces flat between their edges and counted as skipped), `IfcMappedItem` (a component per representation map, named after its type), `IfcBooleanResult` / `IfcBooleanClippingResult` (unions and intersections joined with the Solid Tools when both operands are closed solids, else both operands / the first; differences cut with the Solid Tools' subtract when both operands are closed solids, second operands may be solids, `IfcHalfSpaceSolid`, `IfcBoxedHalfSpace` or `IfcPolygonalBoundedHalfSpace` made into solids larger than the first operand; otherwise the first operand only), openings (`IfcRelVoidsElement`) cut out of their element the same way, each opening body item on its own (elements with mapped items are not solids: not cut); uncut openings and operands are counted as skipped, cuts in the report's `cut`, surface style colors and transparency on items, `IfcIndexedColourMap` face colors, material colors through `IfcRelAssociatesMaterial` (painted on the group), `IfcIndexedTriangleTextureMap` texture coordinates with `IfcBlobTexture`, `IfcPixelTexture` (made a PNG) or `IfcImageTexture` images (URL read next to the file), georeferencing (`IfcSite` RefLatitude / RefLongitude as the model's location, `IfcMapConversion` axis or TrueNorth as north, eastings / northings / height and EPSG code kept as the geo-location's map origin, shown in Model Info > Geo-location and taken by a model without a location; a model or element farther than 1 km from the origin moved near it by whole meters, the map origin following), length unit (SI prefixes, conversion based) and plane angle unit; everything else is counted by class and shown in the status bar (e.g. `IfcAdvancedBrep`, `IfcRevolvedAreaSolid`, half spaces). One dialog option: merge coplanar faces. Export: IFC4 with one site and building; a storey per group whose Type is `IfcBuildingStorey` (at the root or inside `IfcProject` / `IfcSite` / `IfcBuilding` groups or face-less wrappers such as an import's component; elevation from its height; spaces' contents contained in the storey), else one storey; visible group and component instances are elements of the definition's Type (`IfcWall`, `IfcSlab`, `IfcBeam`, `IfcColumn`, `IfcMember`, `IfcPlate`, `IfcRoof`, `IfcStair`, `IfcRamp`, `IfcRailing`, `IfcCovering`, `IfcCurtainWall`, `IfcFooting`, `IfcPile`, `IfcChimney`, `IfcShadingDevice`, `IfcDoor`, `IfcWindow`, `IfcFurniture`, `IfcFurnishingElement`, `IfcSanitaryTerminal`, `IfcLightFixture`, IFC2X3 flow elements; `...StandardCase` as the parent class), else `IfcBuildingElementProxy` (named after the instance or definition, nested ones aggregated under their parent; an IFC element's unnamed mapped components are its own mapped body), groups as `IfcTriangulatedFaceSet` per material, components as representation maps of the class's type named after the definition placed by `IfcMappedItem` (scale and mirroring kept, shear lost), loose root faces as one proxy, materials as `IfcSurfaceStyle` (color, transparency), `Pset_<Class>Common.Reference` (component name) and `Qto_<Class>BaseQuantities.NetVolume` for solid bodies, site RefLatitude / RefLongitude when located, `IfcMapConversion` and TrueNorth from the map origin and north, units option; textures, edges and hidden entities left out; GlobalIds kept from import (group descriptions that read as GlobalIds, unique), others made from the file name (same model, same file) |

Every 3D export has SketchUp's "Export only current selection" option: the selected
entities of the open context, in model coordinates (the app copies them as Edit > Copy
does and exports that model).

## glTF and DXF import

- glTF / GLB: the node hierarchy is kept (nested groups; a mesh used by several nodes
  is a component; a node's own mesh is inlined into its group when used once; a single
  top node is unwrapped). Materials: base color (factor, texture with
  `KHR_texture_transform`), alpha; `KHR_materials_pbrSpecularGlossiness` diffuse;
  `KHR_materials_transmission` as opacity (full transmission: 30% opaque, a guess).
  Metallic, roughness, normal, occlusion and emissive maps have nowhere to go.
- DXF: LINE, (LW)POLYLINE incl. meshes, ARC, CIRCLE, ELLIPSE and SPLINE (curves),
  3DFACE, SOLID/TRACE, HATCH (faces, islands as holes; patterns not drawn), blocks and
  INSERT (components; scale, rotation, arrays, nesting), TEXT / MTEXT / ATTRIB (texts
  without leader; formatting codes removed), DIMENSION (aligned, radius and diameter as
  dimensions; others drawn from their block). Not read: ACIS solids, regions, images,
  leaders, tables, points, hatch patterns, paper space, xrefs.

Checked with `cargo run --release -p mt-kernel --example import_check -- FILE...`
(counts and `mt_kernel::validate` of every definition) on ezdxf `examples_dxf`
(MIT: hatches, texts, dimensions in nested blocks, inserts with attributes,
uncommon entities), dxf-parser `test/data` (MIT), assimp DXF and FBX models and
Khronos glTF-Sample-Assets (CesiumMilkTruck, BoxAnimated, TextureTransformTest,
TransmissionTest, SpecGlossVsMetalRough, VirtualCity, NegativeScaleTest,
OrientationTest). Every DXF and FBX import and every hierarchy group is valid, and
every one of these glTF samples is valid with `Checks::ALL` since mesh import heals
meshes (below).

## Mesh import healing

Every mesh importer (OBJ, STL, FBX, PLY, 3MF, 3DS, glTF, IFC faceted geometry) goes
through `mesh_import::to_entities`, which heals the polygon soup into valid kernel
geometry (`mt_kernel::mesh_heal`) the way SketchUp's importers leave it: points within
the tolerance weld (nearest earlier point); a vertex within 0.75 of the tolerance of a
polygon side splits it, sides that pass that close to each other get a shared vertex
(repeated until settled, at most 8 rounds, stopped when the splits grow); polygons that
then visit a vertex twice or bend out of their plane are ear-clipped and slivers
thinner than that distance are dropped; coplanar faces that overlap are replaced by
the cells of their arrangement (`arrangement::cells`), each cell taking the material,
texture position and side of the earliest face covering it (a face drawn on another
takes that face's material in SketchUp); faces over the same edges are dropped. Faces
of different planes that pass through each other are not intersected (SketchUp's
importers do not either). `import_check` (now also reading OBJ, STL, PLY, 3MF and 3DS;
`--errors` counts every problem by kind) over the 59 assimp / tinyobj / 3MF Consortium /
Stanford bunny files of `formats_check`, the Khronos samples above and the IFC samples:
invalid imports went from 16 to 3 in `formats_check` (`regr01.obj`, `Wuson.stl`,
`heartgears.3mf`) and S_Office (web-ifc) has 3 of its ~3,760 meshes left, all sliver
solids or bundles of nearly collinear long edges thinner than the tolerance, where
splitting does not settle (LEFTOVERS 17). Times (release): TransmissionTest imports in
about 2.5 s (1.0 s unhealed) and validates with `Checks::ALL` in 1.5 s (was 7 s: the
validator's vertex grid now uses the edges' cell size); S_Office in about 4.5 s
(2 s unhealed).

## IFC checks

`import_check` also reads `.ifc` / `.ifczip` (prints the importer's report and world
bounds). Files: buildingSMART `Sample-Test-Files` (IFC 2x3, IFC4 ADD2 TC1 and IFC4.3
ADD2 Simple-Scene sets, IFC4 Reference View samples) and `youshengCode/IfcSampleFiles`
(Duplex architecture, SampleCastle 49 MB, Revit ARC / STR, SampleHouse,
WallElementedCase, BasinFacetedBrep, CubeAdvancedBrep). Every import passes
`validate` with `Checks::TOPOLOGY`; with `Checks::ALL` the multi-item elements fail
where their solids overlap or touch (items are not unioned), as files authored that
way do. The per-element world bounding boxes were compared with IfcOpenShell 0.9
(`use-world-coords`): equal within 1 cm for every element of all files except
boolean-clipped walls (2 in Duplex, first operand only; checked before cutting was added), `wall-with-opening-and-window`
(IfcOpenShell ignores the file's millimeters) and WallElementedCase's non-uniform
mapped items (ours fit the wall, IfcOpenShell's stick out of it). Exports of four of
these imports pass `ifcopenshell.validate` (schema) with the same bounds. SampleCastle
imports in about 2 s (release).

Cutting (2026-10-12), with `import_check` in release on `AC20-FZK-Haus.ifc` (KIT /
buildingSMART community sample, 2.6 MB: 17 of 17 openings, 2 half spaces and 4
polygonal bounded half spaces cut, 0.5 s), the web-ifc test `example.ifc` (1 half
space cut) and `S_Office_Integrated Design Archi.ifc` (31 MB: 58 of 90 openings, 966
faceted brep and 130 half space differences cut, 244 faceted brep differences not cut
because an operand is not closed, 3.6 s). Every cut result passes `Checks::TOPOLOGY`
(checked before it is kept). In S_Office two representation maps of faceted breps
without booleans fail `Checks::TOPOLOGY` ("loop has fewer than 3 edges"), from mesh
import itself, not from cutting. Test files from `ThatOpen/engine_web-ifc`
`tests/ifcfiles/public` (MPL-2.0 repository; files downloaded to a scratch folder).

More geometry and georeferencing (2026-10-12), against IfcOpenShell 0.9.0 (`pip`):
the web-ifc functional fixtures (`tests/functional/fixtures`: revolved solids,
primitives, spheres and cylinders, surface-curve and fixed-reference sweeps, hollow
swept disk, boolean intersection, B-spline face, pixel texture; MPL-2.0 repository),
IfcOpenShell's `test/fixtures/geom` (advanced brep with a through hole, pipe, simple
sweeps) and Bonsai's `geolocation*.ifc` / `ifcviewer-web/georef-*.ifc` (LGPL
repository), `CubeAdvancedBrep` and `ISSUE_171_IfcSurfaceCurveSweptAreaSolid.ifc`.
World bounds equal IfcOpenShell's within 1 cm and volumes within the 24-segment
tessellation (revolved 9.32 vs 9.42 m3, sphere 32.6 vs 33.5, cube with twisted
B-spline sides 0.836 vs 0.833, through hole 0.43807 vs 0.43777), except the
fixed-reference sweeps of `simple_sweep_1` (IfcOpenShell ignores FixedReference: the
same result for any direction) and sweep ends (ours are cut square to the last chord,
up to 1 cm off). Map origins match `ifcopenshell.util.geolocation` (map units, Scale,
no map unit read as the project unit). Every import passes `Checks::TOPOLOGY`; the
127 sample files give the same results as before except where new classes are read.
Exports of SampleHouse, Duplex, Revit ARC, advanced_model and ISSUE_171 pass
`ifcopenshell.validate`; every element of the original (by GlobalId) is in the
export with the same world bounding box (IfcOpenShell on both, 1 cm), except the
Revit ARC site's topography (exported as a proxy) and ISSUE_171's structural members
(topology representations, not imported).

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
PLY, 3MF, VRML, KMZ, 3DS and USDZ, re-imports OBJ/STL/FBX/PLY/3MF/3DS and compares area, bounding box,
materials and textures (`MERGE=0` keeps the triangles). Exports were also read with the
assimp 5.3 command line tool (`assimp info`) as an independent reader.
