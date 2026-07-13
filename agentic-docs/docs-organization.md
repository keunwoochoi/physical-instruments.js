# Documentation organization — one fact, one surface

| Surface | Owns |
|---|---|
| Code, types, docstrings | Executable behavior and local contracts |
| `AGENTS.md` (+ `CLAUDE.md` symlink) | Constitution, authority gates, routing |
| `PRINCIPLES.md` | Mission and durable values |
| `agentic-docs/design/` | Dated design docs — why we chose what we chose (kept forever; superseded docs become stubs pointing at git history) |
| `agentic-docs/reports/` | Dated analysis/post-mortem reports, indexed in its README |
| `agentic-docs/licensing.md` | Port provenance ledger + clean-room policy |
| `agentic-docs/personas/` | Full researched reviewer-persona profiles (evidence layer; the operational lenses live in `skills/review-as/references/`) |
| GitHub Issues and pull requests | All work with a done-state: backlog, live plans/status, ownership, blockers, acceptance criteria, review evidence, validation, and follow-ups |
| Nowhere durable | Transcripts, tool output, scratch notes |

Gates for adding a doc: it must have a single owner surface, a precise name, and a route or index entry.
Banned filename words: `general`, `misc`, `notes`, `utils`, `stuff`, `overview`.

Do not create local TODO, backlog, plan-status, or per-PR decision-log files. A design doc may preserve a durable decision and its rationale, but its implementation state lives only in linked GitHub Issues and pull requests.
