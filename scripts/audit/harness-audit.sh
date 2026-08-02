#!/usr/bin/env bash
# Executable entrypoint for harness invariants. The implementation stays stdlib-only.
# Also proves the audit can fail: scripts/audit/fixtures/ are deliberately broken
# inputs that test_harness_audit.py requires the audit to reject (METR-style).
set -euo pipefail
cd "$(dirname "$0")/../.."
python3 scripts/audit/harness_audit.py
exec python3 scripts/audit/test_harness_audit.py
