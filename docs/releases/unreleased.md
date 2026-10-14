# Unreleased

Changes since 0.1.3, grouped by area (each area under its own `##` heading).

## Saving to .skp keeps more

- Texts and dimensions inside groups and components are saved in the .skp (they were left out), with their tags, and so are screen texts (anywhere; they were dropped before).

## Panels
- Materials: SketchUp `.skm` material files are read (Details > Open Material..., and in material collections) and written (Save As... in the Edit tab and a swatch's right-click menu). The layout follows SketchUp 8's bundled files; newer files are unverified.
- Materials, Components and Styles: Details > Add Collection to Favorites / Remove Collection from Favorites; favorites are listed first in the collection dropdown and kept in `favorites.txt` in the config folder.
- Components: View Options with Small Thumbnails, Large Thumbnails (a grid) or List, for the model's components and collections.
- Components > Statistics: count the selected component, All Geometry (the whole model) or the selection, with Expand counting nested contents.
- Styles: Details > Save As writes SketchUp's zip `.style` (read back by our reader; unverified in SketchUp) or our text `.style`, which now keeps watermarks.
