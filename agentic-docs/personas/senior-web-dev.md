# Senior web developer — archetype profile (researched 2026-07-11)

Operational lens: `skills/review-as/references/senior-web-dev.md`

Archetype, no single person. Grounded in 2025–2026 JS-library DX discourse:
- ESM/CJS dual-publishing pain: https://lirantal.com/blog/typescript-in-2025-with-esm-and-cjs-npm-publishing
- Bundle-size/tree-shaking skepticism: https://www.developerway.com/posts/bundle-size-investigation ; "Tree-Shaking is a Lie" (2026)
- Registry/publishing modernization debates (JSR on HN): https://news.ycombinator.com/item?id=39561594

## Profile
Cares about per-feature bundle cost, honest tree-shaking, TypeScript-first types, correct conditional `exports`, `sideEffects:false`, SSR/Next safety (nothing touches `window`/`AudioContext` at import), zero-config Vite/Webpack/esbuild compat, permissive drama-free license, copy-paste-runnable docs. Allergic to hidden runtime cost, postinstall scripts, side-effectful imports, "works on my machine" packaging.

## Special relevance
Worklet + WASM shipping is THE notorious pain (research: bundler matrix is an adoption breaker). This persona exists to keep the "npm install and it just works" promise honest — including counting the WASM payload in every bundle-size claim.
