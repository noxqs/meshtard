#!/usr/bin/env python3
"""Builds every app icon from one source image on a plain light background.

    python3 tools/make_icons.py <source image>

The background is made transparent by a flood fill from the image border, so
light areas enclosed by the outline (the wireframe half) stay opaque. Writes:
  crates/mt-app/assets/icon.png                     window icon (256 px)
  packaging/linux/icons/hicolor/<n>x<n>/apps/meshtard.png
  packaging/windows/meshtard.ico                    16-256 px
  packaging/macos/meshtard.icns                     16-1024 px
"""
import sys
from collections import deque
from pathlib import Path

from PIL import Image, ImageFilter

ROOT = Path(__file__).resolve().parent.parent
TOL = 40  # how far from the corner colour still counts as background


def transparent(img):
    img = img.convert("RGBA")
    w, h = img.size
    px = img.load()
    bg = px[0, 0][:3]
    near = lambda c: sum(abs(a - b) for a, b in zip(c[:3], bg)) <= TOL
    seen = bytearray(w * h)
    q = deque()
    for x in range(w):
        q.extend([(x, 0), (x, h - 1)])
    for y in range(h):
        q.extend([(0, y), (w - 1, y)])
    while q:
        x, y = q.popleft()
        i = y * w + x
        if seen[i] or not near(px[x, y]):
            continue
        seen[i] = 1
        if x > 0: q.append((x - 1, y))
        if x < w - 1: q.append((x + 1, y))
        if y > 0: q.append((x, y - 1))
        if y < h - 1: q.append((x, y + 1))
    mask = Image.frombytes("L", (w, h), bytes(0 if s else 255 for s in seen))
    # Soften the cut edge a little.
    mask = mask.filter(ImageFilter.MinFilter(3)).filter(ImageFilter.GaussianBlur(0.8))
    img.putalpha(mask)
    return img


def square(img):
    img = img.crop(img.getbbox())
    side = max(img.size)
    side += side // 32  # small margin
    out = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    out.paste(img, ((side - img.width) // 2, (side - img.height) // 2), img)
    return out


def main():
    icon = square(transparent(Image.open(sys.argv[1])))
    size = lambda n: icon.resize((n, n), Image.LANCZOS)
    size(256).save(ROOT / "crates/mt-app/assets/icon.png", optimize=True)
    for n in (16, 24, 32, 48, 64, 128, 256, 512):
        d = ROOT / f"packaging/linux/icons/hicolor/{n}x{n}/apps"
        d.mkdir(parents=True, exist_ok=True)
        size(n).save(d / "meshtard.png", optimize=True)
    size(256).save(ROOT / "packaging/windows/meshtard.ico",
                   sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)])
    size(1024).save(ROOT / "packaging/macos/meshtard.icns")


if __name__ == "__main__":
    main()
