# Unreleased

Changes since 0.1.2, grouped by area (each area under its own `##` heading).

## Performance

- Picking groups and components placed many times no longer puts every copy in world
  space: the pick index and the edge culling look into each definition's mesh in its
  own coordinates, with the same answers as before. With 10,000 placed boxes a rebuild
  after an edit is about 3.5x faster (scene 20 ms to 6 ms, pick index 27 ms to 4 ms,
  first hover 22 ms to 5 ms) and the scene's picking lists take 10 MB instead of 28 MB;
  the FIAT car's pick index builds 3.3x faster.
- Undo after a big edit was measured again on a million-edge grid: 5 to 10 ms when
  repeated, single runs anywhere from 5 to 60 ms on a shared machine (the 50 ms
  reported before was one of those); an undo right after the edit costs 20 to 30 ms,
  mostly saving the edited state for Redo. Unchanged; details in spec/PERFORMANCE.md.
