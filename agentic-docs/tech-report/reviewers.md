# Virtual reviewer pool (≤20) — DRAFT

Two jobs at once: **audience proxies** (who is this for, where do we lose them) and
**adversarial reviewers** (a panel that tries to break the claims). Operationalizable via
the repo's `skills/review-as` + `skills/panel-review`.

**7 reused** from `agentic-docs/personas/` (link, don't duplicate) + **13 new** below.
Each: *catches* / *loves* / *hates*.

## Reused (existing persona files)
1. **Keunwoo Choi** — author/owner, the prompter thread #2 documents. → `[[personas/keunwoo]]`
   *Catches:* does the retelling match what actually happened; is the ear honest.
2. **Hayoung Lyou** — working NY jazz pianist (the "Windup" behind demo #1). → `[[personas/hayoung]]`
   *Catches:* musicality/usability. *Loves:* real feel. *Hates:* uncanny-valley fakeness.
3. **Juhan Nam** — Stanford **CCRMA** PhD, KAIST music-AI prof, ISMIR 2025 chair. → `[[personas/juhan]]`
   THE academic gatekeeper for the ISMIR/AES bar. *Catches:* novelty claims, eval validity,
   related-work gaps, DSP correctness. *Hates:* hype without measurement.
4. **Yotam Mann** — creator of Tone.js; browser music. → `[[personas/yotam]]`
   *Catches:* Web Audio / AudioWorklet correctness, the embeddable interactive layer, DX.
   *Loves:* it runs in a page. *Hates:* jank, non-portable audio.
5. **Jordan Rudess** — virtuoso keyboardist; GeoShred physical-modeling (Julius Smith
   waveguide); pro-AI-in-music. → `[[personas/jordan]]`
   *Catches:* expressivity, playability, physical-modeling authenticity. *Hates:* toys that
   don't respond to touch.
6. **Tech-savvy producer** — aliasing/artifact skeptic archetype. → `[[personas/producer]]`
   *Catches:* aliasing, clipping, HF nasties, "sounds like a cheap softsynth." *Hates:*
   digital artifacts, loudness tricks.
7. **Senior web dev** — packaging / bundle-size / DX archetype. → `[[personas/senior-web-dev]]`
   *Catches:* is the **82 KB** honest (WASM counted?), ESM/`exports`, SSR safety, "npm
   install and it just works." *Hates:* hidden runtime cost.

## New (define here; promote to `personas/` if we adopt them)
8. **General ML / LLM researcher** — reads thread #2 *as research*.
   *Catches:* overclaimed agent autonomy, unfalsifiable process claims, reproducibility.
   *Loves:* a candid, measurable building-with-agents account. *Hates:* "AI did it all."
9. **Agentic-coding practitioner + skeptic** — ships with agents daily; strong priors on
   where they break. **Primary reviewer for thread #2.**
   *Catches:* sanitized narratives, missing failure modes, the "false-verified" moment.
   *Loves:* the honest friction ledger. *Hates:* success theater.
10. **Vibe coder** — builds with AI, not a domain expert; a big slice of the audience.
    *Catches:* jargon walls, unexplained leaps. *Loves:* "I could do this too." *Hates:* gatekeeping.
11. **Real-time DSP / audio-plugin engineer** (JUCE/embedded) — "audio thread is sacred."
    *Catches:* alloc/locks on the sample path, denormals, the 2.67 ms/128-frame budget,
    SIMD/SoA reality vs. intent. *Hates:* hand-waved real-time safety.
12. **Rust / WASM systems engineer** — the crate → WASM side.
    *Catches:* WASM size/repro claims, alloc discipline, AoS-scalar-vs-SoA-SIMD honesty,
    byte-reproducibility. *Hates:* "blazing fast" with no flamegraph.
13. **General SWE** — broad product/backend engineer; the "would I share this Friday" reader.
    *Catches:* story clarity, whether the build arc lands. *Hates:* insider baseball with no payoff.
14. **Product designer (UX/interaction)** — the demo's interaction model.
    *Catches:* click-to-play discoverability, mobile touch targets, the transport metaphor,
    first-run clarity. *Hates:* mystery-meat UI, tiny tap targets.
15. **Graphic / visual designer** — type, color, layout, the *look*.
    *Catches:* typographic hierarchy, falling-note palette, dark/light, figure craft.
    *Hates:* default-bootstrap vibes, inconsistent spacing.
16. **Data-viz / explorable-explanations specialist** — the Distill layer.
    *Catches:* do figures *earn* interactivity or is it decoration; honest axes/baselines;
    accessible fallbacks. *Hates:* chartjunk, interaction for its own sake.
17. **Sound designer** (game/film/synth) — the ear for timbre as material.
    *Catches:* does each instrument have *one committed voice*, mix-readiness, kit distinctness.
    *Hates:* washed-out averages, GM-preset blandness.
18. **Technical writer / science communicator** — clarity+rigor referee.
    *Catches:* structure, unearned claims, missing definitions, whether abstract/conclusion
    do their jobs. *Loves:* intuition-before-formalism. *Hates:* buried lede, hype.
19. **Accessibility (a11y) specialist** — a web audio+visual demo has real stakes.
    *Catches:* keyboard nav, screen-reader story, contrast, `prefers-reduced-motion`, text
    alternatives for sound-carried meaning. *Hates:* mouse-only, sound-only meaning.
20. **Curious lay reader** — smart, non-technical; the widest circle.
    *Catches:* where it loses them, undefined terms, "why should I care." *Loves:* wonder +
    a clear payoff. *Hates:* feeling stupid.

## Bench (swap-ins if we want different coverage)
- **Music educator / pedagogy** — would they teach with it? (overlaps 10, 20)
- **PM / founder** — positioning, "what is this *for*." (overlaps 13, 18)
- **Show-HN skeptic** — will benchmark the 82 KB and poke the eval. (folded into 7, 12, 18)
- **OSS maintainer / licensing** — MIT/AGPL hygiene, CC0 assets, reproducibility. (folded into 7)

## How we'll use them
Not all 20 on every pass. Likely: **Juhan + producer + real-time-DSP + Rust/WASM** on the
artifact's technical claims; **agentic-skeptic + ML-researcher + tech-writer** on the process
thread; **product/graphic/data-viz/a11y** on the interactive; **vibe-coder + lay + general-SWE**
as "did we lose you" tripwires; **Hayoung + Jordan + sound-designer** as the ears.
