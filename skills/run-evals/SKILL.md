---
name: run-evals
description: Render the fixed MIDI corpus through the current build and incumbents; emit AB/ABX comparison page and spectral regression diff. Usage - run-evals [instrument|arrangement]
---

# run-evals

Eval before trust (`PRINCIPLES.md`). Persona/spectral checks gate iteration; human AB/MUSHRA results gate releases.

1. Corpus: `evals/corpus/` — fixed MIDI, includes solo-instrument pieces AND full multi-track arrangements (customer-zero excerpts, MAESTRO excerpts for piano, torture cases: fast repeated notes, extreme velocities, dense polyphony, long release tails).
2. Render our side via offline render (deterministic). Render incumbents (Tone.js synths, smplr, spessasynth+GM) via the scripts in `evals/incumbents/` — those reference WAVs are committed and versioned.
3. Emit: (a) an AB/ABX page in `evals/listening/` pairing our render vs each incumbent, blind, randomized; (b) spectral tripwires (multi-scale STFT distance vs last accepted render) in `evals/metrics/` — regression signal ONLY, never a quality claim.
4. Human gates per roadmap: AB n≥5 (Q1), ABX vs smplr (Q2), MUSHRA hidden-reference (Q3+, MAESTRO Disklavier reference for piano).
5. Record results (who listened, n, result, date) in the decision log.
