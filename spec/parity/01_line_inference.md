# 01 Line tool, inference and axis locks (M4)

Start: new model, default template, window maximized at 1920x1080. Type all lengths with their unit (`mm`) so both apps get the same values whatever the template's units.

| Step | Action | Record |
|---|---|---|
| 1 | Press L. Click the origin. Move the cursor along the red axis, type `3000mm`, Enter | Status bar text and Measurements box label before and after typing. Is the line exactly 3000 mm (Entity Info)? |
| 2 | Move the cursor parallel to green, type `2000mm`, Enter. Then parallel to red (back), type `3000mm`, Enter. Then click the start point | Does a face appear when the loop closes? Does the tool stop the chain or keep drawing? Which way does the face point (front white or back blue-grey up)? |
| 3 | Esc. Press L, hover the midpoint of the front edge, wait 2 s without moving | Inference name and color for midpoint, endpoint, on edge, on face. Is there a hover delay before a point is "acquired"? |
| 4 | From the midpoint of the front edge, click, move to the midpoint of the back edge, click. Esc | Is the face split into two? Is the new line split where it meets other edges? |
| 5 | L, click the origin. Move up the blue axis 1000 mm without clicking. Press Right arrow, then Left, then Up, then Down | Which arrow locks which axis? What happens on Down without a reference edge? Color of the line while locked (thick?) |
| 6 | Still drawing: press Up again | Does a second press release the lock? |
| 7 | Esc. L, click the origin, hover the far corner (3000,2000) for 2 s, then move the cursor up along blue | Dotted "from point" tracking line? Which point names appear? |
| 8 | Esc. L, click the far corner, move the cursor roughly parallel to the front edge, hold Shift, move around | Shift locks which direction? Does releasing Shift free it? |
| 9 | Draw a 500 mm square inside the face, 500 mm from the front-left corner (L, click inside the face, type lengths as in step 1-2) | Does the inner face become a hole plus a separate inner face? |
| 10 | Esc. Select the line drawn in step 4, press Delete | Do the two halves merge back into one face? |
| 11 | Draw a line that crosses an existing edge in the middle (from outside the rectangle to inside, on the ground) | Are both edges split at the crossing? |
| 12 | Type lengths in other units on new lines: `1m`, `25cm`, `10'`, `3'6"`, `12"`, `1500` (no unit) | Accepted? Length shown in Entity Info for each, in model units. |
| 13 | Save as `tests/corpus/2026/scenario_01.skp`. Then `File > Save As`, type "SketchUp 2017", file `tests/corpus/2017/scenario_01.skp` | |

## Expected (meshtard, current implementation)

- Lines chain until a face closes, then the tool ends the chain; Esc cancels the current segment.
- New face on the ground plane faces down (back face up). Is this what SketchUp shows?
- Inference colors: endpoint green, midpoint cyan, on edge red, on face blue. Pick distances 10 px for points, 6 px for edges. Which names and colors does SketchUp use, and does it show tooltips with the names?
- Points need about 1 s of hover before they are acquired for tracking. How long is SketchUp's delay?
- Arrow keys: Right locks red, Left green, Up blue, Down parallel/perpendicular to the reference edge. Shift locks the current inferred direction while held.
- Lines split each other where they cross, a closed loop inside a face makes a hole and an inner face, erasing a splitting edge heals the face.
- A number without unit uses model units.
