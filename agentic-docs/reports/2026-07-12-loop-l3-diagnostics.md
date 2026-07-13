# Loop L3 trajectory-diagnostics report

Date: 2026-07-12
Status: validation evidence for issue #22; this report evaluates metric behavior, not instrument quality.

## Added diagnostic surfaces

- Bounded 10 ms envelope and spectral-centroid trajectories retain raw values, normalized path cost, path length, maximum displacement, and the profile-owned warp limit.
- Partial pairing reports per-mode render/reference frequency, cents residual, level residual, and aggregate absolute residuals around an explicitly declared model. Cases requiring this axis must choose proximity-harmonic windows, a stiff-string coefficient, or measured modal ratios; kick/snare cases no longer inherit the MIDI note as a fictitious harmonic fundamental.
- Per-harmonic 50 ms decay trajectories retain raw dB curves and bounded path evidence for the first six observable harmonics.
- Stereo diagnostics report native channel count, mid/side width, and inter-channel correlation before mono comparison.
- Profile-owned trust and warp thresholds are serialized in every report; kick timing is bounded more tightly and cymbal ultrasonic tolerance is explicitly distinct from pitched/default profiles.

## Synthetic validation

| Mutation | Expected response | Result |
|---|---|---|
| Identical decay trajectory | Zero envelope and partial-decay cost | Pass |
| Faster exponential decay | Envelope and fundamental-decay costs increase monotonically | Pass |
| Two-frame local shift | Bounded path cost falls relative to rigid alignment and maximum displacement stays within two frames | Pass |
| 440 Hz → 445 Hz detune | Fundamental-aware mean absolute cents residual increases from zero | Pass |
| Mild → strong 4 Hz beating | Envelope-trajectory cost increases monotonically from identity | Pass |
| Mild → strong 8 kHz attack burst | Centroid-trajectory cost increases monotonically from identity | Pass |
| Stiff-string coefficient | Fundamental remains fixed while the fourth partial target stretches upward | Pass |
| Non-integer modal ratios | Targets follow declared 1.00/1.59/2.14 ratios rather than integer harmonics | Pass |
| Stereo phase difference → dual-mono collapse | Width falls and correlation rises | Pass |
| Silent partial/stereo input | Partial list is empty and stereo fields are explicitly unavailable; no division-by-zero or NaN | Pass |
| Profile selection | Kick warp and cymbal ultrasonic thresholds differ from default exactly as declared | Pass |

The equation-owned artifact fixture under metric `2026.07.12-l3.3` remains untrusted on crest, sample jump, and ultrasonic energy. Its bounded envelope cost is 3.6441 dB, centroid cost is 52.6781 semitones, mean absolute partial-frequency residual is 8.31 cents, and mean absolute partial-level residual is 0.60 dB across three audible matched partials. Partial summaries exclude components below −80 dB relative to the strongest matched partial so numerical-noise peaks cannot dominate the residual. These values are expected from the deliberately adversarial fixture and are not acceptance thresholds.

## Private-corpus owner-verdict replay

The locally staged C4-ff incumbent render (`a38e05bd2141d0a8cee6ace53bfb93c3bc96a6d85260fe6a14751910762225e8`) was replayed against the licensed Salamander C4-ff canonical (`14ac208246645106af9eb5cc10a3d3eb39c1ac577d5e05c9425b4de6e1509d7b`) without committing either recording. The owner said the decay was already plausible but the felt-hammer attack remained wrong. The new region-owned trajectories localize the discrepancy rather than collapsing it into one curve: the first 50 ms costs 4.5084 dB on envelope and 7.1173 semitones on centroid, while the 50–500 ms body costs only 0.1695 dB and 2.1583 semitones. This supports the attack-versus-body direction of the verdict; it does not turn those values into a quality threshold or claim that the long tail matches.

The jazz-kick replay (`e28615f4b2f4f9a032bb6e83bf0b15be06d1a85bb3aef0793f73feb6006fc066` render versus `001c0b203d04bbc1c21654fe5e95e526d039a25bcbb5ee9b6bc7aaa2932e891d` reference) confirms why drum MIDI notes must not be treated as harmonic fundamentals: its profile now omits the partial ladder and retains glide plus attack/body/tail trajectories. The owner’s “vibraphone-like” tonality complaint still requires the pitch-salience/Q diagnostics routed to the acoustic-drum corpus work; L3 does not mislabel a generic harmonic score as proof.

Reproduction uses `scripts/dev/compare.py` with the exact files identified by those digests, `--profile pitched --expected-f0 261.625565 --partial-model-json '{"type":"proximity_harmonic","search_cents":90}'` for piano, and `--profile kick` for jazz kick. These are diagnostic replays of owner feedback, not tuned or held-out acceptance runs.

## Exact reference compatibility

The corrected L2 exact-reference contract is combined with every L3 surface. A runnable case must bind a verified asset ID, exact path, canonical digest, sample rate, valid axes, and sealed registry/schema identity before the runner reaches source checks, WASM, output creation, rendering, metrics, drift, or listening. Metric calls receive the bound reference contract alongside `expected_f0` and `partial_model`; invalid-axis clearing remains trajectory-aware. Verification binds the sealed case’s render request, partial model, profile-owned thresholds, required axes, renderer metadata, report configuration, and output identities.

The combined metric report schema advances to `1.2.0`, the combined case schema advances to `1.2.0`, the iteration schema is `1.1.0`, and the final metric version advances to `2026.07.12-l3.3`. L1/L2/L3.2 numeric baselines must not be silently reused; the campaign runner rejects metric-version, manifest, schema, registry, contract, path, or reference-identity mismatch. Any future semantic change requires an intentional version change and golden regeneration.
