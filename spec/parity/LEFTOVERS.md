# Leftovers from finished FEATURES.md items

Sub-items noted as "Left" under finished items, ranked by usefulness. Work down this list after the open FEATURES.md items. S/M/L effort; the FEATURES.md item number is in brackets.

## High
1. [ ] Save the model-only data in the .skp (with the trial's sample files, see 39): scenes, model axes, locks, two-point, fog, snaps, advanced attributes and license, shadow flags, polygon flags, leader and dimension styles, environments, tag folders and dashes, extra styles. L [39]
2. [x] Rare kernel failure: after a vertex move an edge can graze another within TOL/2 (`move_leaves_an_edge_grazing_another`, ignored test). M [71] Done: `ops::chain_between` splits a crossing near the line's end when the crossed edge passes more than TOL from that end; the test runs again, plus a second seed from a long run; 24 x 20,000 sequences pass.
3. [x] glTF round trip drops tiny triangles (11 of 59 test files, up to 0.02% area): keep float64 positions on export or weld more carefully on import. S [64] Done: export writes a `_POSITION_LO` attribute (the f32 rounding) that import adds back, so every exact difference is gone. What is left is not glTF: those files import with invalid faces (crossing edges, two-edge hole loops, `formats_check` now prints `INVALID`), whose triangulation drops area in every export format alike; triangle counts also differ by zero-area triangles. Better welding, sliver faces and T-junction splitting on mesh import were tried and made the other formats worse (38 to 48+ differences), so mesh import healing is its own task.
4. [x] Divide: typed segment length; Divide on free-form (welded / Freehand) curves. S [51] Done (`curves::count_for_length`, `divide_length`; Divide tool reads lengths).
5. [ ] Inference: Arc Side and Center and Circle/Polygon Center hints, Intersection with Hidden Section, Shift plane lock for Rotated Rectangle and 3 Point Arc. S [70] Done: Shift plane lock for Rotated Rectangle and 3 Point Arc; polygons are flagged arc curves, so their center is inferred. Left: Arc Side and Center, Intersection with Hidden Section (what they show is not in our help text: trial).
6. [x] Components: Cut Opening applied to the glued face, Always Face Camera drawn, Shadows Face Sun used. M [60] Done (`mt_kernel::behavior`, app `component_behavior.rs`): at scene build, glued Cut-opening instances cut the outermost loops of their z = 0 edges out of coplanar faces under them (a scratch intersection, so doors at a wall's foot notch the face; the model keeps its faces); face-camera instances turn about their blue axis so -Y points at the eye (or back along the view in parallel projection), the scene being rebuilt when the view moves (not during a live tool preview); with Shadows face sun the shadow pass draws a copy of the faces with those instances turned toward the sun (`Viewport::set_shadow_casters`). Left: exports, section cuts/fills and Entity Info area ignore openings; the cut is applied to every coplanar face under the outline, not only the one glued to; a face-camera rebuild per view move is slow in big models (a per-instance GPU transform would avoid it); scale/move grips on face-camera instances use the placed pose; which side faces the camera and whether hidden instances still cut are unverified (trial).

## Medium
7. [x] Sandbox tools: Smoove, Stamp, Drape, Add Detail, Flip Edge (and their toolbar buttons). M [29, 40] Done: `mt_kernel::sandbox` (tests validate every op) and `crates/mt-app/src/sandbox_tools.rs`; Tools > Sandbox menu, five Sandbox toolbar buttons with icons, Instructor pages, one undo step each. Left: Smoove and Add Detail only in the open context (open the terrain group first; Stamp and Drape work on a group from outside); Add Detail's Shift (horizontal move) and drag on a vertex; Stamp's preview is the object's box, not its footprint, and stamps the footprint's outer loop only; Flip Edge has no hover highlight; Smoove falloff shape, default radius (10' / 3 m) and offset (1') from memory: trial.
8. [ ] Import: DXF blocks / INSERT, text, dimensions, hatches, splines, ellipses; glTF node hierarchy and PBR beyond base color; FBX node hierarchy as groups. M [64, 72]
9. [ ] Export: 3DS, USDZ, "Export only current selection", STL ASCII, PLY UVs. M [72]
10. [ ] Panels: Materials collections / sample / thumbnail style; Components collections and Statistics tab; Styles collections, Mix tab, .style files; Tags multi-select, sort, expand/collapse all, Apply Tags to Scenes, Show New Tags, Color by Tag for text and dimensions; Outliner sort / Expand All / section plane rows; Scenes thumbnails and tab search. M [60]
11. [ ] Model Info: font families, Dimensions expert settings, Geo-location map import. M [61]
12. [ ] Model Library: Sketchfab OAuth sign-in, cache size limit, thumbnail queue follows the filter, recent downloads. S-M [68]
13. [ ] Styles: watermark Create Mask and reordering, hidden-geometry and inference colors on the Modeling page. S [65]
14. [ ] Search button on the Getting Started toolbar (icon). S [63]

## Low
15. [ ] IFC import/export (Classifier is out of scope; geometry-only IFC could still help). L [72]
16. [ ] Smithsonian Open Access source (needs an api.data.gov key), library collections and favorites. S [68]

## Needs the trial first
- Exact cursors, toolbar order, dialog looks, inference lock release rules, on-screen widgets (41, 40, 42, 70), and every `unverified` row in STATUS.md.
