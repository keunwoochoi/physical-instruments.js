#!/usr/bin/env bash
# Enforce the public loop-metric schema and equation-owned golden evidence.
set -euo pipefail
cd "$(dirname "$0")/../.."

python3 - <<'PY'
for name in ("numpy", "jsonschema", "pyloudnorm", "scipy", "soundfile"):
    try:
        __import__(name)
    except ImportError as exc:
        raise SystemExit(f"loop-metrics-audit: missing {name}; run python3 -m pip install -r scripts/dev/requirements-loop.txt") from exc
PY

python3 scripts/dev/test_loop_metrics.py
git diff --exit-code -- evals/metrics/loop-v1
echo "loop-metrics-audit: OK"
