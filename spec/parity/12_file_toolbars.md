# 12 File open and save, older versions, toolbars (M3, M8)

Bring on a USB stick: the meshtard Windows release build and every `.skp` that meshtard saved (`tests/corpus/meshtard/*.skp` if present, otherwise save a few with meshtard before the trial).

| Step | Action | Record |
|---|---|---|
| 1 | In SketchUp, File > Open each meshtard-saved file | Opens without error? Any repair prompt? Counts match (Window > Model Info > Statistics)? Arcs shown as arcs or curves? |
| 2 | For one meshtard file, Window > Model Info > Statistics, then Fix Problems | Problems reported. Screenshot. |
| 3 | File > Save As: look at the "Save as type" list | Which versions are offered (2017 to 2025?). Screenshot. |
| 4 | Make a small change, check the title bar | Unsaved marker (asterisk?) and position. |
| 5 | File > New with unsaved changes | Prompt text and buttons. |
| 6 | Drag a `.skp` from Explorer onto the window | Opens it, or imports it as a component? |
| 7 | File > Save A Copy As | Does the title keep the old file name? |
| 8 | View > Toolbars... | Dialog list of toolbars and checkboxes. Screenshot. |
| 9 | Tick Large Tool Set, drag it by its grip to the left edge, then to the right edge, then float it | Docking zones and how they highlight. |
| 10 | Right-click any toolbar | Context menu contents. |
| 11 | Close and restart SketchUp | Are toolbar positions kept? |
| 12 | Save as `tests/corpus/2026/scenario_12.skp`. Then `File > Save As`, type "SketchUp 2017", file `tests/corpus/2017/scenario_12.skp` | |

## Expected (meshtard, current implementation)

- meshtard writes the SketchUp 2017 format. Camera view, scenes, styles, shadows, units, active section and guides inside components are not saved yet; arcs reopen as polyline curves.
- File > New, Save (Ctrl+S), Save As, Save A Copy As; asterisk in the title for unsaved changes; "Save changes?" prompt on New, Open, dropped files and closing.
- A dropped `.skp` opens as a new model in meshtard. What does SketchUp do?
- Toolbars dock to any edge or float, View > Toolbars... and right-click show or hide them, and the layout is kept between runs.
