---
name: finalize-pr
description: Bring a PR to merge-ready state without merging it - verify GitHub issue linkage, review gates, current-head evidence, and follow-up routing. Usage - finalize-pr <PR number>
---

# finalize-pr

1. **Read the GitHub surfaces fresh**: confirm the PR links its source issue with `Closes #N`, its title follows `type(scope): imperative summary`, and its body contains motivation, impact, summary, validation, review focus, gates, follow-ups, and the agentic process trace.
2. **Route boundary work**: fold work required for the PR's claim into the PR. Search before filing, then create or link GitHub issues for separable follow-ups. Do not write local TODO, backlog, plan-status, or decision-log files.
3. **Gates before ready**: CI green on the current head; `scripts/audit/harness-audit.sh` clean; DSP changes have a `dsp-bench` result; instrument/API/packaging changes have a `panel-review` comment on the current head SHA; ports have ledger entries; no actionable review thread remains unresolved.
4. **Verify by driving real behavior**: play it in `apps/playground` or run the offline render and listen/inspect — green tests alone do not count for audio.
5. **Reconcile owner surfaces**: if the change alters a durable contract, update its one owner doc. Put live status, review evidence, and completion notes in the issue or PR, not a parallel local record.
6. **Stop merge-ready**: report the PR URL, current head SHA, checks, review state, issue/follow-up links, and blockers. Merge only with explicit, current human authorization; never self-merge, force-push, or bypass hooks.
