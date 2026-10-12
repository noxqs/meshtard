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
