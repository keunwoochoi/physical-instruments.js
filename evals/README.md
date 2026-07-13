# Evals — eval before trust

Numbers, not intuition (PRINCIPLES.md). Stands up with issue #9. Operational workflow: `skills/run-evals/SKILL.md`.

| Dir | Owns |
|---|---|
| `corpus/` | Fixed MIDI corpus: solo pieces per instrument, **full multi-track arrangements** (customer-zero excerpts; MAESTRO excerpts for piano), torture cases (fast repeats, extreme velocities, dense polyphony, long tails) |
| `incumbents/` | Scripts rendering the corpus through Tone.js synths, smplr, spessasynth+GM — the committed reference WAVs are the bar to beat and permanent regression material |
| `listening/` | Blind AB/ABX web app (iteration) and MUSHRA protocol (release gates; MAESTRO Disklavier as hidden reference for piano) |
| `metrics/` | Spectral regression tripwires (multi-scale STFT vs last accepted render) + committed `dsp-bench` baselines. Regression signal ONLY — never a quality claim |

Human gates per roadmap: AB n≥5 (Q1) → ABX vs smplr (Q2) → MUSHRA "Good" band (Q3) → MUSHRA vs MAESTRO for piano (Q4). Results published at v0.5.

## Reference campaigns

`evals/cases/` owns the declarative tune/held-out matrices. Reference audio remains local and license-governed by `agentic-docs/licensing.md`; stage it beneath a scratchpad root using the canonical relative paths declared by each family manifest.

```sh
python3 -m pip install -r scripts/dev/requirements-loop.txt
npm run loop:validate -- evals/cases/piano.json
npm run loop:run -- evals/cases/piano.json --reference-root /path/to/scratchpad --out /path/to/immutable-iteration --hypothesis "physical hypothesis" --changed-component "PianoVoice soundboard" --drift-baseline /path/to/accepted-auditions
npm run loop:pilot -- --reference-root /path/to/scratchpad --out /path/to/pilot --hypothesis "validate the full campaign path" --changed-component "none"
```

The runner refuses missing references, absent held-out cases, corpus-axis contradictions, dirty source by default, stale shipped WASM, incompatible metric versions, and non-empty iteration directories. It records source/WASM/manifest/reference/render digests and never generates an audition after a red trust gate or failed drift gate.

## Listening evidence

When a baseline-backed campaign reaches `candidate` or `listening_required`, the runner writes a sealed participant-facing `listening/` bundle inside the iteration instead of a label-revealing A/B page. The public manifest and media URLs contain only opaque condition IDs; the separately sealed `listening-analysis.json` outside that served directory owns roles, ABX answer keys, source/prepared-audio digests, gain provenance, and exact campaign identity. The runner requires identical case manifests, reference digests, render protocols, sample rates, channels, frame counts, and durations before pairing anything, then normalizes each condition to the declared BS.1770 target.

Serve only the public `listening/` directory so the private role map is not participant-accessible, keep playback volume fixed, complete every condition through to the end, and export the raw session JSON. The participant digest uses the bundled SHA-256 implementation and does not require a secure browser context, so a physical device may use the host's LAN HTTP address; expose only the public bundle and never the iteration root. If browser storage fails, save the recovery JSON exposed during the session; paste it into the same experiment page's recovery control after an interruption. Validate and analyze exports with the private analysis manifest that was sealed into the iteration root:

```sh
cd /path/to/iteration/listening && python3 -m http.server 8175 --bind 0.0.0.0
# On the listening device, open http://HOST-LAN-IP:8175/ and record the real browser/device in setup metadata.
npm run listening:validate -- /path/to/iteration/listening-analysis.json
npm run listening:analyze -- /path/to/iteration/listening-analysis.json /path/to/session-*.json --out /path/to/analysis.json --markdown /path/to/analysis.md
npm run audit:listening
```

The analyzer retains listener-level raw responses, declared exclusions, setup metadata, play counts, and uncertainty. It always emits `quality_verdict: null`; the human owner applies the listening gate.
