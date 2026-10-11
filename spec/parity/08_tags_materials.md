# 08 Tags, materials, paint, textures, tray panels (M7)

Start: new model, default template. Draw a `2000mm,2000mm` box `1000mm` tall at the origin and make it a group. Draw a second box at x=4000mm, leave it loose. Open every Default Tray panel listed below.

| Step | Action | Record |
|---|---|---|
| 1 | Tags panel: click + twice | Default names (`Tag`, `Tag1`?). Order: Untagged first? Screenshot. |
| 2 | Double-click the first new tag, rename to `Walls`. Click its color swatch, pick red | Rename and color edit work the same? |
| 3 | Select the group, Entity Info > Tag: `Walls` | |
| 4 | Click the eye of `Walls` | Group hidden? |
| 5 | Show it again. Delete `Tag1` (empty). Then delete `Walls` (in use) | Dialog for a tag in use: options offered? |
| 6 | Tags panel: click the Color by tag button | Where is the button? Screenshot. |
| 7 | Materials panel: In Model. Create Material (the + icon): name `Brick red`, color R200 G60 B40, opacity 70 | Dialog fields. Default new material color. |
| 8 | B (Paint Bucket) with `Brick red`, click the loose box's top face | Only the clicked side? |
| 9 | Select the loose box, B, click one of its selected faces | All selected faces painted? |
| 10 | B, click the group from outside | Group gets the material, faces inside default? |
| 11 | B, hold Alt, click a painted face | Samples the material? |
| 12 | B, hold Ctrl and click a face; hold Shift and click a face | Fill connected / replace all? |
| 13 | Materials panel: Browse > Brick and Cladding > pick a brick texture, paint the group's inner faces (edit the group first) | Texture size and alignment on a vertical face. |
| 14 | View > Face Style > Shaded With Textures, then Shaded | Texture shown only in the first? |
| 15 | Outliner panel: expand everything, click the group | Tree contents and icons. Filter box: type `Gr` |
| 16 | Entity Info for a face, an edge, a group, a component | Field list for each. Screenshot each. |
| 17 | Save as `tests/corpus/2026/scenario_08.skp`. Then `File > Save As`, type "SketchUp 2017", file `tests/corpus/2017/scenario_08.skp` | |

## Expected (meshtard, current implementation)

- Tags: add, delete, rename, visibility, color; Entity Info Tag dropdown; Untagged first. No folders or dash patterns.
- Materials: In Model grid, Create Material, edit color and opacity with egui's picker (SketchUp's picker differs). New material color is a guess: what does SketchUp start with?
- Paint Bucket paints the clicked side, or the whole selection when clicking a selected entity. Alt sample and Ctrl/Shift fills are missing.
- Textures from .skp files show in Shaded With Textures and are saved back; meshtard cannot browse SketchUp's library. Check the texture alignment after opening the 2017 file in meshtard.
- Outliner shows the tree, syncs selection and filters by name.
