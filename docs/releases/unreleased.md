# Unreleased

## Performance

- Hovering, picking and inference snapping no longer slow down with model size: only the edges and points near the cursor are tested (a 1-million-edge model went from about 0.3 s per mouse move to well under a millisecond).
- Moving, rotating or scaling part of a large model draws its live preview much faster (stretching connected geometry was quadratic in the size of the selection).
- Rebuilding the view after an edit is faster: components placed many times are tessellated once, simple faces skip the general tessellation work, and the pick index builds faster.
- Select All and the selection highlight are faster on large models.
- A benchmark of large models (synthetic and real files) with before/after numbers is in `spec/PERFORMANCE.md`.
