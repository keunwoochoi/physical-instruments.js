#!/usr/bin/env bash
# Regenerate everything under assets/logo/ from the favicon SVG. The favicon
# is the single source of truth; gen-logo-mosaic.py derives the tile-mosaic
# logo.svg from it, and the PNGs are rendered from that. Never edit any of it
# by hand. Rerun after any favicon change.
set -euo pipefail
cd "$(dirname "$0")/../.."

command -v rsvg-convert >/dev/null 2>&1 || {
  echo "rsvg-convert not found (brew install librsvg)" >&2
  exit 1
}
command -v magick >/dev/null 2>&1 || {
  echo "magick not found (brew install imagemagick)" >&2
  exit 1
}

python3 scripts/dev/gen-logo-mosaic.py

out=assets/logo
for size in 1024 512 256 128; do
  rsvg-convert -w "$size" -h "$size" "$out/logo.svg" -o "$out/logo-$size.png"
  echo "rendered $out/logo-$size.png"
done
