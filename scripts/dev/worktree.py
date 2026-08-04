#!/usr/bin/env python3
"""Own the worktree lifecycle: start every branch in one, delete it once the PR merges.

WHY THE PRIMARY CHECKOUT IS NOT A PLACE TO WORK
The primary checkout is the tree every other tool assumes: `npm run dev` serves from it,
a render or a bench reads from it, and a half-finished branch sitting there silently
changes what those numbers mean. Evidence is bound to a SHA and a clean tree — a primary
tree parked on someone's feature branch breaks that quietly, and nothing in the harness
can tell the difference between "measured on main" and "measured on whatever was checked
out at the time". A linked worktree makes the two physically separate directories.

WHY A MERGED WORKTREE IS DELETED
A worktree whose branch is already on main is a copy of the repo that will never be
touched again, holds a stale node_modules and target/, and is indistinguishable at a
glance from live work. This machine had 20+ of them registered against one repo, most
pointing at directories that no longer exist. `audit` makes that state a failure.

    python3 scripts/dev/worktree.py start feat/thing   # branch + worktree off origin/main
    python3 scripts/dev/worktree.py list               # what exists, and what is merged
    python3 scripts/dev/worktree.py finish [branch]    # remove it, once it is merged
    python3 scripts/dev/worktree.py audit              # fail if a merged one is still here
    python3 scripts/dev/worktree.py prune              # drop registrations whose dir is gone
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path

GREEN, RED, YELLOW, RESET = "\033[32m", "\033[31m", "\033[33m", "\033[0m"


def git(*args: str, cwd: Path | None = None, check: bool = False) -> tuple[int, str]:
    proc = subprocess.run(["git", *args], cwd=cwd, text=True,
                          stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)
    if check and proc.returncode != 0:
        die(f"git {' '.join(args)} failed:\n{proc.stdout.strip()}")
    return proc.returncode, proc.stdout


def die(message: str) -> None:
    print(f"{RED}worktree:{RESET} {message}", file=sys.stderr)
    raise SystemExit(1)


class Tree:
    def __init__(self, path: Path, branch: str | None, primary: bool,
                 prunable: bool, locked: bool) -> None:
        self.path, self.branch = path, branch
        self.primary, self.prunable, self.locked = primary, prunable, locked

    @property
    def name(self) -> str:
        return self.branch or f"(detached at {self.path.name})"


def worktrees(cwd: Path) -> list[Tree]:
    """Parse `git worktree list --porcelain`. The FIRST record is always the primary."""
    _, out = git("worktree", "list", "--porcelain", cwd=cwd)
    trees: list[Tree] = []
    path: Path | None = None
    branch = None
    prunable = locked = False
    for line in out.splitlines() + [""]:
        if line.startswith("worktree "):
            path = Path(line[len("worktree "):])
        elif line.startswith("branch "):
            branch = line[len("branch "):].removeprefix("refs/heads/")
        elif line.startswith("prunable"):
            prunable = True
        elif line.startswith("locked"):
            locked = True
        elif not line.strip() and path is not None:
            trees.append(Tree(path, branch, not trees, prunable, locked))
            path, branch, prunable, locked = None, None, False, False
    return trees


def primary(cwd: Path) -> Path:
    trees = worktrees(cwd)
    if not trees:
        die("not inside a git repository")
    return trees[0].path


def upstream_main(cwd: Path) -> str | None:
    """`origin/main` when it is present. A shallow CI clone has no such ref, and an
    offline machine cannot refresh it — in both cases merge state is UNKNOWABLE, and
    guessing it either blocks real work or waves through a stale worktree."""
    for ref in ("origin/main", "main"):
        if git("rev-parse", "--verify", "--quiet", f"{ref}^{{commit}}", cwd=cwd)[0] == 0:
            return ref
    return None


def unique_commits(cwd: Path, branch: str, base: str) -> list[str]:
    """Commits on `branch` that are not upstream, by PATCH id rather than commit id.

    `merge-base --is-ancestor` alone answers the wrong question twice. It calls a brand
    new worktree "merged" — its branch is still exactly origin/main, so the audit fired
    before the first commit could be made and the pre-commit hook made the branch
    unusable. And it misses a squash merge entirely, which is how most of these PRs land:
    the squashed commit is a different object, so nothing on the branch is an ancestor of
    main even though every line of it is. `git cherry` compares patch ids, so a
    single-commit squash reads as upstream. A multi-commit squash still does not, and
    that is a known hole — `finish` is what closes it, not the audit.
    """
    code, out = git("cherry", base, branch, cwd=cwd)
    if code != 0:
        return ["?"]  # unknowable — treat as work in progress, never as disposable
    return [line for line in out.splitlines() if line.startswith("+")]


def has_history(cwd: Path, branch: str) -> bool:
    """Has anything ever been committed on this branch, per its own reflog?

    Needed because a branch that was merged and a branch that was never used are
    STRUCTURALLY IDENTICAL: both have no commits of their own, and both are ancestors of
    main. Comparing tips instead ("is it exactly origin/main?") worked only until main
    moved — which it did, on this repo, between creating this worktree and auditing it,
    and the audit then declared a branch with no commits on it "merged".

    Only `commit:` entries count. Every other reflog subject — `branch: Created from`,
    `reset: moving to`, `checkout:` — records the ref MOVING, not work landing on it, and
    an earlier version that accepted any entry flagged this very branch as merged the
    moment it was reset onto a newer main.

    The reflog is local state, which is the right scope: so is a worktree. When it has
    been pruned this reads as "never used", so the audit stays quiet rather than blocking
    a first commit on a guess.
    """
    code, out = git("reflog", "show", "--format=%gs", branch, cwd=cwd)
    if code != 0:
        return False
    return any(line.startswith("commit") for line in out.splitlines())


def is_untouched(cwd: Path, branch: str, base: str) -> bool:
    """Started, never worked in — whether or not main has moved on since."""
    return not has_history(cwd, branch)


def is_stale(cwd: Path, branch: str, base: str) -> bool:
    return has_history(cwd, branch) and not unique_commits(cwd, branch, base)


def is_dirty(path: Path) -> bool:
    code, out = git("status", "--porcelain", cwd=path)
    return code != 0 or bool(out.strip())


def destination(root: Path, branch: str) -> Path:
    """Outside the repo, one directory per family.

    An in-repo worktree is a second copy of the tree that every glob, audit, bundler and
    `packages/*` workspace pattern would then walk — and that `git status` in the primary
    tree cannot see. Keeping it next to the repo removes the whole class of problem.
    """
    return root.parent / ".worktrees" / f"{root.name}-{branch.replace('/', '-')}"


# ----------------------------------------------------------------------------- commands
def cmd_start(args: argparse.Namespace, cwd: Path) -> int:
    root = primary(cwd)
    branch: str = args.branch
    dest = destination(root, branch)
    if dest.exists():
        die(f"{dest} already exists — `finish {branch}` first, or pick another name")

    base = upstream_main(cwd)
    if base and not args.no_fetch:
        git("fetch", "origin", "main", cwd=cwd)
        base = upstream_main(cwd)
    if not base:
        die("no origin/main or main to branch from")

    dest.parent.mkdir(parents=True, exist_ok=True)
    code, out = git("worktree", "add", "-b", branch, str(dest), base, cwd=cwd)
    if code != 0:
        die(out.strip())
    print(f"  {GREEN}ok{RESET}    {branch} → {dest}  (from {base})")

    # Share the two build trees rather than rebuilding them per worktree.
    #
    # This is not a convenience. The pre-commit hook runs the full harness audit, and the
    # audit renders through the WASM at target/wasm32-unknown-unknown/release/ — a
    # gitignored cargo artifact. Without it, `test_verify_spec.py` fails with a bare
    # KeyError on a missing metric (observed on the first commit into a worktree), so a
    # brand new worktree could not reach its first commit without a multi-minute cargo
    # build first. A rule that expensive gets bypassed.
    #
    # Symlinked, never copied: an install or a build run in the worktree must update the
    # one shared tree instead of silently diverging from it. cargo takes a lock on its
    # target directory, so two worktrees building at once serialise rather than corrupt.
    for shared in ("node_modules", "target"):
        src = root / shared
        if src.is_dir() and not (dest / shared).exists():
            os.symlink(src, dest / shared)
            print(f"  {GREEN}ok{RESET}    {shared} shared from the primary tree")

    # …and keep those links out of `git status`. `.gitignore` says `node_modules/`, and a
    # trailing slash matches a directory — a SYMLINK to one is a file, so it is not
    # matched, shows up untracked in every status, and `git add -A` would commit it.
    #
    # The exclude goes in the COMMON git dir, not the per-worktree one: git did not honour
    # `$GIT_DIR/info/exclude` for a linked worktree here. The first attempt looked like it
    # worked only because the probe directory was empty, and git never reports an empty
    # directory as untracked — the test below now puts a file inside each, so that
    # confound cannot hide the same mistake twice.
    common = Path(git("rev-parse", "--git-common-dir", cwd=dest)[1].strip())
    if not common.is_absolute():
        common = (root / common).resolve()
    exclude = common / "info" / "exclude"
    exclude.parent.mkdir(parents=True, exist_ok=True)
    existing = exclude.read_text().splitlines() if exclude.exists() else []
    missing = [name for name in ("node_modules", "target") if name not in existing]
    if missing:
        with exclude.open("a") as fh:
            fh.write("\n".join(["", "# worktree symlinks (scripts/dev/worktree.py)", *missing, ""]))

    print(f"\n    cd {dest}\n")
    return 0


def cmd_list(args: argparse.Namespace, cwd: Path) -> int:
    base = upstream_main(cwd)
    for tree in worktrees(cwd):
        tags = []
        if tree.primary:
            tags.append("primary")
        if tree.locked:
            tags.append("locked")
        if tree.prunable:
            tags.append("directory is gone")
        elif not tree.primary and tree.branch and base:
            if is_stale(cwd, tree.branch, base):
                tags.append("MERGED — delete it")
            elif is_untouched(cwd, tree.branch, base):
                tags.append("not started")
            else:
                tags.append(f"{len(unique_commits(cwd, tree.branch, base))} commit(s) not upstream")
        print(f"  {tree.name:<44} {tree.path}  [{', '.join(tags) or '—'}]")
    return 0


def cmd_finish(args: argparse.Namespace, cwd: Path) -> int:
    branch = args.branch or git("rev-parse", "--abbrev-ref", "HEAD", cwd=cwd)[1].strip()
    trees = worktrees(cwd)
    match = next((t for t in trees if t.branch == branch and not t.primary), None)
    if not match:
        die(f"no linked worktree holds '{branch}' (`list` shows what does)")
    if match.prunable:
        git("worktree", "prune", cwd=trees[0].path)
        print(f"  {GREEN}ok{RESET}    {branch}: registration pruned, directory was already gone")
        return 0

    # Refresh before judging: `audit` deliberately stays offline (it runs inside a git
    # hook), so the local origin/main it reads can be hours behind the merge that this
    # command is being run BECAUSE of.
    git("fetch", "origin", "main", cwd=trees[0].path)
    base = upstream_main(cwd)
    if not args.force:
        if is_dirty(match.path):
            die(f"{match.path} has uncommitted changes — commit, or pass --force to discard")
        if not base:
            die("cannot see origin/main, so cannot prove the branch merged; --force to override")
        unique = unique_commits(cwd, branch, base)
        if unique:
            die(f"'{branch}' has {len(unique)} commit(s) not upstream in {base} — it is not "
                f"merged yet, and that is the whole point of the check. --force if you are "
                f"abandoning the work.")

    # Run the removal from the primary tree: git refuses to remove the worktree it is
    # standing in, and the first version of this script failed exactly there.
    code, out = git("worktree", "remove", *(["--force"] if args.force else []),
                    str(match.path), cwd=trees[0].path)
    if code != 0:
        die(out.strip())
    print(f"  {GREEN}ok{RESET}    removed {match.path}")

    code, out = git("branch", "-D" if args.force else "-d", branch, cwd=trees[0].path)
    print(f"  {GREEN}ok{RESET}    deleted branch {branch}" if code == 0
          else f"  {YELLOW}warn{RESET}  branch {branch} kept: {out.strip()}")

    # `cwd` is captured before the removal on purpose: reading Path.cwd() here raises
    # FileNotFoundError when the caller was standing in the directory just deleted, which
    # is the single most likely way to run this command.
    if cwd == match.path or match.path in cwd.parents:
        print(f"\n    cd {trees[0].path}\n")
    return 0


def cmd_prune(args: argparse.Namespace, cwd: Path) -> int:
    before = [t for t in worktrees(cwd) if t.prunable]
    git("worktree", "prune", cwd=primary(cwd))
    print(f"  {GREEN}ok{RESET}    pruned {len(before)} registration(s) whose directory was gone")
    return 0


def cmd_audit(args: argparse.Namespace, cwd: Path) -> int:
    trees = worktrees(cwd)
    base = upstream_main(cwd)
    fails, warns = [], []
    for tree in trees:
        if tree.primary:
            continue
        if tree.locked:
            # `locked` is git's own hands-off marker and `git worktree prune` honours it.
            # A gate that overrides it would be asserting it knows better than the person
            # who set it — and the one on this machine points at a directory that no
            # longer exists, so failing on it can never be satisfied by cleaning up.
            warns.append(f"{tree.name}: locked, so this audit leaves it alone "
                         f"— `git worktree unlock` first if it is dead")
        elif tree.prunable:
            warns.append(f"{tree.name}: registered, but its directory is gone "
                         f"— `python3 scripts/dev/worktree.py prune`")
        elif tree.branch and base and is_stale(cwd, tree.branch, base):
            fails.append(f"{tree.name}: merged into {base}, but the worktree is still here "
                         f"— `python3 scripts/dev/worktree.py finish {tree.branch}`")
    if base is None:
        print(f"  {YELLOW}warn{RESET}  no origin/main ref here; merge state is unknowable, "
              f"skipping the merged check")
    for w in warns:
        print(f"  {YELLOW}warn{RESET}  {w}")
    for f in fails:
        print(f"  {RED}FAIL{RESET}  {f}")
    if fails:
        print(f"\n{RED}worktree audit FAILED{RESET} — a merged worktree is a stale copy of the "
              f"repo that nothing will ever touch again.")
        return 1
    print(f"  {GREEN}ok{RESET}    {len(trees) - 1} linked worktree(s), none of them merged")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cwd", type=Path, default=Path.cwd(), help=argparse.SUPPRESS)
    sub = ap.add_subparsers(dest="command", required=True)

    p = sub.add_parser("start", help="branch off origin/main into a new worktree")
    p.add_argument("branch")
    p.add_argument("--no-fetch", action="store_true")
    p.set_defaults(fn=cmd_start)

    p = sub.add_parser("list", help="every worktree, and whether it is merged")
    p.set_defaults(fn=cmd_list)

    p = sub.add_parser("finish", help="remove a merged worktree and its branch")
    p.add_argument("branch", nargs="?")
    p.add_argument("--force", action="store_true", help="abandon unmerged or dirty work")
    p.set_defaults(fn=cmd_finish)

    p = sub.add_parser("prune", help="drop registrations whose directory is gone")
    p.set_defaults(fn=cmd_prune)

    p = sub.add_parser("audit", help="fail if a merged worktree is still on disk")
    p.set_defaults(fn=cmd_audit)

    args = ap.parse_args()
    return args.fn(args, args.cwd.resolve())


if __name__ == "__main__":
    sys.exit(main())
