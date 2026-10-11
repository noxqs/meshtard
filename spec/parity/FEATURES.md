# Grey menu items: ranked plan

99 menu commands are greyed out (not in `IMPLEMENTED` in `crates/mt-app/src/main.rs`). They are ranked by how often a modeler needs them, then by how much other work they unblock. Work goes top down; each item is ticked when it is enabled, tested and GUI-checked.

Effort: S = under a day, M = a few days, L = a week or more. "Trial" = what to note in SketchUp 2026 before or while building it (see `TRIAL_NOTES.md`).

## Tier 1: everyday modeling (missing these is felt in every session)

1. [x] **Zoom Extents** (`camera.zoom_extents`, Shift+Z) S.
   `Document::extents_camera` already fits the model on open. Expose it as a command keeping the current view direction and projection; include hidden-geometry rules (hidden entities ignored). Trial: margin around the model; does it count guides and section planes?
2. [x] **Standard views and Iso** (`camera.view_*`) S.
   Set eye direction (Top +Z down, Front looks +Y, etc.), then Zoom Extents. Iso direction from SketchUp (trial). Camera up vector for Top/Bottom.
3. [x] **Parallel Projection / Perspective** (`camera.parallel`, `camera.perspective`) S.
   `Camera::perspective` already exists and renders; add the commands and check marks. Make sure picking and inference use the orthographic ray (`Camera::ray`).
4. [x] **Previous / Next camera** (`camera.previous`, `camera.next`) S.
   A camera history in `App` (push on each finished orbit/pan/zoom/view command, cap ~50). Trial: is a single wheel step one entry?
5. [x] **Hide / Unhide (Selected, Last, All)** (`edit.hide`, `edit.unhide_*`) S.
   `hidden` flags exist on edges, faces, instances, annotations and the renderer skips them. Add the commands with undo; "Last" needs the last hidden set kept in the document. Depends on item 8 (Hidden Geometry) to select hidden things for Unhide Selected.
6. [x] **Cut / Copy / Paste / Paste In Place** (`edit.cut|copy|paste|paste_in_place`) M.
   Clipboard = a detached `Entities` (selection copied with its connected edges, components by definition). Paste starts a Move-like placement with the cursor (reuse `modify.rs` Move preview); Paste In Place inserts at the original coordinates in the active context. Internal clipboard first; system clipboard later. Trial: paste grab point (bounding box corner?).
7. [x] **Invert Selection** (`edit.invert_selection`, Ctrl+Shift+I) S.
   Everything visible in the active context minus the selection.
8. [x] **View toggles: Axes, Guides, Hidden Geometry, Scene Tabs** (`view.axes|guides|hidden_geometry|scene_tabs`) S.
   Flags in `StyleSettings` (axes/guides per model, hidden geometry draws hidden edges dashed and faces as a mesh). Save in the file where the format has it. Check marks via `view_checked`.
9. [x] **Edge Style toggles: Edges, Back Edges, Profiles, Depth Cue, Extension** (`view.edges` …) M.
   `profiles` exists in `StyleSettings`; renderer needs: edges off, back edges dashed through faces (second pass without depth test, dashed shader), profile width, depth cue (width by distance), extension (lines overshoot). Trial: default widths and dash pattern.
10. [x] **Window menu opens tray panels** (`window.entity_info|components|materials|styles|tags|scenes|shadows|outliner|soften_edges|fog|environments|instructor`, `window.tray_show`) S.
    Shell API to show the tray and expand a panel; panels not built yet (Fog, Soften Edges, Environments, Instructor) get stubs or stay grey until built. Done for the built panels and Show Tray (check mark); the others stay grey.
11. [x] **Zoom Window** (`camera.zoom_window`, Ctrl+Shift+W) S.
    Drag a box; fit the camera to it along the current direction.
12. [x] **Lock / Unlock** (`edit.lock`, `edit.unlock_*`) S.
    `locked` flag on groups and component instances (read and write it in the file); locked instances can be selected but not edited or moved; drawn with red selection box. Trial: what is blocked exactly.

## Tier 2: modeling power and file exchange

13. [x] **Solid Tools: Outer Shell, Union, Subtract, Intersect, Trim, Split** (`tool.outer_shell`, `tool.solid_*`) L.
    Needs a "is solid" test (closed manifold, already shown in Entity Info?) and a robust mesh boolean in mt-kernel built on `intersect.rs` (intersect faces, classify each face inside/outside the other solid by ray parity, keep/discard, merge coplanar). CUDA candidate for classification on large models. Trial: result naming, which input is kept (Subtract/Trim), what happens to materials. Done: kernel in `mt_kernel::solid`, tools in `crates/mt-app/src/solid_tools.rs` and `pick_tool.rs` (click first and second solid; Outer Shell also on a selection); results are new groups; Entity Info shows Solid Group/Component and Volume.
14. [x] **Export 2D Graphic (PNG/JPG)** (`file.export_2d`) S.
    Render the viewport offscreen at a chosen size (wgpu texture readback), save with `image`. Options dialog: size, transparent background, anti-alias. Done (`crates/mt-app/src/export.rs`, `Viewport::render_image`); dimension/text labels (egui overlay) are not in the image.
15. [x] **Export 3D Model (OBJ, STL, glTF/DAE)** (`file.export_3d`) M.
    Triangulate with `tessellate.rs`; OBJ+MTL with textures first, STL for printing, then COLLADA (SketchUp's main exchange format). Trial: export options per format. Done in `mt_kernel::mesh_export` (OBJ+MTL+textures, binary STL, COLLADA 1.4.1); glTF not done; edges, guides and annotations are not exported; colorized textures are written as the stored image.
16. [x] **Import (OBJ, STL, images, .skp as component)** (`file.import`) M.
    .skp import as a component definition (reader exists); OBJ/STL to faces with merge; images as image entities or textures (needs image entity, see 30). Done for .skp (Place Component), OBJ (MTL colors, no texture maps) and STL (`mt_kernel::mesh_import`, `crates/mt-app/src/import.rs`). PNG/JPEG images are imported as image entities (item 30).
17. [x] **Scenes from the View > Animation menu** (`view.anim_*`) S.
    Scenes and transitions exist in the tray; add Add/Update/Delete/Previous/Next/Play and the Settings dialog (transition time, scene delay). Trial: Update Scene's property dialog. Done (`crates/mt-app/src/animation.rs`); Update Scene updates at once (no property dialog); settings are not saved in the file.
18. [x] **Tag tool and Window > Tags shortcuts** (`tool.tag`) S.
    Click to move entities to the active tag (SketchUp 2026 Tag Tool: Alt to sample, Ctrl to keep selection). Trial: its exact keys. Done (`pick_tool.rs`): click tags, Alt+click samples; Ctrl not done.
19. [x] **Axes tool** (`tool.axes`) S.
    Reuse `AxesPick` from the Make Component dialog; store model axes in the file (model-level transformation) and drive drawing directions/inference from it. Done (`model_axes.rs`, `mt_infer::Axes`): axes drawn and inferred there; not saved in the file (mt-skp does not expose model axes); shape tools' default planes stay world-aligned.
20. [x] **Position Camera, Walk, Look Around, Field of View** (`camera.position|walk|look_around|field_of_view`) M.
    Eye height 5' 6" (verify), walking with collision off, look around rotates about the eye; Field of View is a drag tool plus typed degrees (`35deg`) or focal length (`50mm`).
21. [x] **Model Info** (`window.model_info`) M.
    Dialog pages: Units (format, precision, snapping), Dimensions, Text, Statistics (purge unused), Geo-location (read only), Credits, File. Units already exist in the model; wire to the dialog. Trial: screenshots of every page.
22. [x] **Preferences** (`window.preferences`) M.
    Shortcuts page (rebind and save to user config like `toolbars.toml`), Drawing (click-drag-click), Graphics, General (auto-save, backup). Trial: shortcut export file.
23. [x] **Revert** (`file.revert`) S.
    Reopen the file from disk after a confirm prompt (reuse `confirm.rs`).
24. [x] **Hide Rest of Model / Hide Similar Components** (`view.hide_rest_of_model|hide_similar_components`) S.
    Edit context exists; renderer fades or hides everything outside the context path.
25. [x] **Intersect With Context** (`edit.intersect_context`) S.
    Same as Intersect With Selection limited to the active context; `intersect.rs` already has the core.
26. [x] **Fog** (`view.fog`, `window.fog`) S.
    Fog start/end distance and color in the style; a fragment shader term. Trial: default distances.
27. [x] **Soften Edges panel** (`window.soften_edges`) S.
    Angle slider; soften/smooth selected edges whose face angle is under it (Eraser Ctrl already softens single edges). Done (`soften.rs`, Soften Edges tray panel).
28. [x] **Two-Point Perspective** (`camera.two_point`) M.
    Keep vertical lines vertical: shifted-lens projection; orbit switches it off.

## Tier 3: presentation and less frequent work

29. [x] **Sandbox From Scratch / From Contours** (`tool.sandbox_*`) M. Grid of triangles; contours via triangulation of curve points (Delaunay). Also Smoove/Stamp/Drape tools later. Done (`crates/mt-app/src/sandbox.rs`, `mt_kernel::terrain`, `mt_kernel::delaunay`): grid spacing typed first (default 10' / 3 m, unverified), three clicks; From Contours triangulates the selected edges' vertices. Smoove, Stamp, Drape, Add Detail, Flip Edge not done.
30. [ ] **Images and Match New Photo** (`camera.match_new_photo`) L. Needs an image entity type (read/write in the file), then the match-photo vanishing-point UI. Part 1 done (`crates/mt-app/src/image_entity.rs`): image entities from File > Import of PNG/JPEG, placed with two clicks, textured, Explode to a textured face; read from .skp files but saved as groups with their textured face (the writer cannot mix `add_image` with definitions). Match New Photo is still grey.
31. [x] **3D Text** (`tool.text_3d`) M. Font outlines (ttf-parser) to faces, extrude; dialog with font, height, extrude, filled. Done (`crates/mt-app/src/text3d.rs`, `mt_kernel::text3d`): bundled fonts Ubuntu Light (Ubuntu Font Licence 1.0) and Hack (MIT / Bitstream Vera) from `epaint_default_fonts`, plus system fonts; the result is a component placed with Place Component.
32. [x] **Export Section Slice (DXF/DWG)** (`file.export_section`) M. Section cut lines to DXF (DWG is proprietary: DXF only).
33. [x] **Export Animation (MP4/image sequence)** (`file.export_animation`) M. Offscreen render each frame of scene transitions; image sequence first, video via an encoder later.
34. [x] **Print / Print Preview / Print Setup** (`file.print*`) M. Offscreen render to PDF (printpdf) or the OS print dialog. Trial: scale options.
35. [x] **Templates: New from Template, Save As Template** (`file.new_from_template`, `file.save_as_template`) S. Templates are .skp files in the user config folder; New uses the chosen one.
36. [x] **Generate Report** (`file.generate_report`) M. Attribute/component report to CSV/HTML.
37. [x] **Manage Trays** (`window.manage_trays`) S. Create trays and choose their panels; save like the toolbar layout.
38. [x] **About** (`help.about`) S. Version, license, OpenSKP credit, splash image.

## Writer gaps (found while building M10-M13)

39. [ ] **Save everything meshtard keeps in the model** L. The writer (SketchUp 2017 format, OpenSKP port) dropped or reset: scenes and their settings, styles (edge style options, fog, face style), section plane positions (came back at the origin), model axes, units and snapping, lock flags, Axes/Guides/Hidden Geometry toggles, two-point, image entities (saved as groups). Each needs the format's field found (OpenSKP sources, files saved by SketchUp 2026 in the trial) and a round-trip test in `crates/mt-kernel/tests/export.rs`.
    - Done (M14): section planes reopen in their plane, drawn around the model (the file keeps only the plane; the importer placed them at the origin's foot on it); style settings written into the file's two style records by the item ids OpenSKP's `docs/API_DESIGN.md` confirmed against SketchUp 2026 (edges, back edges, profiles, depth cue, extension with widths, face style except Monochrome, X-ray, View > Axes / Guides / Hidden Geometry / Section Planes / Cuts / Fill, cut width, fill color) and read back from the displayed working copy; units and snapping in the options manager's `UnitsOptions` (keys of SketchUp's Ruby `OptionsManager`, read from SketchUp-saved v17 files too); image entities as `CImage` (root and nested).
    - Left, model-only until a SketchUp-saved sample shows the format (TRIAL_CHECKLIST.md 1b): scenes (the v17 `CViewPage` record is only partly known: name, flags, camera and hidden tags, not the rest or where the page list goes), Monochrome's RenderMode value, fog and Component Edit hiding (style items unknown), model axes, instance locks, two-point, the active section plane, the saved view. Also to confirm in SketchUp: RenderMode 0/1 for Wireframe/Hidden Line, EdgeDisplayMode 0 for Edges off, `CImage` schema, and whether SketchUp 2017 takes the display settings from the style records (written) or the model's own rendering-options block (not decoded).

## Tier 4: online services and plugins (not planned now)

- `file.send_to_layout` (LayOut is a separate Trimble product)
- `window.3d_warehouse`, `extensions.warehouse` (Trimble online services)
- `extensions.manager` (plugins are out of scope for now; see PLAN.md)
- `help.welcome`, `help.knowledge_center`, `help.contact`: done as links to the project page (https://github.com/noxqs/meshtard), opened with the system browser

## Order of work

Tier 1 top to bottom (items 1-4 share camera code, 5/7/8/12 share selection and flags, 9 and 26 share the renderer), then Tier 2 starting with 14-17 (file exchange, high value per effort), then Solid Tools (13), then the rest.
