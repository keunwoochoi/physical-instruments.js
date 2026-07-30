#!/usr/bin/env bash
# Run a `gh` command as the account that owns this repository — atomically.
#
# WHY A WRAPPER AND NOT "just switch accounts first"
# `gh`'s active account is a single machine-global setting in ~/.config/gh/hosts.yml.
# Any other process on the machine — a concurrent agent session working in a different
# repository, another terminal — can change it at any moment, and will, with no signal
# here. Observed on 2026-07-28: the account reverted between one shell invocation and
# the next, twice, while 12 agent processes were running. Switching once is therefore
# not a fix; the setting is contended, not merely wrong.
#
# This wrapper switches and acts inside a single process, so the window in which another
# session can interpose shrinks from "until someone notices" to milliseconds. It also
# re-verifies after switching, so a lost race fails loudly instead of publishing.
#
# Usage:  scripts/gh-owner.sh issue create --title ...
#         scripts/gh-owner.sh api repos/:owner/:repo/...
#
# ALWAYS use this for any `gh` command that WRITES. Reads are harmless but harmless is
# not the standard here: a read as the wrong account still leaks that the account has
# looked at this repository.
set -euo pipefail
cd "$(dirname "$0")/.."

REMOTE=$(git remote get-url origin)
OWNER=$(sed -E 's#^.*[:/]([^/]+)/[^/]+$#\1#' <<<"${REMOTE%.git}")

gh auth switch -u "$OWNER" >/dev/null 2>&1 || true

ACTUAL=$(gh api user --jq .login 2>/dev/null || true)
if [ "$ACTUAL" != "$OWNER" ]; then
  echo "gh-owner: refusing to run — active account is '${ACTUAL:-none}', repo owner is '$OWNER'." >&2
  echo "gh-owner: could not acquire the owning identity. Another process may be competing" >&2
  echo "          for the machine-global gh account. Nothing was executed." >&2
  exit 1
fi

exec gh "$@"
