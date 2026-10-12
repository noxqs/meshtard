# Unreleased

Changes since 0.1.1, grouped by area (each area under its own `##` heading).

## Performance

- Opening big `.skp` files is about 4x faster (a million-edge model: 23 s → 6 s; a 5,000-group model: 0.7 s → 0.35 s; files from SketchUp 2021 and later about 3x faster to read). The model read is exactly the same as before.
- Saving `.skp` is up to 1.7x faster (a million-edge model: 3.7 s → 2.4 s), with the same bytes written.
- Opening a file without a saved view no longer builds the drawn scene twice.
