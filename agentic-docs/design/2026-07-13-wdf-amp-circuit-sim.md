# WDF circuit-simulation guitar amplifier

Date: 2026-07-13
Status: accepted as direction (Keunwoo, verbatim: "amp: it should be circuit
simulation. that's what an amplifier is in the real world.") Supersedes the
behavioral amp chain as the target architecture; the behavioral chain remains
the shipped fallback until the circuit sim beats it at the listening gate.

## Motivation

The current amp is behavioral: ADAA-tanh waveshaper + rail-recovery gain-ride +
post-drive biquad cab/presence. It was reference-fit and gates well, but it is
a description of amp *behavior*, not an amp. A circuit simulation derives the
sag, bias shift, inter-stage loading, tone-stack interaction, and clipping
asymmetry from component values — the behaviors we hand-fit fall out for free,
plus the ones we haven't modeled yet (grid conduction "blocking" distortion,
stage-to-stage loading, tone-stack insertion loss varying with drive).

## Thesis

Wave digital filters (WDF) are the established real-time circuit-sim framework
(Fettweis; Yeh; Werner et al. for R-type adaptors and multi-nonlinearity
roots). `chowdsp_wdf` (BSD-3, header-only C++, ledgered as an approved porting
source since founding) provides the primitive set: adaptors (series/parallel/
polarity), one-ports (R, C, L, resistive voltage source), root nonlinearities
(diode pairs, and the structure for custom roots). Port the needed primitives
to Rust (port-audit ledger per file), then compose:

1. **Preamp**: 1–2 triode gain stages (12AX7, Koren model as the WDF root —
   Dempwolf/Zölzer parameterization acceptable), inter-stage RC coupling.
   Newton iteration at the root with a bounded iteration count and a LUT
   fallback — the audio thread never loops unbounded.
2. **Tone stack**: Fender TMB as a WDF tree (Yeh & Smith's tone-stack analysis
   is the canonical reference); fixed knob positions per channel voicing
   (clean/lead) for v1 — knobs become API later.
3. **Power stage + sag**: cathode-biased push-pull abstraction with a supply
   rail RC (the circuit version of today's gain-ride) — sag/bias-shift emerge
   from the rectifier/supply impedance instead of an envelope follower.
4. **Cab**: keep the current post-EQ biquads for v1 (a measured-IR-informed
   parametric cab is its own later phase).

## Constraints (hard)

- License hygiene: chowdsp_wdf is BSD-3 — port freely WITH ledger entries
  (`agentic-docs/licensing.md` port ledger, one row per ported file). No GPL
  circuit-sim references opened (guitarix, RT-WDF are on the never-open list).
- Audio thread: allocation-free, denormal-flushed, bounded iteration.
  Oversampling only if ADAA-at-the-root can't hold aliasing gates (fizz gate
  exists); 2× max with a half-band pair if needed.
- Budget: amp runs per electric track (≤2 in the demo). Target ≤35 µs/quantum
  per instance at 48 kHz; hard cap 60. The full-demo budget gate (50%) is far
  away, but the fleet guideline stands.
- Behavior gates: everything the behavioral chain passes must pass — singing
  sustain (hold-within-−3dB numbers at or above the gain-ride's), fizz,
  chug LF dominance, IMD presence, EP-disambiguation, velocity contrast — plus
  K-weighted reference fits at least as good on the FreePats lead refs and the
  NSynth-amp'd clean cluster.
- The listening gate: Keunwoo A/Bs behavioral vs WDF on the standardized
  auditions before the WDF chain becomes the default.

## Phases

- **P1 (this round)**: WDF primitive port + single triode stage + tone stack +
  supply sag, clean AND lead channel voicings, behind an engine-internal
  selector (behavioral chain untouched and still default); full gate + fit
  report; auditions.
- **P2**: second triode stage / cascade for the lead channel, grid-conduction
  blocking, knob exposure in the API.
- **P3**: parametric cab from measured speaker curves (Zollner ch. 10),
  possibly per-voice post-clip presence (the filed B3-register item).

## Evidence base

Fettweis 1986 (WDF); Yeh, Abel & Smith 2010 + Yeh thesis (tone stack, triode
sims); Koren 1996 triode model; Dempwolf & Zölzer 2011; Werner et al. 2015
(R-type adaptors, multiple nonlinearities); Pakarinen & Yeh 2009 (review);
chowdsp_wdf source (BSD-3, ported with ledger). Reference audio: existing
ledgered FreePats CC0 lead refs + NSynth amp'd cluster; agent may fetch more
license-verified amp'd corpora.

## Deferred until demanded

Full SPICE-accuracy claims, transformer core hysteresis, user-facing component
values, multi-amp models (Fender/Marshall/Vox switching), IR-based cabs.
