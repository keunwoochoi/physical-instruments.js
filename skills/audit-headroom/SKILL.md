---
name: audit-headroom
description: No note, at any velocity, may clip or ride the master limiter — and every instrument sits at a matched loudness. Clipping is invisible to RMS gates and corrupts every other measurement. Usage - audit-headroom <instrument>
---

# audit-headroom — nothing clips, everything is level-matched

The trombone shipped ~35 dB too hot and CLIPPED ON EVERY NOTE, sitting slammed into the master
limiter its entire life. The damage was not just distortion: the limiter CRUSHED ITS DYNAMICS
FLAT (measured 15 dB pp→ff when the model produced 26), and clipping MANUFACTURES HARMONICS, so
every brightness number taken while it was hot was made on distorted audio. Headroom is a
PREREQUISITE for trusting audit-voice and audit-dynamics.

## The procedure
1. Render EVERY note of the instrument's range (not a coarse sample — a hot note hides between
   grid points) at ≥3 velocities including 1.0.
2. Measure the **SAMPLE peak**, never an RMS envelope. A rich waveform has 6+ dB of crest
   factor: RMS 0.5 is a peak of 1.0. A gate that cannot see a clipped sample cannot see a
   clipped instrument.
3. Pass bar: worst sample peak < 0.95 across the full range × velocities. Reference gate:
   `no_instrument_clips_anywhere_in_its_range` (in lib.rs) — ADD the instrument to it.
4. Level match: solo notes land near the family's target loudness (the makeup_gain / per-voice
   level bake). A trumpet at peak 0.06 is as wrong as one at 1.0 — too quiet to sit in a mix.

## Diagnose
- A single hot NOTE (not the whole range) is usually a body/bore resonance ringing up at one
  pitch, or a bad parameter at that register (the cello hit 0.964 at m63 because a stray sweep
  left it at sul-ponticello beta=0.03). Find the note, fix the cause, don't just turn it down.
- The whole instrument too hot/quiet is the level bake — re-bake against the family reference.

## Gotchas
- **Step the gate every SEMITONE for the worst-case check**, or at least verify the note the
  scan flags. The clip gate steps every 3rd semitone and MISSED the cello's m63.
- Re-bake level AFTER any timbre change that alters crest factor (a warmth lowpass, a
  brightness tilt, a nonlinearity all move the peak).
- The playground / façade exposure is a real ship gate too: an instrument reachable by a user
  must not clip; a dormant one can wait.
