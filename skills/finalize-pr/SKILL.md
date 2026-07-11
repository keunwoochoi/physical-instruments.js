---
name: finalize-pr
description: Merge hygiene for every PR - review gates, verification, and the mandatory decision-log entry. Usage - finalize-pr <PR number>
---

# finalize-pr

1. **Gates before ready**: CI green; `scripts/audit/harness-audit.sh` clean; DSP changes have a `dsp-bench` result; instrument/API/packaging changes have a `panel-review` comment on the CURRENT head SHA; ports have ledger entries.
2. **Verify by driving real behavior**: play it in `apps/playground` (or run the offline render) and listen/inspect — green tests alone don't count for audio.
3. **Merge only with explicit, current human authorization** (`AGENTS.md` authority gates). Never self-merge, never `--force`, never `--no-verify`.
4. **Mandatory closing step**: update `.claude/TODO-2026-07-11.md` — mark items `[x]` with root cause → fix → verification (with numbers) → review outcome; add new Open/Backlog items surfaced. Update `AGENTS.md` routes or owner docs if a convention changed (update the owner, don't echo).
