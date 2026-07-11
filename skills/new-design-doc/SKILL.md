---
name: new-design-doc
description: Scaffold a dated design doc for any feature bigger than one PR, from the template. Usage - new-design-doc <topic-slug>
---

# new-design-doc

1. Copy `agentic-docs/design/TEMPLATE.md` → `agentic-docs/design/YYYY-MM-DD-<topic-slug>.md` (today's real date).
2. Fill every section. The Evidence base must cite primary sources and flag unverified claims. The Status line must state what the doc does NOT authorize.
3. Phased plan = PR-sized phases with measurable gates. Add a Deferred-Until-Demanded list.
4. Get the doc panel-reviewed (or human-reviewed) BEFORE implementation starts (`AGENTS.md` constitution #3).
5. Materialize the phased plan as GitHub issues (root tracker + dependency-linked slices). Live status lives in the issues; the doc records the decision.
