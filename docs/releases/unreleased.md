# Unreleased

Changes since 0.1.1, grouped by area (each area under its own `##` heading).

## Performance

- Undo steps copy only what an edit changed: unchanged groups and components are shared between steps (an edit of a large imported model went from ~0.4 s of copying to ~30 ms). A huge single context is copied on several threads (half the time).
- After an edit, faces it did not change keep their triangles instead of being tessellated again (rebuilds up to ~1.5x faster on real files).
- Edit > Select All and the selection highlight are much faster on huge models (a million edges: Select All 0.7 s to 0.07 s, highlight 1.1 s to 0.25 s).
