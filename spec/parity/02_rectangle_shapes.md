# 02 Rectangle, circle, polygon, and shapes on sloped faces (M5)

Start: new model, default template, window maximized at 1920x1080. Lengths typed with `mm`.

| Step | Action | Record |
|---|---|---|
| 1 | Press R. Click the origin, move toward +red +green, type `4000mm,3000mm`, Enter | Rectangle size and orientation. Which way does the face point? |
| 2 | Press R. Click on the ground away from the first rectangle, move until the square hint appears, click | Name and look of the square hint (diagonal dashed line?). Is there also a golden section hint? |
| 3 | Press R, click, type `-1000mm,-500mm`, Enter | Negative values: drawn in the opposite direction? |
| 4 | Press P, push the 4000x3000 face up `2500mm`, Enter. Select its back top edge (the one at green = 3000) and press M, click the edge, move up blue, type `1500mm`, Enter | You now have a box with a sloped roof face. Screenshot. |
| 5 | Press R, click on the sloped roof face, move the cursor, click | Which direction do the rectangle's sides run on the slope: level line and up the slope, or the model axes? |
| 6 | Select all (Ctrl+A). Press Q, click the origin on the ground (protractor flat), click a point on +red, type `30`, Enter | The box is rotated 30 degrees in plan, so the roof slopes diagonally. |
| 7 | Press R, click on the roof face, move the cursor in a few directions, click | Which direction do the rectangle's sides run on a diagonal roof? Do they follow the roof's level line, the model's red/green axes projected onto the roof, or something else? |
| 8 | Press R on the roof face, click the first corner, press Right arrow, then Left, then Up | Does an arrow key re-orient or re-plane the rectangle? Which plane does each key lock? |
| 9 | Rotated Rectangle (Draw > Shapes > Rotated Rectangle): click on the ground, click a second point at about 45 degrees, move to set the width, click. Then repeat and type `2000mm,30` after the second click | What does the typed pair mean (width and angle)? |
| 10 | Press C, click on the ground, move, type `1000mm`, Enter. Then immediately type `48s`, Enter | Circle radius, default segments (24?), does `48s` change the last circle? |
| 11 | Before clicking the center of a new circle, type `12s`, Enter, then click the center and a radius point | Can the segment count be set before drawing? |
| 12 | Press C, click on a vertical wall face of the box | Does the circle lie on the wall? Default orientation of its first segment? |
| 13 | Draw > Shapes > Polygon: click on the ground, type `8s` before the second click, move, type `600mm`, Enter. Then repeat with Ctrl held | Default sides (6?). Is the radius to a corner or to a side midpoint? What does Ctrl change? |
| 14 | Space, click the edge of the first circle, look at Entity Info, then set Radius to `500mm` there | Is the circle one entity? Fields shown (radius, segments, length). Does the radius edit work? |
| 15 | Save as `tests/corpus/2026/scenario_02.skp`. Then `File > Save As`, type "SketchUp 2017", file `tests/corpus/2017/scenario_02.skp` | |

## Expected (meshtard, current implementation)

- Shapes lie on the clicked face's plane, else on the ground; arrow keys lock the plane.
- On a tilted face the rectangle's sides run along the face's level line and straight up the slope. This was written from memory: check both the axis-aligned slope (step 5) and the diagonal slope (step 7).
- Typed values: rectangle `w,h`, circle/polygon radius, `Ns` for segments or sides. Default segments: circle 24, polygon 6 sides. Is the polygon radius in meshtard measured the same way as in SketchUp?
- Arrow keys after the first point re-orient the shape. Which behaviour does SketchUp have?
- Circles select and report in Entity Info as one entity. Does SketchUp let you edit the radius and segments there (meshtard does not yet)?
