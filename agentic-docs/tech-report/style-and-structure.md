# House style + structure — DRAFT (to agree)

Depends on the audience decision (`audience.md`); written assuming **D / layered**.

## House style (proposed)
Inherited from the v03 report, softened for reach (see `author-and-voice.md`):

- **Numbers with comparators and units, always.** "~82 KB gz (66.7 KB wasm + …), vs. a 150 KB
  budget" not "tiny." "decay ratio 1.57× → 3.10×; real piano 2–4×" not "better decay."
- **Limitations are sections, not footnotes.** Both threads get an honest limits pass.
- **Report what got worse and what didn't reproduce.** The false-verified audio moment and the
  "fix that made it worse" are *kept*, because they are the most instructive lines (v03 ethos).
- **Measured voice; earn every superlative.** "lightweight" is fine because there's a number
  behind it; "blazing" is not.
- **Intuition before formalism** (Olah). A picture/sound of the idea before the equation.
- **Show, then tell.** The reader should *hear* the claim before reading the metric.
- **Sentences may be long if they carry their qualifications; paragraphs stay single-idea.**
- **Sections:** short abstract → hook → body → discussion → limitations → conclusion →
  appendices (repro, eval detail). Abstract and conclusion must each stand alone.
- **Self-aware asides, sparingly.** One or two land; more is a tic.

## Interactive / figure inventory (the Distill layer — we already have the engine live)
Ranked by payoff:
0. **One-octave clickable instruments** (owner idea, 2026-07-23). Tiny inline widgets: a single
   octave, one fixed instrument, click-only (no keyboard shortcuts). Drop them *throughout* the
   prose as "hear this" figures — the reader plays the actual engine, not a video. Better than
   video: live, honest, tiny. Also use them to embed **earlier product versions** so the reader
   *plays the evolution* (the simpler early demo → the current one) rather than watching a clip.
1. **Play it inline.** The actual demo (or a trimmed embed) at the top — hear before read.
2. **Samples vs. physical model, A/B.** Two audio buttons + spectrograms; the size argument made audible.
3. **"Same MIDI, 3 drum kits" switch.** Standard/Rock/Jazz on one bar — the genre-kit point.
4. **Decay-ratio / two-stage decay slider** on a struck string — the piano-realism metric, playable.
5. **Karplus–Strong / waveguide animation** — the tiny-model intuition, drawn and hearable.
6. **The real-time budget meter** — 2.67 ms/128-frame, voices vs. CPU, live.
7. **Bundle-size breakdown** — 82 KB as a stacked bar (wasm/core/worklet), vs. a sample library's MB.
8. **A "process transcript" explorable** — real excerpts from the build session (friction ledger),
   possibly annotated. Thread #2 made legible.
- Every interactive needs an **accessible static fallback** (a11y reviewer) and **honest axes**
  (data-viz reviewer). Motion respects `prefers-reduced-motion`.

## Outline — AGREED SHAPE (2026-07-23): two acts, braided
Working title options (pick later): *"An instrument library the size of a photo"* ·
*"29 instruments in 82 KB, and the agent that built them"* · *"Physical modeling, vibe-coded."*

**Abstract** — the artifact (what/size/how-good, with numbers) *and* the process (built by
directing an agent; what that was honestly like), in one paragraph. Both stand alone.

### ACT I — the artifact, as teaser (accessible spine, expandable rigor)
1. **Hook — play it.** Embedded demo. The surprising claim, made *sensory*: 29 instruments,
   ~82 KB gz, in a browser — hear it first.
2. **Why tiny is possible.** Physical models generate sound from equations, not stored audio →
   intuition (Karplus–Strong / waveguide) + the sound + the size number. (Interactives 5, 7.)
3. **Does it sound good?** — *the rigor spine, non-negotiable (Condition 1).* Honest ear + eval:
   A/B vs. references, per-instrument, the AB/ABX/MUSHRA gates, and where it *doesn't* hold up.
   (Interactives 2, 3, 4.)
4. **Seeds for Act II** (planted, not resolved): "it almost made *no sound* on iPhone"; "the
   audio thread is sacred — the reason is a story we'll get to."

### HINGE — the reveal
"None of this was written line by line. It was built over a couple of days by directing an AI
agent. Here is what that was actually like." (One or two paragraphs; the pivot the whole piece
turns on.)

### ACT II — the process, as payload (carries the deep tech, Condition 2)
5. **The collaboration model + division of labor.** Taste/ear/direction (human) vs.
   implementation/plumbing/diagnosis (agent); the boundary negotiated live.
6. **The making-of as the vehicle for depth:**
   - the **audio saga** → delivers the Web Audio / AudioWorklet / **iOS cold-clock** depth
     (pays off seed #4a);
   - the **CI cascade** → delivers the **WASM / reproducibility / real-time-budget** depth
     (pays off seed #4b, "audio thread is sacred");
   - the **worktree mirage**, the **taste corrections**, the **false-verified** moment.
7. **Expectations & knowledge.** What the prompter correctly expected vs. didn't; what he needed
   to know vs. didn't; **where the agent was wrong** and how it was caught; satisfaction and
   dissatisfaction, verbatim.

### CLOSE
8. **Discussion.** (a) small / physical-modeling audio on the web; (b) building real,
   taste-driven artifacts with agents. Both threads.
9. **Limitations.** Artifact (eval breadth; SoA/SIMD unrealized; instrument gaps) *and* process
   (n=1, one prompter; unfalsifiable bits; the parts we couldn't verify, e.g. real-iOS).
10. **Conclusion.** Both threads, standing alone.
- **Appendix:** reproducibility, eval protocol, the commit-message-as-research-record
  discipline, licensing hygiene (MIT/AGPL clean-room, CC0 demo assets).

## Open style questions for the owner
> **2026-07-28: `writing-samples.md` now answers all three from the owner's published corpus.**
> Recommendations there (pending owner confirmation): **"I"**, quotes **kept raw**, and a
> **tighter essay** than Distill-scale. It also flags that the "self-aware asides, sparingly"
> rule below is wrong — in the real corpus they land about once per section.

- First person **"I"** (personal blog) or **"we"** (the human+agent pair, which is itself a
  statement)? The pronoun is a thesis choice given thread #2.
- How raw are the verbatim process quotes? (They include frustration — "cloudflare is fucked up,"
  "it is even more broken now." Kept as-is, lightly cleaned, or paraphrased?)
- Length target: a long-read (Distill-scale) or a tighter essay with deep appendices?
