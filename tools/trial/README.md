# Trial capture

`capture.rb` saves the sample `.skp` files and text dumps that `spec/parity/TRIAL_CHECKLIST.md` asks for, so the scripted part of the SketchUp 2026 trial takes about 10 minutes. It uses only the documented SketchUp Ruby API.

## Run it

1. Start SketchUp 2026 with a new model from the default template. Close any open model with unsaved work first: the script starts with File > New and later closes models without saving.
2. Window > Ruby Console, then type (with your path):

   ```ruby
   load 'C:/path/to/meshtard/tools/trial/capture.rb'
   ```

   Or paste the whole file into the console.
3. Do not click in SketchUp while the console prints `step: ...` lines. The script stops at four manual steps (Two-Point Perspective, Convert to Polygon, fonts and dimension expert settings, moving a section plane). Each prints what to click. Do it, then type `MeshtardTrial.save_as`, or `MeshtardTrial.skip` to leave that step out. `MeshtardTrial.status` shows progress.
4. When it is done, the console prints a summary and opens the output folder.

## Output

The output goes to `Desktop\meshtard_trial_<timestamp>\` (OneDrive Desktop if that is where Windows keeps it):

| Path | Contents | Goes to in the repo |
|---|---|---|
| `corpus/2026/`, `corpus/2017/` | `empty_default.skp`, and `w39/*.skp`: base box plus one change each (scenes, face styles, edges off, fog, Component Edit, axes, locks, section planes, shadows, polygon / circle, image, tag folders and dashes, texts and dimensions on tags and inside a group and a component, environment, two-point, fonts). Each is saved as 2026 and as SketchUp 2017. | `tests/corpus/2026/`, `tests/corpus/2017/` |
| `variants/<name>.json` | the model state just before each sample was saved: rendering options, shadow info, camera, axes, scenes, tags, entities | `spec/raw/trial_capture/` |
| `dumps/` | version, `shortcuts.txt` (`Sketchup.get_shortcuts`), toolbars and their visibility, rendering options, shadow info, all model options (units included), styles, tags, line styles, environments, camera, view, extensions, `read_default` probes, the default template, SketchUp's preference JSON files | `spec/raw/trial_capture/` (`shortcuts.txt` also to `spec/raw/`) |
| `log.txt`, `results.json`, `README.txt` | what ran, what failed | `spec/raw/trial_capture/` |

Zip the folder and send it back. Check `dumps/preferences/` first: SketchUp's preference files can list recent file paths.

## By hand

A script cannot capture these. Take them after the script, in this order. The section numbers are those of `TRIAL_CHECKLIST.md`.

1. `Window > Preferences > Shortcuts > Export...` to `spec/raw/shortcuts.txt` (1). The script's `shortcuts.txt` is a fallback.
2. With a meshtard-saved file open, the `File > Save As` version list (1, scenario 12).
3. Every menu and submenu with an empty model, greyed items visible: `spec/raw/menus/<menu>.png` (3).
4. The full window with a new model at 1920x1080: `spec/raw/ui/new_model_1920.png` (3).
5. `View > Toolbars...` and every toolbar, buttons in order: `spec/raw/ui/toolbars/<name>.png` (1c, 3).
6. Every Default Tray panel, expanded: `spec/raw/ui/tray/<panel>.png` (3).
7. `Window > Styles > Edit`, every tab: `spec/raw/ui/styles/` (3).
8. Cursors: a recording through every tool (empty space, face, edge; Ctrl, Shift, Alt, Ctrl+Shift), plus close-ups: `spec/raw/cursors/` (1c).
9. The 3D view's corners and any widget, also with Solar North on and a geo-located model: `spec/raw/ui/widgets/` (1d).
10. Inference locks for Line, Rectangle, Move, Rotate and Push/Pull (arrow keys, Shift, Alt): `spec/raw/inference/locks.mp4` (1e).
11. Every Model Info page and every Preferences page: `spec/raw/model_info/`, `spec/raw/preferences/` (4).
12. File > Export and Import dialogs with their Options...: `spec/raw/export/`, `spec/raw/import/` (4).
13. The checks in 1b that need meshtard files: a meshtard file saved in Perspective and one in Parallel Projection open at the same view; a meshtard file with Hidden Line, profiles off, X-ray and cm units shows them (1b).

The scenarios in section 2 are not scripted.

## Uncertain parts

These depend on API behaviour that could not be tried outside SketchUp. Check `log.txt` for errors:

- `Sketchup.open_file` and `Model#close(true)` are assumed to finish before the next step. The script checks the model's path after opening, and does not save a sample when that check fails.
- Monochrome uses `Sketchup.send_action('renderMonochrome:')`, which runs later; the script waits 1.5 s before saving. The `RenderMode` and `EdgeDisplayMode` values used for the others (0 wireframe, 1 hidden line, `EdgeDisplayMode` 0 = edges off) are not documented. Each `variants/*.json` records the values that were actually set.
- Fog distances, shadow time (`Time.utc`), tag dashes (a line style name containing "Dash") and the environment (a small generated `.hdr` file, SketchUp 2025+) are best guesses.
- There is no API for Two-Point Perspective, Convert to Polygon, text and dimension fonts, or dimension expert settings. Those are the manual steps.

## Optional: 2021+ file format checks

If you have time, try the few open-in-SketchUp checks listed in `spec/VFF_FORMAT.md` section 6 (does SketchUp open a re-zipped file, or one with the header value changed). They tell us whether writing SketchUp 2021+ files is feasible later (low priority).
