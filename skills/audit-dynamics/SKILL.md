---
name: audit-dynamics
description: Does harder playing change both LOUDNESS and TIMBRE, the way a real instrument does — not just get louder, and not the same at pp and ff. Usage - audit-dynamics <instrument>
---

# audit-dynamics — velocity must change the tone, not just the level

A real instrument played harder gets louder AND brighter, and by the right amounts. A model
that ignores this sounds like a synth with a volume knob. The trombone measured 1.5 dB of
pp-to-ff range when a real one has ~17-30 — because the model self-oscillated into a limit
cycle whose amplitude was set by geometry, not by how hard you blew.

## The procedure
1. Render the instrument at a fixed set of notes across ≥3 velocity layers (map the reference
   corpus's own layers — v1/v2/v3 or p/mf/f — to the model's velocity input).
2. Measure two curves vs velocity, OURS and the reference:
   - **loudness** (BS.1770 / pyloudnorm integrated, or RMS if consistent) — the pp→ff span in dB;
   - **brightness** (harmonic spectral centroid) — the pp→ff RATIO.
3. Compare the SPANS and the DIRECTION, not single points.

## Pass bar (calibrate to the measured reference, per instrument)
- Loudness span pp→ff is in the real instrument's ballpark (measured: trombone 17 dB, sax wide,
  bowed strings modest).
- Brightness RISES with velocity (real brass ~1.9-2.0×; real bowed strings less). If ours goes
  the WRONG direction (darker when louder), that is a defect, not a calibration.

## Diagnose
- **Loudness that won't span** is often a self-oscillator saturating at a geometry-set limit
  cycle (brass lip, reed). The physics may only give a few dB; the rest is an honest, LABELLED
  velocity gain standing in for missing coupling physics — say so in the source (cf. the
  trombone `dyn_g`), never dress it as brassiness.
- **Brightness that won't rise** means the nonlinearity that should brassen isn't engaging with
  dynamics. For brass: the lip must BEAT harder at ff (embouchure firming), and bore steepening
  (beta) must act. For a fixed operating point, the timbre is fixed too.
- **Inverted brightness** (darker when louder) is usually a wolf/mis-slot inflating the centroid
  at one velocity — measure clean, and check the note is on-pitch first (see audit-tune).

## Gotchas
- **Measure LOUDNESS from a real level, not through the master limiter.** The trombone's
  dynamics read as 15 dB when the model produced 26 — the limiter had crushed them flat because
  the instrument was clipping. Fix headroom (audit-headroom) BEFORE trusting a dynamics number.
- Clipping and mis-slotting both FABRICATE brightness; a velocity that looks brighter may just
  be more distorted. Verify on clean, on-pitch audio.
