#!/usr/bin/env bash
# Cross-family drift tripwire (loop audit 2026-07-12): after any merge, render
# the standardized audition set and compare EVERY family against the last
# accepted set — catches collateral damage the per-family loops can't see.
#
#   scripts/dev/drift-check.sh <baseline-dir> [work-dir]
#
# Exit 1 if any family's canonical render drifts more than THRESH (mr_stft.mean,
# render-to-render, K-weighted). Accept a new baseline by re-pointing the dir.
set -euo pipefail
BASE="${1:?usage: drift-check.sh <baseline-dir> [work-dir]}"
WORK="${2:-$(mktemp -d)/auditions}"
THRESH="${DRIFT_THRESH:-0.35}"
HERE="$(cd "$(dirname "$0")" && pwd)"

node "$HERE/render-auditions.mjs" all "$WORK" >/dev/null

fail=0
for f in "$BASE"/*.wav; do
  name="$(basename "$f")"
  new="$WORK/$name"
  [ -f "$new" ] || { echo "MISSING in new render: $name"; fail=1; continue; }
  d=$(python3 "$HERE/compare.py" "$new" "$f" 2>/dev/null | python3 -c "import json,sys; print(json.load(sys.stdin)['mr_stft']['mean'])" || echo "ERR")
  if [ "$d" = "ERR" ]; then echo "COMPARE ERROR: $name"; fail=1; continue; fi
  flag=""
  awk -v d="$d" -v t="$THRESH" 'BEGIN{exit !(d>t)}' && { flag="  ← DRIFT (> $THRESH)"; fail=1; }
  echo "$d  $name$flag"
done
exit $fail
