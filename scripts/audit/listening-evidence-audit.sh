#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."

python3 -m unittest scripts/dev/test_listening.py
python3 scripts/dev/generate_listening_pilot.py --check
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
python3 scripts/dev/listening.py validate evals/listening/pilot/analysis-manifest.json
python3 scripts/dev/listening.py analyze evals/listening/pilot/analysis-manifest.json evals/listening/pilot/synthetic-results.json --out "$tmp/analysis.json" --markdown "$tmp/analysis.md"
cmp evals/listening/pilot/synthetic-analysis.json "$tmp/analysis.json"
cmp evals/listening/pilot/synthetic-analysis.md "$tmp/analysis.md"
node scripts/dev/listening-e2e.mjs
echo "listening-evidence-audit: OK"
