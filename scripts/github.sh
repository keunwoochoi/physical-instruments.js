#!/usr/bin/env bash
# Repository-owned GitHub CLI entrypoint. It does not trust gh's mutable global active account.
set -euo pipefail

readonly EXPECTED_GITHUB_ACTOR="keunwoochoi"
readonly GH_HOST="github.com"
gh_bin="${GH_BIN:-gh}"

if ! token="$(env -u GH_TOKEN -u GITHUB_TOKEN "$gh_bin" auth token --hostname "$GH_HOST" --user "$EXPECTED_GITHUB_ACTOR")"; then
  printf 'github identity: no stored credential for %s on %s\n' "$EXPECTED_GITHUB_ACTOR" "$GH_HOST" >&2
  exit 1
fi

if [[ -z "$token" ]]; then
  printf 'github identity: stored credential for %s is empty\n' "$EXPECTED_GITHUB_ACTOR" >&2
  exit 1
fi

if ! actual_actor="$(GH_TOKEN="$token" "$gh_bin" api user --jq .login)"; then
  printf 'github identity: could not verify the selected %s credential\n' "$EXPECTED_GITHUB_ACTOR" >&2
  exit 1
fi

if [[ "$actual_actor" != "$EXPECTED_GITHUB_ACTOR" ]]; then
  printf 'github identity: selected credential resolved to %s, expected %s\n' "$actual_actor" "$EXPECTED_GITHUB_ACTOR" >&2
  exit 1
fi

export GH_TOKEN="$token"
unset GITHUB_TOKEN
exec "$gh_bin" "$@"
