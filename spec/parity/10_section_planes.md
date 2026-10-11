# 10 Section planes (M7)

Start: new model, default template. Draw a `3000mm,2000mm` box `2500mm` tall at the origin. Draw a `1000mm` radius circle on the top face and push it down `1000mm` (a pit). Make a second box into a group at x=5000mm.

| Step | Action | Record |
|---|---|---|
| 1 | Tools > Section Plane. Hover the front wall, then the top face | Plane preview size and color; orientation follows the face? |
| 2 | Click on the front wall | Name/symbol dialog appears? Fields. Click Place. Is the cut active at once? Which side is kept? |
| 3 | Move (M) the section plane `-500mm` into the box | Cut follows live? Cut lines color and width. |
| 4 | View > Section Fill on and off | Default fill color. |
| 5 | Double-click the section plane | Toggles active cut? |
| 6 | Right-click the plane: Reverse, Align View, Create Group from Slice | Note each context menu item. |
| 7 | Place a second section plane on the top face | Is the first one deactivated? One active per context? |
| 8 | Edit the group (double-click), place a section plane on its wall inside, close the group | Does it cut only the group? |
| 9 | View > Section Planes off, View > Section Cuts off | |
| 10 | Select a section plane, Delete | |
| 11 | Save as `tests/corpus/2026/scenario_10.skp`. Then `File > Save As`, type "SketchUp 2017", file `tests/corpus/2017/scenario_10.skp` | |

## Expected (meshtard, current implementation)

- Section planes are per context, drawn, selectable, movable, deletable; double-click toggles the active cut; View > Section Planes and Section Cuts work; cut lines and section fill drawn.
- No name/symbol dialog yet; arrows are drawn on the kept side. Which side are SketchUp's arrows on?
- Section planes inside components are not saved by meshtard yet.
