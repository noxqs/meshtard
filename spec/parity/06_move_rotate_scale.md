# 06 Move, Rotate, Scale with copies and arrays (M6)

Start: new model, default template. Draw a `1000mm,1000mm` rectangle at the origin and push it up `1000mm`. Triple-click it to select it.

| Step | Action | Record |
|---|---|---|
| 1 | M, click the near-left bottom corner, move along red, type `2000mm`, Enter | Is the geometry shown live while moving? |
| 2 | Ctrl+Z. M, tap Ctrl once (cursor shows +), click the same corner, move along red, type `1500mm`, Enter, then type `x5`, Enter | Six boxes total, 1500 mm apart? Does `5x` also work? |
| 3 | Ctrl+Z twice. M, Ctrl, copy along green `5000mm`, Enter, then type `/5`, Enter | Four copies between, five total spaces? |
| 4 | After step 3, type `/2`, Enter | Can the array be changed again? |
| 5 | Click empty space. M, click the top face of a box (nothing selected) and drag up | Moves only the face (stretches the box)? |
| 6 | M, hover an edge of a box until red + appear on a face (auto-fold) | Does auto-fold happen? Note any marks. |
| 7 | Select one box (triple-click). Q, click the box's bottom corner with the protractor flat (blue), click a point along red, move, type `45`, Enter | Rotated 45 degrees? Which way is positive? |
| 8 | Q, tap Ctrl, rotate copy by `30`, then type `x4` | Rotational array? |
| 9 | Q with the cursor on a vertical wall | Protractor color and orientation on walls. Does the protractor orientation stay locked with Shift? |
| 10 | Select one box. S | Grip box: number of grips (corners, edges, faces). Grips aligned to the box or to world axes? |
| 11 | S, drag a corner grip, type `2`, Enter | Uniform scale about the opposite corner? |
| 12 | S, drag a face grip, type `0.5`, Enter. Then repeat holding Ctrl | Ctrl scales about the center? |
| 13 | S, drag a corner grip, type `3000mm`, Enter. Then type `1,2,3`, Enter | Typed size vs factor; three-value form? |
| 14 | S, drag a face grip, type `-1`, Enter | Mirror? |
| 15 | Save as `tests/corpus/2026/scenario_06.skp`. Then `File > Save As`, type "SketchUp 2017", file `tests/corpus/2017/scenario_06.skp` | |

## Expected (meshtard, current implementation)

- Move stretches connected geometry; Ctrl copies; `5x` and `/5` make arrays after a copy. Does SketchUp accept `x5` and `5x`, `*5`? Does a second typed array replace the first?
- Move, Rotate and Scale show the geometry live.
- Rotate: center, start, end; typed angle; Ctrl copy. The protractor plane comes from the three points, not SketchUp's own protractor orientation. How does SketchUp choose the plane when hovering?
- Scale: grip box with corner, edge and face grips; typed factors (`2`, `1,2,3`) and sizes (`3000mm`). Is Ctrl (about center) and Shift (uniform) the same as in SketchUp?
- Auto-fold is not implemented. Note whether SketchUp folds here.
