# Author & target voice

## Who — authoritative bio lives in `agentic-docs/personas/keunwoo.md` (don't duplicate)
Keunwoo Choi. Music/audio-AI → LLMs. PhD, QMUL C4DM ("Deep Neural Networks for Music
Tagging"). ETRI → Spotify → ByteDance AI Lab → Gaudio Lab → Genentech/Roche → Upstage
(multimodal LLMs) + adjunct professor, KAIST GSCT. Creator of **kapre**; contributor to
librosa / torchaudio / mirdata. Co-author of the ISMIR 2021 tutorial/book *Music
Classification: Beyond Supervised Learning, Towards Real-world Applications*. Runs
**Ortet.ai**. Singer-songwriter (keunwoo.OOO — *not load-bearing for this report*);
records/edits for jazz pianist **Hayoung Lyou** — which is why the demo's default track is
her "Windup."

Web-research deltas (2026-07-23; keunwoochoi.github.io, Google Scholar):
- General chair of **ISMIR 2025**.
- Frames current work as multimodal LLMs for science/therapeutics (the Ortet "Pocket"
  program). Public site links Projects / Blog / Talks.
- Public voice reads as *technically precise + candidly self-assessing*, "intellectual
  humility paired with confidence in execution."

> **Update 2026-07-28 — the writing samples now exist.** The line above ("no public writing
> samples fetched") is retired. `writing-samples.md` analyses the actual corpus the owner named:
> the **beetbox-eval** self-published tech report, **22 blog posts**, and the **Solar Open 1/2**
> tech reports. Read it before drafting — it **overrides** parts of §"Target voice" below
> (the register is lighter and more first-person than v03; "I" not "we"; jokes are load-bearing,
> not sparing) and settles two of the open style questions in `style-and-structure.md`.

**Why the author matters for this piece.** He sits exactly at music-AI × working-musician ×
current agentic-LLM practice. The report is credible *because* it's this person building a
physical-modeling instrument by directing an agent — DSP taste he can hear, shipped by a
process he is now reporting on. Don't posture expertise the artifact hasn't earned; do lean
on the ear.

## Voice reference 1 — rigor backbone: the Ortet **v03** report
`/Users/keunwoo/OrtetCodes/one/ortet_bio_detector/reports/v03.md`. Hallmarks to inherit:

- **Every claim carries a number with a comparator and units.** Never "improved the decay" —
  "two-stage decay ratio 1.57× → 3.10×; a real piano is 2–4×; our electric guitar measured 2.16×."
- **Limitations are first-class sections framed as open questions**, not footnotes. (v03 §9
  lists six, each an "open question rather than a footnote.")
- **Honest hedging as a refrain.** "agreement-with-annotator, not accuracy against human
  truth"; "a single-run observation … that cannot be leaned on"; "we do not invent one."
- **Reports what didn't reproduce and what got worse.** Failed attempts and broken
  measurements are kept, not hidden — "the most valuable lines in the repository."
- Measured **"we"**; zero hype. Long sentences that carry their qualifications. Tables with
  careful captions. Inline resource links. Issue numbers for future work.
- **Self-aware asides**, used sparingly, land hard (v03: "this report is written by a
  Claude-family model that is nonetheless barred from labeling the data it describes").

## Voice reference 2 — accessible & interactive: Distill + Chris Olah
- **Distill.pub** (the web journal Olah co-founded) and **colah.github.io**: explorable
  explanations; figures that *are* the argument; hover / scrub / drag interactions; clarity
  as a first principle; generous intuition **before** formalism.
- For us the payoff is unusual: **the instrument is already built and live**, so the article
  can *embed playable pieces of it* — the demo itself, A/B spectrograms, a decay-ratio
  slider, an animated Karplus-Strong string, a "same MIDI, 3 drum kits" switch. The figures
  can make sound.

## Target voice for THIS report
**v03 rigor + Olah accessibility + the author's candid public register, at the NIME/AES/
ISMIR award bar.** Fun and exciting *without sacrificing a single honest number.* Crucially,
**the process thread gets the same honesty as the artifact thread** — including the parts
that annoyed the prompter and the places the agent was wrong (e.g. a false "verified" on
audio; see `agentic-process-notes.md`). Honesty is the through-line that makes both threads
worth a serious reader's time.
