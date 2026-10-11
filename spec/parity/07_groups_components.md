# 07 Groups and components (M6, M7)

Start: new model, default template. Draw a `1000mm,1000mm` box `1000mm` tall at the origin (rectangle + Push/Pull). Draw a second identical box at x=3000mm.

| Step | Action | Record |
|---|---|---|
| 1 | Triple-click the first box, Edit > Make Group (or right-click > Make Group) | Selection box look. Group origin and axes (Entity Info, Outliner)? |
| 2 | Double-click the group | Edit in place: how much is the rest of the model faded? Is the rest selectable? Dashed box drawn around the group? |
| 3 | Inside, draw a line across the top face. Press Esc twice | Which Esc closes the group? Does a click outside close it? Edit > Close Group name. |
| 4 | Triple-click the second box, press G (Make Component). In the dialog: name `Box`, description `test box`, Glue to: None, untick "Replace selection" if present, click Set Component Axes and set the origin at the box's top corner, then Create | Dialog fields and their order. Set Component Axes: 3 clicks (origin, red, green)? |
| 5 | If "Replace selection" was unticked, the box is still loose geometry: Ctrl+Z and redo step 4 with it ticked | Default state of "Replace selection". |
| 6 | M with Ctrl: copy the component twice along red, `1500mm` | |
| 7 | Double-click one copy, push its top face up `500mm`, close | Do all three copies change? |
| 8 | Right-click one copy > Make Unique, then edit it | Only that one changes? New definition name (`Box#1`?) |
| 9 | Right-click one copy > Explode | Geometry merges with touching geometry? Are crossing edges split? |
| 10 | Make a small `300mm,300mm` square, push `20mm`, select it, G: name `Plate`, Glue to: Any, Cut opening: off, Create | |
| 11 | Window > Default Tray > Components, In Model tab: click `Plate` and place it on a box wall, then on the top face | Does it glue to each face? Does it rotate with the cursor? Instance counts shown in the list? |
| 12 | Move a glued Plate | Does it stay on the face plane? |
| 13 | Components panel: change `Box` description, then In Model menu > Purge Unused | |
| 14 | Edit > Undo through the whole sequence, then redo | Does undo step out of / into edit contexts? |
| 15 | Save as `tests/corpus/2026/scenario_07.skp`. Then `File > Save As`, type "SketchUp 2017", file `tests/corpus/2017/scenario_07.skp` | |

## Expected (meshtard, current implementation)

- Make Group / Make Component: origin at the selection's bounding-box corner unless set in the dialog. Dialog: name, description, Glue to, Set Component Axes. Which other options does SketchUp's dialog have (Cut opening, Always face camera, Shadows face sun, Replace selection)?
- Edit in place fades the rest of the model and makes it unselectable; Esc, a click outside or Edit > Close Group/Component closes. The fade amount is a guess: estimate SketchUp's.
- Explode welds vertices and overlapping edges only; it does not split edges where they cross. Does SketchUp split them?
- Gluing components lie on matching faces. meshtard does not rotate with the cursor when placing.
- Undo covers the whole model and the edit context.
