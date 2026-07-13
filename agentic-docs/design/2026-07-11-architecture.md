# Architecture: Rust→WASM single-worklet engine, TS API, physical-modeling instruments

Date: 2026-07-11
Status: accepted — authorizes the architecture below for Q1 issues #3–#10. Does NOT authorize: npm publish, per-instrument WASM splitting, threads/SharedArrayBuffer, neural components.

## Motivation

1. Customer zero (music-transcription-app) needs beautiful, zero-download, multi-track MIDI playback; nothing on npm provides it (competitive survey, 2026-07-11: the intersection {beautiful default} ∩ {tiny bundle} ∩ {trivial API} ∩ {permissive license} is empty).
2. Prior physical-modeling-JS efforts failed on packaging and tone curation, not DSP feasibility (Faust physmodels proves feasibility).
3. Legacy PM codebases share fixable flaws: sample-rate hardcoding, global state, mono-only, no MPE, no smoothing, per-sample non-SIMD loops, no denormal handling.

## Thesis

One Rust DSP core compiled to **single-threaded WASM**, hosted in **one AudioWorkletNode**, exposing a TypeScript API. All instruments across all tracks render and **mix inside the single WASM instance**; the library never asks the host app for COOP/COEP headers, never allocates on the audio path, and treats a full multi-track arrangement — not a solo instrument — as the unit of performance budgeting.

## Evidence base

- WASM ~close to native, worst cases ≈66% slower; denormals in recursive filters are the top perf killer → flush-to-zero everywhere (Letz/Orlarey, *Compiling Faust to WebAssembly*, WWW 2018).
- Physical models are recursive single-sample feedback loops: SIMD pays only by batching independent voices across lanes → structure-of-arrays voice banks from day one (cprimozic Rust+WASM+SIMD synth: 27 KB compressed — the bundle-size existence proof).
- WASM SIMD works in AudioWorkletGlobalScope on Chrome 91+/Firefox 89+/Safari 16.4+; `fetch()` unavailable in worklet scope → compile the module on the main thread, `postMessage` it, instantiate in the processor constructor (Chrome AudioWorklet design-pattern doc).
- iOS Safari: gesture unlock; 44.1 kHz context-lock quirk; ~350 MB WASM heap ceiling; mobile 128-frame quantum is fragile → iPhone-first testing, conservative budgets.
- Commuted synthesis ran a 2-key piano on a 25 MHz DSP in 1995 (Smith & Van Duyne); modern budget is ~1000× → real-time piano is tractable, but staged last (Bank 2003 recipe: nonlinear hammer + coupled/detuned strings + sympathetic resonance + shared soundboard IR).

## Design

**Layer 0 — `crates/dsp` (Rust → wasm32, no wasm-bindgen glue on the hot path).**
Owns: SoA voice bank shared by all instruments; per-sample kernels (modal banks, waveguides, exciters); parameter smoothing; denormal flushing; track buses with gain/pan; the final mix. One `process(out_l, out_r, n_frames)` entry point renders the entire arrangement. Allocation-free after init; voice pool sized at init (default 64 voices across all tracks, degrade-by-voice-stealing, never by glitching). Deferred: exact ABI (issue #5 design doc), per-instrument WASM splitting.

**Layer 1 — `packages/core` (TS).**
Owns: AudioContext lifecycle (lazy, SSR-safe — nothing touches `window` at import), worklet host, WASM load/instantiate handshake, ready state, voice/track allocation policy, the event queue (sample-accurate scheduling via ring buffer into WASM), offline render (OfflineAudioContext → WAV). Multi-track is first-class: `createTrack(instrument, {gain, pan})` returns an independent channel over the shared engine; N tracks share one worklet, one WASM heap, one voice pool.

**Layer 2 — `packages/instruments` + `packages/midi`.**
Owns: instrument façades and presets (GM program map), exciter↔resonator↔body composition API (the Elements template); note-list player (`play(notes)` for the customer-zero shape `{midiPitch, startSeconds, endSeconds, velocity, isDrum, instrumentGroup}`), MIDI file parsing, GM drum map, Web MIDI input.

**Instrument sequencing (wow ÷ effort):** modal mallets → plucked string (EKS) → electric piano → winds → bowed string → acoustic piano (v1.x, commuted hybrid, built last on proven primitives).

## Phased plan

Materialized as GitHub issues #3–#10 under roadmap tracker #1. Q1 exit gate: modal mallets pass AB vs Tone.js/smplr (n≥5 humans) AND 32 voices across ≥4 simultaneous tracks ≤50% of the 2.67 ms budget on M1 and mid-tier Android.

**Amendment — owner decision, 2026-07-13: the performance gate is desktop-first.** The two-device form
above was blocking work on a device class the product is not primarily used on, and it was doing so on the
strength of an *estimated* mobile multiplier, because no real device measurement has ever been taken (#5).
Owner: *"it's fine. it will be used perhaps more on desktop."*

The gate is therefore:

- **Desktop (M1) is the gate.** 32 voices across ≥4 tracks ≤ 50% of the 2.67 ms budget. Unchanged, and it
  still blocks.
- **Mobile is a degradation target, not a gate.** Under-budget on a phone we shed voices and shed quality
  tiers; we never glitch, crackle, or go silent without a diagnostic (PRINCIPLES: *degradation is
  acceptable; corruption is not*). A campaign may ship with a mobile tier that is smaller, quieter in
  polyphony, or simpler in topology than the desktop one.
- **Mobile numbers may no longer block a phase**, and — importantly — **estimated mobile numbers may no
  longer be presented as budget rows at all.** If we are not gating on it, we do not get to pretend we
  measured it.

This supersedes the "on M1 **and** mid-tier Android" clause for all downstream budgets. #5 (real device
measurement) remains open and worth doing — it tells us where the mobile tier sits — but it is no longer a
precondition of anything.

## Deferred until demanded

Threads/SharedArrayBuffer; per-instrument WASM code-splitting; neural/DDSP components (offline parameter fitting is v2 research); React wrapper (post-API-stability, ~Q3); MPE surface in the public API (architecture supports per-voice continuous control from day one; API exposure is v0.5+); tempo/transport abstraction (customer zero pre-bakes absolute seconds).
