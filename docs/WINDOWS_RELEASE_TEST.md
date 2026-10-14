# Windows test per major release

At every major release the user boots into Windows and starts a Claude Remote Control session there. The Meshtard development session sends it this checklist; it reports back the results.

1. Download `meshtard-windows-x64.zip` of the new release from https://github.com/noxqs/meshtard/releases and unpack it.
2. `meshtard.exe --version` prints the new version.
3. Start `meshtard.exe`: the window opens, the splash shows, no error dialog. Note the Windows version and GPU.
4. Open `tests/corpus/openskp/gondola_v20.skp` (download from the repo): it opens, note the time.
5. Draw a rectangle, Push/Pull it, Move an edge, Undo twice, Redo twice; Orbit (O, and Alt for no gravity), Pan, Zoom; Save As a new .skp and reopen it.
6. Import an OBJ and an IFC sample; export glTF and STL.
7. Window > Preferences opens; Help > Keyboard Shortcuts lists the shortcuts.
8. Report anything broken with a screenshot. If SketchUp 2026 is installed, also open the .skp files Meshtard saved in SketchUp and report whether they open correctly.

The SketchUp trial capture (`tools/trial/README.md`) is a one-off, not part of every release.
