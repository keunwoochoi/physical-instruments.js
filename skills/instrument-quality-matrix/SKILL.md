---
name: instrument-quality-matrix
description: Apply every quality aspect (audit-tune/voice/dynamics/envelope/headroom/stability) to every instrument, in the right order, and track the scorecard. The comprehensive pass. Usage - instrument-quality-matrix [instrument]
---

# instrument-quality-matrix — every aspect × every instrument

> Owner (2026-07-17): make skills for the different aspects, "and then we eventually,
> comprehensively apply them to each instrument for all of them."

The six aspect skills each focus one direction of quality. This one runs them as a matrix so
no instrument is graded on a spectrogram in one dimension and lifeless in another, and so the
whole library is held to the same bar.

## The aspects (each is its own skill)
| # | skill | the direction |
|---|---|---|
| 1 | `audit-stability` | never NaN / denormal; self-oscillators ignite and hold |
| 2 | `audit-headroom` | nothing clips; level-matched |
| 3 | `audit-tune` | plays the note it was asked, everywhere |
| 4 | `audit-envelope` | attack + decay/release shape |
| 5 | `audit-dynamics` | velocity changes loudness AND timbre |
| 6 | `audit-voice` | fundamental-led, ONE committed voice (pick one, don't average) |

## The ORDER is not arbitrary — earlier aspects gate later ones
Run 1→6. The first two are PREREQUISITES: a clipping or NaN-ing instrument corrupts every
timbre and dynamics measurement (clipping fabricates harmonics; the master limiter crushes
dynamics flat). Never tune voice/dynamics before headroom and stability are green. Tune (3)
before voice (6): a mis-slotted note inflates the centroid, so a "brightness" reading on an
off-pitch note is meaningless.

## The instrument roster (engine `Instrument` enum)
Exposed: piano, epiano, guitar (nylon/steel/electric/distorted), bass, drums (pop/rock/jazz),
marimba, vibraphone, glockenspiel, musicbox, violin, viola, cello, contrabass, trumpet,
trombone, synthpad. Dormant (not shipped): french horn, saxophone.

## The pass, per instrument
1. Pick the instrument and its real reference set (audit-voice: gather MANY tones; ledger them).
2. Run aspects 1→6. For each, record: pass / fail / "early" (works but a known honest gap), with
   the decisive number and, on a fail, the ONE named root cause.
3. Fix the highest-leverage failure (a shared cause — e.g. the bowed Helmholtz-corner darkness —
   fixes a whole family at once; prefer those). One change, re-measure, gate.
4. A shipped instrument must be green on 1-3 and at least "early" on 4-6. A dormant instrument
   records its single blocking aspect and stays hidden.

## Where the scorecard lives
Produce the scorecard FRESH each pass from measurement (do not trust a stale one — journal.jsonl
and stale numbers lie). Record durable status in the GitHub issue for the family (#49 piano,
#50 strings/horns), never a local plan-status file (constitution: issues own work state). Bind
every number to the exact head SHA it was measured at.

## Reusable tooling (built during the string/brass campaign)
- `scripts/dev/ref-analyze.py` — octave-corrected pitch, harmonic ladder, attack, centroid, LUFS, crest
- `scripts/dev/render-for-ref.mjs` — render OURS at the reference's exact notes/velocities to WAV
- `scripts/dev/ref-compare.py` — the gap, per note and aggregated, on every axis
- vibrato-fair band comparison (integrate power AROUND each harmonic) when the source has vibrato

## The meta-rules that override any single metric
- Measure CLEAN (not through the limiter) and measure BEFORE AND AFTER (it has caught two of
  our own regressions).
- Turn a feature OFF before claiming it did something.
- The band/metric is a proxy for ITERATION; human listening gates the RELEASE (PRINCIPLES #2).
- Pick one target and commit; do not tune to the washed-out average.
