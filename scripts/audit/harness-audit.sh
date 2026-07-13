#!/usr/bin/env bash
# Executable entrypoint for harness invariants. The implementation stays stdlib-only.
set -euo pipefail
cd "$(dirname "$0")/../.."
exec python3 scripts/audit/harness_audit.py
