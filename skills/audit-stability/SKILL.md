---
name: audit-stability
description: No NaN, no denormals, no runaway — across the full range, every velocity, sustained for seconds, and for self-oscillators, robust ignition and a limit cycle that holds. Usage - audit-stability <instrument>
---

# audit-stability — it must never blow up or fall silent

A physical model is a feedback loop; feedback loops go unstable. The audio thread is sacred
(PRINCIPLES #4): no NaN, no denormals, bounded work per sample, forever.

## The procedure
1. Render EVERY note × ≥3 velocities, sustained several seconds, plus note-off ring-down. Count
   non-finite samples; there must be ZERO.
2. Flush denormals on every recursive state (`flush_denormal`) — a decaying tail that drops into
   denormals costs 100× CPU and silently blows the budget.
3. For SELF-OSCILLATORS (bowed friction, brass lip, reed) also check:
   - **Ignition**: does it start from silence at EVERY note and velocity, not just ff? (The sax
     speaks only at ff — a marginal loop gain that does not scale with drive. Documented, dormant.)
   - **Limit cycle holds**: it must not creep for seconds before speaking (the trombone crept up
     for 2 s until the lips were made to BEAT), and must not drift off its partial mid-note (a
     wolf / slot slip — see audit-tune).
   - **Regime robustness**: the operating point must not sit ON a knife edge where a small
     parameter nudge flips it silent (the trombone's Y_EQ sat on a regime boundary; moved off it).

## Diagnose
- **NaN from a friction/flow solve**: a numerically pathological friction curve (Friedlander
  ambiguity) or a sqrt-slope singularity in an implicit reed solve pins the state. Prefer a
  one-to-one, closed-form load-line solve (thermal friction; Bilbao/Smith quadratic) over a
  branch-rule mu(v) that depends on rounding.
- **Silence from a stable DC operating point**: a reed/lip that settles to a fixed point never
  oscillates. It needs a startup seed AND loop gain > 1 in the negative-resistance region — the
  operating point (mouth pressure vs closing pressure) must be past threshold, not choked shut.
- **Rate dependence**: if a coefficient's clamp point differs between 44.1 and 48 kHz, the tone
  or tuning drifts with sample rate — a rate-stability test catches it (the piano attack-centroid
  rate gate did).

## Gotchas
- **Your own test harness can NaN when the audio is fine** — an out-of-bounds buffer read or a
  0/0 in a metric. Bound the buffer, guard the divide, and confirm a NaN is in the SAMPLES (via
  the render gate) before diagnosing the DSP.
- Turn a feature OFF before claiming it stabilised something — the trombone bore-nonlinearity
  ablation showed beta=0 gave the same enrichment, so the claim would have been false.
