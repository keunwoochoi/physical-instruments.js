#!/usr/bin/env python3
"""Force every pull request to answer one question: does this change what a user installs?

The package is on npm. A change to the published surface that lands with no changelog
entry is invisible until someone cuts a release and has to reconstruct, from `git log`,
what the version number is supposed to mean. That reconstruction is exactly the thing
this repo refuses to leave to memory, so the answer is declared in the PR body and
checked here.

The declaration is one line, anywhere in the PR body or in a commit message:

    Release-Impact: none — harness only, nothing in the tarball changes
    Release-Impact: patch — fixes the iOS resume path
    Release-Impact: minor — adds two patches to the bank
    Release-Impact: major — removes the deprecated `engine.start()` overload

It is a PROPOSAL about the next release, not a promise to bump anything now: publishing
sits behind an authority gate that only a human lifts, and most PRs correctly leave the
version alone. What a non-`none` PR must do is leave its entry under `## [Unreleased]`
in CHANGELOG.md, so the person cutting the release reads a written record instead of a
diff. While the package is 0.x, a breaking change is a `minor` bump by semver; declare
`major` only once 1.0 has shipped.

    python3 scripts/release/check_release_impact.py                     # vs origin/main
    python3 scripts/release/check_release_impact.py --base <ref>
    python3 scripts/release/check_release_impact.py --body-file body.md
    PR_BODY="$(...)" python3 scripts/release/check_release_impact.py --body-env PR_BODY
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

# The published surface, derived where it can be and named where it cannot.
#
# Publishable package directories are DERIVED (packages/*/package.json without
# "private": true) so adding or unpublishing a package cannot leave a stale list behind.
# These two groups cannot be derived from any manifest:
#
#   NATIVE  — the Rust that compiles into the shipped WASM, plus the toolchain pin and
#             lockfile that decide its exact bytes. None of it appears in `files`; all of
#             it changes what the user runs.
#   STAGED  — the repo-root originals that the build copies into the package before
#             packing. The copies are gitignored, so a diff only ever shows the original.
NATIVE = ("crates/", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml")
STAGED = ("README.md", "LICENSE-MIT", "LICENSE-APACHE")

LEVELS = ("none", "patch", "minor", "major")

# A declaration line. The separator is loose (em dash, hyphen, colon) because the point
# is the decision and the reason, not the punctuation. The level must be a bare word:
# the template ships `<none|patch|minor|major>`, which deliberately does NOT match, so an
# unedited template fails instead of silently reading as "none".
DECLARATION = re.compile(
    r"^[ \t>*-]*Release-Impact:[ \t]*(" + "|".join(LEVELS) + r")\b[ \t]*(?:[—–\-:]+[ \t]*(.*))?$",
    re.IGNORECASE | re.MULTILINE,
)
# Present but unusable — reported separately from missing, because "you left the
# placeholder in" and "you forgot the line" are different mistakes with different fixes.
DECLARATION_ANY = re.compile(r"^[ \t>*-]*Release-Impact:", re.IGNORECASE | re.MULTILINE)

SECTION = re.compile(r"^## +\[([^\]]+)\]", re.MULTILINE)
PLACEHOLDER = re.compile(r"[<{]")

GREEN, RED, YELLOW, RESET = "\033[32m", "\033[31m", "\033[33m", "\033[0m"


class Result:
    def __init__(self) -> None:
        self.fails: list[str] = []
        self.warns: list[str] = []

    def ok(self, msg: str) -> None:
        print(f"  {GREEN}ok{RESET}    {msg}")

    def bad(self, msg: str, detail: str = "") -> None:
        print(f"  {RED}FAIL{RESET}  {msg}")
        if detail:
            print("\n".join(f"        {line}" for line in detail.splitlines()))
        self.fails.append(msg)

    def warn(self, msg: str, detail: str = "") -> None:
        print(f"  {YELLOW}warn{RESET}  {msg}")
        if detail:
            print("\n".join(f"        {line}" for line in detail.splitlines()))
        self.warns.append(msg)


def git(root: Path, *args: str) -> tuple[int, str]:
    proc = subprocess.run(
        ["git", *args], cwd=root, text=True,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
    )
    return proc.returncode, proc.stdout


def file_at(root: Path, ref: str, path: str) -> str | None:
    """The file's content at a ref, or None when it did not exist there."""
    code, out = git(root, "show", f"{ref}:{path}")
    return out if code == 0 else None


def resolve_base(root: Path, requested: str | None) -> str | None:
    """The commit this branch diverged from. A PR's base sha when CI supplies one."""
    candidates = [requested] if requested else ["origin/main", "main"]
    for cand in candidates:
        code, out = git(root, "merge-base", cand, "HEAD")
        if code == 0 and out.strip():
            return out.strip()
        # A base SHA handed over by CI may be an ancestor already; use it as given.
        if requested and git(root, "rev-parse", "--verify", f"{requested}^{{commit}}")[0] == 0:
            return requested
    return None


def publishable_packages(root: Path) -> list[str]:
    dirs = []
    for manifest in sorted((root / "packages").glob("*/package.json")):
        try:
            data = json.loads(manifest.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue
        if data.get("private") is True:
            continue
        dirs.append(f"{manifest.parent.relative_to(root).as_posix()}/")
    return dirs


def is_product(path: str, package_dirs: list[str]) -> bool:
    return (
        path in STAGED
        or path in NATIVE
        or path.startswith(NATIVE)
        or any(path.startswith(d) for d in package_dirs)
    )


def section_entries(text: str | None, version: str = "unreleased") -> set[str]:
    """The content lines under `## [<version>]`, normalized for comparison.

    Compared as a SET rather than by diffing the file: a PR that rewords a neighbouring
    line, or that reflows the section, must not read as "added an entry" — only
    genuinely new content counts.
    """
    if not text:
        return set()
    out: set[str] = set()
    inside = False
    for line in text.splitlines():
        header = SECTION.match(line)
        if header:
            inside = header.group(1).strip().lower() == version.lower()
            continue
        if not inside:
            continue
        norm = " ".join(line.split()).strip()
        if not norm or norm.lower() in {"nothing yet.", "nothing yet", "none."}:
            continue
        out.add(norm)
    return out


def version_at(root: Path, ref: str, pkg_dir: str) -> str | None:
    text = file_at(root, ref, f"{pkg_dir}package.json")
    if not text:
        return None
    try:
        return json.loads(text).get("version")
    except ValueError:
        return None


def semver_delta(old: str, new: str) -> str | None:
    def parts(v: str) -> tuple[int, int, int] | None:
        m = re.match(r"^(\d+)\.(\d+)\.(\d+)", v)
        return (int(m[1]), int(m[2]), int(m[3])) if m else None

    a, b = parts(old), parts(new)
    if not a or not b or a == b:
        return None
    if b[0] != a[0]:
        return "major"
    if b[1] != a[1]:
        return "minor"
    return "patch"


def find_declaration(sources: list[tuple[str, str]]) -> tuple[str, str, str] | None:
    """(level, reason, where) from the first source that carries a usable line."""
    for where, text in sources:
        matches = list(DECLARATION.finditer(text or ""))
        if matches:
            m = matches[-1]  # the last one wins: an edited line beats a quoted example
            return m.group(1).lower(), (m.group(2) or "").strip(), where
    return None


def check(root: Path, base: str, body: str, body_source: str) -> Result:
    r = Result()
    code, out = git(root, "diff", "--name-only", f"{base}...HEAD")
    changed = [p for p in out.splitlines() if p.strip()] if code == 0 else []
    if code != 0:
        r.bad(f"could not diff {base}...HEAD")
        return r
    if not changed:
        r.ok("no files changed against the base — nothing to declare")
        return r

    packages = publishable_packages(root)
    product = [p for p in changed if is_product(p, packages)]

    print(f"== the change ==\n  {len(changed)} file(s) changed against {base[:12]}")
    if product:
        print(f"  {len(product)} of them are in the published surface:")
        for p in product[:20]:
            print(f"        {p}")
        if len(product) > 20:
            print(f"        … and {len(product) - 20} more")
    else:
        print("  none of them are in the published surface")

    # ------------------------------------------------------------------ the declaration
    print("\n== the declaration ==")
    commits = git(root, "log", "--format=%B", f"{base}..HEAD")[1]
    found = find_declaration([(body_source, body), ("a commit message", commits)])
    if not found:
        where = "the PR body or a commit message"
        if DECLARATION_ANY.search(body or "") or DECLARATION_ANY.search(commits):
            r.bad(
                "a `Release-Impact:` line exists but names no level",
                "The template ships a placeholder on purpose. Replace it with one of\n"
                f"{', '.join(LEVELS)} and a reason:\n"
                "    Release-Impact: patch — fixes the note-off race on repeated triggers",
            )
        else:
            r.bad(
                f"no `Release-Impact:` line in {where}",
                "Every PR states whether it changes what a user installs. Add one line:\n"
                "    Release-Impact: none — harness only, the tarball is unchanged\n"
                f"Levels: {', '.join(LEVELS)}. See .github/pull_request_template.md.",
            )
        return r

    level, reason, where = found
    if not reason or PLACEHOLDER.search(reason):
        r.bad(
            f"`Release-Impact: {level}` carries no reason",
            "The level is the claim; the reason is the evidence a reviewer can argue with.\n"
            f"    Release-Impact: {level} — <why, in one line>",
        )
        return r
    r.ok(f"declared `{level}` in {where} — {reason}")

    # -------------------------------------------------------------------- the changelog
    print("\n== the changelog ==")
    head_log = file_at(root, "HEAD", "CHANGELOG.md")
    base_log = file_at(root, base, "CHANGELOG.md")
    if head_log is None:
        r.bad("CHANGELOG.md is missing")
        return r
    gained = section_entries(head_log) - section_entries(base_log)

    bumped: list[tuple[str, str, str]] = []  # (package dir, old, new)
    for pkg in packages:
        old, new = version_at(root, base, pkg), version_at(root, "HEAD", pkg)
        if old and new and old != new:
            bumped.append((pkg, old, new))

    # A PR that CUTS a release empties [Unreleased] by moving its entries into a dated
    # section, so "gained an Unreleased entry" is the wrong question to ask of it. The
    # entries still have to exist; they just live under the new version number now.
    cut = {line for _, _, new in bumped
           for line in section_entries(head_log, new) - section_entries(base_log, new)}

    if level == "none":
        if gained or cut:
            r.bad(
                "declared `none`, but this PR adds a user-facing changelog entry",
                "One of the two is wrong. Either the change is releasable — say so — or the\n"
                "entry belongs in a commit message rather than CHANGELOG.md.\n"
                + "\n".join(f"+ {line}" for line in sorted(gained | cut)),
            )
        else:
            r.ok("no changelog entry, consistent with `none`")
        if product:
            r.warn(
                f"declared `none` while {len(product)} published file(s) changed",
                "Legitimate for a refactor with identical output, or a comment-only edit.\n"
                "If any observable behaviour moved, this is at least a patch.",
            )
    else:
        if gained:
            r.ok(f"{len(gained)} new entry/entries under [Unreleased]")
        elif cut:
            r.ok(f"{len(cut)} entry/entries under the version this PR cuts")
        else:
            r.bad(
                f"declared `{level}`, but nothing was added under `## [Unreleased]`",
                "The version number is a summary of the changelog, so the entry has to exist\n"
                "before the release can be described. Add one line to CHANGELOG.md saying\n"
                "what changed FOR A USER — the engineering record stays in the commit body.",
            )
        if not product and not bumped:
            r.warn(
                f"declared `{level}` while no published file changed",
                "Nothing under " + ", ".join(packages + list(NATIVE)) + " or "
                + ", ".join(STAGED) + " moved. Check the level is right.",
            )

    # ------------------------------------------------------------ a bump, if there is one
    if bumped:
        print("\n== the version bump ==")
        for pkg, old, new in bumped:
            delta = semver_delta(old, new)
            r.ok(f"{pkg}package.json {old} → {new} ({delta or 'no semver delta'})")
            if not re.search(rf"^## +\[{re.escape(new)}\]", head_log, re.MULTILINE):
                r.bad(f"CHANGELOG.md has no `## [{new}]` section for the new version")
            if delta and delta != level:
                # Below 1.0 a breaking change is a minor bump, so a PR may honestly
                # declare `major` intent while bumping the minor. Every other mismatch
                # means the declaration and the manifest disagree about what happened.
                zero_ver = old.startswith("0.") and delta == "minor" and level == "major"
                if not zero_ver:
                    r.bad(
                        f"declared `{level}` but the version moved by a {delta}",
                        f"{pkg}package.json says {old} → {new}.",
                    )
    return r


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--root", default=Path(__file__).resolve().parents[2], type=Path)
    ap.add_argument("--base", help="base ref or sha; default origin/main, then main")
    ap.add_argument("--body-file", type=Path, help="file holding the PR body")
    ap.add_argument("--body-env", help="env var holding the PR body (CI passes it here "
                                       "so no PR text is ever interpolated into a shell)")
    args = ap.parse_args()

    root: Path = args.root.resolve()
    body, body_source = "", "the PR body"
    if args.body_file:
        body = args.body_file.read_text(encoding="utf-8") if args.body_file.exists() else ""
    elif args.body_env:
        body = os.environ.get(args.body_env, "")
    if not body:
        body_source = "the PR body (not supplied)"

    base = resolve_base(root, args.base)
    if not base:
        print(f"{RED}FAIL{RESET}  no base ref to compare against "
              f"(tried {args.base or 'origin/main, main'})", file=sys.stderr)
        return 1

    print(f"\nrelease impact — {root.name}\n")
    r = check(root, base, body, body_source)

    print()
    if r.fails:
        print(f"{RED}UNDECLARED{RESET} — {len(r.fails)} blocking issue(s)")
        for f in r.fails:
            print(f"  - {f}")
        return 1
    print(f"{GREEN}DECLARED{RESET} — {len(r.warns)} warning(s)")
    for w in r.warns:
        print(f"  - {w}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
