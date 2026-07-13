# Loop L2 four-family pilot report

Date: 2026-07-12
Status: runner-plumbing evidence for issue #21 and PR #27; the references are equation-generated test fixtures, so this report makes no instrument-quality claim.

## Purpose

Exercise one declarative command across the requested piano, drums, guitars, and bass matrices without committing or depending on private reference audio. The pilot tests exact reference-contract binding, schema resolution, shipped-WASM rendering, trust gates, failure propagation, iteration sealing, and independent verification.

## Inputs

- Source fixture: `evals/metrics/loop-v1/reference.wav`, generated from equations and committed under the L1 golden contract.
- Staging command: `npm run loop:stage-pilot -- --out /tmp/instruments-loop-pilot-refs`.
- Staged assets: three deterministic files under `references/equation-loop-pilot-v1/` at 16 kHz, 44.1 kHz, and 48 kHz, each bound to an exact verified equation-owned contract and canonical SHA-256.
- Shadow cases: 16 generated manifests under `/tmp/instruments-loop-pilot-refs/manifests`; no synthetic bytes are staged beneath a production-looking reference path.
- Campaign command: `npm run loop:pilot -- --manifest-dir /tmp/instruments-loop-pilot-refs/manifests --reference-root /tmp/instruments-loop-pilot-refs --out /tmp/instruments-loop-pilot-out --hypothesis "Exact equation-owned contracts preserve runner plumbing without impersonating production references." --changed-component "reference-contract-registry" --allow-dirty --skip-wasm-verify`.
- WASM verification was run separately before the pilot: fresh release and shipped SHA-256 both `70f40aafff0e7ef26d2539f3e0643d39a8749d57a95cb48a6e3b805f77fa524a`.

## Results

| Family | Cases | Sealed | Classification | Stop reason |
|---|---:|---:|---|---|
| Piano | 4 | 4 | untrusted | The deliberately unrelated equation reference tripped artifact trust gates; audition correctly withheld. |
| Drums | 4 | 4 | untrusted | The deliberately unrelated equation reference tripped artifact trust gates; audition correctly withheld. |
| Guitars | 4 | 4 | untrusted | The deliberately unrelated equation reference tripped artifact trust gates; audition correctly withheld. |
| Bass | 4 | 4 | untrusted | The deliberately unrelated equation reference tripped artifact trust gates; audition correctly withheld. |

All 16 candidate renders and reports were produced through the shipped WASM. Every family directory passed `loop_campaign.py verify`, including the exact registry snapshot, registry and schema digests, per-case contract digest, declared path, canonical reference digest, report binding, report schema, file set, per-file digests, and completion seal. The aggregate command exited nonzero because all four deliberately unrelated comparisons were untrusted; no audition page was generated.

The committed production piano, acoustic-drum, guitar, and bass cases now stop before source-tree checks, WASM verification, output creation, rendering, metrics, drift, listening-page generation, or sealing because their legacy path-only identities are explicitly unverified. They can run only after an owner commits exact source-asset receipts, canonicalization identity, and canonical SHA-256 contracts.

## Interpretation

The spectral distances are intentionally meaningless because equation fixtures test runner plumbing rather than instrument quality. The useful result is behavioral: verified identities bind before side effects, valid diagnostic evidence seals, red gates propagate to the family and aggregate exit status, synthetic fixtures cannot inherit production provenance from a path, and unverified production paths cannot enter an optimization or listening campaign.
