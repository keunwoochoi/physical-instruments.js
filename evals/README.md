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
