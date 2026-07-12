# match-reference loop protocol (agent runbook)

You are improving ONE instrument family of the instruments.js Rust DSP engine until
its renders match real reference recordings. You work in an isolated git worktree.
(Updated 2026-07-12 per the modeling & loop audit — trust gates, mr_stft, manifest
masking, standardized auditions. Owner-approved.)

## Environment

- Install the pinned metric environment with `python3 -m pip install -r scripts/dev/requirements-loop.txt`; never compare reports produced by an unrecorded dependency set.
- Reference WAVs live under the session scratchpad `references/<family>/` (or
  `references/<corpus>/`). Check `evals/reference-manifest.json` for each corpus's
  known limitations (sr ceiling, level normalization, release gates) BEFORE
  fitting — compare.py applies manifest masks automatically when the ref path
  matches a manifest entry.
- **Staging a new corpus**: verify the license AT SOURCE and ledger it
  (`agentic-docs/licensing.md` + a SOURCES.txt beside the files), then verify the
  audio actually decodes and a spectrogram looks sane (a staged corpus was once a
  404 HTML page; another was level-normalized and silently faked velocity curves).
  Add a manifest entry recording sr and known artifacts.
- Scratch space for your renders: make a subdir under the scratchpad.

## The iteration cycle (repeat ≥5 times or until plateau)

```sh
cargo build -p instruments-dsp --target wasm32-unknown-unknown --release \
  && cp target/wasm32-unknown-unknown/release/instruments_dsp.wasm packages/core/wasm/
node scripts/dev/render-note.mjs <family> <midi> <vel1-127> 3.0 /path/render.wav 4.0 48000 --float32
python3 scripts/dev/compare.py /path/render.wav <reference.wav>
```

1. Compare against ≥3 references spanning register and velocity. HOLD OUT one
   reference (never tune against it; check it at the end for overfit). Report
   tuned AND held-out; a held-out regression is acceptable ONLY with a
   structural-axis justification (name the axes that improved and why the
   composite disagrees).
2. **Check `interpretation` and `gates` FIRST.** `interpretation: untrusted` or a red gate (onset crest above the reference, near-full-scale sample flips, ultrasonic energy, or DC) means every spectral distance in that report is untrusted; inspect the waveform, fix the artifact, and only then read the distances. Never accept an iteration with a red gate.
3. Headline metric: `mr_stft.mean` (multi-resolution, K-weighted, onset-aligned).
   `logmel_dist` axes remain for attack/mid/tail decomposition. Read ALL axes:
   centroid trajectory, envelope (time-to-peak, t60s), partials
   (frequencies→inharmonicity, level tilt), partial_decay_dbps, crest, LUFS delta
   (velocity-curve fit). Identify the LARGEST mismatch.
4. **Ceiling detector**: if your metric optimum sits at a boundary (clip ceiling,
   parameter clamp, gate threshold), waveform inspection is MANDATORY before
   accepting — a metric that improves as you approach a hard limit is usually
   scoring an artifact. (This exact failure shipped a hi-hat tick once.)
5. State a physical hypothesis for the mismatch BEFORE editing (which model
   element?). Search agentic-docs/research-venues.md literature when changing
   equations.
6. Change ONE thing in your instrument's code. Rebuild. Re-measure. Log:
   `hypothesis → change → metric deltas` (keep a running log for your report).
7. Revert regressions immediately. `cargo test` must stay green EVERY iteration —
   add tests for new behavior (tuning at 44.1k AND 48k if you touch delay math).

## Standing gates (every iteration, not just at the end)

- `schema_version`, `metric_version`, input SHA-256 values, runtime versions, resolved configuration, and preprocessing operations are present in every archived report. Pre-L1 `mr_stft` values are not comparable to metric version `2026.07.12-l1`.
- compare.py `interpretation == "trusted"`, `gates.trusted == true`, and `gates.all_pass == true` on every kept iteration's renders.
- **Crest conformance** (pluck/strike families): onset crest within +6 dB of the
  reference's, and the velocity trend must match (refs: pp attack ≪ body).
- Loudness: re-bake your family's `makeup_gain` row with pyloudnorm
  (`scripts/dev/measure-loudness.{mjs,py}`) at the end; family flat at −20.8 ±0.5.
- Budget: your instrument ≤ ~40 µs/quantum for 8 voices (piano is exempted at
  110 µs post-P1 pending P2; do not use that as precedent).

## Hard rules

- Edit ONLY your instrument's regions: its `start_voice` arm + its kernel struct/impl
  in `crates/dsp/src/kernels.rs`, its rows in `body_defaults`/`pickup_defaults`/
  `amp_defaults`/`makeup_gain`. NEVER touch other instruments, `lib.rs` engine
  plumbing, or the TS/JS layers. If you need a new shared facility, note it in your
  report instead of building it.
- Nonlinearities are WELCOME where physically motivated (tension-modulation pitch
  glide on hard hits, felt/pick compliance, mode coupling) — but every nonlinearity
  must be band-limited (ADAA like lib.rs::ln_cosh usage, key-tracked drive, or
  provably sub-Nyquist) and denormal-flushed. No naive waveshapers.
- Know your corpus's blind spots (manifest!): a 16 kHz corpus cannot police
  anything above 8 kHz — the ultrasonic gate is your only guard there; trust it.
- Renders float32 ONLY (int16 flattered decay tails once).
- Run `npm run audit:loop` before publishing iteration evidence; the versioned schema and equation-owned golden reports must remain unchanged unless the PR explicitly changes metric semantics and bumps the metric version.
- Commit your work in the worktree in small, well-messaged commits.

## Final report (your last message — it is the deliverable)

1. Metric table: per held-out + tuned references, before→after on every axis
   (mr_stft.mean headline), gates status.
2. Change log: each iteration's hypothesis/change/delta (one line each).
3. Physics summary: what the model now includes, with citations.
4. **Standardized auditions**: render your family's set via
   `node scripts/dev/render-auditions.mjs <family> <outdir>` — fixed notes,
   velocities, and filenames so the owner can A/B rounds by ear
   (`scripts/dev/ab-page.mjs <old-dir> <new-dir>` builds the listening page).
5. Files touched + commit SHAs.
6. Anything you wanted but couldn't do (shared facilities, engine changes) —
   recommendations only.
