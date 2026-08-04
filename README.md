<p align="center">
  <img src="https://raw.githubusercontent.com/keunwoochoi/physical-instruments.js/main/assets/logo/logo-256.png" width="140" alt="physical-instruments.js — feather tile-mosaic logo">
</p>

# physical-instruments.js

[![npm](https://img.shields.io/npm/v/physical-instruments.js.svg)](https://www.npmjs.com/package/physical-instruments.js)
[![license](https://img.shields.io/npm/l/physical-instruments.js.svg)](LICENSE-MIT)

A physical-modeling instrument library for browsers. 29 instruments in 81 KB gzipped, MIT/Apache-2.0.

[npm package](https://www.npmjs.com/package/physical-instruments.js) | [Demo](https://keunwoochoi.github.io/physical-instruments.js/) | [API](https://github.com/keunwoochoi/physical-instruments.js/blob/main/packages/core/README.md) | [Changelog](https://github.com/keunwoochoi/physical-instruments.js/blob/main/CHANGELOG.md)

```sh
npm install physical-instruments.js
```

```ts
import { createEngine } from "physical-instruments.js";

const engine = await createEngine();          // lazy AudioContext, gesture-safe
const piano  = engine.createTrack("piano");
piano.noteOn(60, 96);                          // velocity changes timbre, not just volume
```

No bundler configuration, no files to copy.

---

# Tech report: physical-instruments.js

@keunwoochoi, July 2026

Instrument libraries for the browser are usually either sample-based — recorded audio — or simple synths. Physical modeling is the old third option: simulate the string, bar, or tube, and compute the sound from the simulation. Although sample-based virtual instruments have flourished in the modern music production workflow, physical modeling still has a benefit in some use-cases as it is extremely lightweight. Historically, the DSP is well studied (STK, Mutable Instruments, decades of papers); what didn't exist was a plain npm library of it for web developers.

I spent some time (and tokens) building one. The coding agent wrote effectively all the code while I listened, orchestrated, and complained. Through the process, I also ended up designing a small harness so that the agent can work on it as autonomously as possible. This report covers the library first and the process second.

## 1. What is `physical-instruments.js`


|                                     |                                                                                         |
| ----------------------------------- | --------------------------------------------------------------------------------------- |
| Instruments in the engine           | 29 — 22 selectable in the demo; the rest reachable via General MIDI, 2 of those dormant |
| Total download, gzipped             | 85,367 B (83 KB)                                                                        |
| ↳ WebAssembly                       | 74,386 B                                                                                |
| ↳ core JS                           | 5,401 B                                                                                 |
| ↳ AudioWorklet                      | 2,689 B                                                                                 |
| ↳ MIDI parser                       | 2,891 B                                                                                 |
| CPU, 6-track arrangement, 33 voices | 309.4 µs per 128-frame quantum = 11.6% of the 2.67 ms real-time budget                  |
| NaN samples / peak, in that render  | 0 / 0.849                                                                               |
| Rust DSP tests                      | 86 passed, 0 failed                                                                     |


For scale -- a modest sampled piano is ~300 MB, i.e. physical-instruments.js is about 300 MB / 83 KB ≈ 3,700 times smaller.

The 29 available instruments are as follows: piano, electric piano, organ, four guitars (nylon / steel / electric / distorted), bass, harp, pizzicato, violin, viola, cello, contrabass, trumpet, trombone, marimba, vibraphone, xylophone, glockenspiel, tubular bells, celesta, music box, synth pad, and three drum kits (standard / rock / jazz). Saxophone and french horn are in the binary but turned off; those models are unfinished.

## 2. Model

Every instrument has the same structure:

```
   noteOn(60, 96)
        │
        ▼
   ┌──────────┐   excitation   ┌──────────────┐   coupling   ┌──────────┐
   │ EXCITER  │───────────────>│  RESONATOR   │─────────────>│   BODY   │──> out
   │ hammer   │<───────────────│  string /    │<─────────────│ soundbrd │
   │ bow      │   reaction     │  bar / bore  │  reflection  │ / bell   │
   │ lip      │                └──────────────┘              └──────────┘
   └──────────┘                       ▲  │
                                      └──┘
                                 delay line, loop filter
                                 (the string, literally)
```

A physical model such as `physical-instruments.js` produces each note by simulating the physical structure that makes the sound. For a plucked string, the resonator is a delay line with a loss filter in the feedback path: a wave travels down the string, reflects, comes back a little darker. For A4 at 48 kHz the delay is 48000 / 440 ≈ 109 samples, so the string is a 109-float array. The same object is the nylon guitar, the steel guitar, the harp, and (three of them, coupled at a bridge) the piano. What changes is the exciter and the box.

Velocity-dependent timbre also comes out of the simulation: hit harder, the simulated hammer contact shortens, the spectrum brightens. A sampler stores extra velocity layers for this, which is a lot of where its megabytes go. Same with the sustain pedal — the other strings are still in the simulation, so they ring sympathetically without any extra data.

The downside is clear and well-known: a physical model is still a model, a simplified simulation of the real world. Human ears are surprisingly sensitive to small details caused by this approximation, especially the nonlinearity.

## 3. Related work

The plucked-string loop is Karplus–Strong (1983), extended by Jaffe & Smith (1983); the general framework is Julius O. Smith's digital waveguide synthesis; the piano-unison physics is Weinreich (1977). The two codebases this project owes the most to are [STK](https://github.com/thestk/stk) (Cook & Scavone — the canonical C++ physical-modeling collection, MIT-style) and [Mutable Instruments](https://github.com/pichenettes/eurorack) (Émilie Gillet's modal and string engines, MIT). Several of our models are ports or descendants of those two lineages, tracked file-by-file in a licensing ledger; algorithms from copyleft projects were reimplemented from the papers without opening the source.

The quality ceiling for physical modeling is commercial: Pianoteq (Modartt), SWAM (Audio Modeling), MODO Bass (IK Multimedia).

The following table summarizes some existing software in this direction.


| Project                                                                                                                           | Models                                                            | Runtime                      | Notes                                                                          |
| --------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- | ---------------------------- | ------------------------------------------------------------------------------ |
| [Tone.js](https://tonejs.github.io/) `PluckSynth`                                                                                 | Karplus–Strong pluck                                              | Web Audio / TS               | production-ready, one technique, a few parameters                              |
| [Gibberish.js](https://github.com/gibber-cc/gibberish) / [genish.js](https://github.com/charlieroberts/genish.js)                 | Karplus–Strong ugen; per-sample DSP graphs                        | JS codegen                   | the build-your-own route: delays, filters, nonlinearities                      |
| [javascript-karplus-strong](https://github.com/mrahtz/javascript-karplus-strong)                                                  | plucked guitar string                                             | JS, pre-AudioWorklet         | small educational implementation                                               |
| [Pink Trombone](https://dood.al/pinktrombone/)                                                                                    | glottis + vocal-tract waveguide                                   | JS                           | an expressive sample-free model in a browser, for voice; not a MIDI instrument |
| [Flues / Stove](https://github.com/danja/flues)                                                                                   | waveguides with pluck / bow / reed / brass interfaces             | JS ES modules + AudioWorklet | closest hand-written-JS relative; self-described as experiments in progress    |
| [Faust](https://faustlibraries.grame.fr/libs/physmodels/) `physmodels.lib` + [FaustWasm](https://github.com/grame-cncm/faustwasm) | strings, bowed strings, winds, brass, modal percussion, membranes | Faust → precompiled WASM     | broadest model coverage; see below                                             |
| [libsonare](https://github.com/libraz/libsonare)                                                                                  | audio engine with built-in instruments                            | C++ → WASM / Node            | new; much broader scope (analysis, mastering, headless DAW)                    |


Among these, Faust is the closest comparison. `physmodels.lib` covers more instrument types than any hand-written JS project, and FaustWasm compiles them to WASM Web Audio nodes with MIDI support. The runtime architecture is the same as this project's: JS for control and scheduling, an AudioWorklet, a precompiled WASM DSP. The difference is scope. Faust is a toolchain — you pick models, compile, tune, and level them yourself — and for building a custom model it is the better starting point. physical-instruments.js instead ships a fixed, loudness-matched instrument set behind one `noteOn()` API, with multi-track mixing in a single worklet and GM routing.

As far as I can find, the lightweight browser projects are either single-technique (Karplus–Strong plucks) or single-instrument (a voice, a clarinet), and no pure-browser library covers a broad instrument set with the articulation and body behavior of the commercial modelers. This one doesn't either; it covers the set, at the quality §4 reports.

## 4. Does it sound good?

In my opinion, the mallets and guitars are good, the piano is close, the bowed strings and brass are behind, and two instruments are off.

**Piano.** My messages to the agent over the two weeks, in order:

> "piano too loud and kinda weird" → "piano sucks" → "piano sounds like harpsichord" → "sounds better." → "now i think the piano sound quality is like maybe yamaha p80 level. not bad i meant. good progress. but we can make it even better!!"

The most useful complaint was this one: *"there is a similarity between this piano model sound and the electric guitar sound. and i don't like it. those twang..."* No metric we had could see what I meant, so the agent built one — the two-stage decay ratio, how much faster the initial "prompt" decay is than the aftersound. A real piano's three unison strings dump energy into the bridge fast in phase, then ring on out of phase (Weinreich 1977); a plucked string just decays.


|                                       | ratio |
| ------------------------------------- | ----- |
| our piano, then                       | 1.57× |
| our electric guitar, same measurement | 2.16× |
| a real piano                          | 2–4×  |
| our piano, now                        | 4.21× |


So the complaint was measurable: by this metric, the piano behaved more like a plucked string than the actual plucked-string instruments did. The fix took three commits — give the three unison strings audible, irregularly detuned weights instead of muting two of them; strike them at different points, one sample apart; delete an artificial pitch sweep the agent had added to imitate the beating that three detuned strings produce anyway. Unison beating now measures 3.5–5.8 dB across harmonics 1–8 (real piano: 2–6 dB), and the deepest tail dip is 5.6 dB where the old code had a −40 dB null.

**Trombone.** My message: *"trombone: so bad. you need auto-research with real trombone sound."* The problem was method: for two sessions the agent had tuned the trombone against target numbers it produced from memory. Then it measured a real one (VSCO-2-CE tenor trombone, CC0, 31 notes × 3 velocities):


|                   | it had claimed | measured from the recording |
| ----------------- | -------------- | --------------------------- |
| centroid, pp → ff | 500 → 1500 Hz  | 454 → 901 Hz                |
| attack            | 30–80 ms       | 80–200 ms                   |
| dynamic range     | 20–30 dB       | 17.3 dB                     |


Against that reference, ours was 3.5× too bright at pp, 12–18 dB too quiet in the 320–1300 Hz band (the band that carries most of a trombone's character), and +49.6 dB too loud between 2 and 5 kHz. The brightness-vs-velocity slope was also inverted — ours got darker as you blow harder. The previous session had gone into pushing brightness up, i.e. making this worse.

**Tuning.** The cello played up to 97 cents flat, and flatter the lighter you bowed. Root cause: a lagging friction-temperature state delayed the stick-slip release by a fixed 10–20 samples regardless of pitch. After the fix, worst error over C1–C4 at pp and ff: 1 cent. The trumpet was +44 cents sharp on its n=3 slot (B3–F4, the core notes). Measured per slot and cancelled in the bore length: notes more than 35 cents off went from 4–9 to 0–1 across the range.

**Drums.** *"no focus on jazz drum kick. it is fundamentally annoying"* — annoying was literal: the kick sang a clear 76 Hz pitch instead of thumping. The agent turned the sentence into a unit test — normalized 45–120 Hz autocorrelation must stay below 0.40. It was 0.629; it is now 0.225.

**Loudness.** I asked whether loudness was actually normalized across instruments (I had specified pyloudnorm). Measuring exposed that 9 of the 29 families had never been LUFS-measured at all. Now 26 of 29 sit at −22.5 LUFS ± ~1 dB (BS.1770 integrated). The exceptions: the dormant saxophone and french horn, and the trumpet, whose gain is capped below the target because full makeup would clip it.

One standing rule: when my ear and the reference metric disagree, my ear wins and the divergence is written into the commit. This happened multiple times.

## 5. Auto-Research

Auto-research is the workflow where the agent iterates on an instrument against reference recordings without me in the loop; I listen at the end of a cycle.

**Harness.** The agent's workflows are defined as text files (skills) in the repo. Per instrument there are six audits — stability, headroom, tuning, envelope, dynamics, voice — run in that order, because clipping or NaN in an earlier stage invalidates the later measurements. CI runs the fixed checks: bundle size, a clip test over every semitone at multiple velocities, DSP cost per audio quantum, and the Rust test suite.

**References.** Each instrument has license-verified recordings: single notes at ≥3 velocities × ≥3 registers, plus one musical phrase. References are stored with sha256 digests and a per-corpus manifest of which measurement axes the recording supports; unsupported axes are omitted from reports.

**Similarity measurement.** The model is rendered at the same notes and velocities as the reference, and the two signals are compared per axis:

- log-mel spectrogram distance (K-weighted, 64 mel bands)
- multi-resolution STFT distance
- integrated loudness (BS.1770)
- attack: envelope and spectral-centroid trajectory over the first 50 ms
- decay: two-stage t60
- partial frequencies and amplitudes (inharmonicity)

There is no combined score; the axes are reported separately.

**Validity gates.** Before the distances are used, the render passes artifact checks: finiteness, clipping occupancy, onset crest relative to the reference (>6 dB excess indicates a click), ultrasonic energy ratio at the native render rate, DC offset, max sample jump, pre-onset energy, release discontinuity. A failed gate marks the report untrusted and its distances are not used. These gates were added after the loop twice improved its score by generating artifacts the metric could not see.

**Protocol.** One iteration changes one parameter or equation, with a citation or a stated derivation. The test suite must stay green. One velocity or register per axis is held out. Reports are schema-validated and store the metric version and input digests. The loop does not commit DSP changes on its own.

---

Demo: [https://keunwoochoi.github.io/physical-instruments.js/](https://keunwoochoi.github.io/physical-instruments.js/)  
Source: [https://github.com/keunwoochoi/physical-instruments.js](https://github.com/keunwoochoi/physical-instruments.js)

