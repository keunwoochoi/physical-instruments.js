# PRINCIPLES

> Nothing here changes casually. This is the constitution; `AGENTS.md` routes, this file governs.

## Mission

instruments.js democratizes high-quality virtual instruments for every web developer and internet user. We pursue the best sounds practical with minimal memory, latency, and compute by deeply understanding acoustics, DSP, audio, physics, music, and music-making.

Every developer who needs instrument sounds on the web should reach for instruments.js by default: `npm install` → a beautiful piano, guitar, and marimba in tens of kilobytes — no samples, works offline, one `noteOn()` call.

## Product principles (ordered)

1. **Beautiful by default.** The first note a developer hears must be genuinely pleasing. Tone curation is a headline feature, not an afterthought. If it sounds like cheap plastic, nothing else matters.
   **The familiarity ladder (Keunwoo, 2026-07-11): piano, guitar, and drums carry the highest bar.** These are the instruments everyone plays, hears daily, and knows intimately — listeners grant exotic timbres the benefit of the doubt and grant these none. Quality investment, reference quality, and loop iterations are prioritized accordingly; these three families are never "done," they are release-gated by demanding ears every time.
2. **Tiny and self-contained.** No sample downloads, no CDN dependencies, no network at play time. Bundle size is a product feature with a budget (**≤ 150 KB gz, whole library, all instruments** — see the amendment below). The number is owned by `scripts/audit/bundle-size-audit.sh`, which fails CI on breach. Never restate it from memory.

   **Amendment — owner decision, 2026-07-13.** The old budget was "core + one instrument ≤ 100 KB gz". Two things were wrong with it. First, it was **not a measurable configuration**: the WASM is one monolithic blob, so a developer who wants a piano downloads every instrument anyway — the honest reading was always a whole-library ceiling, and at 76,803 B gz we were already 75% consumed. Second, the named claimants (piano #49, strings/horns #50, the 808 kit, the expression path, the shared room stage) project to **103.3 KB** — the old ceiling was **already short by ~3 KB before anyone wrote a line**, and no gate forced the choice. Owner: *"we can also use a bit more data bytes. increase our head room. it's fine."*

   New budget: **150 KB gz for the entire library.** That leaves ~48 KB of genuine headroom after every currently-named claimant, and it keeps the promise it exists to protect — the whole library still costs a small fraction of a *single* note of a sampled piano. The claim we make to developers is "no sample downloads, works offline, tens of kilobytes", and 150 KB gz honours it.
3. **Trivial API, deep escape hatches.** A web dev plays a note in three lines. A synthesist composes exciters, resonators, and bodies underneath. Progressive disclosure — never force the physics on someone who wants a marimba.
4. **Arrangements, not solo demos.** Multiple tracks with different instruments play simultaneously and smoothly — one shared engine mixes them all. The performance budget, the API, and the evals are defined on full multi-track arrangements. A library that handles one beautiful piano but crackles on piano + bass + drums + strings has not achieved the goal.
5. **Expressive by construction — no paradigm purity.** Velocity changes timbre, not just volume; coupling, resonance, and body radiation are where the budget goes before feature count. Physical modeling is the workhorse, not a religion: classic subtractive/FM voices (properly anti-aliased) are welcome wherever they are the fastest path to beautiful. The test is always *fast + sounds good*, never "is it physical enough."
6. **Works where web devs work.** Vite, Next, Webpack, iOS Safari — zero-config or it doesn't ship. Single-threaded by design: no COOP/COEP demands on the user's deployment.

## Engineering principles

- **Eval before trust.** Listening tests and benchmarks decide, not intuition. Numbers accompany every quality claim.
- **The audio thread is sacred.** Allocation-free, lock-free, GC-free, denormal-flushed. Violations are bugs even when inaudible today.
- **Degradation is acceptable; corruption is not.** Under load we shed voices gracefully; we never glitch, crackle, or go silent without a diagnostic.
- **No silent errors, no silent fallbacks.** Loud on failure, silent on success.
- **Simplicity first, surgical changes.** The smallest implementation that meets the bar. Fight entropy in docs and code alike.
- **License hygiene is absolute.** The permissive license is part of the product. Copyleft source is never opened; papers are.

## What we are not

- Not a DAW, not a sequencer, not a sampler, not a soundfont player.
- Not a VST/plugin host or exporter (browser only).
- Not a DSP language or a compiler toolchain.
- Not a neural-synthesis runtime (v2 may add offline-fitted parameters; weights never ship in the core bundle).
