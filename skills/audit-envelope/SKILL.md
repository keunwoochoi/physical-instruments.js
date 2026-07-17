---
name: audit-envelope
description: The time-domain shape — the ATTACK (how the note speaks) and the DECAY/release (how it dies). A note that starts or ends wrong is as fake as one with the wrong spectrum. Usage - audit-envelope <instrument>
---

# audit-envelope — how the note starts and how it dies

The spectrum is half the instrument; the other half is its shape in time. A brass note that
takes two seconds to speak, a bowed note that stops dead, a piano that decays in one stage
instead of two — each is instantly wrong to the ear even when the sustain spectrum is right.

## ATTACK
1. Measure time to ~90% of the steady level (5 ms RMS envelope), and the spectral-centroid
   TRAJECTORY over the first ~50-200 ms.
2. Pass bar, per family: a bowed string speaks in tens of ms (heavier string = slower — violin
   ~30 ms, contrabass ~70); brass 30-200 ms depending on register and dynamic; a struck note
   is near-instant with a bright transient that decays.
3. Diagnose: a multi-second brass attack was the lip failing to BEAT (it crept up from the
   noise floor); the fix was the valve, not the envelope. A too-clean attack on a bowed string
   or a piano wants broadband excitation (bow noise, hammer/felt transient) — that "air" IS
   part of the attack. A player TONGUES/PLUCKS: seed the oscillator, don't grow it from silence.

## DECAY / SUSTAIN / RELEASE
1. Struck/plucked: measure the two-stage decay (prompt fast stage, aftersound slow stage) and
   the t60s per register. A real piano's prompt/aftersound ratio is ~2-4×; a fake body gives a
   single exponential (~1.5×) and sounds like an electric guitar.
2. Sustained (bowed/blown): the note must ring DOWN after the excitation lifts, not stop dead
   and not ring forever. A self-oscillator that keeps its drive on after note-off sustains at
   full amplitude — the note-off classification MUST damp it (exhaustive match, not a wildcard,
   so a new instrument is a compile error until classified).
3. Release: no click/pop at note-off. A brass lip slamming shut against a pressurised bore, or
   a bow pinning a still string, both dump a transient — gate the aperture/force by the release
   envelope.

## Gotchas
- **Measure BEFORE and AFTER any change.** A contact-delay "fix" collapsed the piano's two-stage
  decay 4.14× → 1.29% because it truncated the early force; only measuring both caught it.
- A too-short envelope attack that HIDES the true speak time is a measurement trap: a 5 ms RMS
  envelope smooths transients — verify the onset on the raw samples for near-instant attacks.
- "A note that stops dead is as fake as one that never brassens." End a voice when the STRING
  or BORE is quiet, not when the drive envelope is.
