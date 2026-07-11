---
name: dsp-bench
description: Measure DSP cost against the audio-thread budget. Run on every DSP change; CI tripwire mode fails on regression. Usage - dsp-bench [--ci]
---

# dsp-bench

Budget: **2.67 ms per 128-frame quantum at 48 kHz** — for the WHOLE engine output, measured on a full multi-track arrangement (PRINCIPLES #4), not a solo voice.

1. Build `crates/dsp` in release mode (native for iteration; wasm via node/wasmtime for the honest number).
2. Run the bench harness: per-voice cost per instrument; the standard arrangement scenario (≥4 tracks, 32 active voices: e.g. piano-track chords + bass + drums + mallets); worst case (pedal-down sympathetic resonance when piano lands).
3. Report: µs/quantum, % of budget, polyphony headroom, per-instrument breakdown. Compare against the committed baseline in `evals/metrics/`.
4. Gates: standard arrangement ≤50% budget on target hardware (M1 + mid-tier Android; iOS Safari measured manually per release). `--ci`: fail on >10% regression vs baseline.
5. Update the baseline only in a PR that explains the regression/improvement (decision-log entry).
