#!/usr/bin/env bash
# Answer one question -- "could this be released right now?" -- and answer it with
# evidence rather than opinion. It NEVER publishes, tags, or pushes anything.
#
# Release-standard area H was "absent. No version, no changelog, no tag." The mechanics
# are here; crossing the line is not. `npm publish` and GitHub releases sit behind an
# authority gate in CLAUDE.md that only a human lifts, so this script ends by PRINTING the
# commands rather than running them. That separation is the point: everything that can be
# checked automatically is, and the one step that needs a person still needs a person.
#
#     scripts/release/check-release-ready.sh          # fast checks only
#     scripts/release/check-release-ready.sh --full   # + browser and bundler gates
set -uo pipefail
cd "$(dirname "$0")/../.."

FULL=0
[[ "${1:-}" == "--full" ]] && FULL=1

fails=(); warns=()
ok()   { printf '  \033[32mok\033[0m    %s\n' "$1"; }
bad()  { printf '  \033[31mFAIL\033[0m  %s\n' "$1"; fails+=("$1"); }
warn() { printf '  \033[33mwarn\033[0m  %s\n' "$1"; warns+=("$1"); }

PKG=packages/core/package.json
VERSION=$(python3 -c "import json;print(json.load(open('$PKG'))['version'])")
NAME=$(python3 -c "import json;print(json.load(open('$PKG'))['name'])")

echo
echo "release readiness — $NAME $VERSION"
echo

# ---------------------------------------------------------------- version & changelog
echo "== version and changelog =="
if [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  ok "version $VERSION is a release version"
else
  # Not a failure: a draft version is the CORRECT state until a human decides to ship.
  # Saying "FAIL" here would train everyone to ignore a red line that means "as expected".
  warn "version $VERSION is a pre-release — set a final version before tagging"
fi

BASE=${VERSION%%-*}
if [[ ! -f CHANGELOG.md ]]; then
  bad "CHANGELOG.md is missing"
elif grep -q "^## \[$BASE\]" CHANGELOG.md; then
  ok "CHANGELOG.md has a section for $BASE"
else
  bad "CHANGELOG.md has no '## [$BASE]' section"
fi

if grep -q "^## \[Unreleased\]" CHANGELOG.md 2>/dev/null; then
  ok "CHANGELOG.md keeps an Unreleased section"
else
  warn "CHANGELOG.md has no Unreleased section to accumulate into"
fi

if git rev-parse -q --verify "refs/tags/v$BASE" >/dev/null; then
  warn "tag v$BASE already exists — this would be a re-release"
else
  ok "tag v$BASE does not exist yet"
fi

# ---------------------------------------------------------------------- tree and CI
echo
echo "== tree and CI =="
if [[ -z "$(git status --porcelain)" ]]; then
  ok "working tree is clean"
else
  # PRINCIPLES: evidence produced on a dirty tree is not evidence.
  bad "working tree is dirty — every number below would be unattributable to a SHA"
fi

BRANCH=$(git rev-parse --abbrev-ref HEAD)
[[ "$BRANCH" == "main" ]] && ok "on main" || warn "on '$BRANCH', not main"

CI_STATE=$(scripts/gh-owner.sh run list --limit 1 --json headSha,status,conclusion \
             --jq '.[0] | "\(.headSha) \(.status) \(.conclusion)"' 2>/dev/null || true)
if [[ "$CI_STATE" == "$(git rev-parse HEAD)"*"completed success" ]]; then
  ok "CI is green at HEAD ($(git rev-parse --short HEAD))"
else
  bad "CI is not green at HEAD — saw: ${CI_STATE:-no runs}"
fi

# ------------------------------------------------------------------------- identity
echo
echo "== publishing identity =="
if scripts/audit/check-identity.sh >/dev/null 2>&1; then
  ok "the active GitHub account owns this repo"
else
  # Not fatal for the CHECK -- pushes authenticate separately -- but it is fatal for the
  # RELEASE, which is a GitHub-side action. This is the failure that is unrecoverable
  # once it happens in public, so it is called out rather than folded into the summary.
  bad "wrong GitHub account active — a release would be published as the wrong person"
fi

# ------------------------------------------------------------------- the tarball itself
echo
echo "== what would actually be published =="
npm run --silent build >/dev/null 2>&1
node scripts/dev/stage-package-files.mjs >/dev/null 2>&1
LISTING=$(cd packages/core && npm pack --dry-run --json 2>/dev/null)
if [[ -z "$LISTING" ]]; then
  bad "npm pack --dry-run produced nothing"
else
  FILES=$(printf '%s' "$LISTING" | python3 -c "import json,sys;[print(f['path']) for f in json.load(sys.stdin)[0]['files']]")
  SIZE=$(printf '%s' "$LISTING" | python3 -c "import json,sys;print(json.load(sys.stdin)[0]['unpackedSize'])")
  COUNT=$(printf '%s\n' "$FILES" | grep -c . || true)
  printf '%s\n' "$FILES" | sed 's/^/        /'
  ok "$COUNT files, $(( SIZE / 1024 )) KB unpacked"

  for need in dist/index.js dist/index.d.ts wasm/instruments_dsp.wasm \
              worklet/instruments-processor.js README.md LICENSE-MIT LICENSE-APACHE; do
    grep -qx "$need" <<<"$FILES" || bad "the tarball is missing $need"
  done
  # Shipping the source tree or the harness would be a mistake nobody notices until it is
  # on the registry and cannot be unpublished.
  if grep -qE '^(src/|scripts/|crates/|apps/|agentic-docs/)' <<<"$FILES"; then
    bad "the tarball contains source or harness files"
  else
    ok "no source, harness or docs beyond the README"
  fi
fi

# ---------------------------------------------------------------------------- gates
echo
echo "== quality gates =="
run() {  # run <label> <command...>
  local label=$1; shift
  if "$@" >/dev/null 2>&1; then ok "$label"; else bad "$label"; fi
}
run "harness audit"            npm run --silent audit:harness
run "bundle size + WASM freshness" npm run --silent audit:bundle
run "every package carries its licences" node scripts/dev/stage-package-files.mjs --check

if (( FULL )); then
  run "install from the tarball"  node scripts/dev/install-check.mjs
  run "install on WebKit"         env BROWSER=webkit node scripts/dev/install-check.mjs
  run "README quickstart runs"    node scripts/verify/check-quickstart.mjs
  run "Vite, webpack and Next"    node scripts/dev/bundler-check.mjs
else
  warn "skipped install, quickstart and bundler gates — rerun with --full before tagging"
fi

# -------------------------------------------------------------------------- verdict
echo
if (( ${#fails[@]} )); then
  printf '\033[31mNOT READY\033[0m — %d blocking issue(s)\n' "${#fails[@]}"
  printf '  - %s\n' "${fails[@]}"
  exit 1
fi

printf '\033[32mREADY\033[0m — %d warning(s)\n' "${#warns[@]}"
(( ${#warns[@]} )) && printf '  - %s\n' "${warns[@]}"

cat <<EOF

Publishing is behind an authority gate (CLAUDE.md) and this script will not cross it.
When a human decides to release, these are the steps:

  1. set the final version        npm version --no-git-tag-version --prefix packages/core <x.y.z>
  2. move Unreleased into it      \$EDITOR CHANGELOG.md
  3. re-run this check            scripts/release/check-release-ready.sh --full
  4. commit                       git commit -am "chore(release): v<x.y.z>"
  5. tag                          git tag -a v<x.y.z> -m "v<x.y.z>"
  6. push                         git push && git push --tags
  7. publish                      npm publish --access public ./packages/core
  8. GitHub release               scripts/gh-owner.sh release create v<x.y.z> --notes-file CHANGELOG.md

Publishing from this repo is an authority gate in CLAUDE.md. Step 7 is the owner's to run.

Step 7 is irreversible: npm does not allow a version to be republished, and unpublishing
is restricted. Step 3 is what makes step 7 safe.
EOF
