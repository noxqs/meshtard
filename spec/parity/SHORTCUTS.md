# SketchUp 2026 (Windows) default keyboard shortcuts

Researched 2026-10-12. Marks: **[doc]** stated by an official SketchUp page or PDF for
2026; **[inf]** from a secondary source (an older Preferences export or forum reports),
not confirmed for 2026. Meshtard's defaults are in `spec/keymap.toml` (`verified = false`
until checked in the real app: Window > Preferences > Shortcuts > Export).

Sources:

- **HELP**: Default Keyboard Shortcuts, https://help.sketchup.com/en/default-keyboard-shortcuts (updated Sep 2026) [doc]
- **QRC**: Quick Reference Card 2026, Windows, https://download.sketchup.com/QRC2026/HC-QRC2026-en-SU-windows.pdf (linked from https://help.sketchup.com/en/quick-reference-cards) [doc]
- **SEL**: Selecting geometry, https://help.sketchup.com/en/sketchup/selecting-geometry (Ctrl+A, Ctrl+T, Shift+Space) [doc]
- **EXP**: "All Default Keyboard Shortcuts, Sketchup Make for Windows", a Preferences > Shortcuts export attached to https://forums.sketchup.com/t/keyboard-shortcuts-sketchup-for-windows-complete/36923 (2017) [inf]
- **FQ**: https://forums.sketchup.com/t/sketchup-2026-can-i-change-the-exit-shortcut-key-ctrl-q/341060 and https://forums.sketchup.com/t/sketchup-2026-shortcut-key-bug/342241 (users of 2026.0/2026.1 for Windows: Ctrl+Q is Exit and Ctrl+, opens Preferences by default and cannot be changed) [inf]
- **INV**: SketchUp 2019.2 feature-update articles (Invert Selection added with Ctrl+Shift+I on Windows; the official 2019.2 notes are offline) [inf]
- **RN**: release notes 2026.0, https://help.sketchup.com/en/release-notes/sketchup-desktop-20260: Purge Unused is in the Edit menu "and can be assigned a shortcut"; a shortcut can be assigned to show the Environment panel. Neither has a default key [doc].
- **EDIT**: https://help.sketchup.com/en/creating-and-editing-shortcuts and https://help.sketchup.com/en/importing-and-exporting-shortcuts (the Preferences page; see the last section) [doc]

The 2024-2026 release notes name no other new default key. The QRC shows Make Component
without a key while HELP gives G; HELP is taken.

Status: **bound** (same key in Meshtard), **bound differently**, **command exists
unbound**, **command missing**.

## Keys

| Key | Command | SketchUp menu path | Meshtard id | Source | Status |
|---|---|---|---|---|---|
| Space | Select | Tools/Select | `tool.select` | HELP, QRC [doc] | bound |
| Shift+Space | Lasso Select | Tools/Lasso | `tool.lasso` | HELP, SEL [doc] | bound |
| B | Paint Bucket | Tools/Paint Bucket | `tool.paint_bucket` | HELP, QRC [doc] | bound |
| E | Eraser | Tools/Eraser | `tool.eraser` | HELP, QRC [doc] | bound |
| L | Line | Draw/Lines/Line | `tool.line` | HELP, QRC [doc] | bound |
| R | Rectangle | Draw/Shapes/Rectangle | `tool.rectangle` | HELP, QRC [doc] | bound |
| C | Circle | Draw/Shapes/Circle | `tool.circle` | HELP, QRC [doc] | bound |
| A | 2 Point Arc | Draw/Arcs/2 Point Arc | `tool.arc_2pt` | HELP, QRC [doc] | bound |
| M | Move | Tools/Move | `tool.move` | HELP, QRC [doc] | bound |
| P | Push/Pull | Tools/Push/Pull | `tool.push_pull` | HELP, QRC [doc] | bound |
| Q | Rotate | Tools/Rotate | `tool.rotate` | HELP, QRC [doc] | bound |
| S | Scale | Tools/Scale | `tool.scale` | HELP, QRC [doc] | bound |
| F | Offset | Tools/Offset | `tool.offset` | HELP, QRC [doc] | bound |
| T | Tape Measure | Tools/Tape Measure | `tool.tape_measure` | HELP, QRC [doc] | bound |
| O | Orbit | Camera/Orbit | `camera.orbit` | HELP, QRC [doc] | bound |
| H | Pan | Camera/Pan | `camera.pan` | HELP, QRC [doc] | bound |
| Z | Zoom | Camera/Zoom | `camera.zoom` | HELP, QRC [doc] | bound |
| Shift+Z | Zoom Extents | Camera/Zoom Extents | `camera.zoom_extents` | HELP [doc], EXP | bound |
| Ctrl+Shift+E | Zoom Extents (second key) | Camera/Zoom Extents | `camera.zoom_extents` | EXP [inf] | bound differently (one key per command; FEATURES 75) |
| Ctrl+Shift+W | Zoom Window | Camera/Zoom Window | `camera.zoom_window` | EXP [inf] | bound |
| G | Make Component | Edit/Make Component | `edit.make_component` | HELP [doc] | bound |
| I | Image Igloo | Camera/Image Igloo | - | HELP [doc], EXP | command missing (FEATURES 73) |
| K | Back Edges | View/Edge Style/Back Edges | `view.back_edges` | HELP [doc], EXP | bound (was unbound before 2026-10-12) |
| Shift+S | Search | Help/Search | `help.search` | HELP, QRC [doc] | bound |
| Ctrl+N | New | File/New | `file.new` | EXP [inf] | bound |
| Ctrl+O | Open | File/Open | `file.open` | EXP [inf] | bound |
| Ctrl+S | Save | File/Save | `file.save` | EXP [inf] | bound |
| Ctrl+P | Print | File/Print | `file.print` | EXP [inf] | bound |
| Ctrl+Q | Exit | File/Exit | `file.exit` | FQ [inf] | bound (added 2026-10-12) |
| Ctrl+, | Preferences | Window/Preferences | `window.preferences` | FQ [inf] | bound (added 2026-10-12, keymap name `Ctrl+Comma`) |
| Ctrl+Z | Undo | Edit/Undo | `edit.undo` | EXP [inf] | bound |
| Alt+Backspace | Undo (second key) | Edit/Undo | `edit.undo` | EXP [inf] | bound differently (one key per command; FEATURES 75) |
| Ctrl+Y | Redo | Edit/Redo | `edit.redo` | EXP [inf] | bound |
| Ctrl+X | Cut | Edit/Cut | `edit.cut` | EXP [inf] | bound |
| Shift+Delete | Cut (second key) | Edit/Cut | `edit.cut` | EXP [inf] | bound differently (FEATURES 75) |
| Ctrl+C | Copy | Edit/Copy | `edit.copy` | EXP [inf] | bound |
| Ctrl+Insert | Copy (second key) | Edit/Copy | `edit.copy` | EXP [inf] | bound differently (FEATURES 75) |
| Ctrl+V | Paste | Edit/Paste | `edit.paste` | EXP [inf] | bound |
| Shift+Insert | Paste (second key) | Edit/Paste | `edit.paste` | EXP [inf] | bound differently (FEATURES 75) |
| Delete | Delete | Edit/Delete | `edit.delete` | EXP [inf] | bound |
| Ctrl+A | Select All | Edit/Select All | `edit.select_all` | SEL [doc], EXP | bound |
| Ctrl+T | Select None (Deselect All) | Edit/Select None | `edit.select_none` | SEL [doc], EXP | bound |
| Ctrl+Shift+I | Invert Selection | Edit/Invert Selection | `edit.invert_selection` | INV [inf] | bound |
| PageUp | Previous Scene | View/Animation/Previous Scene | `view.anim_previous` | EXP [inf] | bound |
| PageDown | Next Scene | View/Animation/Next Scene | `view.anim_next` | EXP [inf] | bound |
| Shift+F1 | Context Help | Shortcuts list "Sketchup/Context Help" (no menu item) | - | EXP [inf] | command missing (FEATURES 74) |
| (none) | Purge Unused | Edit/Purge Unused | `edit.purge_unused` | RN [doc] | command exists unbound (same as SketchUp) |
| (none) | Environments panel | Window/Environments | `window.environments` | RN [doc] | command exists unbound (same as SketchUp) |

Totals: 48 rows, 46 default keys: 39 keys of 39 commands Meshtard has, all bound
in `keymap.toml` (36 before 2026-10-12, plus K, Ctrl+Q, Ctrl+,); 5 second keys
Meshtard cannot hold (one key per command); 2 keys of missing commands (Image Igloo,
Context Help). The last two rows are commands with no default key.

## Mouse (QRC [doc])

| Input | Action | Status |
|---|---|---|
| Wheel scroll | Zoom | bound |
| Middle drag | Orbit | bound |
| Shift + middle drag | Pan | bound |
| Middle double-click | Re-center view | bound |

## Keys inside tools (QRC, Windows [doc])

The QRC lists these; "present" means Meshtard's tool does the same (STATUS.md rows,
unverified against the real app), "differs" means another key does it.

| Tool | Key / input | Operation | Meshtard |
|---|---|---|---|
| 2 Point Arc | number Enter / number R Enter / number S Enter | bulge / radius / segments | present |
| 3 Point Arc | Alt + / Alt - | segments up / down | differs: Ctrl+plus / minus (FEATURES 76) |
| drawing tools | Arrows | lock direction (up blue, right red, left green, down parallel/perpendicular) | present |
| Circle | Shift | lock current inference | present |
| Circle | number Enter / number S Enter | radius / segments | present |
| Eraser | Ctrl / Shift / Alt | soften / hide / unsoften | differs: unsoften is Ctrl+Shift (FEATURES 76) |
| Field of View | drag, typed value | change field of view | present |
| Follow Me | Alt | face perimeter as path | missing (FEATURES 76) |
| Flip | Ctrl | toggle flip / copy | present |
| Lasso | Shift / Alt / Shift+Alt | add-subtract / add / subtract | differs: Ctrl / Shift / Ctrl+Shift as Select (FEATURES 76) |
| Line | Alt | lock current inference direction | differs: Alt cycles inference modes per the 2026 drawing-basics help page; the two official pages disagree (check in the trial) |
| Line | number Enter | length | present |
| Look Around | number Enter | eye height | present |
| Move | Ctrl / Shift / Alt | copy / lock direction / auto-fold | present |
| Move | number X Enter, number / Enter | external / internal copy array | present |
| Offset | Alt | allow overlap | present |
| Orbit | Alt held | suspend gravity-weighted orbit | differs: Ctrl (FEATURES 76) |
| Orbit | Shift held | Pan | present |
| Paint Bucket | Ctrl / Shift / Ctrl+Shift / Alt | fill adjacent / replace in model / replace on object / sample | present |
| Protractor | Ctrl | toggle guide creation | missing (FEATURES 76) |
| Protractor | Arrows | lock rotation plane | present |
| Push/Pull | Ctrl / double-click / number Enter | copy face / repeat last / distance | present |
| Rectangle | Alt | draw from center | missing (FEATURES 76) |
| Rectangle | w,h Enter | dimensions | present |
| Rotated Rectangle | Shift / Alt | lock direction-plane / lock plane for first edge | not checked |
| Rotate | Ctrl / number Enter / rise:run Enter | copy / angle / slope | present |
| Scale | Ctrl / Shift / number Enter / length Enter | about center / uniform / factor / length | present |
| Search | Shift+S | search commands | present |
| Select | Ctrl / Shift / Ctrl+Shift | add / add-subtract / subtract | present |
| Tape Measure | Ctrl / Arrows / length Enter | guide or measure / lock / resize model | present |
| Zoom | Shift + drag | change field of view | present |

## Preferences > Shortcuts (EDIT [doc])

SketchUp for Windows: a Filter box over the Function list (e.g. "zoom" leaves
Camera/Zoom, Camera/Zoom Extents, Camera/Zoom to Photo, Camera/Zoom Window); the key
under Assigned; an Add Shortcut box and + to add; a warning with Keep Original / Replace
when the key belongs to another function; - to remove; Reset All; Import and Export
(a `Preferences.dat` file, Windows only). Modifiers Shift, Alt and Ctrl can be combined;
keys reserved by the OS are refused.

Meshtard (`crates/mt-app/src/preferences.rs`, `shortcut_list.rs`): Filter (function,
id or key); Function / Assigned list of every menu-bar, right-click and extension
command by menu path; the key is recorded by pressing it; Keep Original / Replace on a
conflict; Remove; Reset All; Import... / Export... (the preferences TOML with the
shortcuts); Show All... opens Help > Keyboard Shortcuts, a list grouped by menu with
Copy and Save as text (Meshtard's own; SketchUp has no such list).
