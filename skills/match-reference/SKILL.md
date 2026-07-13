---
name: match-reference
description: LLM-driven auto-research loop - iteratively evolve a physical model's equations and parameters until its render matches reference recordings, guided by spectral/envelope/loudness metrics. Usage - match-reference <instrument> [reference.wav]
---

# match-reference — the auto-research loop

Karpathy-style autonomous optimization (Keunwoo's framing, 2026-07-11): the goal is
sound-similarity to real references, reached through physically-understood changes —
not black-box curve fitting. This loop was not possible before frontier LLMs; use it.

## Setup
1. References live in `evals/corpus/references/<instrument>/` — short single notes at
   ≥3 velocities × ≥3 registers, plus one musical phrase. License-clean recordings
   only (see agentic-docs/licensing.md; ledger the source).
2. Render harness: follow the `scripts/dev/piano-audition.mjs` pattern — deterministic
   Node render of the SAME notes as the reference set.

## The loop (per instrument)
1. **Render** current model at the reference notes/velocities.
2. **Compare** render vs reference on multiple axes — never a single number:
   log-mel spectrogram distance (multi-scale), attack transient (first 50 ms envelope
   + spectral centroid trajectory), decay structure (two-stage t60s), partial
   frequencies/amplitudes (inharmonicity), LUFS match (pyloudnorm), release/damp tail.
3. **Diagnose physically**: which model component explains the largest mismatch?
   (exciter shape, loop losses, dispersion, coupling, body). State the hypothesis in
   physical terms BEFORE changing anything.
4. **Change ONE thing** — a parameter, or an equation upgrade from the literature
   (search `agentic-docs/research-venues.md` venues) — rebuild, re-measure.
5. **Log** every iteration: hypothesis → change → metric deltas. Revert regressions.
6. **Stop** when metrics plateau or the gate passes; then request the human listening
   pass (metrics gate iteration, ears gate acceptance — PRINCIPLES: eval before trust).

## Hard rules
- Measurement tools get the same rigor as the DSP (windowing, no decimation ghosts —
  see decision log 2026-07-11: a broken centroid metric nearly caused a wrong re-tune).
- Archive only schema-valid reports with their metric version, input digests, resolved configuration, and preprocessing operations; never compare scores across metric versions without an explicit migration report.
- Every equation change cites its source or states its physical derivation.
- Tuning/stability tests (`cargo test`) must stay green every iteration.
- Never overfit to one reference: hold out at least one velocity/register per axis.
