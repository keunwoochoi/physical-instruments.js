#!/usr/bin/env bash
# Owns the public bundle-size contract (issue #46).
#
# Two failures this exists to make impossible:
#   1. A stale shipped WASM. packages/core/wasm/ is committed; if it drifts from
#      what the current Rust source builds, every published size number and every
#      render gate is measuring a binary nobody can reproduce.
#   2. A README that quietly lies. The numbers below are measured here and nowhere
#      else; the README cites this script rather than restating them from memory.
set -euo pipefail
cd "$(dirname "$0")/../.."

BUDGET_GZ=$((150 * 1024))   # PRINCIPLES #2 (owner amendment 2026-07-13): whole library <= 150 KB gz
SHIPPED=packages/core/wasm/instruments_dsp.wasm
BUILT=target/wasm32-unknown-unknown/release/instruments_dsp.wasm

cargo build -q -p instruments-dsp --target wasm32-unknown-unknown --release

if ! cmp -s "$SHIPPED" "$BUILT"; then
  echo "BUNDLE AUDIT FAIL: shipped WASM is stale."
  echo "  shipped $SHIPPED  sha256 $(shasum -a 256 "$SHIPPED" | cut -d' ' -f1)"
  echo "  built   $BUILT  sha256 $(shasum -a 256 "$BUILT" | cut -d' ' -f1)"
  echo "  fix: cp $BUILT $SHIPPED"
  exit 1
fi

gz() { gzip -9c "$1" | wc -c | tr -d ' '; }

# The published surface is every workspace dist a consumer can pull in, not just core.
# packages/midi is on the path for every SMF/GM user, and the string/horn campaign (#50)
# grows it further - so it must be counted, or "all-in" is a lie by omission.
JS_PARTS=(
  packages/core/dist/index.js
  packages/core/worklet/instruments-processor.js
  packages/midi/dist/index.js
)

for f in "$SHIPPED" "${JS_PARTS[@]}"; do
  if [ ! -f "$f" ]; then
    echo "BUNDLE AUDIT FAIL: $f is missing."
    echo "  This audit measures BUILT output. Run 'npm run build --workspaces' first."
    exit 1
  fi
done

wasm_gz=$(gz "$SHIPPED")
core_gz=$(gz packages/core/dist/index.js)
worklet_gz=$(gz packages/core/worklet/instruments-processor.js)
midi_gz=$(gz packages/midi/dist/index.js)
total=$((wasm_gz + core_gz + worklet_gz + midi_gz))
instruments=$(grep -cE '^\s+[A-Z][A-Za-z0-9]* = [0-9]+,' crates/dsp/src/kernels.rs)

printf 'wasm     %6s B gz\n' "$wasm_gz"
printf 'core JS  %6s B gz\n' "$core_gz"
printf 'worklet  %6s B gz\n' "$worklet_gz"
printf 'midi JS  %6s B gz\n' "$midi_gz"
printf 'TOTAL    %6s B gz  (%s KB) — %s instruments, budget %s B\n' \
  "$total" "$((total / 1024))" "$instruments" "$BUDGET_GZ"

if [ "$total" -gt "$BUDGET_GZ" ]; then
  echo "BUNDLE AUDIT FAIL: $total B gz exceeds the $BUDGET_GZ B contract."
  exit 1
fi
echo "bundle-size-audit: OK (shipped WASM fresh; $((total / 1024)) KB gz of the $((BUDGET_GZ / 1024)) KB budget)"
