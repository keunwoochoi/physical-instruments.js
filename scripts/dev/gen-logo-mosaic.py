#!/usr/bin/env python3
"""Generate assets/logo/logo.svg — a rounded-square-tile mosaic of the favicon.

The favicon SVG stays the single owner of the mark; this script derives the
logo from it by sampling a 24x24 grid, snapping every tile to a fixed
two-tone amber palette (plus a lifted plate tone), then applying pixel-artist
cleanup passes. The favicon's carved shaft and barbs are thinner than one
tile, so raw sampling shatters the vane into ragged islands; the passes close
those accidental holes, re-carve the shaft as one deliberate dark staircase
along the favicon's actual shaft segment, and drop stray orphan tiles.

The favicon composes the feather for a tiny square with generous air; at
logo size that air reads as wasted space, so sampling looks at the favicon
through a fit transform (FIT_SCALE about FIT_CENTER) that enlarges and
re-centers the mark on the plate. Tile presence still follows the unzoomed
favicon alpha, so the plate silhouette is unchanged by the zoom.

Requires rsvg-convert (brew install librsvg) and ImageMagick (brew install
imagemagick). Stdlib-only Python.
"""
import math
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FAVICON = ROOT / "apps/playground/favicon.svg"          # source mark (input)
OUT = ROOT / "assets/logo/logo.svg"
FAVICON_OUT = ROOT / "apps/playground/favicon-pixel.svg"  # generated pixel-art favicon

GRID = 24            # tiles across
TILE_FRAC = 0.80     # tile side as a fraction of grid pitch (rest is grout)
CORNER_FRAC = 0.26   # tile corner radius as a fraction of tile side
PLATE_TILE = (34, 40, 54)    # lifted so the tile texture reads against grout
GROUT = "#0b0d11"
LIGHT = (246, 197, 90)
DARK = (196, 138, 44)
SHAFT = (140, 95, 30)
# The favicon's rachis: path "M44 18 L 12 52" in apps/playground/favicon.svg.
SHAFT_SEG = ((44.0, 18.0), (12.0, 52.0))
SHAFT_HALF_WIDTH = 1.15      # favicon units; keeps the carved line one tile wide
# Fit transform: the feather's bounding box centers near (28.5, 35.2) in
# favicon space; show that point at plate center, enlarged 1.15x.
FIT_CENTER = (28.5, 35.2)
FIT_SCALE = 1.15


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


def zoom(cx, cy):
    """Logo-space tile center -> favicon-space sampling point."""
    return (
        FIT_CENTER[0] + (cx - 32.0) / FIT_SCALE,
        FIT_CENTER[1] + (cy - 32.0) / FIT_SCALE,
    )


def seg_dist(p, a, b):
    ax, ay = a
    bx, by = b
    vx, vy = bx - ax, by - ay
    t = max(0.0, min(1.0, ((p[0] - ax) * vx + (p[1] - ay) * vy) / (vx * vx + vy * vy)))
    return math.hypot(p[0] - (ax + t * vx), p[1] - (ay + t * vy))


def main():
    px = load_raster()
    pitch = 64.0 / GRID
    side = pitch * TILE_FRAC
    win = max(1, int(side * 0.55))

    # Pass 0 — classify: 'L'/'D' amber (hue-based, so blends with the carved
    # details still count as glyph), 'P' plate, None outside the plate.
    # Presence comes from the unzoomed favicon alpha; color from the zoomed
    # sample.
    cls = [[None] * GRID for _ in range(GRID)]
    for j in range(GRID):
        for i in range(GRID):
            cx, cy = (i + 0.5) * pitch, (j + 0.5) * pitch
            if sample(px, cx, cy, win)[3] < 150:
                continue
            fx, fy = zoom(cx, cy)
            c = sample(px, fx, fy, win)
            if c[0] - c[2] > 28:
                lum = 0.30 * c[0] + 0.59 * c[1] + 0.11 * c[2]
                cls[j][i] = "L" if lum >= 140 else "D"
            else:
                cls[j][i] = "P"

    def neighbors(j, i):
        out = []
        for dj in (-1, 0, 1):
            for di in (-1, 0, 1):
                if dj == di == 0:
                    continue
                if 0 <= j + dj < GRID and 0 <= i + di < GRID:
                    out.append(cls[j + dj][i + di])
        return out

    def close_holes(glyph):
        """Fill plate tiles mostly surrounded by glyph, to fixpoint."""
        while True:
            fill = [
                (j, i)
                for j in range(GRID)
                for i in range(GRID)
                if cls[j][i] == "P"
                and sum(1 for n in neighbors(j, i) if n in glyph) >= 5
            ]
            if not fill:
                return
            for j, i in fill:
                cls[j][i] = "D"

    # Pass 1 — close holes: sub-tile carved details punch accidental plate
    # holes through the vane; a plate tile mostly surrounded by amber joins
    # the vane as dark amber.
    close_holes(("L", "D"))

    # Pass 2 — re-carve the shaft deliberately: one clean dark staircase
    # along the favicon's rachis, instead of the ragged sampled channel.
    for j in range(GRID):
        for i in range(GRID):
            if cls[j][i] in ("L", "D"):
                if seg_dist(zoom((i + 0.5) * pitch, (j + 0.5) * pitch), *SHAFT_SEG) \
                        < SHAFT_HALF_WIDTH:
                    cls[j][i] = "S"

    # Pass 3 — close again counting the shaft as glyph: holes flanking the
    # carved line are enclosed by the vane and must not stay uncolored.
    close_holes(("L", "D", "S"))

    # Pass 4 — drop orphans: an amber tile with no amber neighbor is sampling
    # noise, not part of the mark.
    for j in range(GRID):
        for i in range(GRID):
            if cls[j][i] in ("L", "D") and not any(
                n in ("L", "D", "S") for n in neighbors(j, i)
            ):
                cls[j][i] = "P"

    colors = {"L": LIGHT, "D": DARK, "S": SHAFT, "P": PLATE_TILE}
    parts = [
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">',
        f'<rect width="64" height="64" rx="15" fill="{GROUT}"/>',
    ]
    for j in range(GRID):
        for i in range(GRID):
            if cls[j][i] is None:
                continue
            cx, cy = (i + 0.5) * pitch, (j + 0.5) * pitch
            r, g, b = colors[cls[j][i]]
            parts.append(
                f'<rect x="{cx - side / 2:.2f}" y="{cy - side / 2:.2f}" '
                f'width="{side:.2f}" height="{side:.2f}" rx="{side * CORNER_FRAC:.2f}" '
                f'fill="#{r:02x}{g:02x}{b:02x}"/>'
            )
    parts.append("</svg>")
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(parts) + "\n")
    print(f"wrote {OUT.relative_to(ROOT)} ({GRID}x{GRID} grid)")

    # The favicon: the same tile grid as true pixel art - one full-bleed hard
    # pixel per tile, no grout, no corner rounding - so the tab icon matches
    # the mosaic logo instead of being a blurry miniature of it.
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {GRID} {GRID}" '
        'shape-rendering="crispEdges">',
        f'<rect width="{GRID}" height="{GRID}" rx="{GRID * 15 / 64:.2f}" fill="{GROUT}"/>',
    ]
    for j in range(GRID):
        for i in range(GRID):
            if cls[j][i] is None:
                continue
            r, g, b = colors[cls[j][i]]
            parts.append(
                f'<rect x="{i}" y="{j}" width="1" height="1" fill="#{r:02x}{g:02x}{b:02x}"/>'
            )
    parts.append("</svg>")
    FAVICON_OUT.write_text("\n".join(parts) + "\n")
    print(f"wrote {FAVICON_OUT.relative_to(ROOT)} ({GRID}x{GRID} px)")


if __name__ == "__main__":
    sys.exit(main())
