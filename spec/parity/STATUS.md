# Parity status

`missing` = not implemented, `differs` = implemented but not matching SketchUp, `matches` = verified against SketchUp 2026, `unverified` = implemented from memory, not yet compared.

Nothing has been compared with real SketchUp 2026 yet (the trial is still to come), so no row is `matches`. `differs` marks known differences; everything else implemented is `unverified`. Code comments marked "unverified" are summarized in the notes.

| Area | Item | Status | Notes |
|---|---|---|---|
| Shell | Menu bar structure | unverified | `menus.toml` draft; only commands meshtard implements are enabled, the rest are grey |
| Shell | Default shortcuts | unverified | `keymap.toml` draft; every shortcut of an implemented command is active (tools, file, edit, camera, G, Ctrl+T) |
| Shell | Toolbars, tray, status bar, Measurements box layout | unverified | `mt-ui` crate; screenshot `meshtard/m3_shell.png`; docking order (toolbars outside tray) a guess |
| Shell | Default toolbars (Getting Started top, Large Tool Set left) | unverified | `toolbars.toml` draft; which toolbars are visible on first start is unsure |
| Shell | Dockable toolbars | unverified | drag by grip to any edge or float; View > Toolbars... and right-click list; layout saved between runs. Rows always packed and wrap (SketchUp may keep gaps or clip); drag shows an outline |
| Shell | Toolbar flyouts (Arcs, Shapes on Getting Started) | differs | opened by clicking the small corner triangle; SketchUp behavior not checked |
| Shell | Toolbar tooltips | unverified | shows "Name (Shortcut)"; SketchUp's exact text not checked |
| Shell | Toolbar icons | differs | own procedural drawings by design (Trimble icons are copyrighted) |
| Shell | Window theme (colors, sizes) | unverified | `ui_theme.toml` draft (Windows 11 light) |
| Shell | Measurements box typing from the viewport | unverified | digit or `.,;-+[<'"/*@=` starts typing, then all text until Enter/Escape |
| Shell | Status bar hints per tool | unverified | wording from memory |
| Shell | Splash screen | differs | drawn inside the main window and fades out after at least 1.2 s; SketchUp uses a separate borderless window |
| Shell | App icon | differs | own design by intent (window and taskbar icon) |
| Shell | Window title and unsaved-changes asterisk | unverified | `*name - meshtard`; SketchUp's Windows convention not checked |
| Tray | Default Tray panels, order, expanded state, width | unverified | `tray.toml` draft; Outliner's presence in a fresh tray unsure |
| Tray | Entity Info | unverified | per entity type rows (length, area, radius, segments, tag dropdown); layout a guess |
| Tray | Tags | unverified | add, delete, rename, visibility, color, active tag pencil; deleting moves entities to Untagged (SketchUp asks); new tag colors a guess |
| Tray | Materials | differs | In Model swatch grid, Create Material, edit color and opacity; egui color picker instead of SketchUp's four pickers; no texture controls; new material color a guess |
| Tray | Components | differs | In Model list with icons and instance counts, place, edit name and description, Purge Unused; no thumbnail grid, Edit tab, Statistics tab or 3D Warehouse |
| Tray | Outliner | unverified | tree, selection, filter; icons and root label a guess |
| Tray | Scenes | differs | Update/Add/Delete, list, click activates; no thumbnails, name/description fields, move up/down or per-scene property toggles |
| Tray | Styles | differs | Edit tab only: edges, back edges, profiles, depth cue and extension with widths, face style, X-ray, section cut width; text instead of icon buttons |
| Tray | Fog | unverified | hidden until Window > Fog; Display Fog, Start and End sliders (percent), Use Background Color and a color; SketchUp has one two-handled Distance slider |
| Tray | Shadows | differs | show, time, date, light, dark, on faces, on ground; no UTC offset, "Use sun for shading" or "From edges"; time slider spans the whole day |
| Tray | Environments, Instructor | missing | listed in `tray.toml` but empty |
| View | Sky and ground | unverified | colors are placeholders |
| View | Axes (solid positive, dotted negative) | unverified | View > Axes toggles them (model setting, not saved in the .skp: the style field is not known) |
| View | Edge style (color, width) | unverified | black, 1 px, soft and hidden edges skipped |
| View | Face shading | unverified | `[shading]` draft: flat diffuse, camera light, two-sided |
| View | Profiles (width 2, borders and silhouettes) | unverified | |
| View | Edge Style toggles: Edges, Back Edges, Profiles, Depth Cue, Extension | unverified | View > Edge Style with check marks; Edges off keeps profiles; back edges dashed 3 px and 1 px wide; depth cue 4 px at the model's near side down to 1 px at its far side, per edge (midpoint); extension 2 px at both ends. Defaults (off, 4, 2) from memory of SketchUp's rendering options; model-only (not read from or written to .skp); not undo steps; hidden edges with Edges off can still be picked |
| View | Fog | unverified | linear in view depth from start to end, given as fractions of the model's depth range (SketchUp's distance scale unknown); towards the sky/ground/background behind each pixel, or a color; faces and model edges (not guides or axes); defaults (off, 0 to 100%, background color) a guess; model-only |
| View | Selection highlight (blue edges, dotted faces, component box) | unverified | box follows component axes |
| View | Face styles: Wireframe, Hidden Line, Shaded, Shaded With Textures, Monochrome | unverified | profiles hidden in Wireframe; material opacity kept in Hidden Line and Monochrome |
| View | X-ray | unverified | faces at 50% opacity, amount a guess |
| View | Textures | unverified | drawn in Shaded With Textures with their imported UV mapping; colorize from memory |
| View | Shadows (sun position, shadow map, light/dark) | unverified | NOAA sun; default location (Boulder) and time (Nov 8, 13:30) from memory; year not stored; toggles not undoable |
| View | Translucent materials | unverified | sorted back to front |
| View | Scene tabs | unverified | click activates; right-click Update/Add/Delete; no Move Left/Right; tab look a guess; View > Scene Tabs shows/hides them (app setting, not saved) |
| View | Guides toggle | unverified | View > Guides hides guides and stops inferring from them (model setting, not saved in the .skp) |
| View | Hidden Geometry | unverified | hidden and soft edges dashed gray, hidden faces translucent with a dotted triangle mesh, hidden groups/components, section planes, dimensions and texts shown and selectable (SketchUp 2020+ splits groups etc. into View > Hidden Objects, not in our menu); colors, dash lengths and alpha from memory; not saved in the .skp |
| View | Scene transitions | unverified | camera animates over the View > Animation > Settings time (2 s default; 0 or transitions off jumps); path a guess; tag visibility switches at once (SketchUp fades) |
| Camera | Default eye, target, FOV | unverified | |
| Camera | Orbit (middle drag, O) | unverified | scenario 00 |
| Camera | Pan (shift+middle, H) | unverified | scenario 00 |
| Camera | Zoom (wheel at cursor, Z) | unverified | scenario 00 |
| Camera | Zoom Extents (Shift+Z), Standard Views, Parallel Projection / Perspective, Previous / Next | unverified | fit to bounding sphere; views animate and fit; Iso from the camera's quadrant; scenario 00 steps 11-14 |
| Camera | Zoom Window | unverified | drag a box (solid either way); centers on it at the target depth and fills the view; tool stays active |
| Camera | Position Camera | unverified | click: stand 5' 6" (typed eye height before clicking) above the point under the cursor (model, else ground plane), looking level the way the camera faced; drag: stand above the press point looking at the release point; then Look Around is active; one history step |
| Camera | Walk | unverified | hold the mouse: speed grows with the distance from the press point (0.5"/s per px, Ctrl runs 3x), up/down walks, left/right turns (0.3 deg/s per px), Shift moves up/down and sideways; no collision; typed eye height above the last positioned ground (0 before); one history step per drag |
| Camera | Look Around | unverified | drag turns the view about the eye (0.2 deg/px, right turns right, up looks up); typed eye height; history like Orbit |
| Camera | Field of View | unverified | drag up narrows (0.1 deg/px, 1-120 deg); typed `35`/`35deg` degrees or `50mm` focal length (35 mm film, 24 mm frame height, vertical FOV); in parallel projection a typed length sets the view height; label "Field of View" / "Height" |
| Camera | Two-Point Perspective | unverified | view looks horizontally with a vertical lens shift keeping the target centered, so verticals stay vertical; orbiting and standard views switch back to Perspective; Parallel / Perspective / Two-Point checked as a radio group; not saved in scenes or the .skp |
| Window | Model Info | unverified | modeless window, pages Animation, Credits, Dimensions, File, Geo-location, Statistics, Text, Units. Units: Architectural / Decimal / Engineering / Fractional, unit, precision (decimals or 1"-1/64"), length snapping, angle precision and snapping; each change one undo step; lengths shown with "~ " when rounded; not saved in the .skp (the writer keeps its template's units); snapping values stored but not used by inference; angle precision not yet applied to angle displays. Statistics: whole-model counts (definitions' contents once) and Purge Unused (component definitions, then materials and tags no entity uses; one undo step). Dimensions/Text: font controls shown disabled. Geo-location read only. File: name, folder, size, version from the header |
| Window | Preferences | unverified | pages Drawing (click style shown, disabled: tools are click-move-click), General (auto-save every N minutes, default on/5 min, to `<name>~autosave.skp` next to a saved model, removed on save; backup on save as `<name>.skb`), Shortcuts (all menu commands, rebind by pressing keys, Remove, Reset All; a taken shortcut moves; saved as differences to `shortcuts.toml` in the config folder and applied at startup). No Graphics, Compatibility, Files, Templates pages |
| Shell | Window menu opens tray panels, Show Tray | unverified | Environments, Instructor, Fog, Soften Edges still grey |
| Select | Click rules (Ctrl add, Shift toggle, Ctrl+Shift remove) | unverified | |
| Select | Double click face/edge, triple click connected | unverified | |
| Select | Window (left to right) vs crossing (right to left) | unverified | solid vs dashed box |
| Select | Edge pick aperture 5 px | unverified | |
| Select | One click on a curve segment selects the curve | unverified | |
| Select | Select All, Select None, Delete | unverified | |
| Select | Invert Selection (Ctrl+Shift+I) | unverified | everything visible and selectable in the open context minus the selection (hidden entities count when Hidden Geometry is on) |
| Edit | Hide, Unhide Selected / Last / All | unverified | undoable; works on the open context's top-level entities (edges, faces, groups, components, section planes, dimensions, texts); hidden entities leave the selection unless Hidden Geometry is on; Last = the set of the last Hide, in the context it was hidden in, used once, kept outside undo; Unhide All does not reach inside groups (from memory) |
| Edit | Lock, Unlock Selected / All | unverified | `locked` on groups and component instances, undoable; locked: selectable, highlighted red (edges and box), not moved/rotated/scaled/painted/push-pulled by tools, not erased, deleted, exploded or opened; Make Unique and Create Group still allowed; Unlock All covers every instance in the model; Entity Info shows a Locked row (no toggle); not read from or saved to the .skp (the flag's place in the format is unknown) |
| Geometry | Merge tolerance 0.001" | unverified | `mt_kernel::TOL` |
| Geometry | Lines split each other where they cross | unverified | |
| Geometry | Closed planar loop creates a face | unverified | |
| Geometry | Face on the ground plane faces down | unverified | other new faces: match a neighbor, else largest normal component positive |
| Geometry | Line across a face splits it | unverified | |
| Geometry | Loop inside a face makes a hole and an inner face | unverified | |
| Geometry | Erasing a shared edge merges coplanar faces | unverified | |
| Geometry | Erasing a splitting edge heals the split line | unverified | collinear edges healed into one |
| Geometry | Curves (arcs, circles, freehand) act as one entity | unverified | select, erase, Entity Info |
| Inference | Point kinds, priorities, colors, tooltip names | unverified | endpoint green, midpoint cyan, on edge red, on face blue; center and intersection marker colors a guess |
| Inference | Center, intersection, guide point | unverified | intersection drawn as a cross |
| Inference | Directions: axes, parallel, perpendicular, from point | unverified | 1 s hover delay before acquiring a point |
| Inference | Shape hints: square, golden section, half circle, tangent | unverified | 2.5 degree tolerance and dashed diagonal color a guess |
| Inference | Pick distances (10 px points, 6 px edges, 10 px directions) | unverified | |
| Inference | Arrow keys and Shift locking | unverified | right red, left green, up blue, down parallel |
| Inference | Inside an open group or component | missing | geometry inside the edited component is not inferred |
| Measurements | Length entry formats | unverified | mm, cm, m, ", ', feet and inches; model units by default |
| Measurements | Typed values per tool | unverified | `w,h`, radius, `24s`, bulge, angle, distance, factor, `5x`, `/5`; see each tool |
| Draw | Line | unverified | chain continues until a face closes; Esc cancels |
| Draw | Freehand | unverified | |
| Draw | Rectangle | unverified | typed `w,h` |
| Draw | Rotated Rectangle | unverified | |
| Draw | Circle (24 segments) | unverified | typed radius, `Ns` |
| Draw | Polygon (6 sides) | unverified | typed radius, `Ns` |
| Draw | Arc (center), 2 Point Arc, 3 Point Arc, Pie (12 segments) | unverified | typed bulge, radius, angle |
| Draw | Shape plane: clicked face, else ground; arrow keys lock | unverified | on tilted faces sides follow the level line and slope; skewed slopes from memory |
| Modify | Eraser | unverified | click or drag with preview; whole curve from one segment; Shift hides, Ctrl softens, Ctrl+Shift unsoftens |
| Modify | Push/Pull | unverified | Ctrl keeps the start face; pushing through cuts a hole; double-click repeats last distance; Ctrl toggles rather than held |
| Modify | Move | differs | Ctrl copy, `5x` and `/5` arrays, hover-select; geometry drawn live but connected geometry stretches only on click |
| Modify | Rotate | unverified | center, start, end; typed angle; Ctrl copy; protractor plane orientation not SketchUp's |
| Modify | Scale | differs | grip box with corner, edge and face grips, typed factors and sizes; Ctrl toggles instead of held; no Shift swap; box not yellow |
| Modify | Offset | differs | typed distance, arcs stay arcs, partial collapses trimmed; corners always mitered; trimmed arc ends become chord pieces |
| Modify | Follow Me | unverified | select path, click profile; soft joints under 30 degrees |
| Modify | Paint Bucket | differs | paints clicked side, group or component, or the selection when clicking a selected entity; no Alt sample, no Ctrl/Shift fills |
| Modify | Intersect Faces with Model, with Selection | unverified | With Context missing |
| Construction | Tape Measure | unverified | parallel guide from an edge, guide point otherwise; guides dashed gray, infinite |
| Construction | Protractor | differs | angle or slope guides; drawn at click radius (SketchUp: fixed screen size); no 15 degree snap; no Ctrl measure-only |
| Construction | Delete Guides | unverified | Edit menu |
| Construction | Dimensions (linear, radius, diameter) | unverified | offset plane, extension overshoot and label format a guess; closed arrows |
| Construction | Text (leader and screen text) | unverified | default text: length, area or coordinates; on components shows coordinates, not the name; formats a guess |
| Groups | Create Group, Make Component | unverified | origin at selection's bounding-box corner, axes parallel to parent |
| Groups | Make Component dialog | differs | name, description, Glue to, Set Component Axes; no Cut opening, Always face camera, Shadows face sun, Replace selection |
| Groups | Component axes | unverified | picked from the dialog; selection box follows them |
| Groups | Gluing | unverified | glued components lie on matching faces; orientation on the face a guess |
| Groups | Placing components | differs | click places at a point; no cursor rotation while placing |
| Groups | Edit in place | unverified | double click; rest faded (amount a guess) and not selectable; Esc, click outside (also on empty space) or Edit > Close closes |
| Groups | Explode | differs | welds vertices and overlapping edges only; no splitting where edges cross |
| Groups | Make Unique | unverified | |
| Section | Section Plane tool | differs | on clicked face or horizontal; no name/symbol dialog; no arrow-key locks |
| Section | Planes per context, select, move, delete; double-click toggles active cut | unverified | plane size from model extent; outline colors a guess |
| Section | Section cut lines and View > Section Planes / Section Cuts | unverified | arrows drawn on the kept side |
| Section | Section fill | unverified | any closed loop filled; dark gray fill color a guess |
| Edit | Undo/Redo | unverified | snapshot per edit, limit 100; covers model and edit context; shadow toggles not recorded |
| Edit | Cut, Copy, Paste, Paste In Place | unverified | internal clipboard (selected edges, faces with their edges, components by definition, dimensions, leader texts; world coordinates); the system clipboard only gets a placeholder text. Paste: ghost follows the cursor held at the bounding box minimum corner (SketchUp grab point unverified), click places and merges like drawing, Esc cancels, then back to Select (unverified); pasted entities selected; Cut is one undo step. Component instances are dropped from the clipboard when another model is opened |
| File | Open `.skp` (2013-2026, VFF and legacy) | unverified | geometry, components, tags, materials, textures, scenes, styles, guides, dimensions; VFF texts and edge tags not read |
| File | Opens at the saved view | unverified | |
| File | Command-line path and dropped `.skp` open | unverified | |
| File | New, Save, Save As, Save A Copy As | differs | writes SketchUp 2017 format; SketchUp 2026 writes VFF |
| File | Round trip | differs | not saved: camera view (opens in Iso), scenes, styles, shadows, units, active section, edge tags, guides and section planes inside components, group/component locks, View > Axes / Guides / Hidden Geometry; arcs reopen as polylines (hidden flags of edges, faces and instances are saved) |
| File | "Save changes?" prompt | unverified | on New, Open, dropped file, close; Yes/No/Cancel, wording a guess |
| File | Saved files open in real SketchUp 2026 | unverified | not tried yet |
| File | Export 2D Graphic (PNG, JPEG) | unverified | options dialog first (Use view size, width/height with the view's aspect, Anti-alias, Transparent background for PNG), then the save dialog; offscreen render; anti-alias = 2x supersampling over the view's 4x MSAA; line widths stay in pixels; selection not drawn; dimension and text labels missing; transparent leaves out sky/ground (and ground shadows); JPEG quality 90. Defaults (view size, anti-alias on, transparent off) from memory |
| File | Export 3D Model: OBJ | unverified | triangles only (no polygon option), flattened groups/components, hidden and hidden-tag geometry skipped, edges not exported; MTL Kd/d, textures written next to the .obj as `<name>_<material>.<ext>` (stored image; colorized textures not recolored); faces without material use `FrontColor`/`BackColor` (names from memory); units choice (default model units); "Swap YZ" (Y up) on and two-sided off by default (from memory) |
| File | Export 3D Model: STL | unverified | binary only, front faces, units choice (default model units), Z up; no ASCII option |
| File | Export 3D Model: COLLADA | unverified | COLLADA 1.4.1, one flattened mesh with a `<triangles>` block per material, inches (`<unit meter="0.0254">`), Z up, lambert colors/textures/opacity, two-sided on by default (from memory); no node hierarchy or component instancing like SketchUp's exporter; no glTF |
| File | Import `.skp` | unverified | the file becomes one component definition (its components come along; materials and tags matched by name) placed with the Place Component tool; scenes and styles of the imported file dropped; name clash handling a guess |
| File | Import OBJ, STL | unverified | options: units (defaults STL mm, OBJ m: guesses), Y up (OBJ default), Merge coplanar faces (on); one group at the context origin, one undo step, selected; vertices welded within tolerance; MTL colors/opacity become materials; OBJ texture maps, normals, UVs and groups ignored |
| File | Import images | missing | needs an image entity (FEATURES.md item 30) |
| File | `MESHTARD_TEST_DIALOG` | differs | test hook: a file whose first line answers every file dialog (GUI checks without a desktop portal) |
| View | View > Animation | unverified | Add/Update/Delete Scene act like the Scenes panel (Update without SketchUp's property dialog); Previous/Next (PageUp/PageDown, from memory) wrap; Play loops through the scenes (transition, then scene delay) until Escape or any click; no Pause/Stop window. Settings dialog: Enable scene transitions, transition time, scene delay; defaults (on, 2 s, 1 s) from memory; app settings, not saved in the model |
| Platform | Linux (X11, Wayland) | unverified | main dev platform |
| Platform | Windows build | unverified | CI-built, not yet run by hand |
| Platform | macOS build | unverified | CI-built, not yet run by hand |
| Platform | macOS native menu bar, Cmd shortcuts, Mac symbols | differs | Windows menu layout plus minimal app menu; SketchUp for Mac's layout differs |
| Platform | HiDPI on Windows and macOS | unverified | not checked |
| Performance | CUDA acceleration | unverified | pick index (LBVH) and ray batches on the GPU from 20k triangles with `--features cuda`; CPU fallback without driver or NVRTC; threshold being measured |

## Grey menu items

Menu items that are shown but not yet implemented are listed in `spec/parity/FEATURES.md`.
