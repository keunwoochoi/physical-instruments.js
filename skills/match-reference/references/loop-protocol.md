# match-reference loop protocol (agent runbook)

You are improving ONE instrument family of the instruments.js Rust DSP engine until
its renders match real reference recordings. You work in an isolated git worktree.

## Environment

- Reference WAVs (NSynth, CC-BY 4.0, 16 kHz, 4 s notes held ~3 s):
  `/private/tmp/claude-501/-Users-keunwoo-Codes-vst-js/20d182ad-a56f-4ef2-add1-8babbac5723a/scratchpad/references/<family>/`
  Filename: `<inst>_<source>_<id>-<midipitch>-<velocity>.wav` (velocity 025/050/075/100/127).
- Scratch space for your renders: make a subdir under that scratchpad.

## The iteration cycle (repeat ≥5 times or until plateau)

```sh
cargo build -p instruments-dsp --target wasm32-unknown-unknown --release \
  && cp target/wasm32-unknown-unknown/release/instruments_dsp.wasm packages/core/wasm/
node scripts/dev/render-note.mjs <family> <midi> <vel1-127> 3.0 /path/render.wav 4.0
python3 scripts/dev/compare.py /path/render.wav <reference.wav>
```

1. Compare against ≥3 references spanning register and velocity. HOLD OUT one
   reference (never tune against it; check it at the end for overfit).
2. Read ALL axes: logmel (attack/mid/tail), centroid trajectory, envelope
   (time-to-peak, t60s), partials (frequencies→inharmonicity, level tilt), LUFS
   delta (velocity-curve fit). Identify the LARGEST mismatch.
3. State a physical hypothesis for it BEFORE editing (which model element?).
   Search agentic-docs/research-venues.md literature when changing equations.
4. Change ONE thing in your instrument's code. Rebuild. Re-measure. Log:
   `hypothesis → change → metric deltas` (keep a running log in your final report).
5. Revert regressions immediately. `cargo test` must stay green EVERY iteration —
   add tests for new behavior (tuning at 44.1k AND 48k if you touch delay math).

## Hard rules

- Edit ONLY your instrument's regions: its `start_voice` arm + its kernel struct/impl
  in `crates/dsp/src/kernels.rs`, its rows in `body_defaults`/`pickup_defaults`/
  `amp_defaults`/`makeup_gain`. NEVER touch other instruments, `lib.rs` engine
  plumbing, or the TS/JS layers. If you need a new shared facility, note it in your
  report instead of building it.
- Speed is a product pillar: your instrument ≤ ~40 µs/quantum for 8 voices in
  `node scripts/dev/render-demo.mjs --bench`-style measurement (stay lean; this is
  still 60× lighter than samples).
- Nonlinearities are WELCOME where physically motivated (tension-modulation pitch
  glide on hard hits, felt/pick compliance, mode coupling) — but every nonlinearity
  must be band-limited (ADAA like lib.rs::ln_cosh usage, key-tracked drive, or
  provably sub-Nyquist) and denormal-flushed. No naive waveshapers.
- NSynth refs are 16 kHz — trust compare.py only below ~7.5 kHz; use your own
  judgment (and rendered spectra) above.
- Commit your work in the worktree in small, well-messaged commits.

## Final report (your last message — it is the deliverable)

1. Metric table: per held-out + tuned references, before→after on every axis.
2. Change log: each iteration's hypothesis/change/delta (one line each).
3. Physics summary: what the model now includes, with citations.
4. Files touched + commit SHAs. 5. Anything you wanted but couldn't do (shared
   facilities, engine changes) — recommendations only.
