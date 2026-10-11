# 09 Scenes, styles, face styles, X-ray, shadows (M7)

Start: new model, default template. Draw a `3000mm,2000mm` box `2500mm` tall at the origin and a `1000mm` radius circle pushed up `500mm` at x=5000mm.

| Step | Action | Record |
|---|---|---|
| 1 | View > Face Style > Wireframe, Hidden Line, Shaded, Shaded With Textures, Monochrome. Screenshot each from the same view | Colors of faces and edges in each. |
| 2 | View > Face Style > X-ray (on top of Shaded), then with Wireframe | X-ray opacity (50%?). Does X-ray combine with Wireframe? |
| 3 | Styles panel > Edit > Edge settings: turn off Edges, then Profiles, then set Profiles width to 4 | Profiles: which edges count (borders, silhouettes)? Default width. |
| 4 | View > Shadows on. Shadows panel: time 08:00, date 21 June, then time 17:00 | Shadow direction; default location (Window > Model Info > Geo-location). Default time and date. |
| 5 | Shadows panel: Light 50, Dark 20; untick On faces, then On ground | |
| 6 | Window > Scenes, click + (Add Scene) | Scene name (`Scene 1`), tab shown above the viewport? |
| 7 | Orbit to a new view, turn shadows off, Wireframe. Add Scene | |
| 8 | Click the `Scene 1` tab | Transition time and easing (Model Info > Animation default: 2 s?). Do face style and shadows switch too? |
| 9 | Change the view while on Scene 2, right-click its tab > Update Scene | Update dialog: which properties to save? |
| 10 | Scenes panel: tick "Include in animation" off for Scene 1, rename Scene 2 to `Top`, delete Scene 1 | |
| 11 | Model Info > Animation: turn scene transitions off, click a tab | Instant switch? |
| 12 | Save as `tests/corpus/2026/scenario_09.skp`. Then `File > Save As`, type "SketchUp 2017", file `tests/corpus/2017/scenario_09.skp` | |

## Expected (meshtard, current implementation)

- Face styles Wireframe, Hidden Line, Shaded, Shaded With Textures, Monochrome; X-ray at 50% opacity. Is 50% right?
- Styles panel toggles edges and profiles; profile width 2 px. Is SketchUp's default 3?
- Shadows: sun position from NOAA formulas, shadow map, light/dark sliders, on faces, on ground. Default location and time were set from memory: write down SketchUp's.
- Scenes: add, update, delete, activate, scene tabs, animated transitions. No per-scene property toggles in meshtard. Which properties does SketchUp store per scene?
- Scenes, styles and shadow settings are not yet saved by meshtard's writer, so this corpus file is important for reading tests.
