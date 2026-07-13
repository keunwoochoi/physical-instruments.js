# Reference-matching auto-research loop

Date: 2026-07-11
Status: accepted as direction (Keunwoo, "maybe now, maybe later") — authorizes building
the loop environment when instrument-quality work resumes; does NOT authorize
downloading/committing reference recordings without a license check.

## Motivation

1. Manual instrument tuning today is already a human-in-the-loop version of this:
   render → metrics (piano-audition.mjs) → physically-motivated change → re-measure.
   The piano v2→v3 session proved the loop works and converges.
2. Frontier LLMs can close this loop autonomously — proposing *equation-level* changes
   with physical justification, not just parameter nudges. This was not possible
   before; it is a boundary-pushing lever this project should fully use (Keunwoo).

## Thesis

For each instrument, run an autonomous iterate loop against license-clean reference
recordings: multi-axis objective comparison (log-mel distance, attack/decay structure,
partials, LUFS) → physical diagnosis → one cited change → re-measure. Metrics gate
iteration; human ears gate acceptance. The deliverable is not just better presets but
a reusable optimization harness (`skills/match-reference`).

## Evidence base

- Piano v2→v3 (decision log 2026-07-11): metric-guided iteration found and fixed
  attack-shape, two-stage-decay, and velocity-physics defects in hours.
- Cautionary: the same session produced a broken centroid metric that nearly caused a
  wrong re-tune — metric rigor is a first-class requirement.
- Adjacent literature: DDSP-style parameter fitting (offline, differentiable);
  our variant substitutes LLM search for gradients, allowing *structural* changes.

## Phased plan

- P1: reference corpus scaffolding (`evals/corpus/references/`, license ledger rows)
  + a `compare.py` (librosa/pyloudnorm) producing the multi-axis report.
- P2: loop runner — a workflow that iterates render→compare→propose→rebuild with
  per-iteration logs, regression auto-revert, and held-out references.
- P3: run per instrument (piano first — it has the richest references, e.g. MAESTRO
  single-note extractions), publish before/after renders + metric tables.

## Deferred until demanded

Gradient-based fitting (DDSP-style) as a complement; neural residual models
(weights never ship in the core bundle — PRINCIPLES); automated listening-test
crowdsourcing.
