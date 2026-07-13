# Higher-capacity piano: make the box real

Date: 2026-07-13
Status: draft (**revision 3**) — proposes an architecture and a staged plan for issue #49.

Revision 2 answers a 7/7 blocking persona panel on revision 1. If you reviewed r1, read
"What revision 1 got wrong" at the end first — the errors are the useful part.

This doc authorizes **nothing to ship**. It selects a first architectural bet, freezes a comparator,
and defines the budgets and gates a prototype must clear. It does **not** authorize any piano model
change, any product-budget increase, samples in the runtime, or any quality claim. Owner blind
listening (AB/ABX for iteration, MUSHRA at the gate, per `PRINCIPLES.md`) remains the acceptance
gate. Phase P0 must complete before any DSP phase begins.

## Motivation

1. Owner direction (#49): the piano "has likely reached the ceiling of small parameter and
   local-topology changes." Post-#41 verdict: *"maybe yamaha p80 level. not bad i meant. good
   progress. but we can make it even better!!"* — a floor accepted, a ceiling raised.
2. North star, unchanged: *"I want our piano sound to be as good as the Pianoteq sound."*
3. `2026-07-11-pianoteq-class-piano.md` phase **P2 (soundboard/radiation) was never done**, and the
   modeling-loop audit names it "the weakest link." This campaign is that phase, escalated from a
   filter swap to a topology change, plus the coupling the topology makes possible.

### What this campaign does NOT do — read before accepting

**It does not improve the attack.** Issue #38's complaint was the attack — *"the felt hammer
attacking the string — that's what we need to improve more"* — and its surviving residual is **R5**
(C4 *pp* attack centroid 359 Hz vs 423 Hz reference; still dark).

**A soundboard is linear.** It changes the transfer, not the excitation, so it is velocity-independent
by construction. **No mechanism in the recommended architecture touches touch response.** R5 is
addressed only by the *deferred* architecture (B).

The pivot to the box is defensible — #49 says explicitly that the remaining gap is no longer a local
calibration problem, and the project's own audit names the board as the weakest link. But it is a
**trade**: the attack stays as it is through P5. That must be a knowing choice, not a discovery at P5.

## Thesis

**Today the piano has no box.** The strings are modeled with real physics; everything downstream is
feed-forward cosmetics. String energy never passes through a soundboard, the board never loads the
string, and no string ever hears another string.

The bet: spend the new capacity on **the radiator and the coupling** — a real frequency-dependent
bridge admittance, a dense modal soundboard **shared across all piano voices**, and then close the
loop so board motion returns into the strings.

That bet stands or falls on one arithmetic claim, so it is stated up front and measured below.
Coupling each string to every mode costs **O(M) per voice** — ~25 µs/voice at M=400, which is
unaffordable on its own. The design therefore couples strings to the board through a **low-rank basis
of P bridge ports**: each voice reads and writes only its port, and the P×M projection runs **once per
sample regardless of polyphony**. *That* is what makes the dominant new cost shared. It is measured,
not asserted.

## Evidence base

### Baseline freeze (the comparator #49 requires)

| Identity | Value |
|---|---|
| Commit | `0bf0ec3c4b4db7cb9aa7ed054ee4a056e1a95ed8` (`piano/hammer-contact-attack`, PR #41 head) |
| WASM sha256 | `1fd2dc6d2e47892f1f07e98e264887ac8334136cc9aacb4365c78ac7df801170` |
| WASM size | 163,430 B raw / 67,088 B gz (`gzip -9`) |

Rebuild: `cargo build -p instruments-dsp --target wasm32-unknown-unknown --release` at that commit.
**This piano is not on `main`** — PR #41 is still draft, awaiting owner listening. The comparator is
necessarily a branch head; **if #41 is rejected, this baseline moves and every number below must be
re-derived.**

CPU, marginal cost per held voice (budget 2666.7 µs @48 kHz, 2902.5 µs @44.1 kHz):

| | 48 kHz | 44.1 kHz |
|---|---|---|
| piano, per voice | **13.22 µs** | 13.38 µs |
| piano ×32 | 438.5 µs (16.4%) | 439.2 µs (15.1%) |
| piano ×64 | 870.5 µs (32.6%) | 887.4 µs (30.6%) |
| idle engine | 19.9 µs (0.75%) | 19.6 µs |

For scale: bass 3.89 µs/voice, guitar 3.70, e-piano 1.96, synth pad 0.40. **The piano is already 3.4×
the next-dearest voice.**

**A cost fact about the comparator itself:** ×64 costs 870 µs at #41's head but 659 µs on the sibling
branch that still overwrites same-key voices. PR #41's voice-lifecycle fix (repeated strikes get their
own voices) is a **~32% CPU regression at full polyphony**, and it is part of what we are budgeting
against.

Memory (`std::mem::size_of`):

| Struct | Bytes |
|---|---|
| `PianoVoice` | **25,760** |
| `Kernel` (enum) | 25,760 — *piano is the largest variant; it sets the size* |
| `Voice` | 25,776 |
| **Voice bank (64 slots)** | **1,649,664** |
| `SympBank` × 16 tracks | 792,192 |

**The governing scaling law.** `Kernel` is a Rust enum sized by its largest variant, so all 64 voice
slots carry piano-sized state even when holding a 280-byte marimba. **+1 KB of per-voice piano state
= +64 KB of engine; shared state costs ×1.** This is the strongest fact in the document, and it kills
any architecture that spends per-voice.

### The engine you are actually budgeting against

Stated plainly, because revision 1 got it wrong:

- **The voice bank is AoS and scalar.** `voices: Vec<Voice>`, per-voice `Kernel` enum dispatched by
  `match` in the render loop. There is **no SIMD anywhere** in `crates/dsp/`. (`AGENTS.md`'s repo map
  calls it a "SoA voice engine" — that line is false, and this PR corrects it.) Every µs figure here
  is a **scalar** measurement, which is the honest basis for a budget.
- **The render loop is block-serial.** `Engine::process` renders each voice's *entire 128-frame block*
  before starting the next; shared and track processing run after all voices are summed. **A
  one-sample string↔board loop cannot be expressed in this loop shape.** This is the largest unknown
  in the plan, and P1 exists to measure it.
- **Voices are owned by tracks**, each with its own bus, body, pan, gain and sends. A board "shared
  per engine" would sum two piano tracks into one radiator and destroy per-track routing. The board is
  therefore **per piano-track**, and the budget counts it that way.

### Soundboard cost — measured and reproducible

A measurement scaffold is committed behind a Cargo feature, so it **never enters the shipped WASM**
(verified: with the feature off, the build is byte-identical to `main`'s shipped binary):

```
cargo build -p instruments-dsp --target wasm32-unknown-unknown --release --features bench-scaffold
npm run bench:soundboard              # SR=44100 npm run bench:soundboard for the other rate
```

**Open-loop board (A2)** — M modes driven by the summed bridge force. ~0.9 ns per mode-sample, flat:

| modes | µs/quantum | % budget |
|---|---|---|
| 128 | 13.6 | 0.51% |
| 200 | 22.9 | 0.86% |
| 400 | 47.1 | 1.77% |

**The open-loop board is nearly free** — 400 modes costs under 2% of budget.

**Closed-loop coupling (A3)** — P bridge ports ↔ M modes, both directions. **Shared: independent of
how many voices sound.**

| ports | modes | µs/quantum | % budget |
|---|---|---|---|
| 4 | 96 | 88.8 | 3.3% |
| 4 | 128 | 117.6 | 4.4% |
| 4 | 200 | 185.1 | 6.9% |
| **6** | **200** | **256.9** | **9.6%** |
| 8 | 256 | 371.7 | 13.9% |
| 8 | 400 | 577.0 | 21.6% |

The coupling, not the resonators, is the cost, and it scales as P×M. This is precisely why the
per-string alternative is dead: projecting each of 32 voices onto 400 modes is **~25 µs per voice**
(≈800 µs at 32 voices — 30% of budget for the coupling term alone), while **6 ports × 200 modes is
257 µs total, whether one note sounds or sixty-four.**

### Mobile: the exit gate is **already violated**, before this campaign

The architecture doc's exit gate is *"32 voices across ≥4 tracks ≤ 50% of budget **on M1 and mid-tier
Android**."* Every number in this doc is desktop, and a mid-tier phone is roughly 3–4× slower. Applying
3.5× to the measured figures:

| config | 16 piano voices | 32 piano voices |
|---|---|---|
| **baseline piano, no board** | 890 µs (33%) | **1,481 µs (56%) — OVER GATE** |
| + open-loop board (200 modes) | 890 µs (33%) | 1,630 µs (61%) — over |
| + closed loop, mobile ladder (4×96) | 1,121 µs (42%) | 1,861 µs (70%) — over |
| + closed loop, desktop config (6×200) | **1,709 µs (64%) — over** | 2,449 µs (92%) — over |

Three things follow, and they are worse than "we need a smaller board":

1. **The frozen comparator already fails the exit gate on a phone.** Baseline piano ×32 = 438.5 µs
   desktop → **~1,535 µs ≈ 58%** at 3.5×, against a 50% gate. ×64 → ~3,045 µs = **114%: dropouts.**
   This campaign did not cause that. It **inherits** it, and nobody caught it because **there is still no
   device measurement at all** — #5 has been open since the day-1 panel.
2. **The degradation ladder does not save mobile, because the board was never the binding constraint
   there.** Dropping 6×200 → 4×96 buys back ~590 µs on a phone (≈900 → ≈311 µs). But **32 piano voices
   alone are ~1,481 µs ≈ 56%.** The 13.22 µs/voice string is what breaks the phone, and raising the
   per-voice ceiling to 20 µs makes it worse, not better.
3. **Therefore A3 — and plausibly A2 — is desktop-only unless piano polyphony on mobile is capped well
   below 32 voices.** That cap is a **product decision, not an implementer's**, and it needs to be made
   explicitly rather than discovered at P4.

Consequences for the plan:

- **#5 is a hard precondition of P0**, not of P4. P0 must produce a **measured mobile baseline** (iPhone 8
  and Pixel 6a, both rates) before any ceiling in the budget table means anything.
- **Every budget row needs a mobile column.** A desktop number sitting under a two-device gate is exactly
  the error this revision exists to stop.
- The **3.5× multiplier above is an estimate standing in for a measurement**, and it is doing load-bearing
  work in a budget — the same sin as r1's unmeasured "3×" for Architecture B. Treat this table as a
  **falsifiable prediction, not a result.**

**The degradation ladder needs a selector, and it cannot be a runtime one.** Truncating a bank of ringing
high-Q modes, or changing P, is a **discontinuity in resonating state** — you cannot drop 104 sounding
modes mid-arrangement without an audible step. The rung must be chosen **at init**, from a device tier
established by a **startup micro-bench inside the gesture-unlock path** (a user-agent string is not a CPU
measurement), and that bench has to fit inside the 20 ms init budget. What happens on a mid-session
thermal throttle — which *will* happen on a phone — must also be stated. None of this exists yet.

### Named residuals (each mechanism must cite one)

Verified against `crates/dsp/src/kernels.rs` at the baseline. This branch contains that file.

- **R1 — The soundboard is a knock generator, not a radiator.** `PIANO_BOARD_MODES = 12`, excited by a
  *synthetic raised-cosine pulse* rather than by string energy, and gated off after 0.9 s. **No string
  energy ever passes through a board.** The string's radiation path is pure EQ, including one **fixed
  270 Hz / −13 dB / Q 1.25** dip standing in for a mobility antiresonance — static, key-independent.
- **R2 — The bridge admittance is first-order, per-note, and one-way.** *(Corrected: r1 claimed there
  was none.)* There **is** a frequency-dependent G(ω) = g0 + g1·H(ω), with a documented
  Giordano-mobility rationale and per-partial decay fits, and it carries a closed-form passivity
  argument. Its real defects: first-order (wrong per-partial spread), couples only the 2–3 unisons of
  *one* note, the board never loads it, and there is no return path.
- **R3 — Sympathetic resonance is a fixed C-major chord.** `SYMP_TUNING` is hardcoded to
  C2 G2 C3 E3 G3 A#3 C4 D4 E4 G4 A4 C5, fed feed-forward, with no return path and **no relationship to
  the notes actually held**. *Play in F# minor and the "sympathetic" bloom rings C major.* This is
  #14's finding, and it is worse than #14 knew.
- **R4 — Bass radiation excess at 20–60 Hz** (*Hz*, not dB — an r1 typo). The +15 dB figure comes from
  the modeling-loop audit, which measured a **6-mode** board; the frozen baseline has **12**. **This
  number must be re-measured on the frozen baseline before it can gate anything.**
- **R5 — Attack timbre.** C4 *pp* centroid 359 Hz vs 423 Hz; A1/C4 *ff* run 3–4% high. **Not addressed
  by this campaign.**
- **R6 — No re-strike into a ringing string.** PR #41 gives repeated strikes separate voices, but a
  hammer striking an *already-ringing* string is still not modeled; the voices simply sum. **A3 does
  not fix this.** Board coupling lets a second strike hear the first through a *diffuse board path* —
  which is not a hammer meeting a moving string. Under-damper re-strike is a **string-level** event,
  out of scope here. *(r1 claimed A3 addressed R6, having conceded 50 lines earlier that it doesn't.
  Claim withdrawn.)*
- **R7 — Phantom/longitudinal partials are a squared-signal spray** through one formant, not a
  longitudinal wave.
- **R8 — No duplex/aliquot scale; the damper is a loss-coefficient rewrite** with no felt contact
  dynamics; no una corda, half-pedal, or sostenuto.
- **R9 — ff aliasing, C7–C8** (#13, open). **Now gated in every phase.** r1 left it untouched, and it
  is the producer's first dismissal criterion.
- **R10 — Everything is anechoic.**

Literature: Weinreich (1977) on coupled strings and the prompt/aftersound split; Conklin (1996) on
soundboard and duplex; Askenfelt & Jansson, and Giordano, on measured bridge driving-point mobility;
Bank (2003) for the nonlinear-hammer + coupled-string + shared-soundboard recipe; Chabassier and
Bilbao for the full-PDE comparison in B; Smith & Van Duyne for the dispersion-allpass technique
already shipped. **Skudrzyk's mean-value method gives the *mean* driving-point impedance and an
asymptotic modal density — it is *not* a modal-damping law** (r1 misapplied it); mode dampings need a
separate source (measured Q's, or a radiation-plus-internal-loss model).

### On the reference corpus — correcting revision 1

r1 claimed the corpus was unreproducible and that every reference-relative number was unfalsifiable.
**That is false against `main`, and it is withdrawn.** `evals/reference-receipts/` and
`scripts/dev/canonicalize_reference_receipt.py` are a genuine checksum-bound rebuild recipe: source
URL, per-file `source_sha256`, byte-exact `canonical_sha256`, an ordered operation list, and a pinned
libsndfile version and PEAK timestamp for byte-exact reproducibility.

The **real** gap is narrower and still blocking:
1. **No fetch step.** The canonicalizer takes `--source-root`; the sources must already be on disk. The
   chain is complete from *source file* → canonical WAV and **broken from URL → source file**.
2. **Coverage is 7 isolated-note stimuli across 2 corpora.** No chords, repeats, pedal, or phrase —
   exactly what #49 demands — and **no incumbents**.

That is #52.

## Design

### Architecture A — Positive-real bridge + port-coupled shared soundboard (recommended)

**A1 — Positive-real bridge admittance.** Replace the first-order G(ω) and the fixed 270 Hz dip with a
real driving-point mobility Y(ω) — ~6 biquads per register, fitted at init to published grand-bridge
mobility curves. Strings terminate into it. → **R2**, **R4**, and the key-independence half of **R1**.

**The stability constraint r1 missed.** The existing bridge carries a closed-form passivity argument
in `kernels.rs` (Re(1/H(ω)) ≥ 1 ⟹ |1 − N·G(ω)| ≤ 1, enforced by test). A least-squares biquad fit to a
*measured* mobility curve is **generically not positive-real**, and the moment Re{Y(ω)} < 0 anywhere,
the termination reflection |R(ω)| > 1 and the string loop diverges — deterministically, at init, for
whichever register got the bad fit. Therefore:

- the fit must be **constrained positive-real** (vector fitting with PR enforcement, or a
  positive-residue parallel-RLC / biquad-sum form), and
- **|R(ω)| ≤ 1 must be an init-time assertion** across the 88-key × velocity grid, not a hope.

A1 is **not** "a strict improvement with no stability risk", as r1 called it. It is the phase with the
*unguarded* failure mode — and it now gets the guard.

**A2 — Shared modal soundboard, coupled through bridge ports.** A modal bank **shared per piano
track**, stereo, synthesized at init from a parametric modal-density and damping law. **No sampled IR
enters the runtime.**

The mechanism r1 omitted — and without which this is merely a fancy global EQ — is that **string
energy must enter the board at the string's own bridge position.** Summing all bridge forces and
filtering once is sum-then-filter: one LTI SISO filter on the summed output, **key-independent by
construction**, which is the exact defect R1 levels at the current 270 Hz dip.

So the board carries mode shapes φ_k(x) sampled at **P bridge ports** spanning the bridge:

- injection: `F_k += Σ_j φ_k(port_j) · f_j`
- readback: `v(port_j) = Σ_k φ_k(port_j) · v_k`

Both run **once per sample regardless of polyphony**.

**Where φ_k comes from — and r2 did not say.** A modal-density law gives ω_k; a damping law gives Q_k.
**Neither gives mode *shapes*** — yet every physical claim in A2 and A3 rests on φ_k(x_n). (The bench
scaffold fills φ with an arbitrary lattice, which is correct for a MAC count and is *not* a model.)
The board is an **orthotropic rectangular plate**: φ_k are separable sin·sin products, with the ribs and
the bridge bar entering as perturbations, and **x_n comes from real bridge geometry** (the speaking-length
termination point of each note along the bridge). That is the cheap, defensible answer, and it is what
P3 must implement — not a lattice.

**How many ports — and r2 chose P from the CPU table, not from physics.** That is a physical parameter
derived from a budget, which is exactly the failure mode this document keeps catching. Two constraints:

- **Spatial Nyquist.** To resolve φ_k along a ~1.3 m bridge for modes up to the crossover, port spacing
  must be below half the modal wavelength — that is **~20 ports, not 6.**
- **Physics, not smoothness.** Measured grand bridge mobility varies **erratically note-to-note** — that
  erratic structure is the *origin* of dead notes, double decays, and note-to-note timbre variation
  (Giordano; Weinreich). Six ports blended linearly give a **monotone, piecewise-linear** bridge across
  the keyboard: R1's key-independence defect is downgraded from rank-1 to rank-6, **not fixed.**

Because cost scales as **P × M**, ports can be bought with modes at constant CPU. Measured
(`npm run bench:soundboard`):

| ports × modes | P×M | µs/quantum | % budget |
|---|---|---|---|
| 6 × 200 (r2 — fails spatial Nyquist) | 1200 | 256.4 | 9.6% |
| 12 × 100 | 1200 | 212.1 | 8.0% |
| **20 × 64** | 1280 | **216.5** | **8.1%** |
| 24 × 48 | 1152 | 189.8 | 7.1% |

**20 ports × 64 modes costs *less* than 6 × 200 (216 µs vs 256 µs) and satisfies spatial Nyquist.** And
64 modes is the *right* number, not a sacrifice — see the crossover below. The r2 configuration was worse
on both axes.

**Modal density and the crossover.** A grand board runs ~0.04–0.06 modes/Hz, and **above the
modal-overlap crossover (~1 kHz) a modal bank is the wrong estimator.** Below the crossover there are only
**~40–60 modes** — so a 200-mode bank was over-provisioning the modal half ~4× while under-provisioning
the ports. The board is **hybrid**: modal below the crossover, statistical above.

**The statistical tail goes *outside* the coupling loop, as a radiation-only stage.** Inside the loop it
would have to be a P-port MIMO system with its own passivity proof — an FDN inside a 64-string feedback
loop is a materially harder argument than a parallel sum of passive second-order resonators, and r2
costed and proved neither. Outside the loop it is cheap, and the passivity argument stays tractable.
→ **R1**, **R4**.

**A3 — Close the loop.** Board port velocity returns into each string's termination as a two-port
wave-scattering junction (WDF-style, passive by construction, unit delay in the loop). The board
becomes a shared coupling medium: every undamped string is re-excited by what every other string put
into it.

→ **R3**: sympathetic resonance and pedal bloom become **emergent and correctly tuned to whatever is
actually held**. The fixed C-major bank is **deleted, not tuned**, freeing 792 KB.
→ opens **R8**: duplex/aliquot segments become additional terminations on the same board.
→ **Not R6.** See above.

**The risks, stated plainly.**

1. A3 creates a feedback loop between 64 strings and one board. Done naively it is a delay-free loop
   and it will blow up. It must be an energy-passive scattering junction with a unit delay, proven
   passive offline. The unit delay itself detunes the coupling by one sample per round trip — small,
   but systematic, and it belongs in the passivity write-up rather than being discovered later.
2. **A3 requires a render-loop restructure that does not exist.** A sample-synchronous string↔board
   loop cannot live inside a block-serial, voice-at-a-time render loop. This is the single largest
   unknown in the plan, and **P1 measures it before anything is built on top of it.**
3. **Passivity in ℝ is not passivity in `f32`.** A passive junction wrapped around a high-Q modal bank
   still leaks energy through rounding. Denormal flushing must reach **every board mode and every
   string rail**, and that per-mode cost sits inside the shared budget above.

### Architecture B — Full stiff-string PDE + implicit hammer collision (rejected as the *first* bet)

**B1** modal/FDTD stiff string carrying two transverse polarizations plus a longitudinal wave
(→ **R7**, **R5**). **B2** an implicit hammer–string collision solver with genuine multi-contact and
re-contact (→ **R5**, **R6**). **B3** 2–4× oversampled contact (→ **R9**).

**Why not first — with an honest correction.** r1 rejected B partly on the grounds that "a
data-dependent Newton iteration on the audio thread" is un-shippable. **That argument is simply
wrong, and this repo refutes it: `wdf.rs` already ships one** — a warm-started damped Newton solve,
`HARD_ITERS = 2`, capped at 8, with oversampling as the sanctioned mitigation. Bounded iteration is
precedented here. r1 was defending the wrong flank.

B is rejected on **cost**, which survives:

- **CPU:** three rails × three strings plus a per-sample nonlinear solve. The piano is *already* 13.22
  µs/voice. Even at 3×, that is ~40 µs/voice → 32 voices = **~1,280 µs = 48% of budget for the piano
  alone**, breaking the exit gate before any other track exists. **That 3× is an estimate, not a
  measurement** — it is doing load-bearing work, and P5 must *spike* it rather than inherit the guess.
- **Memory:** tripling the delay rails takes `PianoVoice` to roughly 60–75 KB → **voice bank ~4–4.8
  MB**, paid by all 64 slots, for an instrument that may be one track of six. *(r1 said "triples all of
  `PianoVoice`"; only the rails triple. The rejection survives the correction.)*

B is the right *second* investment, and its payoff is larger *after* A — a truer string is more
audible through a real radiator than through a static EQ.

### Recommendation

**Architecture A, staged P1 → P2 → P3 → P4.** It attacks the residual the project's own audit named
the weakest link; with the bridge-port basis its dominant cost is **measured, shared, and
polyphony-independent** (257 µs at 6 ports × 200 modes, flat in voice count); it delivers #14 as a
consequence of physics rather than a bolt-on; and every risky part is separable, provable, and can
fail cheaply.

It does **not** improve the attack. That is the trade the owner is being asked to make.

## Phased plan

**Standing gates — every phase, no exceptions.** Each is **dual-rate (44.1 and 48 kHz)**.

- **Aliasing (R9):** no new energy above Nyquist at C7–C8 *ff* versus the baseline. A phase that fails
  this fails, full stop. #13 is a hard precondition of the first phase that touches a nonlinearity.
- **Velocity→timbre:** a velocity ladder must change **tone**, not just level (per-key spectral
  centroid must track velocity monotonically). r1 had no such gate — the *horn* doc had one and the
  piano doc did not.
- **Note-off / release:** a key released mid-phrase, **pedal up and pedal down**, must damp like felt
  on wire, not fade like a fader. P4 rewires the pedal path; nothing may regress here unnoticed.
- **Phrase-level AB:** every gate ABs the **chord, repeat, pedal and phrase** items of the P0 corpus,
  not only the isolated anchors. Otherwise iteration is steered entirely by single notes — which is
  exactly how you arrive at something beautiful in isolation and lifeless in a phrase.
- **Desktop is the gate; mobile degrades.** Owner decision 2026-07-13 (`2026-07-11-architecture.md`).
  We ship a smaller board rung and fewer voices on weak devices — we never glitch. No phase blocks on a
  mobile number, and no estimated mobile number appears as a budget row.

---

**P0 — Corpus, comparator, and re-measurement.** *Blocks every DSP phase.*
Land the fetch step and the coverage described in #52: chords, repeats, pedal, and a musical phrase —
plus **at least one incumbent** (a sampled library or Pianoteq render from identical MIDI). Without an
incumbent, every gate reduces to "better than our previous version," which can converge to a local
optimum forever while the north star is Pianoteq. **Re-measure R4 on the frozen baseline** (its +15 dB
came from a 6-mode board; the baseline has 12).
*Gate:* corpus rebuilds byte-identically on a clean machine from committed receipts; baseline renders
reproduce from the frozen WASM digest; R4 restated against the actual comparator.

**P1 — Sample-interleaved render loop (no new DSP). The cheap failure.**
Restructure the piano track's rendering into a sample-synchronous inner loop over its voices; every
other instrument stays block-serial. **Nothing else changes.**
*Gate:* **output is byte-identical to the baseline** — there is no coupling yet, so it must be.

**The kill criterion must name a configuration, or it is unfalsifiable.** The interleave penalty *is* the
polyphony: it is a function of (voice count × per-voice hot footprint) against L1/L2. Measured on a
desktop with a 128 KB L1d: **8 voices 1.07× · 16 voices 1.12× · 32 voices 1.14× · 64 voices 1.45×.** So a
P1 run at 8 voices on an M-series reports 1.07× and "A3 lives", while the same restructure at 64 voices
sits on the kill line — and the **floor devices have 32–64 KB L1d, a half to a quarter of that**, so the
mobile ratio is strictly worse and it is the one that decides shippability.

P1 is therefore measured at **16 / 32 / 64 held voices, at 44.1 *and* 48 kHz, on M1 *and* the mobile
floor**, and **the kill decision is the mobile ratio at 32 voices.** If it exceeds ~1.5×, A3 is dead and
the campaign falls back to A1 + A2 open-loop — which is still worth shipping on its own.

**And P1 needs a degrade branch, not just a verdict.** Pass/fail throws the architecture away on a result
it could instead degrade past: **sub-block interleaving** — run voices in K-sample chunks (K = 8 or 16),
update the board every K samples, delay the coupling by K — keeps most of the coupling bandwidth (a K=8
delay notches near 3 kHz, well above where board coupling matters) while preserving block-serial cache and
dispatch behaviour. P1 measures K ∈ {1, 8, 16} and reports the frontier.

**P2 — Positive-real bridge admittance (open loop).** A1.
*Gate:* the init-time |R(ω)| ≤ 1 assertion passes across the 88-key × velocity grid; bass 20–60 Hz
excess falls to ≤ +3 dB against the **P0-re-measured** figure; ≤ +5 µs/voice. **The decay gate is
against the *reference corpus's* per-partial t60, not the baseline's** — the baseline's g0/g1 were
themselves solved to hit t60 targets, so gating on them could only be passed by degrading A1 back into
an output EQ. (r1's gate had exactly that flaw.)

**P3 — Port-coupled shared soundboard (open loop).** A2 at **20 ports × 64 modes**, with φ_k from the
orthotropic-plate model and x_n from real bridge geometry; statistical tail outside the loop.
*Gate:* spectral envelope and decay-tail match improve against the P0 corpus; **shared cost ≤ 60
µs/quantum, flat in polyphony**; per-voice cost unchanged; init ≤ 20 ms **on the floor device**;
allocation-free after init.

**The seam gate must be split in two, because r2's version rewarded the very loss it was meant to catch.**
r2 gated on "no port seam — adjacent semitones must not present audibly different bridges", i.e. it gated
*for* cross-note smoothness. But a **seam** (a discontinuity at a port boundary) is an artifact, whereas
**note-to-note variation is physics** — it is where dead notes and double decays come from. A gate that
rewards smoothness is passed most easily by a bridge that has no per-note structure at all, which is R1.
So:
- **artifact check:** the interpolant is continuous across a port boundary (no step);
- **physics check:** the note-to-note mobility **spread** matches the reference corpus. Too *smooth* fails.

**P4 — Close the loop.** A3; retire `SympBank`.
*Gate:* an offline energy test **proves passivity**; a 10-minute stability soak at 64 voices with the
pedal down, **at both rates**, shows zero NaN and no runaway; **shared cost ≤ 270 µs/quantum** (measured
257 µs at 6×200) with a **degradation ladder to 4 ports × 96 modes (89 µs) for mobile**; and
**sympathetic bloom provably tracks held notes** — render an F# minor chord pedal-down and show the
bloom spectrum contains **no C-major partials**. The baseline fails that test by construction; it is
the regression test for this entire issue. Pedal-down must change resonance, not just note length.

**P5 — Spike B** against whatever residuals survive A — starting by *measuring* the 3× per-voice
estimate that currently justifies rejecting it.

## Budgets

Binding. Exceeding any of these is an owner decision, not an implementer's.

**The gate is desktop.** Owner decision, 2026-07-13, recorded in `2026-07-11-architecture.md`: mobile is a
**degradation target, not a gate**. Consequently there are **no estimated mobile rows in this table** — if
we are not gating on it, we do not get to present a guess as a budget. (An earlier revision of this doc did
exactly that, with a made-up ×3.5 multiplier, which is the same sin as r1's unmeasured "3×" for
Architecture B.)

| Budget | Baseline | Ceiling (desktop, measured) |
|---|---|---|
| Per-voice CPU @48 kHz | 13.22 µs | **≤ 20 µs** |
| Shared board, open loop (P3) | 0 | **≤ 60 µs/quantum**, flat in polyphony |
| Shared board, closed loop (P4) | 0 | **≤ 230 µs/quantum** at 20 ports × 64 modes (measured 216.5) |
| **Board instances** | — | **per piano track**, and `MAX_TRACKS = 16` — so the ceiling multiplies. Two piano parts = two boards = ~433 µs. **Hard-cap at one board instance**, or state what the second piano track gets. |
| Idle engine | 19.9 µs | **rises** — a closed-loop board **cannot early-out** when nothing sounds. ~40–90 µs once a piano track exists. |
| Piano-led arrangement | ~16% | **≤ 50% of budget on M1** — the (now desktop-only) exit gate |
| Per-voice state | 25,760 B | ≤ 28 KB (remember: ×64) |
| Shared board state | 0 | ≤ 32 KB **per piano track**; +792 KB freed by retiring `SympBank` |
| Init | — | ≤ 20 ms, inside the gesture-unlock path |
| Degradation | voice stealing | the board degrades by **ports and modes, selected at init** — never mid-arrangement, because truncating ringing high-Q modes is an audible step. This is now the **mobile tier**, not a gate. |

**Mobile, for the record and not as a blocker.** At an estimated ×3.5, 32 baseline piano voices are ~58% of
budget — i.e. **the piano we ship today would already miss the old two-device gate**, with no soundboard at
all. That is now a *degradation* problem, not a *blocking* one: on a weak device we ship fewer voices and a
smaller board rung. #5 (real device measurement) stays open and worth doing — it tells us where the mobile
tier actually sits — but it is **no longer a precondition of any phase**.

**Bundle — one owned, composed number.** `scripts/audit/bundle-size-audit.sh` owns it (#46; repaired in
#63 — it had been omitting `packages/midi` *and* was red-by-construction in CI). Cite the script; do not
restate these from memory.

All-in is **76,803 B gz** (66,722 wasm + 4,715 core JS + 2,682 worklet + 2,684 midi) against the **153,600 B** contract (raised from 102,400 by owner decision 2026-07-13; see PRINCIPLES #2) → **25.0 KB gz of headroom for the project's entire remaining life.** Claimants: this campaign
(**≤ +5 KB gz**), strings/horns (**≤ +10 KB gz**), the 808 kit (~+2 KB gz), plus S0's unbudgeted JS and the
deferred shared room stage — which *both* docs call the biggest cross-family gap and *neither* budgets.

**With the ceiling raised to 150 KB they now fit**, with ~48 KB gz still spare after every named claimant. The audit enforces it; do not restate the number.

The audio-thread constitution is unchanged: no allocation, no locks, no denormals, bounded work per
sample. #49 authorizes more *cost*, not an exemption.

## What revision 1 got wrong

The 7-persona panel blocked r1 **7/7**. Recorded because the failures are the durable part.

1. **The recommendation did not survive its own arithmetic.** r1 asserted A's cost was "shared and
   polyphony-independent" without ever specifying *how* strings couple to modes. With per-string mode
   weights it is ~25 µs **per voice** — over r1's own ceiling. The **bridge-port basis** is the missing
   mechanism, and it is now measured (`npm run bench:soundboard`), not asserted.
2. **A2 as written was a shared static EQ** — sum-then-filter, key-independent: residual R1 rebuilt at
   400 modes. Fixed with per-port mode shapes φ_k.
3. **A1 was called risk-free.** It is the *unguarded* phase: it discards an existing closed-form
   passivity proof, and an unconstrained mobility fit is generically not positive-real.
4. **A3 is structurally impossible in the current block-serial render loop.** Now P1 — gated on
   byte-identical output, designed to fail cheaply and early.
5. **The budgets assumed a SoA/SIMD voice bank that does not exist.** The engine is AoS and scalar.
6. **A3 was claimed to fix R6 (re-strike), having conceded it doesn't.** Withdrawn.
7. **No aliasing, velocity→timbre, note-off, dual-rate, mobile, or incumbent gate existed.** All added.
8. **"The corpus is unfalsifiable" was false.** Withdrawn; the real gap is fetch + coverage (#52).
9. **Skudrzyk misapplied**; "20–60 dB" should have read Hz; R2 overstated ("no bridge admittance" when a
   first-order one exists and carries a passivity proof); B's memory rejection overstated; the
   anti-iteration argument was refuted by this repo's own `wdf.rs`.
10. **r1 audited a tree that did not contain the code it cited.** This revision is merged onto `main`;
    every in-repo claim is now checkable from the branch itself.

## Deferred until demanded

Architecture B in full (spike at P5). Mic models, binaural, listener-position rendering. Historic
temperaments. Morphing between piano models. Key/action mechanism simulation (escapement, jack,
repetition lever). Una corda and sostenuto.

**Half-pedal is *not* deferred** — r1 silently orphaned it (neither phased nor deferred). It is routed
into **P4**, whose gate already requires that pedal-down change resonance rather than note length, and
where a coupled board makes damper loss genuinely continuous.

A room / early-reflection stage: genuinely wanted (R10) and named by the audit as the biggest
cross-family gap — but it is one shared FDN for *all* instruments, it belongs in its own issue rather
than smuggled into the piano, and it must be budgeted against the same ~26 KB gz that this campaign is
already competing for.
