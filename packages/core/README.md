# physical-instruments.js

**Physical-modeling instruments for the browser.** 29 instruments in 81 KB gzipped, no
samples to download, no CDN, works offline.

Physical modeling simulates the string, bar, or tube and computes the sound from the
simulation. Nothing is recorded, so velocity changes *timbre* rather than just level, and
the whole orchestra fits in less space than a single sampled piano note.

**[▶ Try it in your browser](https://keunwoochoi.github.io/physical-instruments.js/)**

## Install

```sh
npm install physical-instruments.js
```

```ts
import { createEngine } from "physical-instruments.js";

const engine = await createEngine();          // lazy AudioContext, gesture-safe
const piano  = engine.createTrack("piano");
piano.noteOn(60, 96);                          // velocity changes timbre, not just volume
```

No bundler configuration and no files to copy. Verified on every CI run against the
**published tarball** in Vite, webpack 5 and Next, and installed clean in both Chromium and
WebKit. The snippet above is executed by CI too, so it is the snippet that ran.

## Instruments

Piano and electric piano, acoustic/steel/electric/distorted guitar, bass, violin, viola,
cello, contrabass, strings, trumpet, trombone, brass, woodwind, voice, organ, marimba,
vibraphone, xylophone, glockenspiel, music box, mallet, percussion, three drum kits, and
synth pads — addressable by family name or General MIDI program.

## Doing more

```ts
// Multi-track: one shared engine, up to 16 tracks, each with gain and pan.
const strings = engine.createTrack("strings", { gain: 0.8, pan: -0.3 });

// Play a timeline and await the end of it.
await engine.play([
  { midiPitch: 60, startSeconds: 0, endSeconds: 1.2, velocity: 100, instrumentGroup: "piano" },
]);

// Deterministic offline bounce to WAV bytes — no realtime playback needed.
const wav = await engine.renderOffline(notes, { float32: true });

// Shared reverb voicing: "off" | "room" (default) | "hall" | "plate" | "spring"
engine.setReverb("hall");
```

`createEngine()` accepts `{ context }` to share your own `AudioContext` (e.g. with Tone.js),
`{ connect: false }` to keep the engine out of `destination` and route `engine.output`
yourself, and `{ workletUrl, wasmUrl }` if you host the assets somewhere unusual.

Imports are SSR-safe: nothing touches `window` or `AudioContext` at import time, so it is
safe to import from a Next server component and construct the engine on the client.

TypeScript types are first-class and shipped.

## Links

- [Repository, tech report, and full API](https://github.com/keunwoochoi/physical-instruments.js)
- [Changelog](https://github.com/keunwoochoi/physical-instruments.js/blob/main/CHANGELOG.md)
- Contributor notes on the API and packaging contracts live in
  [`PACKAGING.md`](https://github.com/keunwoochoi/physical-instruments.js/blob/main/packages/core/PACKAGING.md).

## Licence

MIT OR Apache-2.0, at your option.
