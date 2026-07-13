# Loop L1 metric migration report

Date: 2026-07-12
Status: evidence for issue #20 and PR #24; this report does not claim that any instrument sounds better.

## Compared revisions

- Previous metric: accepted-design base `8a3f888`, before L1 implementation.
- L1 metric: `6928fcf` plus the migration-report commit in PR #24.
- Inputs: committed equation-generated IEEE-float fixtures in `evals/metrics/loop-v1/`; no third-party or private reference audio.

## Semantic deltas

| Check | Previous | L1 | Interpretation |
|---|---:|---:|---|
| Identity fixture `mr_stft.mean` | 0.4752 | 0.0000 | The previous raw periodic cross-correlation selected a window-edge lag for identical notes with leading silence; L1's bounded moving-RMS onset reports a zero-sample shift and restores the identity invariant. |
| 30 kHz tone at 96 kHz resampled to 48 kHz, output RMS | 0.707107 | 0.000940 | Linear interpolation folded the above-Nyquist tone into the comparison band at essentially full level; the polyphase anti-alias filter suppresses it by approximately 57.5 dB. |
| Artifact mutation gate result | red: crest, jump, ultrasonic | red: crest, jump, ultrasonic | Existing artifact detection is preserved; L1 additionally evaluates finite samples, clipping occupancy, peak-relative DC, and case-aware onset/release gates. |
| Report identity | none | schema `1.0.0`, metric `2026.07.12-l1.1`, input SHA-256, runtime/config/operation provenance | Results can now be attributed to exact audio and metric semantics. |

## Compatibility decision

Pre-L1 `mr_stft` values are not numerically comparable to L1 values because both resampling and alignment semantics changed. Any committed drift baseline must be regenerated in an explicit acceptance PR after the L1 metric is reviewed; silently carrying thresholds forward would mix two different measurements.

The review hardening revision advances the metric to `2026.07.12-l1.1`. Zero-frame files now fail with an actionable error, silent or constant onset windows cannot acquire an arbitrary lag, short release windows cannot crash an empty reduction, and rejected or unevaluable alignment marks only that report untrusted while still returning its diagnostic vector. This is a trust-semantic change, so the equation-owned golden report is regenerated intentionally rather than treated as compatible with `2026.07.12-l1`.

The stable compatibility surface is structural: `mr_stft.mean` and the existing named diagnostics remain available to `drift-check.sh`, while new reports expose the semantic version and applied alignment. A red evaluated trust gate sets `interpretation` to `untrusted`; callers must not use the distances as evidence for keeping an iteration.

## Reproduction

```sh
python3 -m pip install -r scripts/dev/requirements-loop.txt
npm run audit:loop
git show 8a3f888:scripts/dev/compare.py > /tmp/pre-l1-compare.py
```

The committed golden fixtures and `scripts/dev/generate_loop_goldens.py` own the post-L1 evidence. Regenerating them is an intentional metric-version operation, never a routine test side effect.
