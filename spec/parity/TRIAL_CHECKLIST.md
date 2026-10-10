# SketchUp 2026 trial checklist

Run on the Windows machine with SketchUp 2026. Prepare everything before starting the trial clock.

Save all output under `spec/` in this repo, named as below.

## 1. Static capture (first hour)

- [ ] `Window > Preferences > Shortcuts > Export...` and save the file as `spec/raw/shortcuts.txt`. Then correct `keymap.toml` from it and set `verified = true`.
- [ ] Open every menu and submenu, screenshot each one into `spec/raw/menus/<menu>.png`. Correct `menus.toml` and set `verified = true`. Note greyed-out items with an empty model.
- [ ] Screenshot the full window at 1920x1080 with a new model from the default template: `spec/raw/ui/new_model_1920.png`.
- [ ] Screenshot every toolbar under `View > Toolbars...`: `spec/raw/ui/toolbars/<name>.png`.
- [ ] Screenshot every Default Tray panel, expanded: `spec/raw/ui/tray/<panel>.png`.
- [ ] Save the empty default model as `tests/corpus/2026/empty_default.skp`.
- [ ] `Window > Model Info` and `Window > Preferences`: screenshot every page into `spec/raw/ui/model_info/` and `spec/raw/ui/preferences/`.
- [ ] `Window > Styles > Edit`: screenshot every tab of the default style.

## 2. Scenarios

Run each file in this folder (`NN_*.md`) step by step. For each one:

- record the screen (OBS or Windows Game Bar) into `spec/raw/scenarios/NN.mp4`
- save the model after the last step as `tests/corpus/2026/scenario_NN.skp`
- write down anything that differs from the "Expected" section of the scenario

## 3. File format corpus

- [ ] For each scenario file, also `File > Save As` in SketchUp 2017 format into `tests/corpus/2017/` (exercises the legacy container).
