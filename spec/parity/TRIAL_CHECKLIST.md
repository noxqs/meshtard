# SketchUp 2026 trial checklist

Scripted part first: run `tools/trial/capture.rb` in the Ruby Console (see `tools/trial/README.md`). It saves the 1b sample files (2026 and 2017), `empty_default.skp` and text dumps, and lists the screenshots still to take by hand.

Run on the Windows machine with SketchUp 2026. Prepare everything before starting the trial clock:

- meshtard Windows release build and a few meshtard-saved `.skp` files on the machine (for scenario 12)
- this repo checked out, with `spec/raw/` and `tests/corpus/2026/` and `tests/corpus/2017/` created
- OBS or Windows Game Bar set up to record the screen
- `TRIAL_NOTES.md` open in an editor next to SketchUp

Save all output under `spec/` and `tests/corpus/` in this repo, named as below. Work top to bottom: the sections are ordered by value per minute of trial time, so stop wherever the trial ends.

## 1. Text exports and file round trip (first 30 minutes)

These are the hardest to get any other way.

- [ ] `Window > Preferences > Shortcuts > Export...` and save the file as `spec/raw/shortcuts.txt`. Then correct `keymap.toml` from it and set `verified = true`.
- [ ] Save the empty default model as `tests/corpus/2026/empty_default.skp`, and `File > Save As` SketchUp 2017 as `tests/corpus/2017/empty_default.skp`.
- [ ] Scenario 12 steps 1 to 3: open the meshtard-saved files in SketchUp and screenshot the Save As version list.

## 1b. Writer format captures (FEATURES.md item 39)

meshtard cannot save these yet because where the SketchUp 2017 format keeps them is unknown. Each capture is a pair of files that differ in one setting only, so a byte diff finds the field. Start every pair from the same model (`File > New`, draw one 100" box, make it a group, Iso view, save), change one thing, save again. Save each file twice: as 2026 into `tests/corpus/2026/w39/` and with `File > Save As` "SketchUp 2017" into `tests/corpus/2017/w39/`, same names.

- [ ] `base.skp`: the starting model, nothing changed.
- [ ] Scenes: `scene_one.skp` with one scene "A" (Add Scene, default properties); `scene_two.skp` with scenes "A" (Iso, perspective) and "B" (Top, Camera > Parallel Projection, tag "T1" hidden in it), "A" selected; `scene_props.skp` like `scene_two.skp` but scene B with only Camera Location ticked in its properties, and a description "desc".
- [ ] Face style: `face_wireframe.skp`, `face_hiddenline.skp`, `face_monochrome.skp` (View > Face Style, nothing else changed). Confirms RenderMode values (meshtard writes 0 / 1, Monochrome unknown).
- [ ] Edges: `edges_off.skp` (View > Edge Style > Edges off). Confirms EdgeDisplayMode 0.
- [ ] Fog: `fog_on.skp` (Window > Fog: Display Fog on, distances moved, Use background color off, a red color).
- [ ] Component Edit: `hide_rest.skp` and `hide_similar.skp` (View > Component Edit, one option each).
- [ ] Axes: `axes_moved.skp` (Tools > Axes: origin at (10", 20", 30"), red axis turned 30 degrees about blue).
- [ ] Lock: `lock_on.skp` (the group locked).
- [ ] Two-point: `two_point.skp` (Camera > Two-Point Perspective).
- [ ] Section planes: `section_active.skp` (a section plane on the box's top face, active) and `section_inactive.skp` (the same plane, not active); `section_moved.skp` (the plane's graphic moved within its plane with Move, if SketchUp allows it).
- [ ] Shadows: `shadows_on.skp` (Shadows on, nothing else) and `shadows_time.skp` (on, 3 July 09:15, Light 60, Dark 30, On Ground off). Finds the shadow block's on/off byte and time encoding.
- [ ] Polygon: `polygon.skp` (Polygon tool, 6 sides) next to `circle6.skp` (Circle tool, 6 segments). Finds the polygon mark (`CArcCurve` byte 0 is the candidate).
- [ ] Saved view (meshtard now writes it; to confirm): open a meshtard-saved file in Perspective and one in Parallel Projection and check SketchUp opens at the same view.
- [ ] Image: `image.skp` (File > Import a PNG as Image). Confirms OpenSKP's uncalibrated `CImage` schema.
- [ ] Open a meshtard-saved file with non-default style settings and units (Hidden Line, profiles off, X-ray, Model Info > Units in cm) and check that SketchUp shows them (style records vs. the model's own rendering options).

## 2. Scenarios with corpus files

Run each scenario file in this folder step by step, in this order (most valuable first):

| Order | File | Covers |
|---|---|---|
| 1 | `02_rectangle_shapes.md` | rectangle, circle, polygon, shapes on sloped and diagonal faces |
| 2 | `06_move_rotate_scale.md` | move, rotate, scale, copies and arrays |
| 3 | `05_pushpull_offset_followme.md` | push/pull, offset, follow me, intersect |
| 4 | `07_groups_components.md` | groups, components, edit in place, explode, make unique, glue, axes |
| 5 | `01_line_inference.md` | line tool, inference, axis locks, typed lengths |
| 6 | `09_scenes_styles_shadows.md` | face styles, X-ray, edge styles, shadows, scenes, transitions |
| 7 | `08_tags_materials.md` | tags, materials, paint bucket, textures, Outliner, Entity Info |
| 8 | `10_section_planes.md` | section planes, cuts, fill |
| 9 | `11_dimensions_text_guides.md` | tape measure, protractor, dimensions, text |
| 10 | `04_selection_erase.md` | selection rules, eraser, hide, soften, undo |
| 11 | `03_arcs_freehand.md` | arcs, pie, freehand |
| 12 | `12_file_toolbars.md` | file open/save, older versions, toolbars |
| 13 | `00_camera_navigation.md` | orbit, pan, zoom, camera defaults |

For each one:

- record the screen into `spec/raw/scenarios/NN.mp4`
- fill the scenario's table in `TRIAL_NOTES.md` as you go
- save the model after the last step as `tests/corpus/2026/scenario_NN.skp`, then `File > Save As`, type "SketchUp 2017", as `tests/corpus/2017/scenario_NN.skp`

Afterwards, run the same steps in meshtard from the recordings and fill the `meshtard same?` column.

## 3. Static capture

- [ ] Open every menu and submenu, screenshot each one into `spec/raw/menus/<menu>.png`. Correct `menus.toml` and set `verified = true`. Note greyed-out items with an empty model.
- [ ] Screenshot the full window at 1920x1080 with a new model from the default template: `spec/raw/ui/new_model_1920.png`.
- [ ] Screenshot every toolbar under `View > Toolbars...`: `spec/raw/ui/toolbars/<name>.png`.
- [ ] Screenshot every Default Tray panel, expanded: `spec/raw/ui/tray/<panel>.png`.
- [ ] `Window > Styles > Edit`: screenshot every tab of the default style into `spec/raw/ui/styles/`.

## 4. Grey-menu reference

Screenshot every dialog and behaviour that meshtard does not have yet (its menu items are greyed out in meshtard), so they can be built later without the trial. One PNG per page or state, into `spec/raw/<area>/`:

- [ ] `Window > Model Info`: every page (Animation, Classifications, Components, Credits, Dimensions, File, Geo-location, Rendering, Statistics, Text, Units, ...) into `spec/raw/model_info/<page>.png`
- [ ] `Window > Preferences`: every page into `spec/raw/preferences/<page>.png`
- [ ] `File > Export > 3D Model`, `2D Graphic`, `Section Slice`, `Animation`: the file dialog and each Options... dialog into `spec/raw/export/`
- [ ] `File > Import`: the file dialog with its type list and Options... into `spec/raw/import/`
- [ ] Solid Tools (Outer Shell, Intersect, Union, Subtract, Trim, Split, Solid Inspector if present) on two overlapping solid groups: before and after for each into `spec/raw/solid_tools/`; note which input is kept and the status bar text for non-solids
- [ ] Tag tool, Axes tool, Soften Edges panel, Revert, View > Component Edit (Hide Rest Of Model, Hide Similar Components): the Tag tool's modifier keys (sample, keep selection) and cursor; Axes tool clicks and its context menu (Reset, Move, Place); Soften Edges defaults (angle, Smooth normals, Soften coplanar) and whether face edges count; the Revert prompt text; Entity Info rows and volume format for a solid group into `spec/raw/m12/`
- [ ] Sandbox (From Contours, From Scratch, Smoove, Stamp, Drape, Add Detail, Flip Edge): one screenshot per tool in use into `spec/raw/sandbox/`
- [ ] `Tools > 3D Text`: the dialog and the placed result into `spec/raw/3d_text/`
- [ ] `File > Print Preview` and `File > Print`: dialogs into `spec/raw/print/`
- [ ] `File > Generate Report`: the dialog, every template, and one generated report into `spec/raw/report/`

## 5. Corpus

The `.skp` files saved in sections 1 and 2 become automated round-trip tests. The tests in `crates/mt-skp/tests/` will read every file under `tests/corpus/2026/` (VFF format) and `tests/corpus/2017/` (legacy format), compare both versions of each scenario with each other, write them back with meshtard's writer and read the result again. Commit the files after the trial; do not edit them in SketchUp afterwards.

- [ ] `tests/corpus/2026/`: `empty_default.skp` and one `scenario_NN.skp` per scenario run
- [ ] `tests/corpus/2017/`: the same names, saved as SketchUp 2017
- [ ] `TRIAL_NOTES.md` filled and copied back into the repo

## 1c. Toolbars and cursors (for FEATURES.md items 40 and 41)

- [ ] Screenshot `View > Toolbars...` (the full list) and every toolbar in it into `spec/raw/ui/toolbars/<name>.png`, buttons in order, with tooltips of any unclear button.
- [ ] Record the cursor for every tool (hover over empty space, over a face, over an edge) and with each modifier (Ctrl, Shift, Alt, Ctrl+Shift): a screen recording `spec/raw/cursors/cursors.mp4` going through the tools in menu order, plus close-up screenshots `spec/raw/cursors/<tool>[_<modifier>].png`. Note when a cursor changes without a key (Solid Tools picks, not-allowed, scale grips, busy).

## 1d. On-screen viewport widgets (FEATURES.md item 42)

- [ ] With a new default model, screenshot the whole 3D view and zoom into each corner: is there an axes indicator or other widget? Repeat with View > Toolbars > Solar North (or the Shadows / Location settings) on, and with a model geo-located.
- [ ] For each widget found: what happens on hover, click, double-click and drag; does it follow orbiting; is it in View menu toggles or Preferences; record it in `spec/raw/ui/widgets/`.

## 1e. Inference locks (FEATURES.md item 70)

- [ ] For Line, Rectangle, Move, Rotate, Push/Pull: record how each lock is set and released: arrow keys (pressing the same key again? another key? Escape?), Shift held vs tapped, Shift over a point (through point), Shift over a face (plane), Shift+Alt, Alt cycling in Line (status bar text), Alt in Rotate/Protractor. Screen recording `spec/raw/inference/locks.mp4`.
