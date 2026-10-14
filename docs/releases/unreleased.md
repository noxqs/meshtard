# Unreleased

Changes since 0.1.4, grouped by area (each area under its own `##` heading).

## Performance

- Groups and components with translucent materials (windows, glass) and edges drawn with the Jitter style are now drawn with GPU instancing too, with the same back-to-front transparency as before. 5,000 placed windows: 88 MB of GPU buffers down to 2.4 MB, uploads after each edit about 25x faster.
