# SketchUp 2026 help pages vs Meshtard

Compiled 2026-10-11 from about 110 pages of help.sketchup.com (text dated Aug-Oct 2026) and the release notes 2024.0, 2025.0, 2026.0, 2026.1, 2026.1.2, 2026.2. Only features Meshtard lacks or only partly has are listed; the ranked todo items are FEATURES.md 44-67. `[doc]` = from the help page or release notes, `[inf]` = inference or memory, to check in the trial. MT = what Meshtard has.

Sources (all under `https://help.sketchup.com/en/`): CM context-menu-items-entities; SEL sketchup/selecting-geometry; FLIP sketchup/flipping-mirroring-rotating-and-arrays; MOVE sketchup/moving-entities-around; STR sketchup/stretching-geometry; SCALE sketchup/scaling-your-model-or-parts-your-model; OFF sketchup/offsetting-line-existing-geometry; DIV sketchup/dividing-splitting-and-exploding-lines-and-faces; BE sketchup/basic-edits; ARC sketchup/drawing-arcs; DRAW sketchup/introducing-drawing-basics-and-concepts; TXT sketchup/adding-text-labels-and-dimensions-model; SEC sketchup/slicing-model-peer-inside; AX sketchup/adjusting-drawing-axes; TAGS sketchup/controlling-visibility-tags; DASH sketchup/applying-dashed-lines-layers; MATP navigating-materials-panel; MATE sketchup/editing-materials; MATR sketchup/replacing-material; MATO managing-and-organizing-materials; TEXP sketchup/positioning-textures; AREA sketchup/calculating-material-area; STY sketchup/creating-and-editing-style; EDGE sketchup/edge-styles; FACE face-style; BG sketchup/customizing-models-background-styles; WM sketchup/watermarking-model; MODS sketchup/customizing-modeling-settings-complement-style; STYC sketchup/managing-model-styles-and-collections; CMPN sketchup/creating-basic-component; REPL sketchup/replacing-components; CCOL sketchup/organizing-component-collections; OUT sketchup/working-hierarchies-outliner; SCN sketchup/creating-scenes; SHD sketchup/casting-real-world-shadows; ENV sketchup/adding-environment-to-models; SNAP sketchup/snaps; TPL sketchup/setting-templates; UNITS managing-units-measurement; GEO geolocation-model-info; GUIDE using-guides; MEAS measuring-distance; VCB using-measurements-box; SOFT sketchup/softening-smoothing-and-hiding-geometry; SRCH sketchup/search-tools-commands-and-extensions; IMG sketchup/importing-and-using-2d-images; GRP sketchup/grouping-geometry; RN24..RN262 release-notes/sketchup-desktop-20240 ... 20262.

## Context menus (CM [doc]; MT: none in the viewport) -> FEATURES 44, 45, 46, 47, 51, 55
- Right-click shows a menu for the entity or selection under the cursor; inapplicable items greyed.
- All entities: Entity Info, Erase, Hide/Unhide, Make Group / Make Component (also for one object since RN25), Zoom Selection.
- Empty space: Select All (was Invert Selection before RN261); Make Group makes an empty group to draw into (GRP).
- Select submenu (SEL): Connected Faces, Bounding Edges, All Connected, All with Same Tag, All with Same Material, Deselect Edges / Faces, Invert Selection; Connected by Snaps (RN25).
- Edges/arcs: Divide (live division points, typed count or length; DIV), Weld Edges, Explode Curve, Find Center, Convert to Polygon.
- Faces: Reverse Faces, Orient Faces, Align View, Align Axes, Area (selection / tag / material; AREA), Make Unique Texture, Combine Textures. Modify Face (Live Components): out.
- Instances (BE): Reset Scale, Reset Skew, Scale Definition, Flip Along > Red / Green / Blue, Change Axes, Unglue, Save As (own .skp), Reload, Soften/Smooth Edges. Dynamic Components, Swap, Warehouse, Connect: out.
- Section planes (SEC): Active Cut, Reverse, Align View, Create Group from Slice, Troubleshoot Section Fill (red circles at open loop vertices), Hide.
- Dimensions (TXT): Edit Text, Text Position (outside start / center / outside end), Type > Radius / Diameter. Leader text: Arrow (None, Dot, Closed, Open), Leader (View Based, Pushpin, Hidden).
- Images: Export, Use As Material, Shadows.
- Drawing axes (AX): Place, Move (dialog with offsets and rotations), Reset, Hide; double-click with the Axes tool places them at once.

## Selection and modify tools -> FEATURES 48, 49, 50, 56, 57, 58, 59
- Lasso (Shift+Space; SEL): freehand clockwise = touched, counterclockwise = inside only; segment mode, double-click to finish.
- Flip tool (FLIP): three semi-transparent planes, drag along an axis; arrow keys pick the plane (Left green, Right red, Up blue); Ctrl = flipped copy; Alt = object vs parent axes; custom magenta plane on a hovered face; arrays `3x`, `3*`, `3/` with Flip and Move.
- Move/Rotate grips on bounding boxes (MOVE, FLIP, RN24, RN25): corner grips, Alt cycles corners / edge midpoints / side centers / center; rotation handles and protractor; Rotate-tool grips (2025); a hidden grip makes the object transparent.
- Move (MOVE, STR): typed `[x,y,z]` absolute and `<x,y,z>` relative (Line tool too, DRAW); Alt autofold; live stretching (MT differs: on click); extruded arc control edge resize.
- Scale (SCALE): Shift uniform, Ctrl about center (held); per-axis commas; Tape Measure rescale of the model or open group.
- Offset (OFF): Alt keeps overlaps; double-click another face repeats.
- Paint Bucket (MATR): Alt sample, Shift all matching, Ctrl all connected same material, Ctrl+Shift same object; Default material.
- Tag tool (TAGS): Shift Replace All, Ctrl Tag All Instances (Alt sample exists).
- Tape Measure (MEAS, GUIDE, RN261): Ctrl cycles Guide Line / Guide Point / Measure; double-click = guide point.
- Line (DRAW): Alt cycles inference modes; Shift through-point lock; Shift+Alt face-plane lock.
- Inference in open groups/components (DRAW): magenta markers, "in group" tooltip; On Section, Hidden Section, Perpendicular to Face, Extend Edge. MT: missing.
- Arc (ARC, RN25): Ctrl+/- segments, `20c`, Alt tangency, double-click fillet repeat, convex click unrounds (2025), midpoint/endpoint drag changes radius, Entity Info radius/segments editable.
- Small: Freehand Ctrl+/-; slope `8:12` in Protractor/Rotate; Alt frees protractor; Orbit double-click centers, Ctrl rolls; Zoom Shift-drag = FOV; Eraser Alt unsoftens (MT Ctrl+Shift); Undo cancels a running tool (RN24).
- Component Snaps (SNAP, RN24, RN25): snap disks, Make/Edit Snaps, snap-to-snap / snap-to-face placement, Alt nearest snap.

## Tray panels -> FEATURES 52, 53, 54, 60, 65, 66
- Entity Info: visibility, lock, cast/receive shadows; instance Name vs Definition; Advanced Attributes (Price, Size, URL, Type, Status, Owner; CMPN); total volume of several solids (RN25); soft/smooth on edges (SOFT); editable length, arc radius and segments; section plane name and symbol; text/dimension color, font, arrow. [inf] face front/back swatches.
- Materials Edit (MATE): color wheel / RGB / HSL / HEX, Reset Color; texture load/replace; size with aspect lock; external editor. Photoreal maps (metalness, roughness, normal, AO): later. Generate Textures (AI): out.
- Materials Select (MATP, MATO): library collections, secondary pane, Details (open/create collection, favorites, Purge Unused, Delete All), Duplicate, Sample, right-click Delete / Area / thumbnail Auto-Cube-Flat (RN26).
- Texture Position (TEXP): red move, green scale/rotate, blue scale/shear, yellow distort pins; flip, rotate, reset, Free Pins; [inf] projected textures.
- Components (REPL, CCOL, CMPN, RN262): Select Instances, Replace Selected, collections, favorites, Edit tab (glue, cut opening, always face camera, shadows face sun, axes), [inf] Statistics tab, In Model search (2026.2).
- Styles (STY, STYC, EDGE, FACE, BG, WM, MODS): several In Model styles, Create / Update / Purge, collections, .style files; Edge endpoints, jitter, color by material / axis, dashes; Face front/back colors, transparency quality, ambient occlusion (RN24, RN26); Background sky / horizon / ground / show ground from below; Watermarks (overlay/background, mask, blend, stretched/tiled/positioned); Modeling colors. Mix tab: [inf].
- Tags (TAGS, DASH, RN25, RN26, RN2612): folders, filter, multi-select, Expand/Collapse/Select All/Purge/Select All Tagged, sort, dashes per tag, Color by Tag (text and dimensions too), Apply Tags to Scenes, Show New Tags.
- Outliner (OUT): rename (also section plane name/symbol), sort, expand all, eye per row, drag to reparent, status icons, selected hidden objects drawn hatched.
- Scenes (SCN, RN26): name, description, Include in Animation, properties to save (camera, hidden geometry/objects, visible tags, active sections, style and fog, shadows, environment, axes), thumbnails, reorder, tab search, undoable scene operations.
- Shadows (SHD): UTC offset, Use Sun for Shading, [inf] from edges.
- Environments (ENV, RN25): HDRI/EXR library, Set Sun Location, skydome, reflections, rotation, exposure.
- Instructor: per-tool help, [inf] no content page found.
- Trays (RN26): panels between trays, floating, Make Tray, double-click re-docks.

## Model Info pages (TPL) -> FEATURES 61
- Components: fade/hide rest of model and similar components ([inf] sliders), show component axes.
- Rendering: anti-aliased textures.
- Statistics: Fix Problems, [inf] nested components.
- Dimensions (TXT): font and size, align to screen or line, text position, endpoints (slash default, dot, closed, open, none), Select All / Update Selected, Expert settings (highlight non-associated [doc]; hide foreshortened / small [inf]).
- Text: screen and leader fonts and colors, default leader and arrow, Select All / Update Selected.
- Units (UNITS): area and volume units, display format, force 0", angle precision applied, snapping used by inference.
- Geo-location (GEO): manual latitude/longitude, clear location, [inf] north angle.

## Text and sections -> FEATURES 62
- Text (TXT, RN25): component leader = component name, double-click face = area label, `<>` keeps the measured value live in custom text.
- Section planes (SEC): name and symbol prompt, arrow-key and Shift locks.

## Shell, view, files -> FEATURES 63, 64, 67
- Search (SRCH, Shift+S): type-ahead command list with concept matching.
- View > Hidden Objects separate from Hidden Geometry (SOFT); Edit > Purge Unused with shortcut (RN26); status bar geo pin and Model Info button; tool icons in Windows menus (RN26).
- Import (IMG, RN26, RN261): image as texture on a face; DWG/DXF import (layers as groups); PDF as image (Windows); 3DS, FBX, KMZ, VRML, XSI, USD/GLB exports (low).

## Out of scope
Live Components / Modify Face, Dynamic Components, Classifier/IFC, Overlays, Ruby console and extensions, collaboration (comments, view links, avatars; RN26), AI Assistant / Render / Generate Object / Generate Textures (RN261), Analysis Hub / Advanced Shadows (RN262), Scan Essentials, Revit Importer, Add Location imagery, Trimble Connect, 3D Warehouse, Migrate Extensions/Preferences, contextual notifications (RN2612).

## Checked and not counted
- Undo history panel: no evidence SketchUp desktop has one [inf]; Edit > Undo shows the operation name.
- Style Builder: removed in 2025 (RN25).
- Already on FEATURES.md: Smoove/Stamp/Drape/Add Detail/Flip Edge (29), Match Photo (30), Preferences pages (43).
