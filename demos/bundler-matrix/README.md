# Bundler matrix — worklet + WASM loading smoke tests

The adoption-critical promise: `npm install instruments.js` → sound with ZERO bundler
configuration. Each fixture is a minimal real app checked headlessly by
`scripts/dev/e2e-fixture.mjs <url>` (click Start → `#status` must read "engine live").

| Bundler | dev | production build | Verified | Notes |
|---|---|---|---|---|
| **Vite 6** | ✅ PASS | ✅ PASS | 2026-07-11, headless Chromium | `new URL(..., import.meta.url)` assets auto-copied (worklet + wasm hashed into dist/assets) — no config |
| Next.js | — | — | not yet | expected to need the documented `workletUrl`/`wasmUrl` escape hatch (self-hosted assets) |
| Webpack 5 | — | — | not yet | |

Until every cell is green, the SUPPORTED path everywhere remains the explicit
`workletUrl`/`wasmUrl` options with self-hosted copies (see `packages/core/README.md`
and `demos/customer-zero/`). Run a fixture:

```sh
cd vite && npm install && npx vite --port 8399   # then: node ../../../scripts/dev/e2e-fixture.mjs http://localhost:8399/
```
