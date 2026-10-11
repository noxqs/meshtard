# 05 Push/Pull, Offset, Follow Me, Intersect (M6)

Start: new model, default template, window maximized at 1920x1080. Lengths typed with `mm`.

| Step | Action | Record |
|---|---|---|
| 1 | R, rectangle at the origin `3000mm,2000mm`. P, click the face, move up, type `1000mm`, Enter | Is the geometry shown live while dragging, or an outline? |
| 2 | P, double-click the top face | Repeats 1000 mm? |
| 3 | P, hold Ctrl (cursor shows +), click the top face, pull up `500mm` | Is the start face kept with a new edge loop between? |
| 4 | On the front wall draw a rectangle `800mm,600mm` from 500 mm in from the corner. P, click it, push into the box, type `2000mm` (the box depth), Enter | Does pushing through to the back wall cut a hole? What does the status bar say? |
| 5 | F (Offset), click the top face, move inward, type `200mm`, Enter | Inner loop at 200 mm? Corners mitered? |
| 6 | F, double-click the top face again | Repeats the last offset? |
| 7 | Draw a circle of radius `600mm` on the ground away from the box. Select the circle's edge only, F, click it, type `100mm` | Offset of an arc/circle stays a circle? Segments kept? |
| 8 | Draw a U shape of lines on the ground (1000, 1000, 1000 mm) and offset it inward by `600mm` (more than half the width) | Which pieces disappear? Does SketchUp trim the collapsed parts like meshtard? |
| 9 | Follow Me: draw a rectangle `3000mm,3000mm` on the ground at x=6000mm. On one of its edges draw a vertical profile: a 300 x 300 mm square on a vertical plane at the corner (use arrow Right/Left to lock the plane). Select the four ground edges, press the Follow Me tool, click the profile | Profile swept around the path? Corners mitered? Path edges kept? |
| 10 | Follow Me with a circle as path and a small rectangle profile at its edge | Joints soft or hard? Which angle threshold makes them soft? |
| 11 | Draw two boxes that overlap (second box 1000 mm offset in red and green). Select both, Edit > Intersect Faces > With Selection. Then repeat on new boxes with Edit > Intersect Faces > With Model | New edges where faces cross? Context menu name of the command. |
| 12 | Save as `tests/corpus/2026/scenario_05.skp`. Then `File > Save As`, type "SketchUp 2017", file `tests/corpus/2017/scenario_05.skp` | |

## Expected (meshtard, current implementation)

- Push/Pull shows the geometry live. Ctrl toggles "create new starting face". Double-click repeats the last distance. Pushing to the back face cuts a hole.
- Offset: typed distance, double-click repeats; arcs stay arcs; corners always mitered (no limit); partial collapses are trimmed. Where a trim cuts an arc, meshtard ends with a chord piece. What does SketchUp leave?
- Follow Me: select the path, then click the profile. Joints under 30 degrees are softened. Is 30 degrees SketchUp's threshold?
- Intersect Faces With Model and With Selection add edges where faces cross. With Context is missing in meshtard.
