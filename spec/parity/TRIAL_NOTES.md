# SketchUp 2026 trial notes

Fill in during the trial. One row per step of each scenario, then one row per feature area. `meshtard same?`: Y, N, or `?` if not checked. Put screenshot or video timestamps in Notes.

## Scenarios

### 00 Camera navigation (M0) (`00_camera_navigation.md`)

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| 1. Do nothing | | | |
| 2. `Window > Model Info > ...` / `Camera > Field of View` | | | |
| 3. Hold middle mouse, drag right 200 px | | | |
| 4. Hold middle mouse, drag up 200 px | | | |
| 5. Middle-drag with cursor over empty sky vs over ground | | | |
| 6. Shift + middle drag | | | |
| 7. Scroll wheel one notch in, cursor at screen center then at a corner | | | |
| 8. Press O, left drag; press H, left drag; press Z, left drag up/down | | | |
| 9. Shift+Z in empty model | | | |
| 10. Double-click middle mouse on ground | | | |
| 11. Box, then Shift+Z | | | |
| 12. Standard Views: Top, Front, Right, Iso | | | |
| 13. Parallel Projection, then Perspective | | | |
| 14. Previous x3, Next x1 | | | |
| Saved 2026 and 2017 files | | | |

### 01 Line tool, inference and axis locks (M4) (`01_line_inference.md`)

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| 1. Press L. Click the origin. Move the cursor along the red axis, type... | | | |
| 2. Move the cursor parallel to green, type `2000mm`, Enter. Then paral... | | | |
| 3. Esc. Press L, hover the midpoint of the front edge, wait 2 s withou... | | | |
| 4. From the midpoint of the front edge, click, move to the midpoint of... | | | |
| 5. L, click the origin. Move up the blue axis 1000 mm without clicking... | | | |
| 6. Still drawing: press Up again | | | |
| 7. Esc. L, click the origin, hover the far corner (3000,2000) for 2 s,... | | | |
| 8. Esc. L, click the far corner, move the cursor roughly parallel to t... | | | |
| 9. Draw a 500 mm square inside the face, 500 mm from the front-left co... | | | |
| 10. Esc. Select the line drawn in step 4, press Delete | | | |
| 11. Draw a line that crosses an existing edge in the middle (from outsi... | | | |
| 12. Type lengths in other units on new lines: `1m`, `25cm`, `10'`, `3'6`... | | | |
| Saved 2026 and 2017 files | | | |

### 02 Rectangle, circle, polygon, and shapes on sloped faces (M5) (`02_rectangle_shapes.md`)

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| 1. Press R. Click the origin, move toward +red +green, type `4000mm,30`... | | | |
| 2. Press R. Click on the ground away from the first rectangle, move un... | | | |
| 3. Press R, click, type `-1000mm,-500mm`, Enter | | | |
| 4. Press P, push the 4000x3000 face up `2500mm`, Enter. Select its bac... | | | |
| 5. Press R, click on the sloped roof face, move the cursor, click | | | |
| 6. Select all (Ctrl+A). Press Q, click the origin on the ground (protr... | | | |
| 7. Press R, click on the roof face, move the cursor in a few direction... | | | |
| 8. Press R on the roof face, click the first corner, press Right arrow... | | | |
| 9. Rotated Rectangle (Draw > Shapes > Rotated Rectangle): click on the... | | | |
| 10. Press C, click on the ground, move, type `1000mm`, Enter. Then imme... | | | |
| 11. Before clicking the center of a new circle, type `12s`, Enter, then... | | | |
| 12. Press C, click on a vertical wall face of the box | | | |
| 13. Draw > Shapes > Polygon: click on the ground, type `8s` before the... | | | |
| 14. Space, click the edge of the first circle, look at Entity Info, the... | | | |
| Saved 2026 and 2017 files | | | |

### 03 Arcs, pie and freehand (M5) (`03_arcs_freehand.md`)

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| 1. Press A (2 Point Arc). Click the origin, click at 2000 mm on red (t... | | | |
| 2. Immediately type `24s`, Enter | | | |
| 3. 2 Point Arc: start at the end of the first arc and move until the a... | | | |
| 4. Draw > Arcs > Arc (center): click a center, click a start point at... | | | |
| 5. Draw > Arcs > 3 Point Arc: click three points on the ground | | | |
| 6. Draw > Arcs > Pie: click center, click start at 1000 mm, type `120`... | | | |
| 7. Close a shape with an arc and three lines so a face appears. Select... | | | |
| 8. Draw > Freehand: drag a squiggle on the ground ending away from the... | | | |
| 9. Shift + Freehand drag | | | |
| 10. Press E, click one segment of the first arc | | | |
| Saved 2026 and 2017 files | | | |

### 04 Selection, erase, hide, soften (M3, M5) (`04_selection_erase.md`)

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| 1. Space. Click the top face | | | |
| 2. Ctrl+click an edge, Shift+click the top face, Ctrl+Shift+click the... | | | |
| 3. Click empty space. Double-click the top face, then double-click an... | | | |
| 4. Triple-click a face | | | |
| 5. Drag a box left to right that fully covers the circle and only half... | | | |
| 6. Drag right to left over the same area | | | |
| 7. Click one segment of the circle | | | |
| 8. Ctrl+A, then Ctrl+T (Select None) | | | |
| 9. Hover an edge from 4, 5, 6, 8 px away and click | | | |
| 10. Press E. Click a vertical edge of the box | | | |
| 11. Ctrl+Z. E, hold Shift and click a vertical edge | | | |
| 12. E, hold Ctrl and click another vertical edge | | | |
| 13. E, hold Ctrl+Shift and click the softened edge | | | |
| 14. E, drag across three edges | | | |
| 15. Undo 3 times, Redo 2 times | | | |
| Saved 2026 and 2017 files | | | |

### 05 Push/Pull, Offset, Follow Me, Intersect (M6) (`05_pushpull_offset_followme.md`)

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| 1. R, rectangle at the origin `3000mm,2000mm`. P, click the face, move... | | | |
| 2. P, double-click the top face | | | |
| 3. P, hold Ctrl (cursor shows +), click the top face, pull up `500mm` | | | |
| 4. On the front wall draw a rectangle `800mm,600mm` from 500 mm in fro... | | | |
| 5. F (Offset), click the top face, move inward, type `200mm`, Enter | | | |
| 6. F, double-click the top face again | | | |
| 7. Draw a circle of radius `600mm` on the ground away from the box. Se... | | | |
| 8. Draw a U shape of lines on the ground (1000, 1000, 1000 mm) and off... | | | |
| 9. Follow Me: draw a rectangle `3000mm,3000mm` on the ground at x=6000... | | | |
| 10. Follow Me with a circle as path and a small rectangle profile at it... | | | |
| 11. Draw two boxes that overlap (second box 1000 mm offset in red and g... | | | |
| Saved 2026 and 2017 files | | | |

### 06 Move, Rotate, Scale with copies and arrays (M6) (`06_move_rotate_scale.md`)

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| 1. M, click the near-left bottom corner, move along red, type `2000mm`... | | | |
| 2. Ctrl+Z. M, tap Ctrl once (cursor shows +), click the same corner, m... | | | |
| 3. Ctrl+Z twice. M, Ctrl, copy along green `5000mm`, Enter, then type... | | | |
| 4. After step 3, type `/2`, Enter | | | |
| 5. Click empty space. M, click the top face of a box (nothing selected... | | | |
| 6. M, hover an edge of a box until red + appear on a face (auto-fold) | | | |
| 7. Select one box (triple-click). Q, click the box's bottom corner wit... | | | |
| 8. Q, tap Ctrl, rotate copy by `30`, then type `x4` | | | |
| 9. Q with the cursor on a vertical wall | | | |
| 10. Select one box. S | | | |
| 11. S, drag a corner grip, type `2`, Enter | | | |
| 12. S, drag a face grip, type `0.5`, Enter. Then repeat holding Ctrl | | | |
| 13. S, drag a corner grip, type `3000mm`, Enter. Then type `1,2,3`, Enter | | | |
| 14. S, drag a face grip, type `-1`, Enter | | | |
| Saved 2026 and 2017 files | | | |

### 07 Groups and components (M6, M7) (`07_groups_components.md`)

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| 1. Triple-click the first box, Edit > Make Group (or right-click > Mak... | | | |
| 2. Double-click the group | | | |
| 3. Inside, draw a line across the top face. Press Esc twice | | | |
| 4. Triple-click the second box, press G (Make Component). In the dialo... | | | |
| 5. If "Replace selection" was unticked, the box is still loose geometr... | | | |
| 6. M with Ctrl: copy the component twice along red, `1500mm` | | | |
| 7. Double-click one copy, push its top face up `500mm`, close | | | |
| 8. Right-click one copy > Make Unique, then edit it | | | |
| 9. Right-click one copy > Explode | | | |
| 10. Make a small `300mm,300mm` square, push `20mm`, select it, G: name... | | | |
| 11. Window > Default Tray > Components, In Model tab: click `Plate` and... | | | |
| 12. Move a glued Plate | | | |
| 13. Components panel: change `Box` description, then In Model menu > Pu... | | | |
| 14. Edit > Undo through the whole sequence, then redo | | | |
| Saved 2026 and 2017 files | | | |

### 08 Tags, materials, paint, textures, tray panels (M7) (`08_tags_materials.md`)

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| 1. Tags panel: click + twice | | | |
| 2. Double-click the first new tag, rename to `Walls`. Click its color... | | | |
| 3. Select the group, Entity Info > Tag: `Walls` | | | |
| 4. Click the eye of `Walls` | | | |
| 5. Show it again. Delete `Tag1` (empty). Then delete `Walls` (in use) | | | |
| 6. Tags panel: click the Color by tag button | | | |
| 7. Materials panel: In Model. Create Material (the + icon): name `Bric`... | | | |
| 8. B (Paint Bucket) with `Brick red`, click the loose box's top face | | | |
| 9. Select the loose box, B, click one of its selected faces | | | |
| 10. B, click the group from outside | | | |
| 11. B, hold Alt, click a painted face | | | |
| 12. B, hold Ctrl and click a face; hold Shift and click a face | | | |
| 13. Materials panel: Browse > Brick and Cladding > pick a brick texture... | | | |
| 14. View > Face Style > Shaded With Textures, then Shaded | | | |
| 15. Outliner panel: expand everything, click the group | | | |
| 16. Entity Info for a face, an edge, a group, a component | | | |
| Saved 2026 and 2017 files | | | |

### 09 Scenes, styles, face styles, X-ray, shadows (M7) (`09_scenes_styles_shadows.md`)

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| 1. View > Face Style > Wireframe, Hidden Line, Shaded, Shaded With Tex... | | | |
| 2. View > Face Style > X-ray (on top of Shaded), then with Wireframe | | | |
| 3. Styles panel > Edit > Edge settings: turn off Edges, then Profiles,... | | | |
| 4. View > Shadows on. Shadows panel: time 08:00, date 21 June, then ti... | | | |
| 5. Shadows panel: Light 50, Dark 20; untick On faces, then On ground | | | |
| 6. Window > Scenes, click + (Add Scene) | | | |
| 7. Orbit to a new view, turn shadows off, Wireframe. Add Scene | | | |
| 8. Click the `Scene 1` tab | | | |
| 9. Change the view while on Scene 2, right-click its tab > Update Scene | | | |
| 10. Scenes panel: tick "Include in animation" off for Scene 1, rename S... | | | |
| 11. Model Info > Animation: turn scene transitions off, click a tab | | | |
| Saved 2026 and 2017 files | | | |

### 10 Section planes (M7) (`10_section_planes.md`)

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| 1. Tools > Section Plane. Hover the front wall, then the top face | | | |
| 2. Click on the front wall | | | |
| 3. Move (M) the section plane `-500mm` into the box | | | |
| 4. View > Section Fill on and off | | | |
| 5. Double-click the section plane | | | |
| 6. Right-click the plane: Reverse, Align View, Create Group from Slice | | | |
| 7. Place a second section plane on the top face | | | |
| 8. Edit the group (double-click), place a section plane on its wall in... | | | |
| 9. View > Section Planes off, View > Section Cuts off | | | |
| 10. Select a section plane, Delete | | | |
| Saved 2026 and 2017 files | | | |

### 11 Dimensions, text, tape measure, protractor (M6) (`11_dimensions_text_guides.md`)

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| 1. T (Tape Measure). Click the front bottom edge (not an endpoint), mo... | | | |
| 2. T, click an endpoint, move up blue, type `1000mm`, Enter | | | |
| 3. T, tap Ctrl, measure between two corners | | | |
| 4. L, draw a line snapping to the guide from step 1 | | | |
| 5. Tools > Protractor. Click a corner on the ground (blue protractor),... | | | |
| 6. Tools > Dimensions. Click two endpoints of the front edge, move out... | | | |
| 7. Dimensions: click the circle's edge, move, click. Then type nothing... | | | |
| 8. Tools > Text: click a box corner, move, click, type `Corner`, click... | | | |
| 9. Tools > Text: click empty space, type `Note`, click outside | | | |
| 10. Edit > Delete Guides | | | |
| Saved 2026 and 2017 files | | | |

### 12 File open and save, older versions, toolbars (M3, M8) (`12_file_toolbars.md`)

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| 1. In SketchUp, File > Open each meshtard-saved file | | | |
| 2. For one meshtard file, Window > Model Info > Statistics, then Fix P... | | | |
| 3. File > Save As: look at the "Save as type" list | | | |
| 4. Make a small change, check the title bar | | | |
| 5. File > New with unsaved changes | | | |
| 6. Drag a `.skp` from Explorer onto the window | | | |
| 7. File > Save A Copy As | | | |
| 8. View > Toolbars... | | | |
| 9. Tick Large Tool Set, drag it by its grip to the left edge, then to... | | | |
| 10. Right-click any toolbar | | | |
| 11. Close and restart SketchUp | | | |
| Saved 2026 and 2017 files | | | |

## Feature areas

### UI

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| Menu bar structure and labels | | | |
| Greyed-out menu items with an empty model | | | |
| Default shortcuts | | | |
| Toolbar layout on first start | | | |
| Toolbar tooltips | | | |
| Toolbar flyouts | | | |
| Default Tray panels, order, width | | | |
| Measurements box label and typing | | | |
| Status bar texts | | | |
| Window theme colors and sizes | | | |

### View and camera

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| Sky and ground colors | | | |
| Axes look | | | |
| Default eye, target, FOV | | | |
| Orbit, Pan, Zoom | | | |
| Zoom Extents, Standard Views, Parallel Projection | | | |

### Inference

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| Point kinds, colors, names | | | |
| Pick distances | | | |
| Hover delay | | | |
| Arrow-key and Shift locks | | | |
| From-point tracking | | | |
| Square, golden section, half circle, tangent hints | | | |

### Drawing tools

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| Line chaining and auto-face | | | |
| Rectangle and Rotated Rectangle | | | |
| Shapes on axis-aligned slopes | | | |
| Shapes on diagonal slopes | | | |
| Circle and Polygon defaults | | | |
| Arcs, Pie, Freehand | | | |
| Typed values per tool | | | |

### Selection and erase

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| Click modifiers | | | |
| Window vs crossing | | | |
| Double and triple click | | | |
| Eraser hide, soften, unsoften | | | |
| Undo limit and menu text | | | |

### Modify tools

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| Push/Pull | | | |
| Move and arrays | | | |
| Rotate and protractor plane | | | |
| Scale grips and typed values | | | |
| Offset and trimming | | | |
| Follow Me joints | | | |
| Intersect Faces | | | |
| Auto-fold | | | |

### Groups and components

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| Make Group / Component origin | | | |
| Make Component dialog | | | |
| Edit in place fade | | | |
| Explode merging | | | |
| Make Unique naming | | | |
| Gluing and placing | | | |
| Components panel | | | |

### Tags, materials, panels

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| Tags panel | | | |
| Materials panel and new material | | | |
| Paint Bucket modifiers | | | |
| Texture alignment | | | |
| Outliner | | | |
| Entity Info fields | | | |

### Scenes, styles, shadows

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| Face styles | | | |
| X-ray opacity | | | |
| Edge and profile settings | | | |
| Shadow defaults (location, time) | | | |
| Scene tabs and transitions | | | |
| Scene properties | | | |

### Section planes

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| Placement and dialog | | | |
| Cut lines and fill | | | |
| Active cut per context | | | |
| Context menu | | | |

### Annotation

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| Tape Measure guides | | | |
| Protractor guides | | | |
| Dimensions look | | | |
| Text defaults | | | |

### File

| Item | SketchUp behaviour | meshtard same? (Y/N) | Notes |
|---|---|---|---|
| meshtard 2017 files open in SketchUp | | | |
| Save As version list | | | |
| Unsaved marker | | | |
| Save changes prompt | | | |
| Dropped .skp | | | |
