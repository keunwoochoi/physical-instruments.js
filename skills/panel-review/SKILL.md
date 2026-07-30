---
name: panel-review
description: Run the full 7-persona USER panel on the product — instruments, sound, UI, public API, and what a visitor lands on. Required before merging instrument, public-API, or user-visible packaging changes. Not for harness/CI/internal-docs changes. Usage - panel-review <PR number|diff|path>
---

# panel-review — the 7-lens gate

## What the panel reviews: THE PRODUCT, as a user meets it

**The seven personas are users of this JavaScript package, not contributors to it.** They
review the instruments, their sound, the demo/UI, the API's ergonomics, and everything a
person encounters when they install and use the library. They do **not** review the
codebase, the harness, CI, build scripts, or internal documentation.

Owner, 2026-07-30: *"the panel review is for the core instruments and its sound and the UI
and everything, but not for things like readme changes. The panel review was about the
product, not the documentation, not the code base. They were users, users of this
JavaScript package."*

| In lane | Out of lane |
|---|---|
| How an instrument sounds; touch, decay, coupling | Test coverage of a harness script |
| The demo page and playground UI | CI job layout, workflow YAML |
| Public API ergonomics — is this pleasant to call? | Internal design docs, commit conventions |
| What a visitor sees on the npm page or the README **as rendered** | Whether a check varies its fixtures enough |
| Install and first-sound experience | Lint rules, file organisation, script structure |

The test is not "does this diff touch a user-facing file" but **"would a user ever
encounter the result of this?"** A README change is in lane when the README is the page a
user lands on; it is out of lane when it is a contributor doc. A packaging change is in
lane through its effect — what installs, what the npm page shows, what the download
weighs — never through its scripts.

**Do not run the panel on a harness-only or internals-only change.** Mis-scoping it is not
free: it returns confident findings about the wrong artifact and buries the real ones. Run
it on the product, or on the user-visible consequences of a change, and give the personas
the rendered surface — the npm page, the demo, the audio — not the diff that produced it.

1. **Fan out**: spawn 7 parallel read-only subagents. Each runs `skills/review-as/SKILL.md` with exactly one of: keunwoo, hayoung, yotam, juhan, jordan, senior-web-dev, producer. Each returns `{persona, verdict, blocking[], non_blocking[]}`.
2. **Aggregate** (in the invoking context):
   - Dedupe findings by file/line (or timestamp for audio); keep highest severity; tag with every persona that raised it.
   - Build the verdict matrix: 7 rows, pass/block.
   - **Headline is always the producer's 10-second dismissal test result.**
   - Rank blocking findings; then non-blocking.
3. **Post** as one AI-labeled comment on the PR (or return inline for docs/diffs).
4. **Gate**: `finalize-pr` refuses instrument/API/packaging merges without a panel comment on the current head SHA. New commits invalidate the panel.

Persona verdicts gate iteration only — human MUSHRA/AB gates (release gates in the roadmap) are never substituted.
