# instruments.js — Agent Constitution & Router

> Read `PRINCIPLES.md` first. It is the constitutional law of this repo.
> This file owns: the operating constitution and routing. Details live in owned docs (see routes).

## Constitution

1. **Truth has owners, not echoes.** Code owns behavior; `scripts/audit/` owns enforceable checks; `agentic-docs/` owns durable policy and decisions; this file owns constitution + routing. When a fact changes, update the owner first. Never copy a fact into a second surface — link to it.
2. **Eval before trust.** No claim that something "sounds good" without evidence: AB/ABX for iteration, MUSHRA for gates, `dsp-bench` for budgets. Persona reviews gate iteration; human listening gates releases.
3. **Design before code.** Anything bigger than one PR starts as a dated design doc (`skills/new-design-doc`). Plans materialize as dependency-linked GitHub issues; live status lives in issues, never in docs.
4. **The audio thread is sacred.** No allocation, no locks, no JS, no denormals on the sample path. Every DSP change passes `dsp-bench` against the 2.67 ms / 128-frame budget — measured on a full multi-track arrangement, not a solo instrument.
5. **License hygiene is absolute.** Port MIT/BSD code freely with ledger entries; NEVER open AGPL/GPL/LGPL source (papers-only reimplementation). See `agentic-docs/licensing.md`.

## Authority gates (off by default — a human lifts one per task, explicitly)

- npm publish / GitHub release: **off**
- git push to `main`, force-push, `--no-verify`, self-merge: **never**
- paid or quota-consuming external resources: **off**
- public posts (Show HN, social, docs deploys): **off**

## Routes

| Task | Always load | Load if triggered |
|---|---|---|
| Implementing DSP | `agentic-docs/design/2026-07-11-architecture.md` | `agentic-docs/licensing.md` when porting |
| Public API / packaging | `packages/core/README.md` | `demos/bundler-matrix/README.md` |
| Any instrument/API/packaging PR | `skills/finalize-pr/SKILL.md` | `skills/panel-review/SKILL.md` (required before merge) |
| Evaluating sound | `skills/run-evals/SKILL.md` | `evals/README.md` |
| Porting third-party code | `skills/port-audit/SKILL.md` | — |
| New feature > 1 PR | `skills/new-design-doc/SKILL.md` | `agentic-docs/design/TEMPLATE.md` |

## After every merged PR

Update `.claude/TODO-2026-07-11.md`: mark items `[x]` with a root-cause + fix + verification note; add new Open/Backlog items surfaced. This is mandatory (`skills/finalize-pr`).

## Repo map

| Path | Owns |
|---|---|
| `crates/dsp/` | Rust DSP core → WASM (SoA voice engine, all instruments, mixing) |
| `packages/core` | TS public API, worklet host, voice/track management |
| `packages/instruments` | Instrument façades, presets, GM program map |
| `packages/midi` | Note-list scheduler, MIDI file parsing, GM drum map, Web MIDI |
| `apps/playground` | Daily does-it-sound-good surface |
| `evals/` | Corpus, incumbent renders, listening tests, regression tripwires |
| `skills/` | Canonical agent workflows (`.claude/commands/` are thin forwarders) |
| `agentic-docs/` | Design docs, reports, licensing ledger, persona profiles |
