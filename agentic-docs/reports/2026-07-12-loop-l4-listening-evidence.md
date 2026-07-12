# Loop L4 listening-evidence report

Date: 2026-07-12
Status: validation evidence for issue #23 and PR #31; synthetic sessions validate the harness only and make no instrument-quality or release claim.

## Evidence contracts

- Experiment and session JSON are schema-validated, reject unknown fields, and bind every session to the exact experiment digest.
- Python and browser canonicalization use the same finite IEEE-double representation, reject unsafe integers and non-finite values, and are regression-tested on integer-valued, ordinary decimal, and small scientific-notation values.
- The seeded randomization records both condition presentation and trial order. The analyzer independently reconstructs both and rejects tampering.
- Campaign listening bundles separate an opaque participant manifest/media directory from a sealed analysis key that owns roles and provenance. Every baseline and candidate source is SHA-256 identified, independently normalized to the declared BS.1770 integrated-loudness target, verified after writing, and recorded with gain/loudness provenance.
- The runner refuses a listening pair unless its case-manifest digest, reference digest, role, render metadata, sample rate, channels, frame count, and duration match. The full-file level-matching window is explicit rather than inferred.
- The browser uses exclusive start-from-zero playback without native per-player volume, seeking, or overlap controls. Submission requires the experiment-declared number of completed plays for every visible condition, and the analyzer independently excludes incomplete playback.
- Raw listener responses, pseudonymous listener/setup metadata, starts, completed plays, listened duration, seed, randomized order, exclusions, and uncertainty remain in the JSON analysis. Duplicate session IDs, duplicate listener IDs, and mixed human/synthetic pools fail closed; A/B ties remain explicit. `quality_verdict` is always null.
- Interrupted sessions recover completed trials from local storage. When storage fails, the page immediately exposes and continuously refreshes an in-progress JSON recovery copy; that copy can be restored after an interruption and remains available for manual copy or download.
- Manual recovery validates the complete browser session shape plus the sealed seed, trial prefix, presentation, response, play count, and playback evidence before mutating application or local-storage state. Browser tests reject tampered seed, presentation, playback, and response copies.
- ABX serves X as an independent opaque public asset; its source/answer mapping exists only in the private analysis key. The browser-to-Python round trip completes X playback and scores the private key without exposing it in participant JSON.

## Hidden-reference and anchor pilot

The committed equation-generated MUSHRA pilot contains one explicit reference, one bit-identical hidden reference, one candidate, and one degraded anchor at -23 LUFS. Six deterministic synthetic sessions are submitted; five are included, and the session rating the hidden reference below the declared threshold is retained raw and excluded with an explicit reason. Bootstrap and Wilson uncertainty are deterministic. This pilot demonstrates harness behavior only; the ratings are not human evidence.

## Campaign round trip

The L2 campaign runner now replaces its label-revealing A/B page with a sealed public `listening/` experiment and a private `listening-analysis.json` key whenever a baseline-backed run reaches `candidate` or `listening_required`. The audit creates a temporary two-trial campaign, prepares its level-matched bundle, verifies that the participant JSON, media IDs, filenames, URLs, and visible text contain no condition role, plays every condition to completion with exclusive controls, reloads after trial one, resumes trial two, exports a browser session, and passes that exact export through the Python analyzer. It then converts the same sealed pair into an ABX fixture whose public X asset contains no answer mapping and verifies browser playback plus private-key scoring. A storage-failure run captures the exposed in-progress recovery JSON after trial one, reloads, rejects four independently tampered copies, restores the untouched copy, and completes the session without losing evidence. Chromium and WebKit are both exercised; browser and Python experiment digests, presentation order, trial order, playback evidence, raw choice, raw-session retention, and null verdict all agree.

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
```
