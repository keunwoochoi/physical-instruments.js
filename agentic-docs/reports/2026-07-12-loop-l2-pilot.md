# Loop L2 four-family pilot report

Date: 2026-07-12
Status: runner-plumbing evidence for issue #21 and PR #27; the references are equation-generated test fixtures, so this report makes no instrument-quality claim.

## Purpose

Exercise one declarative command across the requested piano, drums, guitars, and bass matrices without committing or depending on private reference audio. The pilot tests schema resolution, corpus sample-rate contracts, shipped-WASM rendering, trust gates, failure propagation, iteration sealing, and independent verification.

## Inputs

- Source fixture: `evals/metrics/loop-v1/reference.wav`, generated from equations and committed under the L1 golden contract.
- Staging command: `npm run loop:stage-pilot -- --out /tmp/instruments-loop-pilot-refs`.
- Staged references: 16 canonical paths at the owning corpus's declared rate: piano 48 kHz, drums 44.1 kHz, and NSynth guitar/bass paths 16 kHz.
- Campaign command: `npm run loop:pilot -- --reference-root /tmp/instruments-loop-pilot-refs --out /tmp/instruments-loop-pilot-out --hypothesis "The four-family campaign path produces complete, sealed evidence without private corpora." --changed-component "runner-pilot" --allow-dirty --skip-wasm-verify`.
- WASM verification was run separately before the pilot: fresh release and shipped SHA-256 both `70f40aafff0e7ef26d2539f3e0643d39a8749d57a95cb48a6e3b805f77fa524a`.

## Results

| Family | Cases | Trusted | Classification | Stop reason |
|---|---:|---:|---|---|
| Piano | 4 | 4 | incomplete | No cross-family drift baseline was supplied; audition correctly withheld. |
| Drums | 4 | 3 | untrusted | The deliberately unrelated equation reference exposed a rock-kick onset-crest mismatch; audition correctly withheld. |
| Guitars | 4 | 3 | untrusted | The clean-electric render failed the release-discontinuity gate against the deliberately unrelated fixture; audition correctly withheld. |
| Bass | 4 | 4 | incomplete | No cross-family drift baseline was supplied; audition correctly withheld. |

All 16 candidate renders and reports were produced through the shipped WASM. Every family directory passed `loop_campaign.py verify`, proving its report schema, file set, per-file digests, completion seal, and nested metric-report schemas. The aggregate command exited nonzero because two families were untrusted; no audition page was generated.

## Interpretation

The large spectral distances are intentionally meaningless because one synthetic reference was reused solely to test plumbing. The useful result is behavioral: valid evidence sealed, missing drift stayed incomplete, red gates propagated to the family and aggregate exit status, and the runner did not convert bad evidence into a listening candidate.
