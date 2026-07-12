# instruments.js

**Beautiful physical-modeled instruments for the browser.**
`npm install` → piano, guitar, marimba in tens of kilobytes. No samples. Works offline. One `noteOn()` call. Full multi-track arrangements, one engine.

> **Status: pre-alpha, but playable.** Nothing on npm yet — but the engine is real:
> **13 instruments** — acoustic piano (hammer-collision waveguide), nylon/steel/electric-clean/
> electric-distorted guitars (with body resonance and an ADAA amp stage), bass, e-piano,
> marimba, vibraphone, glockenspiel, music box, synth pad, GM drum kit — in a **24 KB gzipped**
> WASM core (~31 KB gz all-in), loudness-matched to ±0.1 LUFS across families, playing
> multi-track arrangements inside one AudioWorklet. Zero-config under Vite (dev + build,
> headlessly verified); drop a .mid on the playground or plug in a MIDI keyboard.
>
> ```sh
> scripts/dev/serve.sh        # → http://localhost:8173/apps/playground/
> ```

```ts
import { createEngine } from "instruments.js";
import { parseMidi } from "@instrumentsjs/midi";

const engine = await createEngine();               // lazy AudioContext, gesture-safe
const piano = engine.createTrack("piano");         // hammer-collision waveguide piano
piano.noteOn(60, 96);                              // velocity changes timbre, not just volume
piano.pedal(true);                                 // sustain (CC64) — note-offs defer to pedal-up

const song = parseMidi(await file.arrayBuffer());  // any Standard MIDI File
await engine.play(song.notes, {
  pedals: song.pedals,
  tracks: { drums: { gain: 0.6, pan: 0.1 } },      // per-family mix
});

const wav = await engine.renderOffline(song.notes, // deterministic bounce
  { float32: true, onProgress: (f) => console.log(f) });
```

## Why

Everything that sounds good in the browser today is samples (megabytes, CDNs, licenses); everything that is small is a toolchain or sounds like a toy. Physical modeling — digital waveguides, modal synthesis, commuted synthesis — gives velocity-dependent timbre, sympathetic resonance, and expressive control from kilobytes of code. The DSP is proven (STK, Mutable Instruments, Faust); what never existed is the packaging: a permissively-licensed, tone-curated, `noteOn()`-simple library for ordinary web developers. That is this project.

Read `PRINCIPLES.md` for the constitution and `agentic-docs/design/2026-07-11-architecture.md` for the architecture.

## Development

```sh
rustup update && rustup target add wasm32-unknown-unknown
npm install
git config core.hooksPath .githooks   # local quality gate
cargo check --workspace && npm run typecheck --workspaces --if-present
scripts/audit/harness-audit.sh
```

Agent-driven development: start at `AGENTS.md`.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
Third-party porting policy and provenance: `agentic-docs/licensing.md`.
