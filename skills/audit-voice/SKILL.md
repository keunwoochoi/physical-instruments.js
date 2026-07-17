---
name: audit-voice
description: The instrument's HARMONIC CHARACTER — does it lead with the fundamental like the real thing, and does it have ONE committed voice instead of a washed-out average. Usage - audit-voice <instrument>
---

# audit-voice — pick one tone and commit

> Owner (2026-07-17): "compare with more tones … more sounds. pick one. don't always go for
> average. sometimes you need to pick one of the many possibles and commit with it."

This is the timbre aspect, and it carries the most important method-shift in the project.
`match-reference` tunes toward the MEDIAN of a reference set — which averages out exactly the
character that makes one tone gorgeous, and yields something correct in a spectrogram and
lifeless in a phrase. audit-voice does the opposite: gather MANY real tones, choose ONE
target worth chasing, and commit the instrument to it.

## Why this aspect exists
The clearest, oldest failure in the project — the owner's recurring "too EP-like" — was a
harmonic-balance defect the average-matching loop never named: the PIANO put its 2nd and 3rd
harmonics ABOVE the fundamental in the mid register (C4 h2 +3, h3 +6), where every real grand
LEADS WITH THE FUNDAMENTAL (Steinway C4 h2 -15, Kawai -13). No real acoustic instrument in its
sustained body is top-heavy at the fundamental. That single check — "is the fundamental led?"
— catches thin/electric/synthetic character across the whole library.

## The procedure

1. **Gather MANY tones, not one.** Pull several real recordings of the instrument from
   DIFFERENT sources (e.g. VCSL has a Steinway B, a Kawai grand, two uprights — all CC0;
   VSCO-2-CE, Karoryfer, FreePats). Licence-clean only; ledger the source
   (`agentic-docs/licensing.md`). More sources = you SEE the spread and can choose, instead of
   inheriting whatever one library happened to sound like.

2. **Measure the harmonic ladder** with `scripts/dev/ref-analyze.py` (octave-corrected pitch,
   per-harmonic amplitudes rel h1). Print h1..h8 for a handful of representative notes across
   the register at a mezzo-forte layer. Do it for OURS and for EACH real source, side by side.

3. **Read the SHAPE, not the centroid.** Centroid varies between good instruments (a warm
   Steinway vs a bright Kawai can differ 180 Hz at C4 and both are real). The invariant across
   real instruments is the STRUCTURE:
   - a sustained acoustic instrument LEADS WITH THE FUNDAMENTAL (h2, h3 at or below h1);
   - the roll-off is smooth, not spiky;
   - inharmonic bars (marimba, vibes, glock) are the exception — their real overtones sit at
     4:1, 10:1 ratios, so h2/h3 measured at 2f0/3f0 being ~-45 dB is CORRECT, not a defect.

4. **PICK ONE target and say so.** Choose the specific tone the instrument should be — "the
   warm fundamental-led Steinway C4," not "the median of the velocity ladder." Write the
   choice into the commit and the code comment. Committing to a named voice is the point.

5. **Diagnose physically, then commit toward it.** Fundamental too weak / harmonics too strong
   is usually one of: a VELOCITY pickup (senses dx/dt, +6 dB/oct bright — a radiation lowpass
   warms it, cf. the piano `PIANO_WARM_*` and the bowed radiation tilt); an over-fast
   fundamental decay (coupling drains the in-phase mode before the sustain window); or a
   bore/topology that suppresses the mode you want (the sax was odd-only until beta added the
   evens). Change ONE thing, re-measure OURS vs the picked target.

## Pass bar
- Sustained acoustic instruments: the fundamental is the strongest partial (or within a few dB)
  through the mid register — NOT beaten by h2/h3.
- The instrument matches the CHARACTER of the one picked target, not the average of all.

## Gotchas learned the hard way
- **Measure over the right window.** A struck note's balance changes over time; the sustain
  window can read harmonic-heavy even when the attack is fundamental-strong. Say which window.
- **Inharmonicity fools a fixed-bin measure.** A stretched 2nd partial sits sharp of 2f0; both
  ref and ours read low there, so the COMPARISON stays fair, but never call an absolute number.
- **A partial fix is honest; a fake one is not.** If the lever (e.g. a radiation lowpass) only
  gets partway to the target without over-darkening the rest, ship the step and NAME the gap —
  do not crank it into a different defect to hit a number.
- The metric is a proxy. Human listening gates the release (PRINCIPLES #2).
