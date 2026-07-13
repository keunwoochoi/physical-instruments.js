# Modeling & loop audit — 2026-07-12

Status: survey + decisions (Keunwoo request: "overall audit about the modeling & the loop — modeling: you survey and decide; loop: prompt/metric/criteria/similarity").
Author: main loop, with full context of rounds 1–4 and the five in-flight agents.

## Part 1 — Modeling survey and decisions

### Where each family stands

| Family | Engine state | Honest gap (biggest first) |
|---|---|---|
| Piano | Multi-string waveguide, Stulov felt, Weinreich pair + bridge coupling, 88-key Salamander calibration (P1), phantom/longitudinal nonlinearity in flight (P3) | **P2 soundboard**: 6-mode ladder + knock is the weakest link — bass radiation excess (+15 dB, 20–60 Hz), no frequency-shaped bridge admittance (deep-bass prompt/tail conflict), no dense board IR. Then P4 pedal vocabulary; 88-key voicing corpus. |
| Acoustic guitars | AcPluck: dispersion cascades, 2 polarizations, Woodhouse radiation chain, 16-mode body; Helmholtz air + coupled top + pick attack in flight (r4) | Body is **per-voice** — it dies with the note; a real box rings *between* notes. No sympathetic open-string coupling (6 strings always ring). No articulation vocabulary (strum roll, slide, hammer-on). |
| Electric guitars | ElectricVoice: pickup comb, ADAA amp, sag, drive-90 lead; amp compression/cab in flight (r4) | Single tanh stage vs real multi-stage topology; no speaker-cone character beyond biquads; no musical feedback at high gain. |
| Electric bass | Round in flight (attack, pickup comb, roundwound B) | Was structurally under-modeled (click 0, no pickup model) — agent owns it. |
| Drums | Per-kit topologies (membrane-modal jazz, beater-transient rock), kit divergence + brushes in flight (r4) | **No room**: everything is a dry close mic. No snare-buzz coupling (kick/toms exciting snare wires) — the glue of a real kit. |
| Cymbals | 22-band banded noise + bloom gate | Adequate for now; ride bell/wash interaction is a refinement, not a gap. |
| Mallets / EP / pad | Modal banks, tine+pickup, polyBLEP | Low complaint volume. EP oversampling issue stays filed. |

### Decisions (ranked by wow ÷ CPU, cross-family first)

1. **Shared room/early-reflection stage** — the single biggest "sounds like a record" gap across EVERY family. All renders are anechoic-dry; real instrument identity is partly *room*. One allocation-free early-reflection network + short diffuse tail (small FDN, a few KB of state, one per engine not per voice), default subtle, per-track send. Drums benefit most, piano/guitar next. This is the next engine-level facility I will build (main loop, after the five agents merge — it touches the shared bus).
2. **Piano P2 soundboard** — already designed; P1/P3 measurements keep pointing at it. Launch after P3 lands and Keunwoo's ear passes on nonlinearity.
3. **Articulation/performance layer** — strum rolls, drum flams, legato slides, hammer-ons: cheap in the scheduler (note-event preprocessing), large musical payoff; phrase realism is currently limited by note-level realism no matter how good single notes get. Scoped design doc first (it's an API surface question too).
4. **Persistent guitar body + sympathetic open strings** — body bank at track level (like SympBank), open-string sympathetic set for acoustics.
5. **Snare-buzz coupling** (kick/tom → snare wires) — drum-region, pairs with the room stage.
6. **MPE / continuous control** — the models support it naturally (Jordan's blocking issue); becomes urgent when the sound plateau nears.
7. Winds/bowed stay Q3 roadmap. Neural/differentiable fitting stays offline-only (P1's calib pipeline is already halfway to differentiable parameter fitting — worth noting for the eventual "fit params offline, ship tiny tables" story).

## Part 2 — Loop audit (prompt · metric · criteria · similarity)

### What tonight proved about the loop

Two incidents were the loop working *and* failing: agents twice optimized FOR artifacts (ultrasonic click the 16 kHz refs couldn't see; metric optimum "riding the clip ceiling"). Both were caught — but by ear and by post-hoc waveform reading, not by the loop itself. The corpus itself lied twice (level-normalized refs → 5 dB velocity spans; a 404 HTML page staged as a corpus). Conclusion: the metric layer needs *adversarial instrumentation*, not just more axes.

### Metric / similarity — decided upgrades (ordered)

1. **Artifact sanity gates inside compare.py** (build next): a `gates` section computed on every comparison — onset crest vs ref crest envelope, max adjacent-sample jump, >16 kHz energy ratio, DC offset, post-note-off energy. Any gate red → the metric result is flagged untrusted. Agents get "a red gate invalidates the iteration" in the protocol. This directly kills the artifact-optimization failure mode.
2. **Multi-resolution STFT distance** (replace single-window log-mel as the headline number): 3 window sizes (256/1024/4096), K-weighted, averaged — standard in neural-audio eval because single-resolution metrics miss transient vs tonal trade-offs. Onset-align before comparing (cross-correlate first 50 ms) so micro-timing doesn't pollute timbre distance.
3. **Reference corpus manifest** (`references/MANIFEST.json`): per-file sr, trust level, known artifacts (gate time, normalization, room bleed) — compare.py masks known-artifact regions (the NSynth 0.3 s release gate taxing correct tails; measured tonight by the electrics agent). Also codifies: staging a corpus requires decode + spectrogram sanity check (the 404 lesson).
4. **Embedding distance as second opinion** (after 1–3): FAD-style distance on a pretrained audio embedding (CLAP or OpenL3, run locally/offline only) per family. Embeddings trained on real audio penalize "physically plausible but weird" — a complementary failure surface to spectral distances. Never the fitting target (Goodhart); always the cross-check.
5. **Psychoacoustic descriptors for targeted verdicts**: roughness (unison beating quality), sharpness (brightness complaints), specific-loudness difference (ISO 532-style) as named axes — they map 1:1 to how Keunwoo phrases verdicts ("too hard", "too soft", "stomping").
6. **Trajectory comparison over point stats**: heterodyne partial ladders already exist — compare full decay *trajectories* (soft-DTW or per-segment slopes) instead of two-point t60s; same for centroid trajectories (already there) and envelopes.

### Criteria — codified into the protocol

- **Held-out discipline** (already practiced) becomes a rule: tuned AND held-out both reported; a held-out regression is acceptable only with a structural-axis justification (the jazz-kick +3% precedent).
- **Ceiling detector**: if the metric optimum sits at a clip/limit boundary, waveform inspection is mandatory before accepting (the "rides the clip ceiling" lesson, now a rule).
- **Crest envelope conformance** joins tuning/loudness as a standing gate for pluck/strike families.
- **Cross-family drift tripwire**: after every merge, K-weighted distance of every family's standard render against its last-accepted render; > threshold → investigate before push (catches collateral damage the per-family loops can't see).

### Prompt / process

- **Family brief templates** in `skills/match-reference/references/` with slots (verdict verbatim, region fence, refs + known artifacts, budgets, standing gates) — tonight's briefs were hand-written each time; templating stops drift and guarantees the guards (band-limits, both sample rates, measurement-before-modification baseline table) are never dropped.
- **Standardized audition sets**: fixed per-family note/velocity/phrase lists with fixed filenames every round, plus an auto-generated local A/B HTML page (previous vs new, blind toggle). The human gate is the release gate — make his listening fast and diffable.
- **Merge checklist codified** (tonight's conflict-marker incident): full marker sweep before staging, rebuild wasm from merged source, full LUFS + demo/E2E/MIDI gates, decision-log entry. Lives in loop-protocol.
- **Shared-machinery ownership**: PluckVoice/AcPluck serving guitars AND bass is the last shared-struct hazard; if the bass agent's flags justify it, split bass onto its own struct (the ElectricVoice precedent).

### What I am NOT changing

- Persona panel cadence (works; PR-gated), pyloudnorm bake (proven), worktree parallelism + sequential merges (five agents tonight, no losses), papers-only clean-room rule, human ear as the only release gate.

## Phasing

- **Now (main loop, while agents run):** compare.py artifact gates + multi-res STFT + onset alignment; corpus MANIFEST + masking; audition-set standardization + A/B page.
- **After r4 merges:** room/early-reflection stage design doc + implementation; brief templates; drift tripwire script.
- **Then:** embedding-FAD second opinion; psychoacoustic descriptor axes; piano P2; articulation layer design doc.
