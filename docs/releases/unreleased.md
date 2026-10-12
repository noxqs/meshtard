# Unreleased

Changes since 0.1.0, grouped by area.

## IFC

- IFC import reads more solids: revolved solids, profiles swept along curves (surface curve and fixed reference sweeps), hollow and closed pipes, CSG primitives (blocks, cylinders, cones, spheres, pyramids) with unions and intersections done by the Solid Tools, and advanced breps with flat and curved (B-spline, cylinder, cone, sphere, torus) faces.
- IFC import brings in textures (embedded, pixel and image-file textures on triangulated geometry) and the site's location: latitude and longitude, north and map coordinates become the model's geo-location when it has none (Model Info > Geo-location shows the map origin). Sites far from the origin are moved near it, the offset kept in the geo-location.
- Furniture and other objects placed in rooms no longer come in hidden with the room.
- IFC export writes real element classes (walls, slabs, doors, windows, columns and more) from the Type an IFC import set, keeps imported names and GlobalIds, writes one storey per storey group, a `Reference` property and net volumes, and the site's location and map conversion.
