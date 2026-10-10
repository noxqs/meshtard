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

## Expected (meshtard, current implementation)

- Middle drag orbits around the camera target (turntable around world Z).
- Shift + middle drag pans; a point at the depth of the camera target follows the cursor.
- Wheel zooms toward the point under the cursor (on the plane through the target, facing the camera), 20% of the distance per notch.
- O / H / Z select the Orbit / Pan / Zoom tools; left drag then does the same; Space returns to Select.
