# physical-instruments.js — tech report: writing knowledge base

Durable planning + research for a rich, interactive technical report on
**physical-instruments.js** (repo: `keunwoochoi/physical-instruments.js`) — a lightweight (~82 KB gz)
physical-modeling instrument library — **and on how it was built agentically**.

This folder is *not the draft.* It is the knowledge base the draft is written **from**.
The draft lands later (as `draft.md` here, or a dated file in `agentic-docs/reports/`)
once we've agreed the plan.

> Status: **planning.** Files marked DRAFT/OPEN are for us to agree on together;
> files marked RESEARCH/CAPTURE are populated and maintained as fact.

## Quality bar (owner, 2026-07-23)
**ICML-workshop level, or an award-level AES / NIME / ISMIR-workshop paper.**
→ real evaluation, honest limitations, clearly-stated novelty, related-work grounding,
reproducibility — *plus* a Distill/Olah interactive layer. The rigor register of the
Ortet **v03** report (see `author-and-voice.md`), made accessible, fun, and playable.

## Two braided threads
1. **The artifact.** A ~82 KB gz WASM physical-modeling engine, 29 instruments, a
   playable browser demo (live: https://keunwoochoi.github.io/physical-instruments.js/). The DSP,
   the evals, the "audio thread is sacred" real-time budget, the clean-room licensing.
2. **The agentic process.** How it was built by directing an LLM agent: the collaboration,
   the friction, what the prompter correctly expected vs. didn't, what he needed to know
   vs. didn't, satisfaction and dissatisfaction. **This build session is the primary
   source**, captured in `agentic-process-notes.md`.

The two are braided, not stacked: the process explains the artifact and vice versa.

## Files
| File | Kind | State |
|---|---|---|
| `author-and-voice.md` | research | who the author is + target voice (links `personas/keunwoo.md`) |
| `writing-samples.md` | research | **the owner's actual voice**, from primary sources (beetbox-eval, 22 blog posts, Solar Open 1/2). Read before drafting; it overrides parts of `author-and-voice.md`. |
| `reviewers.md` | draft | the ≤20 virtual reviewer pool (reuses `agentic-docs/personas/`) |
| `audience.md` | **decision OPEN** | audience definition, options A–D |
| `style-and-structure.md` | **draft, to agree** | house style + proposed outline, figures, interactives |
| `agentic-process-notes.md` | capture | raw material for thread #2, from the build session |
| `README-draft.md` | **draft** | the report as it will land in the repo README. Written 2026-07-28, then fully rewritten the same day after owner feedback ("typical AI slop") — see `writing-samples.md` §0 for the tics that were purged. Every number measured at HEAD `ce671d7`. |
| `draft.md` | superseded | earlier "we"-academic opening, written before the writing samples were fetched. Keep for the two-act long-read plan only. |

## Decision log
| Decision | State |
|---|---|
| Audience | **OPEN** — `audience.md` |
| Reviewer pool | drafted — `reviewers.md` |
| House style | draft — `style-and-structure.md` |
| Outline / figures / interactives | draft — `style-and-structure.md` |
| Format | Markdown now; interactive (Distill-style) later |

## Related repo assets — truth has owners, so link don't copy (PRINCIPLES #1)
- `agentic-docs/personas/` — 7 existing `review-as` personas: keunwoo, hayoung, juhan,
  yotam, jordan, producer, senior-web-dev.
- `PRINCIPLES.md`, `CLAUDE.md` — the project constitution ("audio thread is sacred",
  eval-before-trust, commit-message-as-research-record). These disciplines are themselves
  part of the story.
- `agentic-docs/design/`, `agentic-docs/reports/` — prior design docs and reports.
- `packages/core/README.md` — public API + bundle-size contract (note: its 74 KB / 15
  instruments is **stale**; code says ~82 KB / 29 — see `agentic-process-notes.md`).

## Housekeeping
These are **uncommitted local planning docs** in a now-public repo. Some content is candid
(process friction, verbatim owner quotes, personal bio). Nothing here is committed or
deployed until the owner decides what becomes public.
