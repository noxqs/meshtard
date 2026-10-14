# Unreleased

Changes since 0.1.4, grouped by area (each area under its own `##` heading).

## Performance

- Groups and components with translucent materials (windows, glass) and edges drawn with the Jitter style are now drawn with GPU instancing too, with the same back-to-front transparency as before. 5,000 placed windows: 88 MB of GPU buffers down to 2.4 MB, uploads after each edit about 25x faster.

## Windows

- No console window opens next to Meshtard any more (release builds are Windows GUI programs).

## Fixes

- Export: OBJ, STL, PLY, 3MF, FBX and 3DS now default to the unit their import assumes, so a model exported and imported back keeps its size (STL came back 25.4x too small).
- Import: a glTF model is held by the low corner of its bounding box while placing it, so it lands where you click instead of as far away as it was from its own origin.
