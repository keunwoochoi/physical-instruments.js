# Shared engine extraction (`@instrumentsjs/engine`)

Date: 2026-08-02
Status: accepted — option **(a)** keep the siblings separate. No shared-engine extraction, no monorepo. Owner call quoted in the Decision record.

## Motivation

1. The instrument-agnostic plumbing — worklet host, WASM handshake, voice/track management, event queue, offline render, SMF/GM mapping, audit scripts, persona panel shape, CI templates — was **copied** from this repo into `subtractive-synthesizers.js` (and will be copied again for FM). The subtractive architecture doc (2026-07-28) measured ~1,900 LOC transferable as-is and ~5,500 with edits, chose "copy now, extract later", and opened a day-one tracking issue conditioned on this repo shipping 0.1 with a stable API.
2. This repo has shipped **v0.1 / v0.1.1**. The revisit condition is met.
3. Drift is already real: harness fail-correctly fixtures (#100), commit-msg enforcement (#101), instrument-id generation (#98), and identity wrappers landed on different schedules across the siblings. Every bug fixed twice is not theoretical.

## Thesis

The family should converge on **one published engine package** that owns the audio-thread host and the TS control plane, with each instrument family shipping only its DSP kernels, presets, and product façade. The extraction is a **release-train and packaging** decision more than a DSP one: the sound stays in per-family WASM modules; the shared package is the host.

## Evidence base

| Claim | Source | Confidence |
|---|---|---|
| Transferable surface ~1.9k LOC as-is / ~5.5k with edits | subtractive `agentic-docs/design/2026-07-28-architecture.md` § shared-plumbing | High (sibling measurement at copy time) |
| Copy was deliberate under a freeze | same doc; licensing ledger provenance + source SHA | High |
| Revisit after physical 0.1 | same doc | High — 0.1 shipped |
| Drift already visible | this session's #98–#101 ports of sibling harness pieces | High |
| Both packages on npm as separate products is desirable | PRINCIPLES product framing; separate READMEs/playgrounds | Medium (owner taste) |
| End-user installing both would download two engines under (a) | architecture doc cost statement | High |

Unverified: exact current LOC delta between the two copies after post-0.1 edits; exact npm download co-install rate (likely near zero today).

## Options

| | Approach | What it buys | What it costs | Release-train impact |
|---|---|---|---|---|
| **(a)** | Keep copying | Zero coupling; each family ships on its own clock | Drift; double fixes; dual download if a page loads both | None — status quo |
| **(b)** | Extract `@instrumentsjs/engine` (or `@physical-instruments/engine` renamed later); both depend on it | One host, one worklet handshake, one offline-render path; clean sibling diffs | Versioning discipline; a breaking host change bumps every family; extraction PR touches this repo | New package publish train; families pin a semver range |
| **(c)** | Monorepo of all families | No publish matrix between host and kernels | Couples release trains; contradicts the sibling-directory product shape the owner chose | Single version; single CI; largest process change |

### What would live in `@instrumentsjs/engine` (if b)

- TS: AudioWorklet load/handshake, track/voice manager, event queue, `renderOffline`, public `NoteEvent` / `Track` / `Engine` types that are family-agnostic
- Worklet glue that calls a **family-provided** WASM ABI (`ij_*` or a versioned successor)
- Shared audit/harness pieces that are truly family-agnostic (identity wrapper pattern, commit-msg hook, fail-correctly fixture runner) — optional; may stay per-repo

### What would **not** live there

- Any instrument kernel, preset, GM program map content, playground, or eval corpus
- Family-specific quality matrix aspects

### ABI boundary (the hard part of b)

Today the WASM export surface is this repo's `ij_*` C ABI. Subtractive copied it. Extraction requires:

1. Freeze a **versioned** ABI document (functions, struct layouts, event opcodes).
2. Each family WASM implements that ABI.
3. The engine package loads **one WASM module URL/bytes per Engine instance** supplied by the family package.

If the ABIs have already diverged, (b) starts with an alignment PR in each repo before the extract.

## Recommendation

**(b), staged — not a big-bang move.**

1. **Phase 0 (this issue):** accept or amend this doc. No code.
2. **Phase 1 — inventory PR (physical only):** generate a mechanical diff of `packages/core` + worklet + offline render vs the subtractive copy at recorded SHAs; list ABI symbols both sides export; open follow-up issues for any divergence. Gate: a checked-in inventory markdown under `agentic-docs/` (or a script output), not a rewrite.
3. **Phase 2 — extract package in-tree:** create `packages/engine` inside *this* repo first, move host code behind the existing public API so `physical-instruments.js` re-exports unchanged. Gate: transport baseline byte-identical; playground/e2e green; no npm major bump required if the public export path is stable.
4. **Phase 3 — publish `@instrumentsjs/engine@0.1.0`:** only with explicit owner publish authority. Gate: packaging matrix (bundlers) green; PACKAGING.md updated.
5. **Phase 4 — subtractive (and later FM) depend on it:** delete their copies; pin semver. Gate: sibling CI green; licensing ledger updated with the extract provenance.

**Why not (a):** the revisit trigger fired; continuing to copy now is a decision to pay drift forever, not a temporary release hedge.

**Why not (c):** the owner already chose sibling directories and separate product identities. A monorepo can be revisited if publish overhead dominates, but it is the wrong default while each family still has independent taste/eval loops.

**Why staged (b) rather than immediate multi-repo extract:** this repo's public API is the stability surface users already depend on; in-tree extraction (phase 2) proves the boundary without a forced coordinated release.

## Phased plan (PR-sized, after acceptance)

| Phase | PR | Gate |
|---|---|---|
| 0 | Design accepted (checkbox on #103) | Owner quote in this doc Status line |
| 1 | Inventory + ABI symbol list | Script or doc diff against subtractive SHA |
| 2 | In-tree `packages/engine` + re-export | `test:transport-baseline` identical; e2e green |
| 3 | npm publish engine | Owner lifts publish authority; bundler matrix |
| 4 | Subtractive consumes engine | Sibling CI; ledger entry |

## Deferred until demanded

- Shared eval/persona infrastructure packages
- Shared playground shell
- Forcing FM to wait on engine 1.0 (FM may still copy once if engine is not published yet — record SHA)
- Renaming npm scope if `@instrumentsjs` is unavailable (then `@physical-instruments/engine` or owner-chosen scope)

## Cost

- Design-only for this issue: zero runtime.
- Full (b) eventual: one more package in the dependency graph; smaller family packages; dual-install pages download one host + two WASMs instead of two hosts + two WASMs.

## Decision record

- Owner decision: **(a) keep them separate** — no shared `@instrumentsjs/engine` extraction, no monorepo. The siblings keep copying instrument-agnostic plumbing; revisit only if drift cost dominates.
- Date: 2026-08-02
- Verbatim quote: *"ok let's keep them separate. that is my decision."*
- If (b): N/A — decision is (a)
