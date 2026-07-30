#!/usr/bin/env bash
# Render assets/logo/*.png from the favicon SVG. The favicon is the single
# source of truth; everything under assets/logo/ is derived — never edit the
# PNGs by hand. Rerun after any favicon change.
set -euo pipefail
cd "$(dirname "$0")/../.."

command -v rsvg-convert >/dev/null 2>&1 || {
  echo "rsvg-convert not found (brew install librsvg)" >&2
  exit 1
}

src=apps/playground/favicon.svg
out=assets/logo
mkdir -p "$out"

for size in 1024 512 256 128; do
  rsvg-convert -w "$size" -h "$size" "$src" -o "$out/logo-$size.png"
  echo "rendered $out/logo-$size.png"
done
