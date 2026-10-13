# Unreleased

Changes since 0.1.1, grouped by area (each area under its own `##` heading).

## Performance

- Groups and components placed many times are drawn with GPU instancing: one copy of each definition's faces and edges on the GPU, drawn once per instance, instead of a copy per instance. 10,000 instances of a component: 43 MB of GPU buffers down to 4.5 MB, uploads after each edit 15x faster; real files with repeated components 2-3x less GPU memory. What is drawn is unchanged.
