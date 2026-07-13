---
name: finalize-pr
description: Bring a PR to merge-ready state without merging it - verify GitHub issue linkage, review gates, current-head evidence, and follow-up routing. Usage - finalize-pr <PR number>
---

# finalize-pr

1. **Read the GitHub surfaces fresh**: confirm the PR links its source issue with `Closes #N`, its title follows `type(scope): imperative summary`, and its body contains motivation, impact, summary, validation, evidence freshness, review focus, gates, follow-ups, and the agentic process trace.
2. **Route boundary work**: fold work required for the PR's claim into the PR. Search before filing, then create or link GitHub issues for separable follow-ups. Do not write local TODO, backlog, plan-status, or decision-log files.
3. **Prove evidence freshness**: derive the full SHA with `git rev-parse HEAD`, confirm it equals the PR's GitHub `headRefOid`, and insert that programmatic value rather than hand-transcribing a short hash. Inventory every CI, test, build, benchmark, panel, sealed campaign, and human-listening claim. Evidence from another head or changed bound input is historical unless the current outputs are proved byte-identical and that proof is recorded. Report missing evidence; never silently carry it forward.
4. **Gates before ready**: CI green on the current head; `scripts/audit/harness-audit.sh` clean; DSP changes have a current-head `dsp-bench` result; instrument/API/packaging changes have a `panel-review` comment on the current head SHA; ports have ledger entries; no actionable review thread remains unresolved.
5. **Verify by driving real behavior**: play it in `apps/playground` or run the offline render and listen/inspect — green tests alone do not count for audio.
6. **Reconcile owner surfaces**: if the change alters a durable contract, update its one owner doc. Put live status, review evidence, and completion notes in the issue or PR, not a parallel local record.
7. **Stop merge-ready**: report the PR URL, current head SHA, checks, review state, evidence freshness, issue/follow-up links, and blockers. Merge only with explicit, current human authorization; never self-merge, force-push, or bypass hooks.
