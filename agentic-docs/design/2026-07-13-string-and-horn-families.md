# String and horn families: the continuously-excited instrument

Date: 2026-07-13
Status: draft (**revision 3**) — proposes scope, anchor instruments, and a staged plan for issue #50.

Revision 2 answers a 7/7 blocking persona panel on revision 1. "What revision 1 got wrong" is at the
end; if you reviewed r1, start there.

Owner selections recorded 2026-07-13 (#50 acceptance criteria 1–2):
- **First string anchor: cello** (bowed).
- **Horn family: trombone, trumpet, saxophone** — "horn" widened past brass to include the single-reed
  saxophone. **Trombone first.**
- **Sequencing:** fold the minimum gesture set into this campaign rather than blocking on all of #12.

This doc authorizes **no implementation** — not a broad orchestra, not additional GM families, not a
product-budget increase, and not any port. Each phase needs its own issue; any port needs
`skills/port-audit` first.

## Motivation

1. The owner wants to expand into strings and horns (#50).
2. #50 warned against silently collapsing "strings" to one instrument or assuming one brass. Both
   choices are now explicit.
3. #50's body asserted the repo "already has reusable … breath/control … building blocks." **It does
   not**, and the plan must not be built on that. Verified against `crates/dsp/` (this branch now
   contains it): there is no breath, bow, reed, or lip code anywhere, and no continuous-control path of
   any kind.

## Thesis

**The engine cannot make a bowed or blown sound, and the missing piece is not the bow or the reed — it
is the sustained gesture that drives them.**

The entire note-scoped WASM ABI is `ij_note_on(track, midi, vel)` and `ij_note_off(track, midi)`.
**Nothing about a sounding note can change.** That is sufficient for every instrument the engine has,
because a struck or plucked instrument is *fully determined at the moment of excitation* — after the
hammer leaves the string, the note is just a decay.

A bowed or blown instrument is the opposite. Its excitation is **continuous and coextensive with the
note**. Take that away and there is no instrument left — you have a filtered oscillator with an
envelope. **That is a pad.** And it is literally what ships: in the GM map `strings → 8` and
`brass → 8`, and instrument 8 is the subtractive synth pad. That was never laziness — it is what the
ABI permits.

So the bet is not "add a violin model." It is: **build the continuously-excited nonlinear waveguide
once, and make the four instruments variations on it.**

**How far that unification actually goes — stated honestly, because r1 overstated it.** McIntyre,
Schumacher & Woodhouse (1983) do unify the bowed string, the reed, and the flute jet under one
skeleton: *a linear resonator described by a reflection function, driven by a nonlinear excitation.*
The skeleton is real, and it is the shared core. But the **resonator** differs in every instrument, and
in three of four cases it breaks the MSW frame:

- the **bow** acts at an *interior* point of the string (two-sided scattering); the reed and lip act at
  a *termination* (one-sided). Different junction topology.
- MSW's reed is explicitly **quasi-static and memoryless**, so it cannot be the source for a *resonant*
  lip valve. That lineage is Fletcher's valve classification and Elliott & Bowsher, not MSW.
- MSW presumes a **linear resonator** — which the brass centrepiece here (nonlinear wave steepening in
  the bore) directly violates. Steepening is a *distributed bore nonlinearity* and cannot be lumped into
  a reflection function.
- the **saxophone** needs a spherical-wave conical guide, not a cylindrical one.

So the **junction-in-a-loop skeleton is genuinely shared; the resonator is not.** Leverage is real but
partial, and "trumpet is largely a reparameterization of trombone" is the one place it fully holds.
r1 priced in leverage the physics only partly delivers.

## Evidence base

**Verified in-repo, on this branch (which now contains the source it cites):**
- The note-scoped ABI is `ij_note_on` / `ij_note_off` only; `ij_pedal` is track-wide. No per-note
  continuous control, and **no note identity** — notes are addressed by `(track, midi)`, which is already
  ambiguous when the same key is struck twice while ringing.
- `strings`, `brass`, `voice`, `synth` all map to instrument 8 (`SynthPad`) in the GM group map.
- No bow, breath, reed, or lip code in `crates/dsp/`.
- `StringLoop` is a **single-delay-loop Karplus-Strong waveguide**, and it **cannot host a bow**: a
  friction junction needs the string velocity arriving at the bow point **from both directions** in order
  to scatter, which requires a two-rail bidirectional waveguide. **The existing string primitive is not
  reusable for the cello.** This is the largest honest cost in the plan.
- The engine is **AoS and scalar** — `Vec<Voice>`, enum-dispatched; no SIMD anywhere. Every budget below
  is a **scalar** budget. (r1 justified its budgets as "SIMD-friendly across the SoA voice bank." There is
  no SoA voice bank.)
- Measured voice costs @48 kHz (budget 2666.7 µs): synth pad 0.40 µs/voice, guitar 3.70, bass 3.89,
  e-piano 1.96, piano 13.22. Idle engine 19.9 µs.
- **`wdf.rs` already runs a warm-started damped Newton solve on the audio thread** (`HARD_ITERS = 2`,
  capped at 8), with oversampling as the sanctioned mitigation. **Bounded iteration is precedented here** —
  which matters a great deal below.

**Licensing:**
- **VSCO-2-CE** (`github.com/sgossner/VSCO-2-CE`), **CC0-1.0, already verified in
  `agentic-docs/licensing.md`.** The ledger cites it only for crash and upright-piano references and marks
  it scratchpad-only. r1 asserted it contains solo cello, trombone, trumpet and saxophone. **That is not
  verified** — neither the instruments' presence nor their articulation coverage. P0 must verify it before
  any model claim. The lesson from the piano P1 round applies directly: *the "staged" Salamander corpus
  was once a 404 HTML page.* **Verify references are actual audio at staging time, not at fit time.**
- **STK** (`thestk/stk`), MIT, already ✅ port-approved in the ledger, ships `Bowed`, `Brass`,
  `Saxofony`, `Clarinet`, `BandedWaveguide` — Cook & Scavone's canonical implementations. **A gift and a
  trap:** these models are pedagogical and will produce a recognizable-but-toy cello fast enough to tempt
  us into stopping. `skills/port-audit` and its legacy-flaws checklist are **mandatory before a line is
  copied.** The gate is owner listening, not "it sounds like a cello."

Literature: MSW (1983) for the skeleton; **Schelleng** for the bow force/velocity regime diagram;
Friedlander and Schumacher on the multi-valued friction characteristic; Woodhouse on bow–bridge distance
and Helmholtz corner sharpening; **Fletcher**, and **Elliott & Bowsher**, on outward-striking lip valves;
Msallam/Vergez and Campbell on brass wave steepening; Benade on conical bores and tone-hole lattice
cutoff; Scavone on conical waveguides.

## Design

### Layer 0 — The gesture set (the usable slice of #12)

**A bowed string needs four continuous controls, not three.** r1's headline — *"three scalars, and they
map exactly onto MPE X/Y/Z, so MPE's three are precisely the physical minimum"* — was **only true because
it conflated bow force with bow velocity.** Those are independent axes: **velocity** sets amplitude,
**force** selects the Schelleng regime (sul tasto / normale / raucous) at a given bow–bridge distance.
The playable space is a **plane**, not a line. Collapse them and every note is one canned bow-arm — you
cannot play a slow heavy *p* or a fast light *f*.

Worse, r1 had **no bow direction at all.** Bow velocity is **signed**: a down-bow→up-bow change is a
zero-crossing *inside a sustained note*, not a new note, and it is the primary rhythmic articulation of a
string player. MPE Z (pressure) is unipolar, so **as r1 specified it the ABI structurally could not
express a bow change** — every long note was one infinite bow.

| dim | bowed string | brass / reed |
|---|---|---|
| `pitch` | vibrato, portamento | slide, lip bend |
| `position` | bow–bridge distance β | embouchure / lip tension |
| `force` | bow force (selects the Schelleng regime) | breath pressure |
| `drive` | **signed** bow velocity — direction *and* speed | *(unused — breath is unsigned)* |

Three suffice for brass and reed; **the bowed string needs the fourth.** MPE's X/Y/Z carry three, and an
extra CC carries `drive`. The honest statement: **MPE's three per-note dimensions are necessary but not
sufficient for a bowed string.** That does not weaken the case for implementing them — it means the
headline was wrong.

**Articulation is discrete, not continuous — and it must not triple-book one concept.** r2's first enum
(`{attack, tongued, slurred, legato}`) had *three* mechanisms for "do not re-attack" (`slurred`, `legato`,
and `tie_from`) with no way to disambiguate, no member for any *string* articulation the S2 gate actually
demands, and — fatally — **no bow direction**, so a note-list cello line rendered every note on the same
bow. That is verbatim the defect r2 charges r1 with: *"every long note was one infinite bow."*

**One mechanism per concept:**

- **Continuation is `tie_from` and nothing else.** `slurred` / `legato` are removed from the enum.
- **`onset`** describes how *this* note starts: `{ normal, tongued, martelé, spiccato, détaché }`.
- **`bow_dir`** ∈ `{ down, up }` on `note_on`, for bowed instruments. **Bow direction is not solely a CC.**
  It has to be reachable from a bare note-list, because that is the majority case, and because S2's
  bow-change gate is otherwise unreachable from the corpus driver. The default gesture **alternates bow
  direction per note** unless the caller specifies one; `drive`'s sign follows `bow_dir` and a CC lane may
  override it continuously.

**`tie_from` semantics — spelled out, because r2 promised "an explicit continuation rule" and never gave
one:**

- `note_on(id_B, …, tie_from: id_A)` **transfers** voice A to id B. **No `note_off` for A is expected or
  required**; sending one is a no-op, not a double-decay.
- Subsequent `note_expr` for the tied note is addressed to **id B**. Expression addressed to a retired id
  is dropped **and reported** (never silent).
- Continuation has two physically distinct forms and **the ABI must distinguish them**, because they sound
  different: `tie_from` + `onset: normal` = a **portamento** (the delay length glides); `tie_from` +
  `onset: détaché`/`slurred-finger` = a **discontinuous length change under a continuous bow** (a slurred
  finger change on a stopped string). r2's single `tie_from` could not express the difference.

**Brass has two pitch dimensions and needs a rule relating them.** Sounding pitch is a function of `midi`,
`pitch` (slide / lip bend) *and* `position` (lip tension, which selects the harmonic slot). That is a
many-to-one inverse — a given pitch is playable in several slot/slide combinations — and r2 never resolved
it, which makes the S3 slotting gate untestable and the default gesture unwritable. **S0 must specify the
`midi → (slot, slide)` mapping** (with the conventional preferred-position table), and a lip slur *within*
a slot must be distinguishable from a slurred interval *across* slots.

### S0 is a **delta** against an accepted doc that already owns this surface

**`agentic-docs/design/2026-07-12-instrument-controls.md` is accepted (7/7 panel) and it owns the event
transport.** Revisions 1 and 2 of this document cited it **zero times** and reinvented its surface — a
constitution #1 violation (*truth has owners, not echoes*). What it already owns, and what S0 must
therefore **extend rather than redesign**:

- a **preallocated fixed-capacity Rust/WASM event ring**, `ij_queue_event`, and **`ij_process` performing
  sample-offset segmentation** — so "frame offsets" are **not a novel r2 correction**; they exist, are
  specified, and are tested at offsets 0/1/63/127.
- an **equal-frame priority order**: `track` → `control` → `pedal/off` → `on`, then insertion sequence.
- an **overflow policy**: ingress **atomically rejects an over-dense batch** with `event-overflow`; it
  *"never defers events, admits deadline-breaking work, or partially schedules a batch."*
- a **reserved additive seam `ij_set_voice_control(handle, id, value)`** with an **engine-minted,
  generation-safe voice handle**, where *"a stolen/stale handle must fail instead of retargeting another
  voice"* — explicitly named as *"the compatible future target for pressure, bend, timbre, or deliberate
  active-voice modulation."* That is **this campaign's seam.** It was reserved for us.
- a live-input lead of `max(20 ms, measured device postMessage p99 + two render quanta)`, and a
  `lateEvents` diagnostic.

**Reconciling identity.** The accepted doc wants an **engine-minted generation-safe handle** (so a stolen
voice fails loudly instead of being retargeted). But an engine-minted handle **cannot be returned across
`postMessage`** — the API is fire-and-forget and notes are scheduled in the future and batched, so at call
time the voice does not exist. Both constraints are real, and they are satisfiable together:

> **The caller mints the `note_id`; the engine binds it to a voice slot with a generation counter.**
> Expression addressed to a `note_id` whose voice was stolen or never started **fails loudly and is
> reported** — it never retargets another voice. That preserves the accepted doc's *safety property*
> while respecting the transport's *ownership* reality.

Id reuse, lifetime, and the unknown-id policy are the whole contract of a caller-minted scheme, and **S0
owns writing them down.**

**Reconciling overflow — and dropping my "coalesce" policy.** The accepted doc's ingress policy is
**reject-the-batch atomically**, and it is *better* than the coalescing I proposed: it is bounded, atomic,
reported, and it never silently mutates a gesture. Coalescing belongs **on the main thread**, as a
rate-limiter that decimates redundant CC values *before* `postMessage` — not in the ring. So:

- **ingress keeps the accepted reject-batch policy**, and S0 re-sizes `MAX_EVENTS_PER_QUANTUM` for
  four-dimensional expression density (the accepted doc already specifies committing that constant at the
  largest value whose p99 stays under 50% of budget **on target mobile**);
- **main-thread decimation** keeps the stream inside that bound;
- **note lifecycle events are never dropped.**

**The ABI, as a delta:**

```
ij_note_on   (track, note_id, midi, vel, onset, bow_dir, tie_from, frame_offset)
ij_note_expr (track, note_id, dim, value, frame_offset)    // dim ∈ {pitch, position, force, drive}
ij_note_off  (track, note_id, release_vel, frame_offset)
ij_pedal     (track, depth, frame_offset)                  // CONTINUOUS 0..1, not a boolean
```

**`ij_pedal` is widened to a continuous depth in S0, and this is not optional.** Today it is
`ij_pedal(track, on: u32)` — a **boolean**. The piano doc routes **half-pedal** into its P4, but the four
continuous dims above are **per-note** and do not include the pedal, so as r2 stood, continuous damper
depth would have required a **second ABI break landing after S0** — which is exactly the fault r2 charges
r1 with over `release_vel`. S0 is *"the cheapest possible moment to break the ABI"*; widening the pedal
costs one `f32` and it happens now, or half-pedal is still orphaned with a phase label stuck on it.

- **Frame offsets on every event.** Without them "sample-accurate" is a lie — events land on quantum
  boundaries, i.e. **2.67 ms granularity on the sustain control of a self-oscillating instrument**. And a
  stepped bow force is not a gain zipper: it **kicks the junction's operating point discontinuously.**
  Expression is applied with a per-sample ramp, whose cost sits inside the per-voice budget.
- **`release_vel` is carried now.** r1's `note_off(id)` dropped it, guaranteeing a *second* ABI break —
  and #12 names release velocity as in-scope. It costs one `f32`.
- Per-note identity also fixes the existing latent `(track, midi)` ambiguity for repeated same-key strikes.

**Why the gesture must not be collapsed to its latest value.** r1's "drop-newest under saturation" was a
stuck-note hazard — it **freezes** bow force at a stale value, and on a self-oscillating instrument that is
a note **stuck at full sustain**. But r2's fix (unconditional coalescing) was also wrong, and it
contradicted the very sample-accuracy it was arguing for. *"Only the latest value matters"* is **false for
a continuous scalar**: the martelé force spike, the force dip at a bow change, and the tongue's reed
damping are **sub-quantum spikes**, and collapsing a spike to its endpoint **erases the articulation** —
deleting exactly the expressivity this campaign exists to add.

The accepted reject-batch ingress (above) resolves this correctly: **the trajectory is preserved
event-by-event at its frame offset**, over-dense batches are rejected atomically and *reported*, and the
main thread decimates redundant values before they are ever sent.

**Live input latency is bounded by the transport, not by the ABI — and PRINCIPLES #6 forbids the only
real fix.** Frame offsets give sample accuracy for **scheduled** events. For a player bowing **live**,
input crosses `postMessage` and lands at the next quantum boundary — **up to 2.67 ms, and no ABI parameter
can change that.** The only fix is a `SharedArrayBuffer` control ring, and PRINCIPLES #6 bans it
("*single-threaded by design: no COOP/COEP demands*"). So:

- the S0 sample-accuracy gate is **scoped to scheduled events**, explicitly;
- **live** expression is bounded by the accepted doc's lead — `max(20 ms, device postMessage p99 + two
  render quanta)` — and gated on **jitter**, since on a self-oscillating string main-thread jitter is an
  audible wobble in the bow force of a note **already sounding**;
- if that is not good enough for live bowing, **retiring PRINCIPLES #6 is an owner decision**, and this
  doc surfaces it rather than quietly gating on something unachievable.

**Voice stealing must change, and r1 asserted it wouldn't.** Existing stealing was designed for struck and
plucked voices, whose tails are low-amplitude and duck out. **A self-oscillating bowed or blown voice is
at full amplitude the instant it is stolen** — stealing it is a hard step to zero, i.e. a click. A full
arrangement will exceed 64 slots routinely. Continuously-excited voices get an explicit **release ramp**
on steal, with a click gate.

**Default gesture — without it, the eval corpus renders silence.** Neither r1 doc said what happens when a
producer drops a cello on a track and plays the keyboard with no CC lane drawn. **That is the majority
case**, and the engine's actual driver is a note-list / SMF scheduler. If `force`/`drive` default to 0, a
bowed or blown note is **silent**, and S2/S3's dynamics and articulation gates have nothing to render.

Every continuously-excited instrument ships a **default velocity→gesture envelope** (attack / sustain /
release of the drive control, per-preset shape). Two rules the envelope must carry, or it produces a
canned, lifeless result:

- **Bow direction alternates by default** across successive notes (see `bow_dir`). Without this, a bare
  note-list line is one infinite bow — the very defect this design set out to fix.
- **Per-note jitter applies to solo voices, not only to sections.** r2 specified jitter only for the
  ensemble layer (S4), so a fast détaché or double-tongued line would give every note an *identical*
  envelope — the machine-gun artifact. Onset, force and β get small per-note variation on every voice.

Standing gate: **it must sound professional from bare MIDI notes**, including a fast repeated passage.

### Layer 1 — The MSW core (shared)

**`BiWaveguide`** — a two-rail bidirectional waveguide, frequency-dependent termination filters, and a
scattering port at an arbitrary interior point. This is what `StringLoop` is not.

Continuous pitch means a **modulated delay length**, and that is where bowed and brass models actually go
wrong: an allpass interpolator transients and detunes under modulation, while a Lagrange interpolator
lowpasses the loop as a function of the fraction. **Name the interpolator and gate its modulation
artifacts** — vibrato and the trombone slide are the entire point of the pitch dimension.

**`NonlinearJunction`** — the MSW valve, in three specializations:
- **Bow:** stick–slip friction. **This is a state machine, not a curve.** The Friedlander characteristic
  is *multi-valued* — the load-line intersection has up to three roots, and selecting one requires
  carrying stick/slip state. That is a branch, and it *is* the hysteresis we want.
- **Reed:** a pressure-controlled beating valve (reed spring + Bernoulli flow; the reed can close).
- **Lip:** an **outward-striking mass–spring–damper whose own resonance interacts with the bore modes** —
  this is *why* brass slots onto harmonics. It is a 2-DOF implicit system per sample. **It is not a curve
  and cannot be tabulated.**

**Solution method — r1 defended the wrong flank.** r1 ruled out Newton as "data-dependent and branchy" and
prescribed a 1-D lookup: *"constant-time, branch-free."* But **this repo already ships a bounded Newton
solve on the audio thread** (`wdf.rs`), so iteration is precedented, not forbidden. And r1 then
contradicted itself — insisting the lip "must be modeled as a resonance, not a curve" and then proposing
exactly a curve. A static valve table is precisely **STK's simplification** — the one r1 calls "toy-like."

The honest method, per junction:

| junction | method | table rank |
|---|---|---|
| bow | 2-D characteristic (relative velocity × force) + **stick/slip state bit** | 2-D + branch |
| reed | 2-D (pressure × embouchure) | 2-D |
| lip | **bounded warm-started Newton**, following `wdf.rs`'s pattern and cap | ODE — not tabulated |

Bounded work per sample is preserved. "Branch-free" is not, and was never required.

**Anti-aliasing — and "2× on the junction" is not a realizable topology.** Both headline features are
broadband nonlinearities **inside feedback loops**, so aliased energy **recirculates and detunes the loop**
rather than merely adding a noise floor: the bow's Helmholtz corner is a near-discontinuity *by design*,
and brass shock formation synthesizes energy above Nyquist *by construction*.

r2 said "2× oversampling on the junction", leaning on `wdf.rs` as precedent. **That does not transfer.**
`wdf.rs` wraps a 31-tap half-band FIR pair around a **memoryless feedforward** triode. You cannot drop that
around a junction sitting **inside a delay-line feedback loop**: the up/down-sample filters inject ~15
samples of **group delay into the loop**, retuning and lowpassing the instrument. There are only two sound
options, and the doc must name one per junction:

| | approach | cost |
|---|---|---|
| **bow** | **delay-free band-limiting** — ADAA, or a BLAMP on the stick–slip transition. The Helmholtz corner is a *ramp discontinuity*, which is exactly what BLAMP is for. | ~free; no loop delay |
| **brass bore** | **run the whole waveguide at 2×** — delay lengths doubled, termination/dispersion filters redesigned at 96 kHz. Shock formation is a *distributed* bore nonlinearity; nothing local band-limits it. | **doubles the entire string/bore cost, not the junction's** |

And **2× is asserted, not derived** — for shock formation it is very likely **not enough**. The bench
scaffold now exists (`npm run bench:soundboard` pattern); **S1 measures the alias floor vs oversampling
ratio** rather than guessing, which is precisely how the piano doc fixed its own unmeasured multiplier.

**Stability torture test — the most dangerous instruments in the repo have none.** The bow, reed and lip
are **self-oscillating nonlinear feedback loops driven by four user-reachable continuous controls with
per-sample ramps**. r2's only stability gate was "self-oscillation starts and stops with the force
control", plus the sax apex. The piano doc has an init-time |R(ω)| ≤ 1 assertion *and* a 10-minute
64-voice soak; this doc had neither. **S1 gate:** sweep the reachable (force × β × drive × pitch × ramp
rate) space — including the **Schelleng-diagram boundaries the design deliberately exposes to the
player** — for NaN, blow-up, and dropout, at **both rates**, with a 10-minute soak.

### Layer 2 — Bores and bodies

- **Cello:** two-rail bowed string → bridge admittance → body. The body reuses the **shared modal-body /
  bridge-port primitive** from the piano campaign (`2026-07-13-higher-capacity-piano.md`, A2) with
  different modes. Genuine shared leverage — and a reason to sequence the piano's A2 first.
- **Trombone / trumpet:** bore waveguide + a **bell reflection function** (the bell is essentially a
  high-pass: low modes reflect and sustain the oscillation, highs radiate out), plus **nonlinear wave
  steepening** at *ff*. That steepening is the one thing physical modeling wins outright: the *blat* of a
  loud brass note is a propagation nonlinearity, and **no sample library can produce it as a continuous
  function of dynamic.** Trumpet is then largely a reparameterization of bore, bell and lip.
- **Saxophone:** a **conical** bore (spherical wave variables, not cylindrical — genuinely different DSP),
  a **tone-hole lattice** with its characteristic cutoff, and the reed at the mouthpiece. The
  truncated-cone **apex reflection is a first-order filter that is marginally stable in the lossless
  limit** — a known numerical landmine, and it gets an explicit stability gate.

### Layer 3 — Sections. The thing that actually fixes `strings → pad`

r1 deferred ensembles, which means **after the cello ships, `strings → 8` is still a synth pad** — because
in a track, "strings" is a *section*, not a solo lead line.

A section is not a new model: it is N voices of the same core with **per-voice jitter** (bow force, β,
bow-change timing, intonation, onset). Without that jitter, 8 identical deterministic cellos **phase-lock
into one large fake cello** with comb filtering.

This is cheap, and it is the difference between "we modeled a cello" and "a producer can use this." It is
**S4** — not "deferred until demanded."

## Phased plan

**Standing gates, every phase, dual-rate (44.1 / 48 kHz):** no aliasing above Nyquist at *ff*; mobile
(iPhone 8 / Safari 16.4 floor, Pixel 6a; p99 < 50% of budget, `droppedQuanta == 0`); allocation-free,
lock-free, denormal-safe; **sounds professional from bare MIDI notes** (the default gesture envelope).

**S0 — The gesture set.** Caller-minted note ids; four continuous per-note dims with frame offsets and
per-sample ramps; `onset` + `bow_dir` + `tie_from` (with the semantics above); release velocity; a
**continuous `ij_pedal` depth**; the `midi → (slot, slide)` brass mapping; frame-offset transport that
coalesces **only on saturation** and never drops a lifecycle event; release-ramp voice stealing; the
default gesture envelope (with bow alternation and per-note jitter); MPE + CC (breath CC2, CC74) input in
`packages/midi`; a playground control surface. **Ships no new instrument and is independently valuable.**

This is the **highest-blast-radius phase and it is first** — and it is also the **cheapest possible moment
to break the ABI**: all three packages are `version: 0.0.0` and unpublished.

*Gate:* the **public TS surface is specified and the three-line path still works** — `noteOn(60)` /
`noteOff(60)` must survive as an overload, or PRINCIPLES #3 ("a web dev plays a note in three lines") is
being retired, and **that would be an owner decision, not an implementation detail.** Type-level compat
test; the README's three-line example compiles and runs; **`demos/bundler-matrix` still zero-config**
(Vite / Next / Webpack); **SSR-safe** — `navigator.requestMIDIAccess` is a textbook import-time SSR
landmine, and SSR-safe imports are a contract that must never break. A sounding note's force modulates
sample-accurately; zero allocation on the event path; **existing instruments render byte-identically**
(drift-check across the 80 standardized auditions); full-arrangement `dsp-bench` unchanged; **an explicit
core-JS + worklet gz ceiling** — r1 budgeted WASM only, and this phase grows the *JS* half of the contract
against a 4.7 KB module.

⚠️ The audio drift-check **cannot** detect a broken public signature, a broken `exports` map, or a types
regression — it compares audio bytes. r1 named it as S0's mitigation. The API/DX gates above are not
optional extras.

**S1 — `BiWaveguide` + MSW junctions.** Port-audit STK first; build the core; 2× oversampling.
*Gate:* self-oscillation **starts and stops with the force control**; Helmholtz motion visible in the
string waveform; bounded work per sample (bounded Newton permitted, unbounded not); **the delay
interpolator is named and its modulation artifacts measured** under vibrato and a full slide; each
junction's table rank and size declared. **#46 (bundle contract) is a precondition** — this is the first
WASM-touching phase.

**S2 — Cello.** Bowed string + shared modal body + bridge admittance.
*Gate:* the VSCO-2-CE corpus **verified and staged by checksum** (#52) — *including that the instrument is
actually present*, which is currently unverified; isolated articulations **including bow change**
(down↔up reversal — the seam that gives away every fake bowed line, and the moment a stick-slip junction is
most likely to drop out of oscillation) and a **slow-bow *pp* onset** (which must be allowed to scratch,
not fade in); détaché, legato, dynamics ladder, vibrato; **at least one multi-track musical context**;
≤ 10 µs/voice; per-voice state ≤ 20 KB — **it must not become the new size-setting `Kernel` variant** (see
the piano doc's ×64 law); owner blind listening + panel.

**S3 — Trombone.** Lip valve (bounded Newton) + bore + bell reflection + slide + steepening.
*Gate:* as S2, plus — the bore must **slot** (lip tension selects the harmonic; demonstrated, not
asserted); ***ff* must brassen** (measurable spectral enrichment with dynamic, not merely level); and a
**breath-release / note-end** gate: a note that stops dead is as fake as one that never brassens.

**S4 — Sections.** Per-voice jitter over the S2/S3 cores; remap GM `strings` and `brass` off the synth pad.
*Gate:* a section does not comb-filter or phase-lock; **a GM MIDI file with a string part stops sounding
like a pad**; the arrangement still fits the budget.

**S5 — Trumpet** (reparameterization of S3). **S6 — Saxophone** (conical bore + tone-hole lattice + reed;
plus an **apex stability** gate and a register-break gate).

## Budgets

Requirements, not measurements. **Scalar engine — no SIMD exists.**

| Budget | Requirement |
|---|---|
| Per-voice CPU @48 kHz | **≤ 10 µs/voice, inclusive of 2× oversampling** — between guitar (3.70) and piano (13.22) |
| Arrangement | 8 cello + 4 trombone + 16 piano + bass + drums ≤ **50% of budget, on M1 *and* mid-tier Android** |
| Per-voice state | **≤ 20 KB** — must not become the size-setting `Kernel` variant (paid ×64 across the bank) |
| Shared state | junction tables synthesized at init, **≤ 64 KB total** — *and each junction's table rank must be declared*, because a 2-D bow table is not a 1-D one |
| WASM | **≤ +25 KB raw / ≤ +10 KB gz** for the whole family. r1 gave **no gz ceiling** for the single largest WASM addition proposed anywhere in the repo — and **gz is the only unit the public contract is written in** |
| core JS + worklet + **midi** | **≤ 12,500 B gz combined** (today: core 4,715 + worklet 2,682 + **midi 2,684** = 10,081 B gz; S0 gets **≤ ~2.4 KB gz** of growth). r2 wrote "S0 carries an explicit gz ceiling" — *a promise to have a number is not a number.* Note S0's largest JS growth lands in **`packages/midi`** (MPE + CC parsing), which #61's audit was **not even counting**; repaired in #63. |
| Init | **≤ 20 ms on the floor device** (iPhone 8), inside the gesture-unlock path |
| **Live input latency + jitter** | **≤ 1 quantum end-to-end, jitter ≤ 1 ms p99.** Neither r1 nor r2 had a timing budget at all — the words "latency" and "jitter" did not appear. Frame offsets fix *scheduled* events and do **nothing** for live MPE, which crosses main-thread → `postMessage` → worklet. On a struck piano, main-thread jank is a slightly late note. **On a self-oscillating bowed string it is an audible wobble in the bow force of a note already sounding** — it modulates the timbre of a note you are holding. This gets a gate, or S0 ships an expression path that cannot be performed on. |
| Degradation | voice stealing **with a release ramp** (time constant and click-gate threshold declared; a voice currently receiving expression is stolen last); expression applied at frame offsets, **coalescing only on ring saturation**; **note lifecycle undroppable**; every coalesce or drop **reported**, never silent |

**Bundle — the composed number.** All-in is **76,803 B gz** (wasm 66,722 + core 4,715 + worklet 2,682 +
midi 2,684) against the **153,600 B** contract (raised from 102,400 by owner decision 2026-07-13; see PRINCIPLES #2) → **25.0 KB gz of headroom for the project's entire remaining
life.** Owned by `scripts/audit/bundle-size-audit.sh` (#46; repaired in #63 — it had omitted `packages/midi`
entirely and was red-by-construction in CI). **Cite the script; never restate these from memory.**

Claimants: this campaign (≤ +10 KB gz), the piano (≤ +5), the 808 kit (~+2), S0's JS (≤ ~2.4), and the
deferred shared room stage that both docs want and neither budgets. **With the ceiling raised to 150 KB (owner decision, 2026-07-13) they now fit**, with ~48 KB gz spare after every named claimant — including the shared room stage that neither doc had budgeted. The audit enforces it.

## Risks

1. **S0 is a breaking public-API change and it is first.** Mitigated by the API/DX gates above — *not* by
   the audio drift-check, which cannot see an API break. Cheapest possible moment: nothing is published.
2. **The bowed string is the hardest sound in this plan.** The stick–slip junction is easy to make
   *oscillate* and hard to make *musical*. Budget owner listening early and often, not at the end.
   *(Cello is **not** on the familiarity ladder — that is piano, guitar, drums; r1 said otherwise. The
   cello-over-violin choice stands on its own merits: violin is far less forgiving of intonation and
   vibrato error, and a mediocre violin is instantly recognizable as fake.)*
3. **STK will tempt us to stop too early.** The port-audit legacy-flaws checklist exists for exactly this.
4. **Shared dependency on the piano campaign** (the modal-body / bridge-port primitive). If the piano's A2
   slips, S2 waits or duplicates. Prefer waiting.
5. **The MSW leverage is partial** (see Thesis). Do not budget as though four instruments share one core.

## What revision 2 got wrong

The panel blocked r2 as well. Corrected above:

1. **Bare MIDI still could not change bow.** Bow direction lived *only* in the sign of `drive`, settable
   only via a CC lane that a note-list does not emit — so an SMF cello line rendered **every note on the
   same bow**, which is r2's own indictment of r1, relocated into the default path. `bow_dir` is now a
   `note_on` field, and the default gesture **alternates**.
2. **Unconditional coalescing contradicted the frame offsets r2 had just added.** "Only the latest value
   matters" is false for a continuous scalar — the martelé spike, the bow-change force dip, and the tongue's
   reed damping are **sub-quantum spikes**, and collapsing them to an endpoint erases the articulation.
   Coalescing is now a **saturation fallback only**.
3. **No latency or jitter budget existed** — the words did not appear. Frame offsets fix *scheduled* events
   and do nothing for live MPE; on a self-oscillating string, main-thread jitter is an audible wobble in the
   bow force of a note **already sounding**. Now budgeted and gated.
4. **`tie_from` promised "an explicit continuation rule" and never gave one.** Now specified — including the
   distinction between a portamento and a slurred finger change under a continuous bow, which r2's single
   `tie_from` could not express.
5. **The articulation enum triple-booked "do not re-attack"** (`slurred`, `legato`, `tie_from`), had no
   string articulation the S2 gate demanded, and no bow direction. Split into `tie_from` (continuation) and
   `onset` (attack type).
6. **Brass had two pitch dimensions and no rule relating them to the note** — making the S3 slotting gate
   untestable and the default gesture unwritable. S0 now owns the `midi → (slot, slide)` mapping.
7. **Half-pedal was "routed to P4" while `ij_pedal` stayed a boolean** — forcing a *second* ABI break after
   S0, precisely the sin r2 charges r1 with. `ij_pedal` is widened to a continuous depth in S0.
8. **Per-note jitter was specified only for sections**, so fast repeated solo notes would machine-gun.

## What revision 1 got wrong

The panel blocked r1 **7/7**.

1. **"Three scalars = exactly MPE X/Y/Z = precisely the minimum" was only true because it conflated bow
   force with bow velocity.** Two independent axes; a bowed string needs four.
2. **No bow direction.** Bow velocity is signed, MPE Z is unipolar — the ABI structurally could not express
   a bow reversal, the single most important string articulation.
3. **No tonguing and no legato/continuation semantics** — while r1's own S2/S3 gates *demanded* legato and
   slotting. The doc named an acceptance criterion its own ABI could not meet.
4. **The note-id ABI could not work through the transport.** A WASM return value cannot cross
   `postMessage`, and notes are scheduled in the future and batched. Ids are now caller-minted.
5. **"Drop-newest" was a stuck-note hazard** — it freezes bow force at full sustain on a self-oscillating
   instrument, and a dropped `note_off` rings forever. Now: coalesce; lifecycle undroppable.
6. **"Sample-accurate" was unachievable** — no frame offsets, no interpolation contract. Both added.
7. **`note_off` dropped release velocity**, guaranteeing a second ABI break. Added; it costs one `f32`.
8. **No default gesture** — a plain MIDI note into a bowed model is **silent**, which would have made the
   eval corpus render nothing at all.
9. **"Voice stealing unchanged" was a category error** — a self-oscillating voice is stolen at full amplitude.
10. **The table-junction argument defended the wrong flank** (`wdf.rs` already ships bounded Newton) and
    **contradicted itself** (lip "must be a resonance", then proposed a curve).
11. **Zero aliasing story**, in a design whose two headline features are broadband nonlinearities inside
    feedback loops. 2× oversampling is now budgeted and gated.
12. **The MSW "same object" claim was overstated**, and it mis-attributed the resonant lip valve to MSW.
13. **Sections were deferred** — which would have left `strings → synth pad` *still true* after the cello shipped.
14. **VSCO-2-CE's contents were asserted, not verified.**
15. **Budgets assumed a SoA/SIMD voice bank that does not exist**, gave no gz ceiling, budgeted zero JS, and
    had no mobile, SSR, or bundler gate.
16. **Cello was placed "on the familiarity ladder's upper half."** The ladder is piano, guitar, drums.

## Known gaps, accepted for now

- **No string-selection dimension.** Four dims are enough to *sound* a cello, not to *voice* one: the same
  pitch on the C string and the G string is a different colour, and a cellist chooses deliberately. An open
  string also cannot vibrato and decays differently. Bare MIDI will pick for us. Accepted for S2; revisit
  once the core is real.
- **`drive` is unused for brass/reed** but the dim enum is shared, so a generic controller will send it to a
  trombone and get nothing. Either define it (breath noise is the obvious candidate) or make the
  per-instrument dim set explicit in the ABI. To be settled in S0.
- **Tonguing is a discrete `onset` only.** Legato tonguing, hard tongue and doodle tonguing are a
  *continuum*, and the reed junction table (pressure × embouchure) has no axis for tongue-on-reed damping.
- **Note-id reuse policy** must be stated in S0: coalescing is keyed on `(note_id, dim)`, so a scheduler that
  recycles ids could inject expression into a live voice.

## Deferred until demanded

Violin, viola, double bass. French horn. Clarinet, flute, oboe, bassoon (the reed and jet cores make these
cheap *later* — that is the point of Layer 1, but they are not authorized here). Pizzicato and col legno.
Mutes; sul ponticello / sul tasto. Breath-controller hardware.

**Full MPE beyond the per-note dimensions above** — and note what actually remains in #12 once S0 lands:
**zone / master-channel routing and bend-range negotiation.** That is an input-layer concern, not a missing
architecture. Saying so stops #12 reading as a hole in the engine.

The shared room / early-reflection stage — wanted by every family, owned by its own issue, and competing
for the same ~26 KB gz.
