#!/bin/sh
# Installs the meshtard binary, icons and desktop entry for the current user.
#   packaging/linux/install.sh [path/to/meshtard]
set -e
here=$(dirname "$0")
bin=${1:-target/release/meshtard}
prefix=${PREFIX:-$HOME/.local}
install -Dm755 "$bin" "$prefix/bin/meshtard"
for d in "$here"/icons/hicolor/*/apps; do
    size=$(basename "$(dirname "$d")")
    install -Dm644 "$d/meshtard.png" "$prefix/share/icons/hicolor/$size/apps/meshtard.png"
done
install -Dm644 "$here/meshtard.desktop" "$prefix/share/applications/meshtard.desktop"
gtk-update-icon-cache -q "$prefix/share/icons/hicolor" 2>/dev/null || true
update-desktop-database -q "$prefix/share/applications" 2>/dev/null || true
