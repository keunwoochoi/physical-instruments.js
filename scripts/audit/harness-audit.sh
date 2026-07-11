#!/usr/bin/env bash
# Executable owner of harness invariants (AGENTS.md constitution #1).
# Run locally via .githooks/pre-commit and in CI. Repair procedure: skills/harness-audit/SKILL.md
set -u
cd "$(dirname "$0")/../.."
fail=0
err() { echo "AUDIT FAIL: $*" >&2; fail=1; }

# 1. CLAUDE.md must be a symlink to AGENTS.md
[ -L CLAUDE.md ] && [ "$(readlink CLAUDE.md)" = "AGENTS.md" ] || err "CLAUDE.md must be a symlink to AGENTS.md"

# 2. skills <-> commands: every skill has a thin forwarder, every forwarder has a skill
for d in skills/*/; do
  name=$(basename "$d")
  [ -f "$d/SKILL.md" ] || err "skills/$name missing SKILL.md"
  [ -f ".claude/commands/$name.md" ] || err "skills/$name has no .claude/commands forwarder"
done
for f in .claude/commands/*.md; do
  name=$(basename "$f" .md)
  [ -d "skills/$name" ] || err "orphan forwarder .claude/commands/$name.md (no skills/$name)"
  [ "$(wc -l < "$f")" -le 5 ] || err ".claude/commands/$name.md is not thin (>5 lines) — content belongs in skills/"
done

# 3. review-as must have all 7 persona lenses, each with a full profile
for p in keunwoo hayoung yotam juhan jordan senior-web-dev producer; do
  [ -f "skills/review-as/references/$p.md" ] || err "missing persona lens: $p"
  [ -f "agentic-docs/personas/$p.md" ] || err "missing persona full profile: $p"
done

# 4. every design doc declares a Status line
for f in agentic-docs/design/*.md; do
  case "$f" in *TEMPLATE.md) continue;; esac
  grep -q '^Status:' "$f" || err "$f missing 'Status:' line"
done

# 5. authority gates still declared off in AGENTS.md
grep -q 'npm publish / GitHub release: \*\*off\*\*' AGENTS.md || err "AGENTS.md publish authority gate text altered/missing"

# 6. licensing ledger + clean-room policy present
grep -q 'papers-only' agentic-docs/licensing.md || err "licensing.md clean-room policy missing"

# 7. files referenced by AGENTS.md routes exist
grep -oE '`(agentic-docs|skills|packages|demos|evals)/[A-Za-z0-9._/-]+`' AGENTS.md | tr -d '\`' | sort -u | while read -r p; do
  [ -e "$p" ] || echo "AUDIT FAIL: AGENTS.md references missing path: $p" >&2
done
# (subshell can't set fail; re-check)
grep -oE '`(agentic-docs|skills|packages|demos|evals)/[A-Za-z0-9._/-]+`' AGENTS.md | tr -d '\`' | sort -u | while read -r p; do [ -e "$p" ] || exit 42; done || fail=1

# 8. rolling TODO freshness (warn only)
todo=$(ls .claude/TODO-*.md 2>/dev/null | sort | tail -1)
if [ -n "$todo" ]; then
  age_days=$(( ( $(date +%s) - $(stat -f %m "$todo" 2>/dev/null || stat -c %Y "$todo") ) / 86400 ))
  [ "$age_days" -le 30 ] || echo "AUDIT WARN: $todo untouched for ${age_days}d — roll it (skills/harness-audit)" >&2
else
  err "no .claude/TODO-*.md rolling decision log"
fi

[ "$fail" -eq 0 ] && echo "harness-audit: OK" || exit 1
