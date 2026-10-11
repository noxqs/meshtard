# 00 Camera navigation (M0)

Start: new model, default template, window maximized at 1920x1080.

| Step | Action | Record |
|---|---|---|
| 1 | Do nothing | Screenshot. Measure sky zenith/horizon color, ground color, axis colors and widths, which axis halves are dotted. |
| 2 | `Window > Model Info > ...` / `Camera > Field of View` | Note default field of view and camera eye/target (Ruby console: `Sketchup.active_model.active_view.camera.eye` / `.target` / `.fov`). |
| 3 | Hold middle mouse, drag right 200 px | How many degrees does the view rotate? Around which point? |
| 4 | Hold middle mouse, drag up 200 px | Tilt angle. Can the camera go below the ground? Is there a limit at straight down? |
| 5 | Middle-drag with cursor over empty sky vs over ground | Does the orbit pivot change? |
| 6 | Shift + middle drag | Pan: does the point under the cursor stay under the cursor? |
| 7 | Scroll wheel one notch in, cursor at screen center then at a corner | Zoom factor per notch; does it zoom toward the cursor? |
| 8 | Press O, left drag; press H, left drag; press Z, left drag up/down | Same behavior as mouse buttons? What does Z drag do (dolly or FOV)? |
| 9 | Shift+Z in empty model | What does Zoom Extents do with no geometry? |
| 10 | Double-click middle mouse on ground | Re-centers view? |
| 11 | Draw a rectangle `1000mm,600mm` from the origin, Push/Pull it `400mm`. Shift+Z | Margin around the box: how much of the view does it fill? |
| 12 | Camera > Standard Views > Top, Front, Right, Iso (one after another) | Is each change animated? Does each one also zoom to fit? Which corner does Iso look from, and does it depend on where the camera was? |
| 13 | Camera > Parallel Projection, then Camera > Perspective | Does the box keep its size on screen? Which item shows a check mark? |
| 14 | Orbit a little, wheel-zoom 3 notches, then Camera > Previous three times, then Next once | Is one wheel spin one step? Is a menu view change one step? Is Previous animated? |
| 15 | Save as `tests/corpus/2026/scenario_00.skp` and Save As 2017 into `tests/corpus/2017/scenario_00.skp` | |

## Expected (meshtard, current implementation)

- Middle drag orbits around the camera target (turntable around world Z).
- Shift + middle drag pans; a point at the depth of the camera target follows the cursor.
- Wheel zooms toward the point under the cursor (on the plane through the target, facing the camera), 20% of the distance per notch.
- O / H / Z select the Orbit / Pan / Zoom tools; left drag then does the same; Space returns to Select.
- Shift+Z keeps the view direction and fits the model's bounding sphere (a margin, size unverified).
- Standard views and Zoom Extents animate like a scene change; each standard view also fits the model. Iso looks from the quadrant the camera is in.
- Parallel / Perspective keep the size of things at the camera target; the active one has a check mark.
- Previous / Next: one drag or wheel spin is one step; menu view changes are one step each; history holds 50 views.
