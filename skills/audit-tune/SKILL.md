---
name: audit-tune
description: Does the instrument play the pitch it was ASKED for, across its whole range and at every velocity — the most basic failure there is, and the hardest to see by eye. Usage - audit-tune <instrument>
---

# audit-tune — an instrument must play the note it is asked

An instrument that plays a different note than the one sent is broken in the most basic way,
and every OTHER gate (render, budget, NaN, level) can be green while it happens. The cello
shipped up to 97 CENTS FLAT — nearly a semitone — with all other checks passing.

## The procedure
1. Render the instrument at every note of its real range (or every 3rd semitone for a scan)
   at ≥2 velocities.
2. Measure f0 by **AUTOCORRELATION with parabolic refinement**, NOT a spectral-peak search.
   Reference: the Rust gates `cello_plays_in_tune_at_every_bow_force`,
   `bowed_family_plays_in_tune_across_range`, `trumpet_plays_in_tune_across_range`.
3. Report cents error per note; flag anything past the pass bar.
4. Diagnose physically before changing anything (see below), fix ONE cause, re-measure.

## Pass bar
- Every note within ~25 cents of target across the range.
- The error MUST NOT depend on the excitation strength: a real bowed string flattens a FEW
  cents under bow force (Schelleng), not a semitone; a brass note does not drift with velocity.

## Diagnose — the causes seen so far
- **Lagging state in the loop.** The cello's 97-cent flatness was a thermal-friction state
  whose lag delayed the slip, so the loop ran long (10-20 samples, INDEPENDENT of pitch — a
  fixed lag, not a rail-length bug, which is how it was diagnosed). Fixed by speeding the state.
- **Loop-length budget.** A fractional-delay allpass must absorb the exact remainder after the
  integer rails and the filter phase delays; getting it wrong is ~90 cents at C4.
- **Brass slot pull.** The outward-striking lip pulls the sounding pitch sharp of the bore
  mode, by an amount that depends on the harmonic n (trumpet n=3 was +44c). Measured per slot,
  cancelled by pre-flattening the bore (`brass_slot_pull_cents`). Velocity-robust: verify pp
  and ff give the same pull before hardcoding a table.
- **Register residual.** The bowed family ran ~27c flat only at the TOP (short loop, thermal
  residual) — invisible on the cello, caught by the violin. Test the whole family, not one.

## Gotchas
- **Autocorrelation is octave-robust DOWN by construction** (bound the search below 2×period),
  which is why it is trusted here. A spectral-peak or a naive octave-UP corrector rails on
  bright high notes (violin, trumpet top octave) and reports 1000+ cent phantom errors — those
  are ANALYZER failures, not synthesis. Cross-check a suspicious reading against the band error
  (a real octave miss makes the band error explode; an analyzer glitch does not).
- Gate only the range where the measurement is trustworthy; note where it is not and why.
