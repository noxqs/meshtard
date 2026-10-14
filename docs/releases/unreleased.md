# Unreleased

Changes since 0.1.3, grouped by area (each area under its own `##` heading).

## Performance

- After an edit, undo or redo only what changed is drawn again: the scene keeps the
  colours, texture coordinates and triangles of everything the edit did not touch
  (per group or component and per 256 entities), instanced components keep their
  meshes even when moved, and the pick index, snap geometry and culling indexes keep
  their unchanged blocks. Only changed vertices and edges are sent to the GPU. A small
  edit of a million-edge model rebuilds in ~60 ms instead of ~1.5 s, the FIAT car in
  ~35 ms instead of ~110 ms, the gondola in ~13 ms instead of ~23 ms.
- The first mouse move after an edit no longer builds the culling indexes from
  scratch (million-edge grid: ~10 ms instead of ~630 ms).
- Selecting a few entities (or moving them) sends only their edges to the GPU
  (million-edge grid: 0.2 ms instead of ~130 ms); Select All is drawn in full as
  before.
- Move, Rotate and Scale send only new face indices when they start and end, not every
  vertex again.
