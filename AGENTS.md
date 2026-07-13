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

## GitHub workflow

- Search existing issues and pull requests before creating a new work item.
- Every implementation PR starts from or adopts a GitHub issue. Use a `type(scope): imperative summary` title; the issue body owns motivation, evidence, desired outcome, scope and constraints, acceptance criteria, and validation expectations. Use the forms in `.github/ISSUE_TEMPLATE/`.
- The issue is the live control plane: assignment records ownership; comments record material decisions and blockers; checkboxes record acceptance. Do not create local TODO, backlog, plan-status, or per-PR decision-log files.
- Open implementation PRs as drafts. The PR body links the source issue with `Closes #N`, states impact and validation, names review focus, and routes every separable follow-up to an issue. Use `.github/pull_request_template.md`.
- Evidence is immutable-input-bound: every current claim names the exact head SHA or artifact seal it validates. After any head change, rerun the evidence or label it historical before requesting review.
- When the owner asks to wrap a progressive session, stop new scope and use `skills/wrap-session/SKILL.md` to reconcile published stacks, evidence freshness, exact heads, blockers, and resume points.
- Keep durable architecture and policy in their owner docs. Keep changing plans, status, review evidence, and completion state in GitHub.

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

## Repo map

| Path | Owns |
|---|---|
| `crates/dsp/` | Rust DSP core → WASM (SoA voice engine, all instruments, mixing) |
| `packages/core` | TS public API, worklet host, voice/track management |
| `packages/instruments` | Instrument façades, presets, GM program map |
| `packages/midi` | Note-list scheduler, MIDI file parsing, GM drum map, Web MIDI |
| `apps/playground` | Daily does-it-sound-good surface |
| `evals/` | Corpus, incumbent renders, listening tests, regression tripwires |
| `skills/` | Canonical tool-neutral agent workflows, exposed through `.agents/skills` and `.claude/skills` |
| `.github/` | Issue forms, PR template, and CI workflows |
| `agentic-docs/` | Design docs, reports, licensing ledger, persona profiles |
