# 11 Dimensions, text, tape measure, protractor (M6)

Start: new model, default template. Draw a `3000mm,2000mm` box `2500mm` tall at the origin. Draw a `1000mm` radius circle on the ground at x=6000mm.

| Step | Action | Record |
|---|---|---|
| 1 | T (Tape Measure). Click the front bottom edge (not an endpoint), move in, type `600mm`, Enter | Parallel guide line at 600 mm? Infinite, dashed, color? |
| 2 | T, click an endpoint, move up blue, type `1000mm`, Enter | Guide point with a guide line to it? |
| 3 | T, tap Ctrl, measure between two corners | Measures only, no guide? Status bar value. |
| 4 | L, draw a line snapping to the guide from step 1 | Inference name on a guide ("On Guide"?) |
| 5 | Tools > Protractor. Click a corner on the ground (blue protractor), click along red, move, type `30`, Enter. Then repeat and type `1:2` | Angle guide; slope form accepted? |
| 6 | Tools > Dimensions. Click two endpoints of the front edge, move out, click | Text size, arrow style, units format. |
| 7 | Dimensions: click the circle's edge, move, click. Then type nothing, right-click > Type | Radius or diameter? How to switch? |
| 8 | Tools > Text: click a box corner, move, click, type `Corner`, click outside | Leader text. Default text for a point (coordinates?) and for a face (area?). |
| 9 | Tools > Text: click empty space, type `Note`, click outside | Screen text: fixed to screen? |
| 10 | Edit > Delete Guides | All guides removed, guide points too? |
| 11 | Save as `tests/corpus/2026/scenario_11.skp`. Then `File > Save As`, type "SketchUp 2017", file `tests/corpus/2017/scenario_11.skp` | |

## Expected (meshtard, current implementation)

- Tape Measure: parallel guide from an edge, guide point from a point; guides are dashed grey and infinite and are used by inference. Ctrl toggle for measure-only: check SketchUp's.
- Protractor makes angle or slope guides.
- Dimensions: linear, radius, diameter. Text: leader text and screen text. All saved to .skp.
- Default text content for a point, edge and face: from memory, check SketchUp's.
