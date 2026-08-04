<!-- PR title: type(scope): imperative summary -->

## Motivation
<!-- Why this change is needed now. -->

Closes #

## Impact
<!-- User, developer, sound-quality, performance, compatibility, or operational impact. Write "None" where appropriate. -->

## Release impact
<!-- Does this change what a user installs? The published surface is `packages/core/`, `crates/`, and the root README and licences; everything else ships with nothing.
       none   nothing a user could observe — harness, CI, docs, evals, playground, internal refactor
       patch  a fix, or behaviour a user notices, with no API change
       minor  new API, new instrument, new capability (below 1.0 this is also where breaking goes)
       major  breaking, once 1.0 has shipped
     Anything but `none` writes its entry under `## [Unreleased]` in CHANGELOG.md in THIS PR.
     This is a proposal about the next release; publishing stays behind the authority gate.
     Checked by `npm run check:release-impact`. -->

Release-Impact: <none|patch|minor|major> — <one line: what a user would notice>

## Summary
<!-- Small list of concrete changes; do not restate the diff line by line. -->

## Validation
<!-- What was driven for real at the exact current head (playground listen, offline render, dsp-bench numbers), not just green tests -->

## Evidence freshness
<!-- Exact current head SHA derived with `git rev-parse HEAD` and verified against the PR `headRefOid`; current CI/test/build/bench/panel/campaign/listening evidence; predecessor evidence explicitly labeled historical; skipped or unavailable gates stated as missing, never implied pass. -->

## Review focus
<!-- Files, assumptions, risks, listening timestamps, or evidence that deserve concentrated review. -->

## Gates
- [ ] CI green (incl. harness-audit)
- [ ] Release impact declared above; a non-`none` level carries its CHANGELOG `[Unreleased]` entry
- [ ] Worked in a linked worktree; it is deleted once this merges
- [ ] Every validation claim is current-head or explicitly labeled historical
- [ ] `dsp-bench` result attached (DSP changes)
- [ ] `panel-review` comment on current head SHA (instrument/API/packaging changes)
- [ ] Licensing ledger updated (ports)
- [ ] Source issue acceptance criteria reconciled
- [ ] Separable follow-ups linked as GitHub issues

## Follow-up after merge
<!-- "None" or bullets ending in issue links. Do not create local TODO/backlog entries. -->

## Agentic process trace
| Field | Value |
|---|---|
| Harness/model | |
| Skills invoked | |
| Abandoned/wasted routes | |
