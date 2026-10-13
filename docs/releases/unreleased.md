# Unreleased

Changes since 0.1.2, grouped by area (each area under its own `##` heading).

## Downloads

| File | Platform | Notes |
|---|---|---|
| `meshtard-linux-x64.tar.gz` | Linux x64 (Vulkan or GL) | Needs a Vulkan or OpenGL driver; file dialogs use the XDG desktop portal. Unpack and run `./meshtard` once: it offers to install itself for your user (`~/.local/bin`, with an applications menu entry and icon), or use Help > Install for This User. |

## Linux

- Install for This User: started from anywhere else, Meshtard offers to install itself in `~/.local/bin` with its icon, an applications menu entry and the `.skp` file type (Install / Not now / Don't ask again), and offers again when a newer version is started; also in Help > Install for This User. Window > Preferences > General turns the question back on and uninstalls (only the files the install wrote). `meshtard --version` prints the version.
