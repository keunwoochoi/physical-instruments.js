# Bundler matrix — worklet + WASM loading smoke tests

Populated in Q2 (roadmap gate for v0.1): minimal Vite, Next.js, and Webpack fixtures that each `npm install` the library and play a note with ZERO bundler configuration. CI-run.

This is the notorious adoption breaker for AudioWorklet+WASM libraries — the whole "npm install and it just works" promise lives or dies here. If any framework needs per-app hacks, we ship an official loader shim or scope claims honestly (approved plan, Q2 kill/pivot criteria).
