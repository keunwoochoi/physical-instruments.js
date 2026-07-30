# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

This file records what changed for **users of the package**. The engineering record — what
was wrong, how it was found, what was measured, what was tried and abandoned — lives in
commit messages and pull requests, and is deliberately not duplicated here.

## [Unreleased]

Nothing yet.

## [0.1.1] — 2026-07-30

### Fixed

- **iOS/Safari: recover from an audio interruption.** The engine now resumes from WebKit's
  non-standard `"interrupted"` `AudioContext` state, not only from `"suspended"`. An iOS
  context reports `"interrupted"` at load and after any phone call, Siri invocation, route
  change or backgrounding, so the library's own resume path never fired on that platform.
  First touch worked anyway — WebKit resumes on the user gesture itself — but an
  interruption arriving mid-session left the engine silent with no way back.

  Found by a new mobile-WebKit check (iPhone emulation, touch, no autoplay escape hatch),
  which reported `state "interrupted", 44100 Hz` at load where every desktop browser
  reports `"suspended"`.

## [0.1.0] — 2026-07-30

First public release of `physical-instruments.js`.

### What it is

Physical-modeling instruments for the browser: the string, bar, or tube is simulated and
the sound is computed from the simulation, so there are no samples to download. One Rust/
WASM engine in a single AudioWorklet serves every track.

### Added

- **29 instruments** across piano, guitars, bowed strings, brass, woodwind, voice,
  mallets, organ, drum kits and synth pads, addressable by family name or General MIDI
  program.
- `createEngine()` — lazy `AudioContext`, gesture-safe, resolves its own worklet and WASM
  with no bundler configuration; `workletUrl` / `wasmUrl` remain available for exotic
  hosting.
- Multi-track mixing (16 tracks) with per-track gain and pan, and `setInstrument()` to
  re-point a track without leaking a slot.
- Timeline playback via `play()`, with sustain-pedal (CC64) support.
- `renderOffline()` — deterministic offline bounce to stereo WAV bytes, 16-bit PCM or
  float32.
- Shared reverb voicing: `off`, `room` (default), `hall`, `plate`, `spring`.
- `onStats()` for active-voice telemetry, and first-class TypeScript types throughout.
- SSR-safe imports: nothing touches `window` or `AudioContext` at import time.

### Verified for this release

- **Zero-config in Vite, webpack 5 and Next**, measured on the packed tarball rather than
  a workspace link — each bundler builds the library into a real app whose built bundle
  renders audio.
- **Install from the published tarball** renders audio in both Chromium and WebKit.
- The README quickstart is executed against the installed package on every CI run, so the
  snippet a reader copies is the snippet that ran.
- Size is enforced, not claimed: **82,476 B gzipped** (74,386 wasm + 5,401 core JS + 2,689
  worklet) is what `npm install physical-instruments.js` delivers, carrying all 29
  instruments. `scripts/audit/bundle-size-audit.sh` owns this number, prints it separately
  from the workspace total, and fails if the committed WASM drifts from what the Rust
  source builds.

### Not in this release

`@instrumentsjs/instruments` and `@instrumentsjs/midi` remain unpublished while the core
API settles. `@instrumentsjs/react` is deliberately deferred until demanded — a React app
consuming the core directly is the intended stress test of the framework-free story.

[Unreleased]: https://github.com/keunwoochoi/physical-instruments.js/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/keunwoochoi/physical-instruments.js/releases/tag/v0.1.1
[0.1.0]: https://github.com/keunwoochoi/physical-instruments.js/releases/tag/v0.1.0
