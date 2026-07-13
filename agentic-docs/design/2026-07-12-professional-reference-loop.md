# Professional reference-matching loop v3

Date: 2026-07-12
Status: accepted — owner-approved 2026-07-12; authorizes the L1–L4 evaluation-harness phases below, but does NOT authorize reference downloads, embedding-model downloads, instrument-model changes before the loop gate, npm publication, or release activity.

## Motivation

1. The current loop is already useful: `scripts/dev/compare.py`, `evals/reference-manifest.json`, standardized auditions, blind A/B pages, and drift checks turned several audible failures into measured diagnoses. The durable lessons and remaining recommendations are owned by `agentic-docs/reports/2026-07-12-modeling-loop-audit.md`.
2. The current metric implementation has no synthetic or golden validation suite, no versioned report schema, no content digests, no declarative tuned/held-out case matrix, and no batch campaign runner. A metric can therefore change meaning or silently mis-handle a corpus without failing CI.
3. The next targets are the familiarity-ladder families: piano, drums, guitars, and bass. Professional-quality iteration on these families requires the loop to distinguish model error, performance/articulation error, room/mix mismatch, corpus defects, and measurement defects instead of collapsing them into one spectral number.
4. The loop must optimize evidence, not a score. Objective metrics accelerate diagnosis and reject artifacts; controlled human listening remains the only authority on whether a candidate sounds better.

## Thesis

Build two coupled loops. The deterministic inner loop renders a declared case matrix, verifies corpus and signal integrity, rejects untrusted comparisons, reports physically interpretable diagnostic trajectories, and records every input needed to reproduce the result. The evidence outer loop checks held-out generalization and blind human preference, then uses listening outcomes to calibrate diagnostic thresholds without turning any metric into a proxy for musical quality.

No single aggregate score is an acceptance gate. A candidate is kept only when artifact gates pass, the stated physical hypothesis improves its named diagnostics, held-out behavior remains acceptable, DSP and packaging gates remain green, and the listening gate prefers it.

## Evidence base

- Multi-resolution spectral losses expose complementary transient and tonal errors that one STFT resolution misses; Parallel WaveGAN used a multi-resolution spectrogram objective and validated the resulting system with listening tests ([Yamamoto, Song, and Kim, 2020](https://arxiv.org/abs/1910.11480)). This supports MR-STFT as a diagnostic family, not as a standalone quality verdict.
- Time-series alignment should tolerate small local timing differences without allowing unrestricted warping to erase attack or decay defects. Soft-DTW formalizes differentiable sequence alignment and states its quadratic complexity ([Cuturi and Blondel, 2017](https://proceedings.mlr.press/v70/cuturi17a.html)). The proposed loop therefore uses bounded, banded trajectory alignment and always reports the warp path cost and maximum displacement.
- ITU-R BS.1387 defines an objective perceptual measurement method intended to aid assessment of audio-system impairments ([ITU-R BS.1387-2](https://www.itu.int/rec/R-REC-BS.1387-2-202305-I/en)). Instrument identity mismatch is a different domain, so PEAQ-style perceptual components may inform diagnostics but an ODG-like score must not be treated as a realism score.
- Fréchet Audio Distance compares distributions in a learned audio embedding and was proposed for music-enhancement evaluation ([Kilgour et al., 2019](https://www.isca-archive.org/interspeech_2019/kilgour19_interspeech.html)). Its training-domain dependence and distribution-level semantics make it a second opinion only, never an inner-loop target and never a per-note acceptance gate.
- Formal subjective methods remain the authority: ITU-R BS.1116 covers controlled assessment of small impairments, while ITU-R BS.1534 specifies MUSHRA for intermediate quality ([ITU-R BS.1116-3](https://www.itu.int/rec/R-REC-BS.1116-3-201502-I/en), [ITU-R BS.1534-3](https://www.itu.int/rec/r-rec-bs.1534/en)). The loop must preserve hidden references, anchors, randomization, listener-level raw results, and uncertainty rather than reporting preference counts alone.
- Repository-specific evidence is direct code audit rather than an external claim: the current `compare.py` uses linear-interpolation resampling, fixed-window point summaries for envelopes and centroids, unpaired peak lists for partials, and a monolithic unversioned JSON report; the audit requested post-note-off gates and trajectory comparison, but those are not yet implemented.

## Design

### Layer 0 — declarative corpus and case contracts

`agentic-docs/licensing.md` remains the owner of corpus provenance and license decisions, while `evals/reference-manifest.json` remains the owner of known limitations and analysis constraints. Add a schema-validated case manifest per family that owns case identity, reference path pattern, MIDI note, velocity layer, articulation, note-on and note-off times, analysis region, channel policy, profile, and role (`tune`, `held_out`, `listening_only`, or `artifact_anchor`). A case declares which axes are trustworthy; the runner must refuse an undeclared or contradictory comparison instead of guessing.

Every report records the reference digest, candidate WAV digest, source commit, WASM digest, case-manifest digest, metric version, Python/runtime versions, and the complete resolved analysis configuration. Scratchpad audio stays uncommitted; committed manifests and digests make local results auditable without redistributing references.

### Layer 1 — deterministic signal preparation

Split loading, channel policy, onset detection, bounded alignment, masking, loudness handling, and resampling into independently tested functions. Replace linear interpolation with a documented band-limited polyphase resampler. Preserve native-rate copies for artifact gates, report every crop/pad/shift/resample operation, and make alignment fail closed when the permitted lag is exceeded.

Produce both level-preserving and level-normalized views. Loudness, velocity response, crest, and dynamics use the level-preserving view; timbre distances may use the normalized view only when the report labels that fact. Corpus flags such as `level_normalized` disable invalid axes rather than merely adding a note.

### Layer 2 — trust gates and diagnostic vectors

Trust gates run first and invalidate downstream interpretation when red: non-finite samples, clipping occupancy, peak-relative adjacent-sample jumps, relative DC, ultrasonic energy at the render's native rate, onset crest conformance, unexpected pre-onset energy, and post-note-off discontinuity or energy where case metadata makes that axis valid. Thresholds are profile-owned, versioned, and covered by fixtures; no threshold may be tuned on the candidate that it gates.

Diagnostics remain a vector: multi-resolution spectral convergence and log-magnitude error; K-weighted log-mel attack/body/tail views; fundamental-aware partial matching with cents and level residuals; envelope, centroid, and per-partial decay trajectories; stereo width and correlation for stereo cases; loudness and velocity-ladder response; and profile-specific views such as kick glide or cymbal bloom. Bounded trajectory alignment may summarize a curve, but raw downsampled trajectories remain in the report.

Sharpness, roughness, and specific-loudness approximations enter only after synthetic validation and listening calibration. They are named descriptors for hypotheses such as "too hard", "beating is synthetic", or "bass is stomping", not general quality scores. Learned embeddings remain an optional outer-loop second opinion.

### Layer 3 — metric validation

Add deterministic synthetic fixtures and metamorphic tests. The suite must prove identity behavior, channel-policy behavior, amplitude invariance only for normalized axes, bounded shift tolerance, monotonic response to detuning and decay changes, and gate sensitivity to injected impulses, DC, clipping, pre-ringing, ultrasonic energy, and release discontinuities. A resampling fixture must reject alias images and preserve in-band tone level within a declared tolerance.

Add small committed golden WAV fixtures generated from equations, not third-party recordings, plus versioned golden JSON reports. Any intentional metric change updates the metric version and golden reports in a PR that explains the semantic delta. CI runs the synthetic suite without access to the private reference corpus.

### Layer 4 — reproducible campaign runner

Add one batch entrypoint that resolves a family case manifest, renders the exact matrix through the shipped WASM path, evaluates every pair, and writes an immutable iteration directory containing machine-readable reports, a concise Markdown table, resolved configuration, logs, and digests. The runner requires an explicit hypothesis, changed component, tuned cases, and held-out cases before it starts.

The runner computes Pareto deltas by named axis and marks results `untrusted`, `regressed`, `candidate`, or `listening_required`; it never edits DSP code, auto-reverts Git, or declares that audio is better. A candidate requires all trust gates, unit tests, loudness, full-arrangement budget, cross-family drift, and held-out policy to pass before an audition page is generated.

### Layer 5 — human calibration and release evidence

Iteration uses randomized blind A/B or ABX with stable level matching and no visual identity leak. Release gates use the appropriate protocol and store per-listener raw responses, randomization seed, listening setup, confidence intervals, and exclusions. Descriptor thresholds may be calibrated against accumulated listening outcomes only on a frozen calibration split; the held-out listening set remains untouched until a release gate.

Instrument campaigns start only after the runner can reproduce a pilot matrix end to end. The intended campaign order is piano, drums, acoustic and electric guitars, then electric bass, matching the owner request and the familiarity ladder. Each campaign gets its own scoped hypothesis and model-design owner; this document authorizes only the shared loop after acceptance.

## Phased plan

1. **PR L1 — metric kernel and trustworthiness tests.** Refactor signal preparation and metric modules without adding new instrument features; add synthetic/golden fixtures, report schema/version/digests, band-limited resampling, fail-closed axis disabling, and the missing release/pre-onset gates. Gate: deterministic reports across two consecutive runs; all synthetic mutations trigger only their expected gates/axes; identical fixtures are zero within declared numerical tolerance; CI is green.
2. **PR L2 — case manifests and batch runner.** Add schema validation, family case manifests, tuned/held-out roles, immutable iteration output, shipped-WASM rendering, Pareto deltas, and cross-family drift integration. Gate: one command reproduces a pilot matrix for piano, drums, guitar, and bass from locally available references; a missing digest, invalid trust flag, absent held-out case, stale WASM, or red artifact gate fails before an audition is produced.
3. **PR L3 — trajectory diagnostics and profile calibration.** Add fundamental-aware partial pairing, bounded envelope/centroid/partial-decay trajectory comparison, stereo diagnostics, and profile-owned thresholds. Gate: synthetic detune, decay, beating, stereo-collapse, and transient mutations move the intended axes monotonically; the pilot reports explain known owner verdicts without regressing artifact detection; runtime stays practical for an iteration matrix.
4. **PR L4 — listening evidence and optional second opinions.** Upgrade the A/B surface to record reproducible randomized trials and uncertainty; add embedding distance only if model weights, license, offline execution, and domain validation pass a separate review. Gate: a hidden-reference/anchor pilot demonstrates that the listening harness records and analyzes trials correctly; no objective metric is presented as a release verdict.
5. **Campaign P1 onward — instrument work.** Run the accepted loop on piano first, then drums, guitars, and bass. Each physical-model change is a separate scoped PR with DSP benchmarks, both-sample-rate tests, standardized auditions, held-out evidence, and owner listening approval.

## Deferred until demanded

- Differentiable or gradient-based parameter fitting.
- Automatic DSP code generation, automatic Git revert, or autonomous merge decisions.
- Cloud evaluation services, paid APIs, crowdsourcing, or quota-consuming embedding endpoints.
- Shipping neural weights or an embedding runtime with the product.
- A universal scalar "realism" or "professional quality" score.
- Committing or redistributing reference recordings beyond their verified licenses.
- Replacing expert listening with PEAQ, FAD, MR-STFT, or any other objective metric.
