# Leftovers from finished FEATURES.md items

Sub-items noted as "Left" under finished items, ranked by usefulness. Work down this list after the open FEATURES.md items. S/M/L effort; the FEATURES.md item number is in brackets.

## High
1. [ ] Save the model-only data in the .skp (with the trial's sample files, see 39): scenes, model axes, locks, two-point, fog, snaps, advanced attributes and license, shadow flags, polygon flags, leader and dimension styles, environments, tag folders and dashes, extra styles. L [39]
2. [ ] Rare kernel failure: after a vertex move an edge can graze another within TOL/2 (`move_leaves_an_edge_grazing_another`, ignored test). M [71]
3. [ ] glTF round trip drops tiny triangles (11 of 59 test files, up to 0.02% area): keep float64 positions on export or weld more carefully on import. S [64]
4. [ ] Divide: typed segment length; Divide on free-form (welded / Freehand) curves. S [51]
5. [ ] Inference: Arc Side and Center and Circle/Polygon Center hints, Intersection with Hidden Section, Shift plane lock for Rotated Rectangle and 3 Point Arc. S [70]
6. [ ] Components: Cut Opening applied to the glued face, Always Face Camera drawn, Shadows Face Sun used. M [60]

## Medium
7. [ ] Sandbox tools: Smoove, Stamp, Drape, Add Detail, Flip Edge (and their toolbar buttons). M [29, 40]
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
