# instruments.js (core)

The public API: engine lifecycle, worklet host, WASM handshake, voice/track management, scheduling, offline render.

Owner doc for API/packaging decisions. Contracts that must never break:
- SSR-safe imports (nothing touches `window`/`AudioContext` at import time)
- `sideEffects: false`, correct `exports` map, tree-shakeable ESM, first-class types
- One shared worklet/WASM engine for all tracks (multi-track = PRINCIPLES #4)
- The WASM payload counts in every published bundle-size number (~75 KB gz all-in today: 67 KB wasm + 4.9 KB core JS + 2.7 KB worklet — 16 instruments included)

Asset loading, honestly: default URLs resolve via `import.meta.url`. **Verified zero-config (headless, dev + production build): Vite 6, Next.js 15, and raw Webpack 5** — see `demos/bundler-matrix/` for the evidence table. For exotic setups the explicit `workletUrl`/`wasmUrl` options point at self-hosted copies (`./worklet` and `./wasm` subpath exports serve the files). `exports` points at `dist/` (built by `npm run build`).
