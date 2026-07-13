# Acoustic drum reference corpus v2

Date: 2026-07-12
Status: draft — this document proposes a source-diverse kick/snare evidence program for issue #43. It does not authorize acquiring audio, unsealing a holdout, redistributing audio, changing DSP, accepting a kit, or making pop/rock/jazz quality claims before source receipts, a frozen split, trust audits, panel approval, and human listening pass.

## Motivation

The owner correctly challenged the evidence behind the pop, rock, and jazz kits, especially kick and snare. Lessons carried forward:

1. Four isolated cases cannot identify six core targets. The audited matrix has one pop kick, one rock kick, one rock snare, and one held-out jazz brush/snare; it has no pop snare, jazz kick, real velocity ladder, or source-independent held-out kick/snare pair.
2. A genre label is not an acoustic specification. “Pop,” “rock,” and “jazz” must resolve to recorded construction, head/tuning/damping, beater or brush technique, microphone role, room contribution, and velocity behavior.
3. One microphone perspective cannot own both the physical voice and the produced record. Close/direct channels may fit excitation and shell/body behavior; overhead, mid, and room channels are separate radiation/listening evidence and cannot be silently folded into the same numeric target.
4. A single hit is not a drum. Every credible native hit and layer must survive canonicalization; three summary regions cannot replace the full velocity/timbre curve, and amplitude-scaled copies do not count as dynamics or repetitions.
5. A held-out source is not fresh if its audio has already informed a fit. DRSKit has already been inspected and used during jazz work, so it is tune/calibration material only, never a fresh holdout.
6. The 808 campaign is comparatively better specified because it records original-hardware provenance and invalid axes explicitly. Acoustic kits need at least that level of honesty, plus source diversity, performer variation, live control, spatial behavior, and full-arrangement stress.

## Thesis

Build the acoustic-kit corpus around physical and recording axes first, then map validated targets to product presets. Each pop, rock, and jazz kick/snare target requires at least ten independent acquisition families: four or more tune families, two threshold-calibration families, and four globally sealed holdout families. The evidence retains real strike distributions, repeated hits, and explicit close versus spatial microphone roles. Absolute velocity-to-loudness evidence is valid only within one unchanged recording chain; cross-source comparisons own timbre and envelope shape only unless gain calibration is documented. Results describe the exact frozen target bundle and may not claim to represent all pop, rock, or jazz drums.

The corpus remains private and content-addressed. Git stores normalized source metadata, access/unsealing records, license receipts, immutable digests, canonicalization operations, event manifests, invalid-axis declarations, and content-addressed reports—not reference audio. DSP fitting cannot begin until source selection proves that every claimed axis is observable and the split was frozen before any holdout audio was opened.

## Evidence base

### Current-state audit

- The exact acoustic matrix at commit [`e59291d`](https://github.com/keunwoochoi/instruments.js/blob/e59291d1f7450e7c5e0f0ac2e07995fd37e3f885/evals/cases/drums.json) contains only four cases: pop kick ff, rock kick mf, rock snare ff, and held-out jazz brush mf. The audit finding is tracked in [issue #43](https://github.com/keunwoochoi/instruments.js/issues/43).
- The exact reference contract at [`e59291d`](https://github.com/keunwoochoi/instruments.js/blob/e59291d1f7450e7c5e0f0ac2e07995fd37e3f885/evals/reference-manifest.json) identifies the existing `virtuosity-drums` material as CC0 and declares room bleed, but it does not make the four-case matrix source-diverse.
- The four declared pop/rock/jazz files are absent from the visible scratchpad, while implementation comments attribute rock material to CC-BY Muldjord/Naked sources that the generic manifest can incorrectly inherit as Virtuosity/CC0. Unregistered references currently receive an empty corpus contract rather than a license/invalid-axis failure. P0/P1 must close these trust holes before acquisition or fitting.
- A staged Virtuosity receipt already records real soft/mid/hard jazz-kick close hits plus matched mid and overhead perspectives, without level normalization. This is useful tune evidence but cannot be its own held-out source.

### Primary source candidates

- [Virtuosity Drums’ official product notes](https://versilian-studios.com/virtuosity-drums/) and [manual](https://versilian-studios.com/Distro/VirtuosityDrumsManual.pdf) document a CC0 contemporary-jazz kit, up to 36 natural dynamic layers for kick/snare/toms, independent kick/snare close microphones, overhead/mid/room/vintage positions, kick damping, snares on/off, bleed control, and diverse snare articulations. It has dense natural dynamics but no kick/snare round robins, so it is tune material rather than repetition or holdout proof.
- [DRSKit’s official DrumGizmo page](https://www.drumgizmo.org/wiki/doku.php?id=kits%3Adrskit) declares CC-BY-4.0, says the handcrafted kit is intended from jazz through rock, and documents 13 channels including front/back kick, top/bottom snare, overheads, and ambience. Its audio has already been inspected and used to fit jazz, so it is permanently contaminated for holdout use and may serve only tune or threshold-calibration roles.
- [SM Drums’ official project page](https://smmdrums.wordpress.com/for-reaper/) documents dry, unnormalized WAVs with 127 kick velocity layers ×2 round robins and 127 snare layers ×4 round robins, including no-ring and studio-ring snare variants. The author page does not display an explicit legal grant; a secondary catalog’s “Public Domain” label is insufficient. SM Drums remains rights-blocked until a bundled license or written author confirmation is retained.
- [Naked Drums’ licensed SFZ repository](https://github.com/sfzinstruments/WilkinsonAudio.NakedDrums) and [catalog entry](https://sfzinstruments.github.io/drums/naked_drums/) declare CC-BY-4.0, a Yamaha Recording Custom 22-inch kick, two documented snares, multiple room/overhead/close channels, ten round robins, and up to five velocity layers. Existing implementation attribution means P0 must conservatively classify its access history before assigning a role.
- [MuldjordKit’s official DrumGizmo page](https://drumgizmo.org/wiki/doku.php?id=kits%3Amuldjordkit) declares CC-BY-4.0, identifies a Tama Superstar metal/rock kit, and documents inside-kick D112/trigger, snare top/bottom, overhead, and ambience channels. It also declares a snare phase-inversion requirement and known low-layer defects; those caveats must become enforceable operations/exclusions. Existing implementation attribution means it is presumptively tune material.
- [CrocellKit’s official DrumGizmo page](https://drumgizmo.org/wiki/doku.php?id=kits%3Acrocellkit) declares CC-BY-4.0, identifies the actual metal-band recording kit, and documents independent inside/outside kick, top/bottom snare, three overhead, and two ambience channels. The archive contains 51 left-kick hits, 49 right-kick hits, and 98 center-snare hits; left/right double-pedal articulations must remain distinct. It is a metadata-only rock holdout candidate only if the access ledger proves its audio has never been opened.
- [Kitty’s official DrumGizmo page](https://drumgizmo.org/wiki/doku.php?id=kits%3Akitty) declares CC-BY-4.0, describes a modern pop/rock hybrid kit, documents 14 independent channels, and separates kick in/out, snare top/bottom, overhead, room, and trash microphones. Hardware identity is intentionally undisclosed, so construction-dependent axes are invalid. It is a metadata-only calibration or holdout candidate until P0 verifies archive identity, hit coverage, processing, and access history.
- [ShittyKit’s official DrumGizmo page](https://drumgizmo.org/wiki/doku.php?id=kits%3Ashittykit) declares CC-BY-4.0, identifies an 18-inch kick and 14×5 snare, documents close/overhead/M-S room capture, and says the source is unprocessed apart from time adjustment between close microphones and overhead. Its old fixed velocity groups and pre-existing time adjustment require explicit invalidity/operation records. It is a metadata-only compact/jazz challenge or holdout candidate until P0 verifies its distributions and access history.
- [DrumGizmo’s official sampling workflow](https://drumgizmo.org/wiki/doku.php?id=getting_dgedit) instructs recording at least 30 hits per drum from very light to very hard with separate close, overhead, and room tracks. This supports distributional velocity/repetition gates rather than one hand-picked hit per label.
- [Big Rusty Drums’ official page](https://shop.karoryfer.com/pages/free-big-rusty-drums) and [CC0 repository](https://github.com/sfzinstruments/karoryfer.big-rusty-drums) document more than 4,400 samples from a 24-inch kick and 14×8 snare using sticks, brushes, and mallets, with close/overhead capture, snare bottom, damping variants, center/edge/rimshot/sidestick hits, and brush stirs/flutters. It is a source-independent brush/articulation candidate, not a velocity-curve authority until exact coverage is audited.
- [Swirly Drums’ official page](https://shop.karoryfer.com/pages/free-swirly-drums) documents CC0 brush-only sampling, controllable snare stirs/flutters, center/edge hits, and a brushed kick among more than 4,700 samples. It is a brush-technique tune candidate, not automatically a jazz target: the source says its drums are punk/metal instruments played gently with brushes.
- [Ben Burnes’ official brushed-drum page](https://ben-burnes.gumroad.com/l/bb_brushed) declares CC0 Yamaha Birch Custom Absolute snare recordings with two brush types. It remains an optional challenge candidate until its downloaded manifest proves real dynamic/repetition coverage and a complete license receipt.
- [ENST-Drums’ primary ISMIR paper](https://ismir2006.ismir.net/PAPERS/ISMIR0627_Paper.pdf) documents isolated hits and professional performances from three drummers and their own kits, with sticks, rods, mallets, brushes, close kick/snare channels, and stereo overheads. Its research-use terms are not a permissive audio grant, so it stays authority-blocked.
- [RWC Musical Instrument Sound’s primary database page](https://staff.aist.go.jp/m.goto/RWC-MDB/rwc-mdb-i.html) documents professional performers, multiple manufacturers/styles, and three dynamics including individual drum-kit sounds. It remains authority-blocked because access and audio-use terms are unclear.
- The physical case model follows published snare-drum coupled-system work rather than treating a snare as one filtered noise burst ([Bilbao, JASA 2012](https://www.research.ed.ac.uk/en/publications/time-domain-simulation-and-sound-synthesis-for-the-snare-drum/)). Velocity is a physical axis because measured membrane spectra and modal behavior change with striking force ([Dahl, nonlinear drum-membrane study](https://www.research.ed.ac.uk/files/16389380/Nonlinear_Effects_in_Drum_Membranes.pdf)).
- Attack is trajectory evidence, not one duration scalar: controlled timbre-perception work found attack temporal centroid more explanatory than attack time alone ([Kazazis, Depalle, and McAdams, JASA 2021](https://www.mcgill.ca/mpcl/files/mpcl/kazazis_2021b_jasa.pdf)).

Anything not verified from a primary license/source page remains a candidate, not evidence. Commercial libraries, unclear “royalty free” packs, normalized previews, mixed song stems, and copyleft code or assets are excluded.

Rejected core candidates are recorded rather than forgotten: Aasimonster has documented inter-channel timing errors; IDMT-SMT-Drums is CC-BY-NC-ND with insufficient acoustic mic provenance; Salamander has only two velocity levels plus normalized/defective files; AVL provides buses rather than preserved direct/overhead/room stems. They may not silently re-enter as calibration truth.

## Design

### 1. Truth ownership, receipts, and reproducible evidence

The committed registry at `evals/reference-sources/acoustic-drums/registry-v2.json` owns normalized source metadata and stable foreign keys. Per-source committed receipts under `evals/reference-sources/acoustic-drums/receipts/` own URL, retrieval date, immutable upstream version/archive checksum, exact license text/checksum, attribution, access status, original format, disclosed processing, kit/capture facts, file inventory hashes, and valid/invalid axes. `agentic-docs/licensing.md` owns policy and links these receipts without repeating them.

Private archives live at `$IJ_REFERENCE_ROOT/sources/<source_id>/<archive_sha256>/`; private canonical audio lives at `$IJ_REFERENCE_ROOT/canonical/acoustic-drums-v2/<source_group_id>/`. Committed machine-readable reports live under `evals/evidence/acoustic-drums/v2/`; prose may interpret a report only by linking its content digest.

P1 adds one copy-paste interface for both licensed local audio and public fixtures:

```sh
npm run drums:corpus -- audit --registry evals/reference-sources/acoustic-drums/registry-v2.json --reference-root "$IJ_REFERENCE_ROOT" --out /tmp/drum-corpus-evidence
npm run drums:corpus -- verify-evidence evals/evidence/acoustic-drums/v2
```

Each canonical report payload records schema version, tool commit, runtime versions, command/config digest, registry digest, source receipt and archive digests, canonical/event-manifest digests, and output digests. Volatile generation time, actor, machine-local paths, and host identity live in an unhashed attestation sidecar; identical inputs must produce a byte-identical canonical payload and digest. Public CI runs schema, adversarial, duplicate/leakage, and stale-report fixtures without licensed audio; a rights-cleared local run consumes the same schema and emits the same artifact shape. Missing, stale, mismatched, or unregistered evidence fails closed.

### 2. Identity, access history, and frozen roles

`source_family_id` is a stable opaque digest over the original audio-acquisition lineage before ports, edits, repackaging, or derivative formats; normalized upstream project, acquisition/session, performer, and kit identities back the digest. `source_group_id` is a stable opaque digest for one exact session + performer + physical kit + microphone setup within that family. `hit_id` is the source group + articulation + upstream physical-hit index and is identical across every microphone channel for that physical strike. Registry uniqueness and foreign-key checks reject unknown parents, duplicate audio, aliases masquerading as independent families, cross-role reuse, and one hit mapped to inconsistent energy or articulation metadata.

Every source family has one global role: `tune`, `threshold_calibration`, or `sealed_holdout`; all derivative groups and microphones inherit it. A committed append-only access ledger records `metadata_only`, `audio_opened`, or `fit_used`, the first access/unseal issue and UTC time, actor, purpose, and archive digest. A holdout is eligible only if its family was `metadata_only` when the split was frozen; metadata/license-page inspection is allowed, audio preview or waveform inspection is not. DRSKit is recorded as `fit_used` and cannot be a holdout. Unknown historical access is conservatively contaminated.

Before any DSP campaign, each core kick/snare preset target must have at least ten eligible independent source families: at least four tune families, two threshold-calibration families, and four globally sealed holdout families. P3 simulation may raise but never lower those floors. The split is frozen in a reviewed commit before acquisition. No source listed here is assigned a final role; P0 remains blocked until the access audit and coverage inventory make that allocation possible. Leave-one-tune-family-out folds and the four-family final holdout set are fixed then, and every fold/family is reported. Holdout evaluation is intentionally one-shot: a failed final set blocks the target until four entirely new never-opened families are registered; P0 does not pretend one family can fund repeated tries.

### 3. Group-coherent canonicalization and sample-rate policy

Native masters are immutable. Canonicalization may decode, extract preserved channel roles, apply an explicitly sourced group-coherent polarity operation, choose one onset/crop anchor from the designated direct channel for each physical hit, apply that same crop to every channel, and zero-pad to a declared duration. It may not independently align microphones, normalize hits, suppress bleed, denoise, EQ, compress, or average channels. Original interchannel sample offsets survive; the report records the anchor channel, offset vector, polarity vector, and pre/post digests.

Canonical 44.1 kHz and 48 kHz files are each generated directly from the native master with one pinned resampler/version/configuration; resampling is never chained. Candidate renders use the same two rates. The rate-parity report must show the same qualitative hypothesis result and no rate-specific artifact at both rates before a DSP PR can pass.

Every metric case names a `projection_id`. Physical dry-mono fitting compares the engine’s dry voice only with a designated direct-microphone projection. Each hypothesis also declares whether direct-microphone coloration is an intended product target, an explicit observation-transfer model, or a nuisance excluded by invariant metrics; it cannot leak silently into nominal exciter/resonator parameters. Multichannel spatial evidence uses declared direct/overhead/room roles and a separately defined stereo observation projection; microphone channels never count as independent samples and the same dry mono render is never scored against all microphones as if they were interchangeable targets. Level-matched listening copies are separately hashed derivatives and never replace raw-amplitude canonicals.

### 4. Full velocity curve, deterministic events, and repetitions

Every credible native hit and layer is retained. One frozen energy rank per physical hit comes from trusted upstream energy metadata or a pinned window/metric on the designated direct channel and propagates unchanged to all microphones. Ties break by `hit_id`. Soft/medium/hard are empirical tercile summaries only; the primary velocity analysis covers the complete ordered curve, explicitly including the low-energy ghost transition and high-energy saturation region.

Engine events use the frozen MIDI velocity grid `[8, 24, 40, 56, 72, 88, 104, 120, 127]`, fixed note times, fixed seeds, and immutable event IDs. The reference mapping from each engine event to empirical ranks/quantiles is preregistered before candidate rendering; no favorable hit may be selected afterward. Event manifests record tempo, articulation, MIDI velocity, reference quantile rule, repetition order, seed, overlap, and projection.

A source/articulation is numerically eligible only with at least 24 physical hits and at least 8 hits in each tercile. Worst-decile checks require at least 40 eligible hits; 24–39 hits use a preregistered worst-quartile check, and lower coverage is listening/challenge evidence only. The final P0 inventory may raise these floors if calibration uncertainty remains too wide.

### 5. Product hypotheses and candidate allocation

| Preset | Candidate tune pool | Candidate calibration/holdout pool, subject to frozen access audit | Core physical target |
|---|---|---|---|
| Pop | DRSKit, Naked Drums, any already-opened Kitty material | unopened Kitty, Crocell, or another P0-qualified independent family | dry/direct studio kick and snare, controlled low-band decay, clear but non-metallic attack, full natural velocity curve |
| Rock | MuldjordKit, DRSKit, any already-opened Crocell material | unopened Crocell, Kitty, or another P0-qualified independent family | harder beater/inside-kick attack, stronger high-mid snare crack and wire band, larger sustained shell/room energy without one narrow modal ring |
| Jazz stick | Virtuosity Drums, DRSKit, ShittyKit if historically opened | unopened ShittyKit or another P0-qualified compact acoustic family | less click-dominated kick, diffuse rather than pitched decay, audible mid/overhead radiation, controlled snare-wire texture and source-credible room |
| Jazz brush | Swirly Drums or historically opened brush material | unopened Big Rusty/Ben Burnes or another independent brush family | center/edge strikes plus separately modeled stir/flutter gestures; no continuous-brush release claim until the control and holdout gates pass |

These rows are hypotheses, not role assignments and not permission to force every source toward a stereotype. Because each source family has one global corpus role, P0 must choose allocations that satisfy every target without reusing one family across tune/calibration/holdout. If ten eligible families per core target cannot be assembled, that target remains blocked rather than weakening independence.

### 6. Metrics, uncertainty, and decision rule

Kick and snare retain MR-STFT, multiscale log-mel, loudness, and artifact diagnostics and add:

- attack energy trajectories in 0–5, 5–20, and 20–50 ms windows by low, low-mid, high-mid, and high bands;
- onset crest, spectral flux, and centroid trajectories rather than one attack scalar;
- band-wise decay slopes and time-varying spectral centroid, with room/bleed invalidity applied by source/mic role;
- pitch salience, peak Q, and harmonic concentration to reject the owner’s “vibraphone-like” narrow tonal kick failure;
- snare shell-to-wire/noise energy, high-band wire decay, and noise-to-tonal balance;
- within-source velocity-to-loudness and velocity-to-timbre trajectories, monotonicity, ghost transition, and saturation behavior;
- repeated-hit median, normalized MAD, lower-tail behavior, and eligible worst-decile/worst-quartile artifacts.

The independent statistical unit is the acquisition family, with source group nested within family and physical hit nested within group. A hierarchical clustered bootstrap resamples families first, then groups and physical hits; microphone channels are joint observations and never independent replicates. Each of the four holdout families must also pass its adjusted family-wise non-inferiority bound; an aggregate cannot rescue one failed family. Normalized MAD means `1.4826 × median(abs(x - median(x)))` and is estimated from the two threshold-calibration families’ repeat-difference distributions for the same voice, role, velocity region, rate, and metric. Even with these floors, inference is scoped to the exact registered families rather than a population-level genre claim.

Before the first candidate render, each DSP PR freezes its physical hypothesis, synthesis paradigm, governing equations, physical parameter mapping/bounds, browser-budget rationale, primary metric, required secondary gates, observation projection, event/reference mapping, renderer commit, competitive-baseline manifest, decision rule, alias threshold/mitigation, per-voice CPU/memory ceiling, tail-discontinuity bound, and deterministic seed/parameter torture procedure. Tune data may then optimize declared parameters, but it may not select a favorable outcome metric or change the decision rule. Required secondary safety axes use simultaneous 95% confidence bounds with Holm adjustment; descriptive diagnostics do not become gates after results are seen. Leave-one-tune-family-out results are all shown. The provisional design targets—at least 0.5 normalized-MAD improvement on the primary tune axis and no more than +0.25 normalized-MAD held-out regression—are not authoritative until P3 calibrates and freezes them before candidate rendering. A red attack, tonality, velocity, artifact, rate-parity, or held-out gate cannot be hidden by an aggregate score.

### 7. Temporal, spatial, live, and human evidence

Phrase manifests contain fixed 8-bar patterns at 60, 120, and 180 BPM with 8th/16th/32nd same-velocity repeats, alternating ghost/accent strokes, kick/snare interleaving, rolls, fills, tempo changes, and overlapping tails. Each runs as dry and through the shared spatial stage. Spatial listening rejects implausible room buildup, detached early reflections, collapsed depth, or one kit occupying a different acoustic space from the arrangement.

Stress manifests force simultaneous drum voices, full-arrangement polyphony, voice stealing, and long-tail overlap under shared reverb. Acceptance requires deterministic output, preserved priority attacks, graceful tail shedding, and no clicks, discontinuities, unintended choke, NaN/Inf, or silence.

The committed baseline manifest at `evals/baselines/acoustic-drums-v2.json` pins exact Tone.js, smplr, and SpessaSynth package/asset versions, licenses, configuration, and content digests; an ineligible engine is rejected explicitly rather than silently omitted. Incumbent, candidate, competitive baselines, hidden acoustic reference, and low-quality anchor render the identical immutable MIDI/event, tempo, seed, rate, projection, spatial-stage, and preregistered loudness-handling manifests. This baseline registry owns versions so the design doc does not echo mutable package facts.

The trained-playability gate names an owner and at least one trained drummer or keyboard-drum performer. Each plays every candidate on a physical velocity-sensitive controller for at least 10 minutes across velocities 1–127, soft-to-hard touch, ghost notes, repeated strokes, rolls/fills, tempo changes, and the full arrangement. Controller calibration rejects dead zones, non-monotonic response, or audible stepping between the nine metric points. The issue freezes interface, monitoring chain and level, audio buffer, browser/device, latency measurement method, incumbent, and randomized/blinded incumbent-candidate mapping before the session; the report records invitations and fights plus structured harshness, mechanical-sameness, tail-buildup, and willingness-to-continue ratings at the start, five minutes, and finish.

Final perceptual quality requires a preregistered MUSHRA campaign, not only the playability panel. P5 freezes at least 20 eligible trained listeners or the larger sample from an 80%-power analysis for a 10-point paired difference; randomized blinded trials include the hidden acoustic reference, a declared low-quality anchor, incumbent, candidate, and every eligible competitive baseline, with at least 20% repeated items. Exclusions are fixed before collection (hidden-reference median below 80, anchor not ranked below the hidden reference, or repeated-item absolute difference above 20); all exclusions remain reported. The candidate passes only if the Holm-adjusted 95% lower bound of its paired improvement over the incumbent is above 5 MUSHRA points, its adjusted lower bound is no worse than 5 points below the best competitive baseline, its median is at least 70, and owner preference also passes. P0 should add at least one license-clean natural professional performance as a private spatial challenge; if none is authoritative, spatial generalization stays explicitly blocked.

Live input reports median and p95 input-to-audio latency, event jitter, dropout count, and callback load against the exact incumbent. P5 measures the device baseline and freezes an absolute ceiling plus a non-inferiority margin before candidate testing; zero dropouts is mandatory. A candidate cannot choose its ceiling after measurement.

The physical iOS issue must name an actually available iPhone or iPad model and exact iOS/Safari version before P6 begins. On that device the procedure covers gesture unlock, repeated kick/snare/brush playback across velocities 1–127, a measured native touch/pad-to-audio path even when external MIDI is unavailable, external live controller input when supported, full-arrangement p50/p95 callback load and latency, dropout count, interruption/background/resume, recovery, and evidence export. Desktop WebKit is useful evidence but never substitutes for this gate.

Continuous brush stir/flutter remains blocked until the public control contract can express time-varying per-note pressure, rate, damping, position/timbre, and release gestures with deterministic gesture manifests. Its eventual polyphonic gate requires independent simultaneous control streams with no cross-voice leakage, unrelated choke, or tail corruption during voice stealing. Strike-only brush evidence may remain provisional; it cannot be labeled continuous brush modeling.

### 8. DSP stability and budget gates

Every later DSP PR runs 44.1/48 kHz full-velocity single-hit and repeat torture tests across every reachable parameter bound and adversarial automation transition, using a committed exhaustive seed set or reproducible property-based generator. It includes finite/bounded-state, tail-termination, denormal, hard-strike alias-energy, deterministic voice-steal/overlap, and worst simultaneous-drum full-arrangement checks. Every nonlinear operation names its alias mitigation and must stay below the pre-render calibrated alias threshold. The report enforces frozen per-voice and mix CPU/memory ceilings, callback load, tail-energy/discontinuity bounds, and `dsp-bench` against the 2.67 ms / 128-frame arrangement budget. Solo-instrument timing cannot waive an arrangement regression.

## Phased plan

### P0 — registry, access audit, and split freeze

One PR creates the normalized registry/receipt schema, records candidate metadata and historical `metadata_only|audio_opened|fit_used` state without acquiring new audio, resolves generic-license misattribution, and freezes globally exclusive roles. Gate: every pop/rock/jazz core voice has at least four tune families, two calibration families, and four never-opened holdout families; the archive/license authority and capture axes are traceable; role leakage, alias families, and unknown access fail closed. If coverage is insufficient, P0 reports the block and no source is unsealed.

### P1 — trust auditor and public evidence contract

One PR implements the two `drums:corpus` commands, normalized IDs/foreign keys, immutable access transitions, group-coherent multichannel rules, event manifests, duplicate/leakage rejection, report freshness, and public synthetic/adversarial fixtures. Gate: public CI verifies the exact report shape without private audio, every adversarial fixture fails for its intended reason, and existing loop audits remain green.

### P2 — tune and threshold-calibration canonical campaign

One PR acquires only frozen tune/calibration roles, retains every credible native hit/layer, creates native-derived 44.1/48 canonicals, generates full-curve and temporal manifests, and calibrates repeat-distribution floors. Gate: coverage eligibility is reported per voice/source/articulation, multichannel identity is coherent, no holdout audio is opened, and no DSP changes.

### P3 — metric calibration and incumbent baseline

One PR validates drum diagnostics against repeat-vs-repeat and cross-source baselines, freezes robust-scale and simultaneous-inference rules, publishes every leave-one-family-out fold and failure example, pins the competitive-baseline manifest, and renders the untouched incumbents/baselines from identical event manifests. Gate: each gating metric ranks known relationships predictably, numeric sound-quality thresholds and baseline digests are frozen before candidate work, and every untrusted existing claim is explicit.

### P4 — phrase, spatial, and reverb evidence rehearsal

One PR rehearses deterministic phrase/stress manifests, dry/shared-space playback, full-arrangement voice-steal stress, storage/export, and the holdout report pipeline using synthetic sealed fixtures. Gate: evidence is reproducible, shared reverb is audibly and numerically present when enabled, dry and spatial projections remain distinct, no evidence path silently falls back, and the failed synthetic holdout cannot be reused for iteration.

### P5 — live, human, and physical-device protocol rehearsal

One PR rehearses trained-player forms, randomized live A/B, fatigue ratings, MUSHRA anchors/controls/repeats/exclusions, live latency/jitter capture, full-range controller calibration, and the named physical iOS procedure on incumbents. It also freezes a producer quick-check artifact that starts from each untouched factory preset, exposes soft/hard kick and snare attacks/tails, then enters the same full arrangement with visible CPU/xrun evidence and an immediate `PASS|DISMISS`. Gate: perceptual sample size/thresholds, latency ceiling/non-inferiority margin, and baseline identities are frozen; evidence export/recovery works; named humans/device owners accept the protocol.

### P6 — DSP campaigns, one voice/preset hypothesis per PR

Only after P0–P5 pass, open separate kick/snare modeling PRs with one preregistered physical hypothesis. The hypothesis, paradigm/equations, outcome metrics, mappings, thresholds, renderer, baselines, resource/artifact limits, and decision rule freeze before the first candidate render. Each campaign then fits only declared parameters on tune data and freezes the candidate commit; only afterward may the custodian unseal/canonicalize the assigned four-family holdout set. A failed holdout ends that hypothesis and may not inform another iteration; retry remains blocked until four new sealed families exist. Final evidence includes both sample rates, temporal/spatial/live gates, the producer `PASS|DISMISS`, adequately powered MUSHRA, full-arrangement `dsp-bench`, exact-head panel, trained-player report, owner blind listening, and physical iOS evidence. Jazz kick #39 is re-evaluated rather than grandfathered; no prior fit survives solely because it improved the old four-case matrix.

## Deferred until demanded

- Toms, hats, rides, crashes, and percussion beyond phrase-level drift guards.
- Shipping reference audio or a sampler runtime in the product.
- Emulating commercial production chains, sample replacement, gated reverb, or mastered-record loudness.
- A genre classifier or a claim that one acoustic kit defines all pop, rock, or jazz.
- Learned embeddings until weights, license, offline execution, and drum-domain validity pass separate review.
- Continuous brush release until a public time-varying control path and two independent license-clean source families pass the gesture, dynamic, and repetition gates.
