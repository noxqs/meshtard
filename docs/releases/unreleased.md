# Unreleased

Changes since 0.1.1, grouped by area (each area under its own `##` heading).

## macOS

- **Tool modifier keys follow SketchUp for Mac.** Inside tools, Option does what Ctrl does on Windows (copy with Move, Rotate and Push/Pull, Scale about the center, Paint Bucket fill, Eraser soften, Select add) and Command does what Alt does (Move auto-fold, Offset overlap, Follow Me along the face's edge, Paint Bucket sample, Line inference cycle). Orbit, Lasso, Rectangle and the arcs take Option where Windows uses Alt, as on SketchUp's 2026 Mac reference card. The status bar names the Mac keys. Windows and Linux are unchanged.
- **File > Close (Cmd+W).** Asks to save changes, then starts a new untitled model and opens the Welcome window. SketchUp for Windows has no File > Close, so Windows and Linux do not get one.

## Tools

- **Camera > Image Igloo (I).** With two or more scenes that have a matched photo, shows all the photos at once, each where its camera took it. Drag to orbit among them; the photo nearest the middle of the view is outlined; Enter or a double-click opens its scene; the arrow keys go to the next photo on that side; Esc leaves.
- **Line tool Alt keeps cycling the inference modes.** SketchUp's newer help page (September 2026) says Alt (Command on the Mac) cycles All Inferences On / Off / Parallel and Perpendicular Only, while the older 2026 reference card says it locks the inference direction. Meshtard follows the newer page; this is still to be checked against SketchUp itself.

## Model Library

- **Two Meshtards can share the download cache.** When the cache is trimmed to its size limit, a model that another running Meshtard is downloading or placing is no longer removed. Locks left behind by a Meshtard that crashed expire after two minutes.

## Panels

- Styles: SketchUp's own `.style` files (zip archives) open with Details > Open... and show in style collections: their style items, watermarks and thumbnail are read (the archive layout is inferred, unverified). Our own text `.style` files keep working; the two are told apart by content, and Save As still writes ours.
- Styles: thumbnails in the Select list, collections and the Mix tab, rendered from a small sample block with each style (or the `.style` file's own image).
- Styles Mix tab: drag a style from the list onto Edge, Face, Background, Watermark or Modeling Settings to take that group; picking a style and clicking a group still works.
- Materials: textured materials show their texture, on a cube or flat by the Auto / Cube / Flat thumbnail setting (collection images too). Images are decoded in the background.
- Components: rendered thumbnails for the model's components (again after each change) and for `.skp` files in collections (read in the background), one per frame while the panel is open.

## Performance

- Opening big `.skp` files is about 4x faster (a million-edge model: 23 s → 6 s; a 5,000-group model: 0.7 s → 0.35 s; files from SketchUp 2021 and later about 3x faster to read). The model read is exactly the same as before.
- Saving `.skp` is up to 1.7x faster (a million-edge model: 3.7 s → 2.4 s), with the same bytes written.
- Opening a file without a saved view no longer builds the drawn scene twice.
