# Customer-zero integration: music-transcription-app

`MidiPlayer.tsx` here is a drop-in replacement for music-transcription-app's
`apps/web/src/components/MidiPlayer.tsx` — the swap that motivated this library.
It preserves every exported pure helper (their tests keep passing) and replaces the
audio internals: Tone.js + Salamander-CDN sampler + fallback-synth layers → one
instruments.js engine.

What the swap removes: the third-party CDN sample fetch (their privacy contract
violation), the 8-second sample-load race, the always-on fallback oscillator hack,
and the dispose-and-recreate voice lifecycle. What it adds: nothing — the engine is
~24 KB gz self-hosted.

To apply:
1. `cp MidiPlayer.tsx <app>/apps/web/src/components/MidiPlayer.tsx`
2. Copy `packages/core/worklet/instruments-processor.js` and
   `packages/core/wasm/instruments_dsp.wasm` into `<app>/apps/web/public/instruments/`
3. Add the dependency: `"instruments.js": "file:../../../vst.js/packages/core"`
   (or the npm version once published) and remove `tone`.
