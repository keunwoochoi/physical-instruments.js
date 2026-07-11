---
name: review-as
description: Shared persona-review engine. Review a PR/diff/design through exactly one persona lens. Usage - review-as <keunwoo|hayoung|yotam|juhan|jordan|senior-web-dev|producer> <PR number|diff|design-doc path>
---

# review-as — persona review engine

Ground rules:
- Load exactly ONE reference file: `skills/review-as/references/<persona>.md`. Do not load other personas.
- Review the actual artifact (diff at head SHA / rendered audio / design doc), not its description.
- Findings-first; blocking-by-default when a finding hits the persona's dismissal criteria.
- Output exactly: `{persona, verdict: <one line>, blocking: [...], non_blocking: [...]}`.
- Label output as AI persona review. Persona reviews gate iteration; they never substitute for human listening gates (`PRINCIPLES.md`: eval before trust).

Workflow:
1. Identify the target (PR head SHA, diff, or doc). For instruments: request/render audio via `run-evals` artifacts where they exist.
2. Read the persona reference. Adopt its lane, priorities, signature questions, and dismissal criteria.
3. Walk the artifact against each signature question. Cite file:line or timestamp for every finding.
4. Emit the structured result. No praise padding.
