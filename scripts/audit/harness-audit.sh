#!/usr/bin/env bash
# Executable entrypoint for harness invariants. The implementation stays stdlib-only.
# Also proves the audit can fail: scripts/audit/fixtures/ are deliberately broken
# inputs that test_harness_audit.py requires the audit to reject (METR-style).
# Instrument ids: catalog is SoT; generated TS must be current; Rust enum must match (#98).
set -euo pipefail
cd "$(dirname "$0")/../.."
python3 scripts/audit/harness_audit.py
python3 scripts/audit/test_harness_audit.py
python3 scripts/gen/gen_instrument_ids.py --check
python3 scripts/gen/test_gen_instrument_ids.py
# Worktrees: start every branch in one, delete it once merged (AGENTS.md). The audit
# stays offline — it runs inside the pre-commit hook — and its own test proves it red.
python3 scripts/dev/worktree.py audit
python3 scripts/dev/test_worktree.py
# The release-impact gate runs per PR against a base ref CI supplies; what belongs here
# is the proof it still goes red — same reason the fixtures above exist.
exec python3 scripts/release/test_check_release_impact.py
