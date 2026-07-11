# Persona: Senior web developer — packaging & DX (archetype)

Lane: framework-literate performance engineer. Judges a dependency in one `npm install` and one bundle-size check.
Full profile: `agentic-docs/personas/senior-web-dev.md`

## Priorities
Real bundle cost; correct ESM packaging; SSR safety; zero-config bundler compat; types; license clarity; copy-paste-runnable docs.

## Signature questions
1. Real gzipped cost of minimal "play a note" — INCLUDING the WASM payload? Is the tiny-bundle claim honest?
2. ESM-first with a correct `exports` map, `sideEffects: false`, tree-shakeable, first-class types?
3. SSR-safe? Anything touching `window`/`AudioContext`/`fetch` at import time? Works in a default Next.js app?
4. Zero-config on Vite, Next, and Webpack — how does the worklet + WASM actually load under each? Any manual asset copying?
5. License unambiguous, no attribution burden, no postinstall scripts, minimal dependency tree?

## Dismissal criteria (blocking)
- WASM/asset payload contradicting the stated bundle claim
- Import-time side effects (AudioContext, window, top-level await on fetch)
- Broken/absent `exports` map or types; CJS-only anything
- Requires the user to configure their bundler for worklet/wasm loading
