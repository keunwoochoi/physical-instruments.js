# instruments.js (core)

The public API: engine lifecycle, worklet host, WASM handshake, voice/track management, scheduling, offline render.

Owner doc for API/packaging decisions. Contracts that must never break:
- SSR-safe imports (nothing touches `window`/`AudioContext` at import time)
- `sideEffects: false`, correct `exports` map, tree-shakeable ESM, first-class types
- One shared worklet/WASM engine for all tracks (multi-track = PRINCIPLES #4)
- Zero-config loading under Vite, Next, and Webpack (verified by `demos/bundler-matrix/`)
- The WASM payload counts in every published bundle-size number

Packaging (build output, publishing config) is finalized alongside issue #5; until then `exports` points at source for workspace-internal typechecking.
