# 03 Arcs, pie and freehand (M5)

Start: new model, default template, window maximized at 1920x1080, top view (Camera > Standard Views > Top). Lengths typed with `mm`.

| Step | Action | Record |
|---|---|---|
| 1 | Press A (2 Point Arc). Click the origin, click at 2000 mm on red (type `2000mm`, Enter), move up, type `500mm`, Enter | Is `500mm` the bulge (sagitta)? Default segment count? |
| 2 | Immediately type `24s`, Enter | Does it change the arc just drawn? |
| 3 | 2 Point Arc: start at the end of the first arc and move until the arc turns cyan/magenta, click | Name of the tangent hint and its color. Half circle hint? |
| 4 | Draw > Arcs > Arc (center): click a center, click a start point at 1000 mm, move, type `90`, Enter | Is the typed value the angle in degrees? Which direction (clockwise or counter-clockwise) is positive? |
| 5 | Draw > Arcs > 3 Point Arc: click three points on the ground | Does it pass through all three points? |
| 6 | Draw > Arcs > Pie: click center, click start at 1000 mm, type `120`, Enter | Is a face created? Are the two radii plain edges? |
| 7 | Close a shape with an arc and three lines so a face appears. Select the arc, check Entity Info | One entity? Fields (radius, segments, arc length). |
| 8 | Draw > Freehand: drag a squiggle on the ground ending away from the start. Then drag one ending on its start point | Is the result one curve entity? Does the closed one make a face? How many segments for a 2 s stroke? |
| 9 | Shift + Freehand drag | What does Shift do (polyline vs curve)? |
| 10 | Press E, click one segment of the first arc | Is the whole arc erased? |
| 11 | Save as `tests/corpus/2026/scenario_03.skp`. Then `File > Save As`, type "SketchUp 2017", file `tests/corpus/2017/scenario_03.skp` | |

## Expected (meshtard, current implementation)

- Default arc segments 12. Typed values: bulge for 2 Point Arc, angle for Arc and Pie, `Ns` for segments right after drawing.
- Tangent and half circle hints are shown. What color and name does SketchUp use for each?
- Arcs, circles and freehand curves select, erase and report in Entity Info as one entity.
- Saved arcs reopen as polyline curves in SketchUp, not editable arcs (known gap). Check when opening the 2017 file in step 11: does Entity Info show "Arc" or "Curve"?
