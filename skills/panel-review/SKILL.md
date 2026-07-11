---
name: panel-review
description: Run the full 7-persona review panel on a PR, diff, or design doc. Required before merging any instrument, public-API, or packaging change. Usage - panel-review <PR number|diff|path>
---

# panel-review — the 7-lens gate

1. **Fan out**: spawn 7 parallel read-only subagents. Each runs `skills/review-as/SKILL.md` with exactly one of: keunwoo, hayoung, yotam, juhan, jordan, senior-web-dev, producer. Each returns `{persona, verdict, blocking[], non_blocking[]}`.
2. **Aggregate** (in the invoking context):
   - Dedupe findings by file/line (or timestamp for audio); keep highest severity; tag with every persona that raised it.
   - Build the verdict matrix: 7 rows, pass/block.
   - **Headline is always the producer's 10-second dismissal test result.**
   - Rank blocking findings; then non-blocking.
3. **Post** as one AI-labeled comment on the PR (or return inline for docs/diffs).
4. **Gate**: `finalize-pr` refuses instrument/API/packaging merges without a panel comment on the current head SHA. New commits invalidate the panel.

Persona verdicts gate iteration only — human MUSHRA/AB gates (release gates in the roadmap) are never substituted.
