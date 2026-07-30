<!--
DRAFT of the report. Written from the KB in this folder. Voice = v03 rigor + Olah
accessibility + Keunwoo's candid register (see author-and-voice.md). Structure = the agreed
two-act shape (audience.md, style-and-structure.md). [TODO ...] marks a real number/eval result
that must come from the repo — we do NOT invent numbers (v03 discipline). Status: Act-I opening only.
-->

# Title — MENU (owner deciding; not locked)

- **A — artifact-only** (your candidate, cleaned): *physical-instruments.js: Physical Models of
  Musical Instruments in 82 KB of JavaScript*
- **B — both threads**: *physical-instruments.js: 29 musical instruments in 82 KB — and the
  agent that modeled them*
- **C — playful, both** (Distill register): *An instrument library the size of a photo, built by
  conversation*
- **D — tight, both**: *29 instruments in 82 KB, written by talking to an agent*

*Trade-off:* A leads with the artifact hook and saves the process reveal for inside (matches our
Act structure); B/C/D put both threads in the title, honest to what the piece actually is.

---

## Abstract

`physical-instruments.js` is a browser instrument library that **synthesizes 29 instruments from
physical models** — vibrating strings, struck bars, blown tubes — rather than from stored
recordings, and ships the whole thing in **~82 KB gzipped** (66.7 KB WebAssembly + 5.3 KB
JavaScript + 2.6 KB audio worklet). That is about the size of one small photograph, for a
library a sampled equivalent would spend hundreds of megabytes on. Every note is computed on the
audio thread in real time, inside a 2.67 ms / 128-frame budget. We describe the models and
characterize the sound honestly, using **objective metrics** — multi-resolution STFT distance,
attack and decay envelopes, partial structure, and integrated (BS.1770) loudness — against
reference recordings, together with the author's ear. **We ran no formal human listening study,
and we say so rather than imply one.** The second half of the paper is about **how it was built:
by directing a general-
purpose AI coding agent over roughly two days**, almost entirely in natural language. We give a
candid account of that collaboration — what the human specified and what he delegated, what he
correctly expected and what surprised him, and where the agent was confidently *wrong* — because
the failure modes that bit were rarely in the DSP and almost always in the seams. The artifact is
live and playable; the reader can hear every claim in this paper by clicking it.

---

## 1. Introduction

A software musical instrument can be built in two broad ways. The dominant approach is sampling:
record a real instrument across many pitches and dynamics and replay the recordings. Sampling is
faithful but heavy, because the fidelity lives in stored audio — a single well-sampled piano is
routinely hundreds of megabytes to several gigabytes. The alternative is physical modeling:
describe the instrument as a physical system — a string under tension, a struck bar, an air
column — and compute its output from those equations at playback. A model stores no audio; its
footprint is the size of its code and coefficients.

`physical-instruments.js` is a physical-modeling instrument library for the browser. It provides
29 instruments [TODO: confirm final count/list against `crates/dsp/src/kernels.rs`], each
synthesized on the audio thread in real time within a 2.67 ms per 128-sample budget, and it ships
in approximately 82 KB gzipped all-in: 66.7 KB of WebAssembly, 5.3 KB of JavaScript, and a 2.6 KB
audio worklet [TODO: re-confirm at the reported commit via `scripts/audit/bundle-size-audit.sh`;
these numbers are owned by that audit, not restated from memory]. That is smaller than a few
seconds of a single recorded note. The compactness is not the result of aggressive compression; it
is a property of the representation, which §[why-tiny] examines directly.

This paper makes two contributions. The first is the artifact and its evaluation: the models used,
the real-time voice architecture that keeps them inside the audio-thread budget, and an honest
characterization of how each instrument sounds — by the author's ear and by objective metrics
(multi-resolution STFT distance, attack/decay, partial structure, integrated BS.1770 loudness)
against reference recordings — including the instruments where the model is not yet convincing.
No formal human listening study was run; we report objective measurements and the author's
judgment, and we do not present either as a controlled perceptual result [TODO: pull the
per-instrument objective numbers from `evals/` where they exist]. The second is a documented account of how the library was
produced: not written line by line, but built by directing a general-purpose AI coding agent in
natural language over roughly two days. We treat that process as a case study and report it
plainly — the division of labor between human judgment and agent execution, the expectations that
held and the ones that did not, and the failure modes, which concentrated in the software seams
around the instrument (the browser audio lifecycle, the build-and-deploy pipeline, device-specific
timing) rather than in the DSP itself. We include the cases in which the agent was confidently
wrong, as they are the most informative.

The artifact is live, and the figures in this paper are playable where relevant: a reader can
trigger a note and hear the model compute it rather than a stored recording. [TODO: live link;
one-octave interactive figures placed at the points they illustrate.]

<!-- Remaining Act-I sections (why-tiny; the models; evaluation) and the Act-II process
     narrative follow once this register is confirmed. -->}
