# Loop L4 listening-evidence report

Date: 2026-07-12
Status: validation evidence for issue #23 and PR #31; synthetic sessions validate the harness only and make no instrument-quality or release claim.

## Evidence contracts

- Experiment and session JSON are schema-validated, reject unknown fields, and bind every session to the exact experiment digest.
- Python and browser canonicalization use the same finite IEEE-double representation, reject unsafe integers and non-finite values, and are regression-tested on integer-valued, ordinary decimal, and small scientific-notation values. The bundled SHA-256 implementation keeps digest verification available on a physical device over a LAN HTTP origin without granting that device access to the private iteration root.
- The seeded randomization records both condition presentation and trial order. The analyzer independently reconstructs both and rejects tampering.
- Campaign listening bundles separate an opaque participant manifest/media directory from a sealed analysis key that owns roles and provenance. Every baseline and candidate source is SHA-256 identified, independently normalized to the declared BS.1770 integrated-loudness target, verified after writing, and recorded with gain/loudness provenance.
- Browser audits serve each campaign from its public bundle as the HTTP root and assert that sibling private A/B and ABX analysis keys return 404; omitting roles from public JSON is not treated as sufficient blinding by itself. Hostile manifest prompt markup renders only as text, and missing, malformed, or structurally invalid manifests terminate with an actionable no-data-collected state.
- The runner refuses a listening pair unless its complete case-manifest/schema identity, complete reference-registry/schema identity, per-case reference-contract evidence, reference digest, role, render metadata, sample rate, channels, frame count, and duration match. The full-file level-matching window is explicit rather than inferred.
- Campaign analysis-key v2 retains each exact reference contract plus candidate manifest/schema, registry/schema digests, and an unpredictable study nonce in private provenance. Opaque condition identifiers derive from that private nonce rather than enumerable commits, case IDs, and roles, and validation re-derives the private mapping from the sealed nonce. The public participant experiment remains role-free, contract-free, and nonce-free, so exact provenance strengthens analysis without leaking a derivable answer key.
- The browser uses exclusive start-from-zero playback without native per-player volume, seeking, or overlap controls. Submission requires the experiment-declared number of completed plays for every visible condition, and the analyzer independently excludes incomplete playback.
- Raw listener responses, pseudonymous listener/setup metadata, starts, completed plays, listened duration, seed, randomized order, exclusions, and uncertainty remain in the JSON analysis. Duplicate session IDs, duplicate listener IDs, and mixed human/synthetic pools fail closed; A/B ties remain explicit. `quality_verdict` is always null.
- Interrupted sessions recover completed trials from local storage. When storage fails, the page immediately exposes and continuously refreshes an in-progress JSON recovery copy; that copy can be restored after an interruption and remains available for manual copy or download.
- Manual recovery validates the complete browser session shape plus the sealed seed, trial prefix, presentation, response, play count, completed-play counters, and listened duration against freshly loaded stimulus metadata before mutating application or local-storage state. Browser tests reject tampered seed, presentation, playback duration, and response copies.
- Automatic local-storage recovery uses the same validator, removes malformed or deterministically contract-invalid records with an actionable warning, continues to a valid older record, retains structurally valid sessions when audio metadata fails transiently, and evicts rejected metadata promises so a later reload can retry without evidence loss.
- ABX serves X as an independent opaque public asset; its source/answer mapping exists only in the private analysis key. The browser-to-Python round trip completes X playback and scores the private key without exposing it in participant JSON.

## Hidden-reference and anchor pilot

The committed equation-generated MUSHRA pilot contains one explicit reference, one bit-identical hidden reference, one candidate, and one degraded anchor at -23 LUFS. Six deterministic synthetic sessions are submitted; five are included, and the session rating the hidden reference below the declared threshold is retained raw and excluded with an explicit reason. Bootstrap and Wilson uncertainty are deterministic. This pilot demonstrates harness behavior only; the ratings are not human evidence.

## Campaign round trip

The L2 campaign runner now replaces its label-revealing A/B page with a sealed public `listening/` experiment and a private `listening-analysis.json` key whenever a baseline-backed run reaches `candidate` or `listening_required`. The audit creates a temporary two-trial campaign, serves only that public directory, proves the sibling private key is unavailable, verifies that the participant JSON, media IDs, filenames, URLs, and visible text contain no condition role, plays every condition to completion with exclusive controls, reloads after trial one, resumes trial two, exports a browser session, and passes that exact export through the Python analyzer. It then serves an ABX fixture from its own isolated public root, proves the answer key is unavailable, and verifies browser playback plus private-key scoring. The automatic-recovery run blocks WAV metadata, proves the valid stored session remains present but unavailable for unchecked resume, restores connectivity, recovers it, then removes malformed and valid-JSON-but-invalid records before offering the valid completed session again. A storage-failure run captures the exposed in-progress recovery JSON after trial one, reloads, rejects four independently tampered copies, restores the untouched copy, and completes the session without losing evidence. Chromium and WebKit are both exercised; browser and Python experiment digests, presentation order, trial order, playback evidence, raw choice, raw-session retention, and null verdict all agree.

## Corrected dependency replay

L4 now contains the exact corrected L1–L3 dependency chain: zero/short/silent metric inputs and rejected alignment fail closed under `2026.07.12-l1.1`; runner subprocess/UTF-8/output-path failures are explicit; exact asset contracts stop ambiguous references before side effects; and metric `2026.07.12-l3.3` combines declared proximity/stiff-string/modal partial models, attack/body/tail trajectories, stereo, and trajectory-aware invalid-axis clearing with sealed reference identity. The listening experiment digest remains `ed890e7bdfc129cbbbb8b3447665cffb47846d2c9cddbd9dc069681aaa55656e`, proving the migration did not silently rewrite the frozen hidden-reference/anchor protocol.

Exact-head local validation after the dependency merge: loop suite 71/71; listening Python suite 17/17; frozen hidden-reference/anchor digest unchanged; Chromium and desktop WebKit full A/B, ABX, MUSHRA, recovery, storage-failure, tampered-import, hostile-prompt, invalid-manifest, private-nonce, exact-contract campaign, and browser-to-Python round trips; Rust 70/70; TypeScript typecheck/build; sustained 16-voice multi-track benchmark 65.0 µs/quantum (2.44% of the 2.67 ms budget); harness and diff checks. Physical iOS Safari remains external and is not inferred from desktop WebKit.

## Deliberate limits

- No real listener preference result is committed or inferred from the synthetic pilot.
- The harness records evidence and uncertainty; it does not decide whether a candidate ships.
- Learned embeddings remain absent. Their weights, license, offline execution, and domain validity still require a separate review.
- Raw human session files remain local unless the experiment owner deliberately preserves them as evidence.

## Reproduction

```sh
python3 -m pip install -r scripts/dev/requirements-loop.txt
npm install
npx playwright install chromium webkit
npm run audit:listening
LISTENING_BROWSER=webkit node scripts/dev/listening-e2e.mjs
```
