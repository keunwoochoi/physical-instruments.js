---
name: harness-audit
description: Audit and repair the agent harness itself - cross-tool skill exposure, frontmatter, links, GitHub templates, authority defaults, and owner-surface boundaries. Usage - harness-audit
---

# harness-audit

1. Run `scripts/audit/harness-audit.sh`. It owns the enforceable checks; this skill owns the repair procedure.
2. Fix the owner surface for every failure: restore tool-neutral skill symlinks; repair skill frontmatter or local links; keep Claude commands as thin forwarders; restore authority defaults; repair issue/PR templates; route live work out of local docs and into GitHub; update licensing or design owners when their checks fail.
3. Re-run until clean. If a check is wrong, change the audit in the same PR and explain the reason in the source issue or PR. Never bypass it or create a parallel local exception note.
