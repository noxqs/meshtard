# Unreleased

Changes since 0.1.1, grouped by area (each area under its own `##` heading).

## Performance

- Groups and components placed many times are drawn with GPU instancing: one copy of each definition's faces and edges on the GPU, drawn once per instance, instead of a copy per instance. 10,000 instances of a component: 43 MB of GPU buffers down to 4.5 MB, uploads after each edit 15x faster; real files with repeated components 2-3x less GPU memory. What is drawn is unchanged.
- Undo steps now copy only the parts of a model an edit changed, even inside one huge group or the model root: after a small edit of a million-edge model, the undo step takes ~1 ms instead of ~0.14 s, and Undo ~1 ms instead of ~0.13 s. Edits on imported models no longer pause to compare the whole model first (Move on real files 5 to 50x faster).

## Fixes

- Undo after a Move, Rotate or Scale drew the restored geometry wrong: edges that met the moved entities were still drawn stretched to where the operation had put them (a box "half restored" after undoing a moved edge, then a rectangle "with the moved corner"). The model itself was restored correctly; the drawing now is too. Edit > Undo during a running Move also puts back what its preview hid.
