# Unreleased

Changes since 0.1.1, grouped by area (each area under its own `##` heading).

## Panels

- Styles: SketchUp's own `.style` files (zip archives) open with Details > Open... and show in style collections: their style items, watermarks and thumbnail are read (the archive layout is inferred, unverified). Our own text `.style` files keep working; the two are told apart by content, and Save As still writes ours.
- Styles: thumbnails in the Select list, collections and the Mix tab, rendered from a small sample block with each style (or the `.style` file's own image).
- Styles Mix tab: drag a style from the list onto Edge, Face, Background, Watermark or Modeling Settings to take that group; picking a style and clicking a group still works.
- Materials: textured materials show their texture, on a cube or flat by the Auto / Cube / Flat thumbnail setting (collection images too). Images are decoded in the background.
- Components: rendered thumbnails for the model's components (again after each change) and for `.skp` files in collections (read in the background), one per frame while the panel is open.
