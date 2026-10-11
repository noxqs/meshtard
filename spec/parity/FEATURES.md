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
5. [ ] **Hide / Unhide (Selected, Last, All)** (`edit.hide`, `edit.unhide_*`) S.
   `hidden` flags exist on edges, faces, instances, annotations and the renderer skips them. Add the commands with undo; "Last" needs the last hidden set kept in the document. Depends on item 8 (Hidden Geometry) to select hidden things for Unhide Selected.
6. [ ] **Cut / Copy / Paste / Paste In Place** (`edit.cut|copy|paste|paste_in_place`) M.
   Clipboard = a detached `Entities` (selection copied with its connected edges, components by definition). Paste starts a Move-like placement with the cursor (reuse `modify.rs` Move preview); Paste In Place inserts at the original coordinates in the active context. Internal clipboard first; system clipboard later. Trial: paste grab point (bounding box corner?).
7. [ ] **Invert Selection** (`edit.invert_selection`, Ctrl+Shift+I) S.
   Everything visible in the active context minus the selection.
8. [ ] **View toggles: Axes, Guides, Hidden Geometry, Scene Tabs** (`view.axes|guides|hidden_geometry|scene_tabs`) S.
   Flags in `StyleSettings` (axes/guides per model, hidden geometry draws hidden edges dashed and faces as a mesh). Save in the file where the format has it. Check marks via `view_checked`.
9. [ ] **Edge Style toggles: Edges, Back Edges, Profiles, Depth Cue, Extension** (`view.edges` …) M.
   `profiles` exists in `StyleSettings`; renderer needs: edges off, back edges dashed through faces (second pass without depth test, dashed shader), profile width, depth cue (width by distance), extension (lines overshoot). Trial: default widths and dash pattern.
10. [ ] **Window menu opens tray panels** (`window.entity_info|components|materials|styles|tags|scenes|shadows|outliner|soften_edges|fog|environments|instructor`, `window.tray_show`) S.
    Shell API to show the tray and expand a panel; panels not built yet (Fog, Soften Edges, Environments, Instructor) get stubs or stay grey until built.
11. [ ] **Zoom Window** (`camera.zoom_window`, Ctrl+Shift+W) S.
    Drag a box; fit the camera to it along the current direction.
12. [ ] **Lock / Unlock** (`edit.lock`, `edit.unlock_*`) S.
    `locked` flag on groups and component instances (read and write it in the file); locked instances can be selected but not edited or moved; drawn with red selection box. Trial: what is blocked exactly.

## Tier 2: modeling power and file exchange

13. [ ] **Solid Tools: Outer Shell, Union, Subtract, Intersect, Trim, Split** (`tool.outer_shell`, `tool.solid_*`) L.
    Needs a "is solid" test (closed manifold, already shown in Entity Info?) and a robust mesh boolean in mt-kernel built on `intersect.rs` (intersect faces, classify each face inside/outside the other solid by ray parity, keep/discard, merge coplanar). CUDA candidate for classification on large models. Trial: result naming, which input is kept (Subtract/Trim), what happens to materials.
14. [ ] **Export 2D Graphic (PNG/JPG)** (`file.export_2d`) S.
    Render the viewport offscreen at a chosen size (wgpu texture readback), save with `image`. Options dialog: size, transparent background, anti-alias.
15. [ ] **Export 3D Model (OBJ, STL, glTF/DAE)** (`file.export_3d`) M.
    Triangulate with `tessellate.rs`; OBJ+MTL with textures first, STL for printing, then COLLADA (SketchUp's main exchange format). Trial: export options per format.
16. [ ] **Import (OBJ, STL, images, .skp as component)** (`file.import`) M.
    .skp import as a component definition (reader exists); OBJ/STL to faces with merge; images as image entities or textures (needs image entity, see 30).
17. [ ] **Scenes from the View > Animation menu** (`view.anim_*`) S.
    Scenes and transitions exist in the tray; add Add/Update/Delete/Previous/Next/Play and the Settings dialog (transition time, scene delay). Trial: Update Scene's property dialog.
18. [ ] **Tag tool and Window > Tags shortcuts** (`tool.tag`) S.
    Click to move entities to the active tag (SketchUp 2026 Tag Tool: Alt to sample, Ctrl to keep selection). Trial: its exact keys.
19. [ ] **Axes tool** (`tool.axes`) S.
    Reuse `AxesPick` from the Make Component dialog; store model axes in the file (model-level transformation) and drive drawing directions/inference from it.
20. [ ] **Position Camera, Walk, Look Around, Field of View** (`camera.position|walk|look_around|field_of_view`) M.
    Eye height 5' 6" (verify), walking with collision off, look around rotates about the eye; Field of View is a drag tool plus typed degrees (`35deg`) or focal length (`50mm`).
21. [ ] **Model Info** (`window.model_info`) M.
    Dialog pages: Units (format, precision, snapping), Dimensions, Text, Statistics (purge unused), Geo-location (read only), Credits, File. Units already exist in the model; wire to the dialog. Trial: screenshots of every page.
22. [ ] **Preferences** (`window.preferences`) M.
    Shortcuts page (rebind and save to user config like `toolbars.toml`), Drawing (click-drag-click), Graphics, General (auto-save, backup). Trial: shortcut export file.
23. [ ] **Revert** (`file.revert`) S.
    Reopen the file from disk after a confirm prompt (reuse `confirm.rs`).
24. [ ] **Hide Rest of Model / Hide Similar Components** (`view.hide_rest_of_model|hide_similar_components`) S.
    Edit context exists; renderer fades or hides everything outside the context path.
25. [ ] **Intersect With Context** (`edit.intersect_context`) S.
    Same as Intersect With Selection limited to the active context; `intersect.rs` already has the core.
26. [ ] **Fog** (`view.fog`, `window.fog`) S.
    Fog start/end distance and color in the style; a fragment shader term. Trial: default distances.
27. [ ] **Soften Edges panel** (`window.soften_edges`) S.
    Angle slider; soften/smooth selected edges whose face angle is under it (Eraser Ctrl already softens single edges).
28. [ ] **Two-Point Perspective** (`camera.two_point`) M.
    Keep vertical lines vertical: shifted-lens projection; orbit switches it off.

## Tier 3: presentation and less frequent work

29. [ ] **Sandbox From Scratch / From Contours** (`tool.sandbox_*`) M. Grid of triangles; contours via triangulation of curve points (Delaunay). Also Smoove/Stamp/Drape tools later.
30. [ ] **Images and Match New Photo** (`camera.match_new_photo`) L. Needs an image entity type (read/write in the file), then the match-photo vanishing-point UI.
31. [ ] **3D Text** (`tool.text_3d`) M. Font outlines (ttf-parser) to faces, extrude; dialog with font, height, extrude, filled.
32. [ ] **Export Section Slice (DXF/DWG)** (`file.export_section`) M. Section cut lines to DXF (DWG is proprietary: DXF only).
33. [ ] **Export Animation (MP4/image sequence)** (`file.export_animation`) M. Offscreen render each frame of scene transitions; image sequence first, video via an encoder later.
34. [ ] **Print / Print Preview / Print Setup** (`file.print*`) M. Offscreen render to PDF (printpdf) or the OS print dialog. Trial: scale options.
35. [ ] **Templates: New from Template, Save As Template** (`file.new_from_template`, `file.save_as_template`) S. Templates are .skp files in the user config folder; New uses the chosen one.
36. [ ] **Generate Report** (`file.generate_report`) M. Attribute/component report to CSV/HTML.
37. [ ] **Manage Trays** (`window.manage_trays`) S. Create trays and choose their panels; save like the toolbar layout.
38. [ ] **About** (`help.about`) S. Version, license, OpenSKP credit, splash image.

## Tier 4: online services and plugins (not planned now)

- `file.send_to_layout` (LayOut is a separate Trimble product)
- `window.3d_warehouse`, `extensions.warehouse` (Trimble online services)
- `extensions.manager` (plugins are out of scope for now; see PLAN.md)
- `help.welcome`, `help.knowledge_center`, `help.contact` (could open project pages instead)

## Order of work

Tier 1 top to bottom (items 1-4 share camera code, 5/7/8/12 share selection and flags, 9 and 26 share the renderer), then Tier 2 starting with 14-17 (file exchange, high value per effort), then Solid Tools (13), then the rest.
