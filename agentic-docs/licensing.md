# Licensing ledger & clean-room policy

instruments.js is dual-licensed **MIT OR Apache-2.0** (user's choice, Rust-ecosystem convention).
The permissive license is part of the product. This file is the single owner of porting policy and provenance.

## Clean-room policy (papers-only for copyleft)

- **Permissive sources (MIT/BSD/similar): port freely.** Every ported file gets a ledger entry below and a header comment naming origin + license.
- **Copyleft sources (GPL/LGPL/AGPL): NEVER open the source.** Not "read for understanding" — never open. Algorithms from copyleft projects are reimplemented from published papers only. If you catch yourself with copyleft source in context, stop, note it in the ledger's incident log, and hand the implementation to a fresh context that has not seen it.
- Faust libraries are licensed **per-function**: audit each function used; only STK-4.3/MIT/BSD functions may influence shipped code.

## Approved porting sources (license verified 2026-07-11)

| Source | License | Status | Contents we may take |
|---|---|---|---|
| STK (thestk/stk) | MIT-style | ✅ port | Plucked, StifKarp, Bowed, Mandolin, Flute, Clarinet, Brass, Saxofony, ModalBar, BandedWaveguide, Shakers/PhISEM |
| Mutable Instruments (pichenettes/eurorack + stmlib) | MIT | ✅ port | Rings, Elements, Plaits string/modal engines. NOT the GPL SuperCollider wrapper (mi-UGens) |
| chowdsp_wdf | BSD-3 | ✅ port | WDF primitives (adaptors, nonlinear elements) |
| Faust physmodels.lib | mixed per-function | ⚠️ audit each function | permissive functions only; prototyping reference |
| NESS (Edinburgh) | MIT | ✅ reference/offline | offline algorithms, soundboard-IR generation; not real-time code |

## Reference-only (copyleft — papers-only, never open source files)

| Source | License | Papers to use instead |
|---|---|---|
| OpenPiano | AGPL-3.0 | Bank 2003 (EURASIP), Smith & Van Duyne 1995 (commuted piano) |
| SDT Sound Design Toolkit | GPL-3.0 | Rocchesso/Avanzini/Fontana modal-impact & friction papers |
| Csound opcodes (barmodel, prepiano, wg*) | LGPL-2.1 | Bilbao, *Numerical Sound Synthesis* |
| mi-gen / MIMS / miPhysics | GPL | CORDIS-ANIMA papers (Cadoz et al.) |
| guitarix, RT-WDF | GPL | (not needed) |

## Reference audio (match-reference loop)

| Source | License | Use | Redistribution |
|---|---|---|---|
| NSynth test set (Engel et al. 2017, Magenta/Google) | CC-BY 4.0 | local reference corpus for render↔reference comparison (scratchpad only) | NOT committed to the repo; if references ever ship, add attribution per CC-BY |
| virtuosity-drums (github.com/studiorack/virtuosity-drums) | CC0-1.0 (verified) | ride + hi-hat measurement targets for the cymbal engine and exact close-kick campaign sources; the committed identity and transformation receipt is [`evals/reference-receipts/drums-jazz-virtuosity-kick-v1.json`](../evals/reference-receipts/drums-jazz-virtuosity-kick-v1.json) | source audio is not committed; CC0 = no attribution required (provenance kept anyway) |
| VSCO-2-CE (github.com/sgossner/VSCO-2-CE) | CC0-1.0 (verified) | crash measurement target + Upright Piano 44.1 kHz refs (Player_dyn{1,2,3}_rr1_{006,014,026}.wav → A1/C♯3/C♯5, piano round-2 full-band checks; scratchpad only) | not committed |
| Salamander Grand Piano (Alexander Holm; tonejs.github.io mirror + full V3 SFZ+FLAC pack from freepats.zenvoid.org/Piano/SalamanderGrandPiano, fetched 2026-07-12) | CC-BY-3.0 | per-key piano calibration references, P1 campaign — 30 keys × 16 velocity layers, 48 kHz FLAC; the committed identity and transformation receipt for four attack cases is [`evals/reference-receipts/piano-salamander-attack-v1.json`](../evals/reference-receipts/piano-salamander-attack-v1.json) | source and canonical audio are not committed; attribute Alexander Holm if references ever ship |
| FreePats FSBS Electric Guitar bridge dist2 (freepats.zenvoid.org/ElectricGuitar, EGuitarFSBS-bridge-dist2-SFZ+FLAC-20220911.7z) | CC0-1.0 (verified: cc0.txt in archive + page statement, fetched 2026-07-12) | distorted lead-channel references — sustain envelope, band balance, 2.5–7.5 kHz flatness targets (electrics round 3; scratchpad only) | not committed; CC0 = no attribution required (provenance kept anyway) |
| Karoryfer Black And Blue Basses (github.com/sfzinstruments/karoryfer.black-and-blue-basses) | CC0-1.0 (verified: `license` file in repo = full CC0 legal code, fetched 2026-07-12) | electric-bass reference round: darkblack (fingered, neck PU) primary tuned corpus E1/A1/D2/G2 × p/mf/f; babyblue (picked, bridge PU) picked-articulation gap measurement (scratchpad only) | not committed; CC0 = no attribution required (provenance kept anyway) |
| Karoryfer Growlybass (github.com/sfzinstruments/karoryfer.growlybass) | CC0-1.0 (verified: LICENSE in repo, fetched 2026-07-12) | electric-bass cross-instrument held-out checks (aggressive DI Jazz Bass, roundwounds; scratchpad only) | not committed |
| FreePats Electric Bass YR (github.com/freepats/electric-bass-YR) | CC0-1.0 (verified: LICENSE.txt in repo + freepats.zenvoid.org page, fetched 2026-07-12) | electric-bass third-instrument held-out sanity refs, finger + pick (scratchpad only) | not committed |
| Greg Sullivan E-Pianos (github.com/sfzinstruments/GregSullivan.E-Pianos, fetched 2026-07-12) | CC-BY-3.0 (verified: repo LICENSE = full CC-BY 3.0 legal code; README attribution statement) | Wurlitzer EP200 4-velocity ladders (bark/attack/harmonic-balance targets) + Pianet T long-decay sanity refs, EP round (scratchpad only). Corpus caveats in evals/reference-manifest.json (peak-normalized, trimmed tails, built-in tremolo on low notes) | not committed; attribute Greg Sullivan if references ever ship |
| jRhodes3c/3d (sfzinstruments) | CC **BY-NC** 4.0 — NOT usable | none (rejected at license check 2026-07-12; LICENSE grants BY-SA to Strudel only). Best free Rhodes set otherwise; author invites license requests at jjlearman@gmail.com — a human-side email could unlock it | n/a |

## Port ledger

Every ported file: `| path | origin file | origin license | date | PR | notes |`

| path | origin | license | date | PR | notes |
|---|---|---|---|---|---|
| `crates/dsp/src/wdf.rs` (WDF primitives section) | chowdsp_wdf `include/chowdsp_wdf/wdft/{wdft_base,wdft_one_ports,wdft_adaptors,wdft_sources}.h` (Chowdhury-DSP) | BSD-3-Clause | 2026-07-13 | (worktree, feat/first-sound) | Rust port of the templated `wdft` one-ports (Resistor, Capacitor, ResistiveVoltageSource) + adaptors (WDFSeries→`Series`, WDFParallel→`Parallel`, PolarityInverter) + `WDFMembers`→`Wdf` + `BaseWDF`→`Port` trait. Allocation-free monomorphized (no Box/dyn); wave-scattering equations faithful to the origin. LICENSE header verified at github.com/Chowdhury-DSP/chowdsp_wdf (BSD-3-Clause, Copyright 2022 Chowdhury-DSP). Legacy-flaws fixed at port: per-instance fs (no global SR), f64 documented exception (triode Newton conditioning), denormal-flushed recursive state. The 12AX7 Koren root + Fender TMB tone stack (Yeh–Smith bilinear) in the same file are clean-room reimplementations from published papers, NOT ported code. |

## Incident log

_(none)_
