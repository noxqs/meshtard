# Unreleased

Changes since 0.1.4, grouped by area (each area under its own `##` heading).

## Performance

- Groups and components with translucent materials (windows, glass) and edges drawn with the Jitter style are now drawn with GPU instancing too, with the same back-to-front transparency as before. 5,000 placed windows: 88 MB of GPU buffers down to 2.4 MB, uploads after each edit about 25x faster.

## Windows

- No console window opens next to Meshtard any more (release builds are Windows GUI programs).

## Fixes

- Export: OBJ, STL, PLY, 3MF, FBX and 3DS now default to the unit their import assumes, so a model exported and imported back keeps its size (STL came back 25.4x too small).
- Import: a glTF model is held by the low corner of its bounding box while placing it, so it lands where you click instead of as far away as it was from its own origin.
## Tools

- **Image Igloo:** Ctrl + arrow keys (Option + arrows on the Mac) turn among the photos taken from about the same spot, such as one station's panorama; Shift + arrow keys go to the next or previous scene of a photo series (scenes named alike apart from a trailing number). Matched photos are still not saved in .skp files.
## Extensions

- Signed extensions: packages and Warehouse catalogs can carry ed25519 signatures. The Extension Manager, the Warehouse and the install prompt say "Signed by ..." or "Unsigned"; installing an unsigned extension asks first ("Install Anyway"), and a package or catalog whose signature does not match is refused. Add the keys you trust in Extension Manager > Trusted keys. Authors sign with `meshtard --extension-keygen` and `meshtard --extension-sign`.
- Update notifications: the Extension Warehouse lists installed extensions that have a newer version, with an Update button. An optional "Check for updates when Meshtard starts" setting (off by default) does the same at startup.
- Extensions can keep attribute dictionaries on groups and components (`get_attribute`, `set_attribute`, `delete_attribute`); they are saved in the .skp file, and attribute dictionaries of opened .skp files are now kept when saving.
- Extension tools can have their own mouse cursor (a PNG with a hotspot).
- A single permission of an installed extension can be taken back (or approved) in the Extension Manager without reinstalling it.
