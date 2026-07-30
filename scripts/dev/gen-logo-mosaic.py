#!/usr/bin/env python3
"""Generate assets/logo/logo.svg — a rounded-square-tile mosaic of the favicon.

The favicon SVG stays the single owner of the mark; this script derives the
logo from it by sampling a 24x24 grid and snapping every tile to a fixed
two-tone amber palette (plus a lifted plate tone), so the logo reads as
deliberate pixel art rather than a blurry downscale.

Requires rsvg-convert (brew install librsvg) and ImageMagick (brew install
imagemagick). Stdlib-only Python.
"""
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FAVICON = ROOT / "apps/playground/favicon.svg"
OUT = ROOT / "assets/logo/logo.svg"

GRID = 24            # tiles across
TILE_FRAC = 0.80     # tile side as a fraction of grid pitch (rest is grout)
CORNER_FRAC = 0.26   # tile corner radius as a fraction of tile side
PLATE = (18, 21, 28)         # favicon plate #12151c
PLATE_TILE = (34, 40, 54)    # lifted so the tile texture reads against grout
GROUT = "#0b0d11"


def snap(c):
    """Fixed palette: amber-ish hues -> light/dark amber, else plate tone."""
    if c[0] - c[2] > 28:
        lum = 0.30 * c[0] + 0.59 * c[1] + 0.11 * c[2]
        return (246, 197, 90) if lum >= 140 else (196, 138, 44)
    return PLATE_TILE


def load_raster():
    """Favicon -> 128x128 RGBA grid via rsvg-convert + magick txt dump."""
    with tempfile.NamedTemporaryFile(suffix=".png") as tmp:
        subprocess.run(
            ["rsvg-convert", "-w", "512", "-h", "512", str(FAVICON), "-o", tmp.name],
            check=True,
        )
        txt = subprocess.run(
            ["magick", tmp.name, "-resize", "128x128", "-depth", "8", "txt:-"],
            check=True, capture_output=True, text=True,
        ).stdout
    px = [[(0, 0, 0, 0)] * 128 for _ in range(128)]
    pat = re.compile(r"^(\d+),(\d+):\s+\((\d+),(\d+),(\d+),(\d+)\)")
    for line in txt.splitlines():
        m = pat.match(line)
        if m:
            x, y, r, g, b, a = map(int, m.groups())
            px[y][x] = (r, g, b, a)
    return px


def sample(px, fx, fy, win):
    """Alpha-weighted mean around favicon-space coords (0..64; raster 2px/unit)."""
    cx, cy = int(round(fx * 2)), int(round(fy * 2))
    rs = gs = bs = as_ = n = 0
    for dy in range(-win, win + 1):
        for dx in range(-win, win + 1):
            x, y = cx + dx, cy + dy
            if 0 <= x < 128 and 0 <= y < 128:
                r, g, b, a = px[y][x]
                rs += r * a
                gs += g * a
                bs += b * a
                as_ += a
                n += 1
    if n == 0 or as_ == 0:
        return (0, 0, 0, 0)
    return (rs // as_, gs // as_, bs // as_, as_ // n)


def main():
    px = load_raster()
    pitch = 64.0 / GRID
    side = pitch * TILE_FRAC
    win = max(1, int(side * 0.55))
    parts = [
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">',
        f'<rect width="64" height="64" rx="15" fill="{GROUT}"/>',
    ]
    for j in range(GRID):
        for i in range(GRID):
            cx, cy = (i + 0.5) * pitch, (j + 0.5) * pitch
            c = sample(px, cx, cy, win)
            if c[3] < 150:
                continue  # outside the rounded plate corners
            r, g, b = snap(c)
            parts.append(
                f'<rect x="{cx - side / 2:.2f}" y="{cy - side / 2:.2f}" '
                f'width="{side:.2f}" height="{side:.2f}" rx="{side * CORNER_FRAC:.2f}" '
                f'fill="#{r:02x}{g:02x}{b:02x}"/>'
            )
    parts.append("</svg>")
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(parts) + "\n")
    print(f"wrote {OUT.relative_to(ROOT)} ({GRID}x{GRID} grid)")


if __name__ == "__main__":
    sys.exit(main())
