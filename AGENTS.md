# instruments.js — Agent Constitution & Router

> Read `PRINCIPLES.md` first. It is the constitutional law of this repo.
> This file owns: the operating constitution and routing. Details live in owned docs (see routes).

## Constitution

1. **Truth has owners, not echoes.** Code owns behavior; `scripts/audit/` owns enforceable checks; `agentic-docs/` owns durable policy and decisions; GitHub Issues and pull requests own work state and evidence; this file owns constitution + routing. When a fact changes, update the owner first. Never copy a fact into a second surface — link to it.
2. **Eval before trust.** No claim that something "sounds good" without evidence: AB/ABX for iteration, MUSHRA for gates, `dsp-bench` for budgets. Persona reviews gate iteration; human listening gates releases.
3. **Design before code.** Anything bigger than one PR starts as a dated design doc (`skills/new-design-doc`). Plans materialize as dependency-linked GitHub issues; live status lives in issues, never in docs.
4. **The audio thread is sacred.** No allocation, no locks, no JS, no denormals on the sample path. Every DSP change passes `dsp-bench` against the 2.67 ms / 128-frame budget — measured on a full multi-track arrangement, not a solo instrument.
5. **License hygiene is absolute.** Port MIT/BSD code freely with ledger entries; NEVER open AGPL/GPL/LGPL source (papers-only reimplementation). See `agentic-docs/licensing.md`.

## Authority gates (off by default — a human lifts one per task, explicitly)

- npm publish / GitHub release: **off**
- git push to `main`, force-push, `--no-verify`, self-merge: **never**
- paid or quota-consuming external resources: **off**
- public posts (Show HN, social, docs deploys): **off**

## Commit messages own the engineering record

**This is not style guidance. It is the primary research output of this project.**

The diff shows *what* changed. It can never show what was wrong, how we knew, what we measured, what we tried that failed, or what caught the error — and that is the part that is unrecoverable once the session ends. A commit message is the only artifact that carries that reasoning **permanently attached to the code it explains**. Issues get closed, PRs get squashed out of memory, chat transcripts vanish. `git log` does not.

This work will be written up. **Write every commit as if the technical report is being drafted from `git log` alone, because it will be.**

Every non-trivial commit body records, in prose:

1. **The defect, and how it was actually found.** By ear? By a metric? By a persona review? By the owner? Say so, and quote the owner verbatim if that is what happened — *"there is a similarity between this piano model sound and the electric guitar sound … those twang"* is worth more than any paraphrase.
2. **The measurement, before and after, with units.** Not "improved the decay" — *"two-stage decay ratio 1.57× → 3.10×; a real piano is 2–4×; our own electric guitar measured 2.16×."* A number with no comparator is not evidence.
3. **The root cause, named.** Not the symptom. Why the code did the wrong thing.
4. **What was tried and abandoned, and why.** Including — *especially* including — fixes that made things **worse**, diagnoses that turned out to be **wrong**, and measurements that were themselves **broken**. These are the most valuable lines in the repository and they exist nowhere else. Do not quietly drop a failed attempt.
5. **The cost.** CPU, memory, bundle. A quality claim without a cost is half a claim.

If a commit changes how something sounds and the body does not contain a number, it is not finished.

The same discipline governs the PR body's **Agentic process trace** (below) and the engineering journey log (#51). The commit is the finest-grained surface and the one that never gets detached from the code; the journey log is the narrative across sessions. Neither replaces the other.

## GitHub workflow

- Search existing issues and pull requests before creating a new work item.
- Every implementation PR starts from or adopts a GitHub issue. Use a `type(scope): imperative summary` title; the issue body owns motivation, evidence, desired outcome, scope and constraints, acceptance criteria, and validation expectations. Use the forms in `.github/ISSUE_TEMPLATE/`.
- The issue is the live control plane: assignment records ownership; comments record material decisions and blockers; checkboxes record acceptance. Do not create local TODO, backlog, plan-status, or per-PR decision-log files.
- Open implementation PRs as drafts. The PR body links the source issue with `Closes #N`, states impact and validation, names review focus, and routes every separable follow-up to an issue. Use `.github/pull_request_template.md`.
- Evidence is immutable-input-bound: every current claim names the exact head SHA or artifact seal it validates. After any head change, rerun the evidence or label it historical before requesting review.
- When the owner asks to wrap a progressive session, stop new scope and use `skills/wrap-session/SKILL.md` to reconcile published stacks, evidence freshness, exact heads, blockers, and resume points.
- Keep durable architecture and policy in their owner docs. Keep changing plans, status, review evidence, and completion state in GitHub.
- Keep filling in the PR body's "Agentic process trace" table. The **abandoned/wasted routes** row is not a formality — it is the primary record of what did not work, and it is unrecoverable from the diff.
- **Append to the engineering journey log (issue #51) at the end of any substantial session or campaign.** One comment, never an edit to the issue body, never a local journal file. Record what was abandoned, what caught the error, verbatim owner quotes (marked as such — the agent operates the owner's GitHub account, so authorship is not evidence of voice), decisive numbers, and any harness rule you added because of a failure. The log exists so the project's process can be written up; failures are the contribution.

## Routes

| Task | Always load | Load if triggered |
|---|---|---|
| Implementing DSP | `agentic-docs/design/2026-07-11-architecture.md` | `agentic-docs/licensing.md` when porting |
| Public API / packaging | `packages/core/README.md` | `demos/bundler-matrix/README.md` |
| Any instrument/API/packaging PR | `skills/finalize-pr/SKILL.md` | `skills/panel-review/SKILL.md` (required before merge) |
| Evaluating sound | `skills/run-evals/SKILL.md` | `evals/README.md` |
| Porting third-party code | `skills/port-audit/SKILL.md` | — |
| New feature > 1 PR | `skills/new-design-doc/SKILL.md` | `agentic-docs/design/TEMPLATE.md` |
| Progressive or stacked session wrap | `skills/wrap-session/SKILL.md` | `skills/finalize-pr/SKILL.md` for any potentially merge-ready PR |
| Any issue or PR | `.github/ISSUE_TEMPLATE/` | `.github/pull_request_template.md` when opening or finalizing a PR |
| Piano (issue #49) | `agentic-docs/design/2026-07-13-higher-capacity-piano.md` | the earlier pianoteq-class piano doc, on the product stack, for phase history |
| Strings / horns (issue #50) | `agentic-docs/design/2026-07-13-string-and-horn-families.md` | `skills/port-audit/SKILL.md` before touching STK |
| Ending a session or campaign | journey log — issue #51 (append one comment; never edit the body) | — |

## Repo map

| Path | Owns |
|---|---|
| `crates/dsp/` | Rust DSP core → WASM (voice bank, all instruments, mixing). **The bank is AoS and scalar today** — `Vec<Voice>`, enum-dispatched, no SIMD. The architecture doc's "SoA voice bank" is an unrealized intent, not a description; budget accordingly (#62). |
| `packages/core` | TS public API, worklet host, voice/track management |
| `packages/instruments` | Instrument façades, presets, GM program map |
| `packages/midi` | Note-list scheduler, MIDI file parsing, GM drum map, Web MIDI |
| `apps/playground` | Daily does-it-sound-good surface |
| `evals/` | Corpus, incumbent renders, listening tests, regression tripwires |
| `skills/` | Canonical tool-neutral agent workflows, exposed through `.agents/skills` and `.claude/skills` |
| `.github/` | Issue forms, PR template, and CI workflows |
| `agentic-docs/` | Design docs, reports, licensing ledger, persona profiles |
