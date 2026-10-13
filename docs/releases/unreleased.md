# Unreleased

Changes since 0.1.2, grouped by area (each area under its own `##` heading).

## Fixes

- Damaged or hostile files no longer crash, freeze or exhaust memory when opened or imported: every reader was fuzzed (`.skp`, IFC/STEP, DXF, glTF/GLB, OBJ, STL, PLY, FBX, 3MF, 3DS, `.style`, `.mtx`, texture images) and what it found is fixed. Examples: NaN or huge vertex coordinates in mesh files, DXF arcs with absurd angles and dimension blocks nesting each other, glTF files with accessors of the wrong type, FBX arrays and IFCZIP/3MF archives claiming gigabytes, IFC Boolean results naming the same solid at every level, FBX meshes and 3MF components copied millions of times, PLY elements with billions of empty records. Such files now give the usual "Could not open/import" message (or import without the broken parts). See `spec/FUZZING.md`.
- Images (textures, photos, watermarks, thumbnails) larger than 8192 × 8192 pixels are refused with an error instead of being decoded; a file only claiming such a size can no longer allocate gigabytes.
- Should a reader still fail unexpectedly, the error dialog explains it and the open model stays as it was, instead of the app closing.
