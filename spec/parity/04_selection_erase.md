# 04 Selection, erase, hide, soften (M3, M5)

Start: new model, default template. Draw a 2000mm x 2000mm rectangle at the origin and push it up 1000mm. Draw a 500 mm circle on the ground away from the box.

| Step | Action | Record |
|---|---|---|
| 1 | Space. Click the top face | Highlight look: dotted blue faces? Edge color? |
| 2 | Ctrl+click an edge, Shift+click the top face, Ctrl+Shift+click the edge | Cursor icon for each modifier. Final selection. |
| 3 | Click empty space. Double-click the top face, then double-click an edge | Face + its edges; edge + its faces? |
| 4 | Triple-click a face | All connected geometry? |
| 5 | Drag a box left to right that fully covers the circle and only half the box | Solid box line, only fully enclosed items selected? |
| 6 | Drag right to left over the same area | Dashed box, everything touched selected? |
| 7 | Click one segment of the circle | Whole circle selected? |
| 8 | Ctrl+A, then Ctrl+T (Select None) | Shortcuts work? Edit menu item names. |
| 9 | Hover an edge from 4, 5, 6, 8 px away and click | Largest distance that still picks the edge. |
| 10 | Press E. Click a vertical edge of the box | Edge and both faces gone? |
| 11 | Ctrl+Z. E, hold Shift and click a vertical edge | Hidden (View > Hidden Geometry shows it dashed)? |
| 12 | E, hold Ctrl and click another vertical edge | Softened: faces shaded smooth? Does the edge still select? |
| 13 | E, hold Ctrl+Shift and click the softened edge | Unsoftened? |
| 14 | E, drag across three edges | Preview of edges to erase while dragging (highlight color)? |
| 15 | Undo 3 times, Redo 2 times | Edit menu shows "Undo Erase"? Undo count limit (Preferences > General)? |
| 16 | Save as `tests/corpus/2026/scenario_04.skp`. Then `File > Save As`, type "SketchUp 2017", file `tests/corpus/2017/scenario_04.skp` | |

## Expected (meshtard, current implementation)

- Click selects; Ctrl adds, Shift toggles, Ctrl+Shift removes. Double-click selects a face with its edges or an edge with its faces; triple-click selects everything connected.
- Left-to-right drag is a window (solid box, enclosed only); right-to-left is crossing (dashed, touching).
- Edge pick aperture 5 px. What is SketchUp's?
- Eraser: Shift hides, Ctrl softens and smooths, Ctrl+Shift unsoftens; dragging previews the edges to erase.
- Undo keeps 100 steps. Does SketchUp have a limit, and does it name the operation in the Edit menu?
