# skills/ — the canonical workflows of this repository

These are the repository's skills: tool-neutral, checked-in procedures that encode HOW the work
is done here, so the discipline survives across sessions and agents. Each is a `SKILL.md` under
`skills/<name>/`, exposed to agents through the `.claude/skills` and `.agents/skills` symlinks,
routed from `AGENTS.md`, and invoked as `/<name>`.

They are not suggestions. The instrument-quality skills are enshrined in `PRINCIPLES.md`
("Every instrument is held to the quality matrix") and gated in `skills/finalize-pr`.

## Instrument quality — the core discipline
Instrument quality is six measurable aspects, each its own skill, run in DEPENDENCY ORDER
(earlier aspects gate later ones — a clipping or NaN-ing voice corrupts every timbre and
dynamics measurement; a mis-slotted note fabricates brightness):

| order | skill | the direction it owns |
|---|---|---|
| 1 | `audit-stability` | never NaN / denormal; self-oscillators ignite and hold |
| 2 | `audit-headroom` | nothing clips (SAMPLE peak, not RMS); level-matched |
| 3 | `audit-tune` | plays the note it was asked, everywhere (octave-safe autocorrelation) |
| 4 | `audit-envelope` | attack + decay/release shape |
| 5 | `audit-dynamics` | velocity changes loudness AND timbre |
| 6 | `audit-voice` | fundamental-led, ONE committed voice — **pick one, don't average** |

`instrument-quality-matrix` runs all six across an instrument (or the whole library) and tracks
the scorecard. `match-reference` is the research loop that iterates a model toward real
recordings; `audit-voice` is where the "pick one target tone and commit" method lives.

Each skill encodes the meta-rules learned the hard way and shared across all of them:
measure CLEAN (not through the master limiter); measure BEFORE AND AFTER; turn a feature OFF
before claiming it did something; autocorrelation is octave-safe, spectral-peak is not; the
band/metric is a proxy for ITERATION, human listening gates the RELEASE.

## Process
- `finalize-pr` — bring a PR to merge-ready (issue linkage, gates, evidence freshness). The
  instrument-quality aspects are part of its gate.
- `panel-review` — persona listening review, required before an instrument/API/packaging merge.
- `wrap-session` — reconcile a progressive/stacked session (published stacks, evidence, resume).
- `new-design-doc` — start anything bigger than one PR as a dated design doc.

## Research & safety
- `run-evals` — the corpus, listening tests, and regression tripwires.
- `port-audit` — port third-party (MIT/BSD only) code with a licence-ledger entry.
- `dsp-bench` — the 2.67 ms / 128-frame budget on a full arrangement.
- `harness-audit` — the repo's own consistency checks (skills, links, forms).
- `review-as` — review through a named persona's ears.
