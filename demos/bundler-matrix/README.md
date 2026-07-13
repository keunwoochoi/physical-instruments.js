# Bundler matrix — worklet + WASM loading smoke tests

The adoption-critical promise: `npm install instruments.js` → sound with ZERO bundler
configuration. Each fixture is a minimal real app checked headlessly by
`scripts/dev/e2e-fixture.mjs <url>` (click Start → `#status` must read "engine live").

| Bundler | dev | production build | Verified | Notes |
|---|---|---|---|---|
| **Vite 6** | ✅ PASS | ✅ PASS | 2026-07-11, headless Chromium | `new URL(..., import.meta.url)` assets auto-copied (worklet + wasm hashed into dist/assets) — no config |
| **Next.js 15** | ✅ PASS | ✅ PASS | 2026-07-11, headless Chromium | zero config; client-component import prerenders statically (SSR-safe verified); assets emitted by webpack build |
| **Webpack 5 (raw)** | ✅ PASS | ✅ PASS | 2026-07-11, headless Chromium | zero config; worklet + wasm emitted as asset modules automatically |

The explicit `workletUrl`/`wasmUrl` options remain available for exotic setups
(see `packages/core/README.md` and `demos/customer-zero/`). Run a fixture:

```sh
cd vite && npm install && npx vite --port 8399   # then: node ../../../scripts/dev/e2e-fixture.mjs http://localhost:8399/
```
