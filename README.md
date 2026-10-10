# meshtard

A SketchUp 2026 clone in Rust. See [PLAN.md](PLAN.md) for the roadmap and [spec/](spec/) for what is copied from SketchUp.

## Build and run (Linux)

Needs Rust 1.90+ and a Vulkan driver.

```sh
cargo run --release -p mt-app
```

`MESHTARD_SPEC_DIR=spec cargo run -p mt-app` reads menus and shortcuts from `spec/` at startup instead of the copy built into the binary, so spec edits show without rebuilding.

## Test

```sh
cargo test --workspace
```

The render tests in `crates/mt-render/tests/headless.rs` need a Vulkan adapter. Without a GPU, install Mesa's software driver (`apt install mesa-vulkan-drivers`). They print `SKIPPED` when no adapter exists. The rendered image is saved to `target/tmp/default_view.png`.

## Layout

| Crate | Role |
|---|---|
| `mt-spec` | Loads `spec/*.toml` (menus, keymap, default model) and checks consistency |
| `mt-render` | Camera and wgpu viewport renderer |
| `mt-app` | The application: window, menus, navigation |

## Credits

`.skp` support will be a Rust port of [OpenSKP](https://github.com/iamahsanmehmood/openskp) (MIT, Copyright (c) 2024-2026 Ahsan Mehmood).
