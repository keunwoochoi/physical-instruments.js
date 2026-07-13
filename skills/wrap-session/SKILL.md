---
name: wrap-session
description: Close a progressive or stacked work session without starting new scope - reconcile exact heads, evidence freshness, published-branch topology, blockers, and deterministic resume points in GitHub. Usage - wrap-session [tracker issue]
---

# wrap-session

Use this when the owner says to wrap up, debrief, stop after the current iteration, or otherwise close a progressive session without demanding an immediate hard stop.

## 1. Freeze scope, not safety

- Stop opening hypotheses, branches, issues, or opportunistic cleanup. Finish only the smallest bounded in-flight change that the owner explicitly kept alive.
- Preserve every active constraint through the wrap. For a quiet/no-sound window, record its absolute deadline and timezone in the issue; do not open an audio device, browser audition, or playback path before it expires. Offline analysis is not playback, but avoid any command whose output-device behavior is uncertain.
- Do not turn “wrap” into implicit authorization to merge, publish, force-push, bypass hooks, or relax a release gate.

## 2. Inventory from fresh repository state

- Fetch remotes, then record every in-scope branch, upstream, exact head SHA, base branch, ahead/behind count, clean/dirty state, PR URL, draft state, checks, review decision, and unresolved review-thread count.
- Derive every full SHA from `git rev-parse` or the GitHub API; never hand-transcribe or expand an abbreviated hash. Before posting, verify local `HEAD`, upstream, PR `headRefOid`, and every current-head SHA in the final body agree exactly.
- Read the source issues and PR bodies fresh. Treat the repository and GitHub as authoritative; do not reconstruct live state from a local status document or memory.
- For stacked work, record dependency order and which parent head each child actually contains.

## 3. Reconcile evidence freshness

Every validation claim is bound to immutable inputs. A new commit makes a “current-head” claim historical until it is rerun or exact output identity is proved and recorded.

| Evidence | Bound identity | Freshness rule |
|---|---|---|
| CI, tests, build, benchmark | Exact head SHA, toolchain, command, and relevant environment | Any head change invalidates current-head wording; rerun it or label it historical. |
| Persona panel | Exact PR head SHA | Any commit invalidates the panel. Run the full required panel again before merge-ready. |
| Sealed render/campaign | Source head, shipped executable, metric code/version, schemas, registry, manifest, references, and artifact digests | Any bound-input change requires a new seal; predecessor evidence stays useful only when labeled historical. |
| Human listening | Exact stimulus hashes, question/protocol, and result | Carry forward only after proving the current stimulus outputs are byte-identical and recording that proof; qualitative feedback may guide work but is not a formal gate. |

- Put the exact current head and freshness status at the top of each PR body. Never leave an old “exact head” section looking current after a merge-only, documentation, receipt, calibration, or fixup commit.
- Do not delete predecessor evidence. Mark it historical, name the head or artifact hashes it belongs to, and state what must be rerun.
- State skipped, deferred, unavailable, and owner-blocked validation explicitly. Absence is not a pass.

## 4. Synchronize published stacks without rewriting them

- Before any rebase, inspect `git merge-base`, `git rev-list --left-right --count`, the upstream branch, and whether a PR publishes the branch.
- Never rebase a published branch when updating it would require force-push. Merge the updated dependency branch non-force, or stop and report the topology conflict. If a requested rebase begins replaying unrelated published history or conflicts at the stack root, abort it before changing the remote.
- After synchronization, verify each child contains the intended parent head and that local and upstream heads match. Never report a rebase that was aborted as completed.

## 5. Leave a reviewable repository state

- Commit and push bounded finished work to its existing branch; do not push to `main`. Leave each in-scope worktree clean and synchronized, or identify the exact dirty files and owner of the remaining state.
- Update each PR body with its exact head, current validation, historical evidence warning, remaining gates, and linked follow-ups. Keep blocked work draft.
- Route separable blockers to searched GitHub issues. Do not solve broad mobile, packaging, API, or licensing gaps inside an unrelated wrap commit.
- Run `finalize-pr` only for PRs that may honestly be merge-ready; otherwise record why they remain draft. Never use wrap-up as a reason to weaken a gate.

## 6. Debrief in the tracker

Post one compact source-of-truth comment containing the exact branch/PR head matrix, what shipped, evidence that is current, evidence that became historical, validation not run, remaining human/device/CI gates, authority actions not taken, and one deterministic resume point per unfinished job.

The chat response should link that tracker comment and summarize only the owner-relevant result. Do not create a parallel local wrap report.
