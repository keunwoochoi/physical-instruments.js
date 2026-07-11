---
name: harness-audit
description: Audit and repair the agent harness itself - symlinks, forwarders, doc links, ledger coverage, decision-log freshness. Usage - harness-audit
---

# harness-audit

1. Run `scripts/audit/harness-audit.sh`. It owns the enforceable checks (constitution #1); this skill owns the repair procedure.
2. For each failure, fix the OWNER surface: broken symlink → re-link; orphan `.claude/commands/` forwarder → create/delete pair; dangling doc reference → fix the link or the doc; ported file missing from `agentic-docs/licensing.md` → add the ledger row (and verify license!); design doc without a Status line → add one; stale TODO (>30 days untouched) → roll to a new dated file, archive the old.
3. Re-run until clean. If a check is wrong, change the script in the same PR with a decision-log note — never bypass it.
