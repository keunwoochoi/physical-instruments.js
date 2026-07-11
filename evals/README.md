# Evals — eval before trust

Numbers, not intuition (PRINCIPLES.md). Stands up with issue #9. Operational workflow: `skills/run-evals/SKILL.md`.

| Dir | Owns |
|---|---|
| `corpus/` | Fixed MIDI corpus: solo pieces per instrument, **full multi-track arrangements** (customer-zero excerpts; MAESTRO excerpts for piano), torture cases (fast repeats, extreme velocities, dense polyphony, long tails) |
| `incumbents/` | Scripts rendering the corpus through Tone.js synths, smplr, spessasynth+GM — the committed reference WAVs are the bar to beat and permanent regression material |
| `listening/` | Blind AB/ABX web app (iteration) and MUSHRA protocol (release gates; MAESTRO Disklavier as hidden reference for piano) |
| `metrics/` | Spectral regression tripwires (multi-scale STFT vs last accepted render) + committed `dsp-bench` baselines. Regression signal ONLY — never a quality claim |

Human gates per roadmap: AB n≥5 (Q1) → ABX vs smplr (Q2) → MUSHRA "Good" band (Q3) → MUSHRA vs MAESTRO for piano (Q4). Results published at v0.5.
