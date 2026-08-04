#!/usr/bin/env python3
"""Prove the worktree lifecycle refuses the things it exists to refuse.

Every fixture is a throwaway repo with a real `origin` (a second clone on disk), because
the interesting question — "is this branch merged?" — is only answerable against a
remote, and a fake that answers it in-process would be testing nothing.

    python3 scripts/dev/test_worktree.py
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TOOL = ROOT / "scripts/dev/worktree.py"


def clean_env() -> dict[str, str]:
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    env["GIT_CONFIG_NOSYSTEM"] = "1"
    return env


class Fixture:
    """A repo with an origin, a main branch, and nothing else."""

    def __init__(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="worktree-")
        base = Path(self.temp.name)
        self.env = clean_env()
        self.origin = base / "origin.git"
        self.repo = base / "repo"

        seed = base / "seed"
        seed.mkdir()
        self.git(seed, "init", "-b", "main")
        (seed / "README.md").write_text("seed\n")
        self.git(seed, "add", "-A")
        self.git(seed, "commit", "-q", "--no-gpg-sign", "-m", "seed")
        self.git(base, "clone", "-q", "--bare", str(seed), str(self.origin))
        self.git(base, "clone", "-q", str(self.origin), str(self.repo))

    def git(self, cwd: Path, *args: str) -> str:
        proc = subprocess.run(
            ["git", "-c", "core.hooksPath=", "-c", "user.name=fixture",
             "-c", "user.email=fixture@example.invalid", *args],
            cwd=cwd, env=self.env, text=True,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=True)
        return proc.stdout

    def tool(self, *args: str, cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
        return subprocess.run([sys.executable, str(TOOL), "--cwd", str(cwd or self.repo), *args],
                              cwd=cwd or self.repo, env=self.env, text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)

    def worktree_path(self, branch: str) -> Path:
        return self.repo.parent / ".worktrees" / f"{self.repo.name}-{branch.replace('/', '-')}"

    def commit_in(self, path: Path, name: str) -> None:
        (path / name).write_text(name)
        self.git(path, "add", "-A")
        self.git(path, "commit", "-q", "--no-gpg-sign", "-m", name)

    def _land(self, branch: str, *merge_args: str) -> None:
        """Land the branch the way GitHub does: on the remote, not in this checkout."""
        self.git(self.repo, "push", "-q", "origin", branch)
        clone = self.repo.parent / f"lander-{branch.replace('/', '-')}"
        self.git(self.repo.parent, "clone", "-q", str(self.origin), str(clone))
        self.git(clone, "merge", *merge_args, f"origin/{branch}")
        if "--squash" in merge_args:
            self.git(clone, "commit", "-q", "--no-gpg-sign", "-m", f"{branch} (squashed)")
        self.git(clone, "push", "-q", "origin", "main")
        self.git(self.repo, "fetch", "-q", "origin")

    def merge_to_main(self, branch: str) -> None:
        self._land(branch, "-q", "--no-ff", "--no-gpg-sign", "-m", "merge")

    def squash_to_main(self, branch: str) -> None:
        self._land(branch, "-q", "--squash")

    def close(self) -> None:
        self.temp.cleanup()


class WorktreeLifecycle(unittest.TestCase):
    def setUp(self) -> None:
        self.fx = Fixture()
        self.addCleanup(self.fx.close)

    def test_start_creates_the_worktree_outside_the_repo(self) -> None:
        out = self.fx.tool("start", "feat/thing")
        self.assertEqual(out.returncode, 0, out.stdout)
        dest = self.fx.worktree_path("feat/thing")
        self.assertTrue(dest.is_dir(), out.stdout)
        # Outside the repo: an in-repo copy is walked by every glob and audit.
        self.assertNotIn(self.fx.repo, dest.parents)

    def test_start_shares_the_build_trees_and_hides_them_from_status(self) -> None:
        """Without the shared build trees the pre-commit audit cannot pass in a fresh
        worktree; without the exclude, `git add -A` would commit the symlinks."""
        for shared in ("node_modules", "target"):
            (self.fx.repo / shared).mkdir()
            # Non-empty on purpose: git never reports an EMPTY directory as untracked, so
            # an empty fixture would pass whether or not the exclude works at all.
            (self.fx.repo / shared / "artifact.bin").write_text("built")
        self.fx.tool("start", "feat/thing")
        dest = self.fx.worktree_path("feat/thing")
        for shared in ("node_modules", "target"):
            self.assertTrue((dest / shared).is_symlink(), shared)
        self.assertEqual(self.fx.git(dest, "status", "--porcelain").strip(), "")

    def test_start_refuses_to_reuse_a_destination(self) -> None:
        self.fx.tool("start", "feat/thing")
        out = self.fx.tool("start", "feat/thing")
        self.assertEqual(out.returncode, 1, out.stdout)
        self.assertIn("already exists", out.stdout)

    def test_finish_refuses_an_unmerged_branch(self) -> None:
        self.fx.tool("start", "feat/thing")
        self.fx.commit_in(self.fx.worktree_path("feat/thing"), "work.txt")
        out = self.fx.tool("finish", "feat/thing")
        self.assertEqual(out.returncode, 1, out.stdout)
        self.assertIn("not upstream", out.stdout)
        self.assertTrue(self.fx.worktree_path("feat/thing").is_dir())

    def test_finish_refuses_a_dirty_worktree(self) -> None:
        self.fx.tool("start", "feat/thing")
        (self.fx.worktree_path("feat/thing") / "scratch.txt").write_text("unsaved")
        out = self.fx.tool("finish", "feat/thing")
        self.assertEqual(out.returncode, 1, out.stdout)
        self.assertIn("uncommitted changes", out.stdout)

    def test_finish_removes_a_merged_worktree_and_its_branch(self) -> None:
        self.fx.tool("start", "feat/thing")
        self.fx.commit_in(self.fx.worktree_path("feat/thing"), "work.txt")
        self.fx.merge_to_main("feat/thing")
        out = self.fx.tool("finish", "feat/thing")
        self.assertEqual(out.returncode, 0, out.stdout)
        self.assertFalse(self.fx.worktree_path("feat/thing").exists())
        self.assertNotIn("feat/thing", self.fx.git(self.fx.repo, "branch", "--list"))

    def test_finish_deletes_the_branch_after_a_squash_merge(self) -> None:
        """`git branch -d` refuses here — a squash or rebase merge rewrites the commits,
        so ancestry says unmerged while every patch is upstream. Observed for real: the
        worktree was removed and the branch was left behind."""
        self.fx.tool("start", "feat/thing")
        self.fx.commit_in(self.fx.worktree_path("feat/thing"), "work.txt")
        self.fx.squash_to_main("feat/thing")
        out = self.fx.tool("finish", "feat/thing")
        self.assertEqual(out.returncode, 0, out.stdout)
        self.assertIn("deleted branch feat/thing", out.stdout)
        self.assertNotIn("feat/thing", self.fx.git(self.fx.repo, "branch", "--list"))

    def test_audit_fails_while_a_merged_worktree_survives(self) -> None:
        """The rule, stated as a gate: merged means gone."""
        self.fx.tool("start", "feat/thing")
        self.fx.commit_in(self.fx.worktree_path("feat/thing"), "work.txt")
        self.assertEqual(self.fx.tool("audit").returncode, 0)

        self.fx.merge_to_main("feat/thing")
        out = self.fx.tool("audit")
        self.assertEqual(out.returncode, 1, out.stdout)
        self.assertIn("merged into", out.stdout)

        self.fx.tool("finish", "feat/thing")
        self.assertEqual(self.fx.tool("audit").returncode, 0)

    def test_audit_warns_but_passes_when_the_directory_is_already_gone(self) -> None:
        """A registration pointing at nothing risks nothing; it is noise, not a defect."""
        self.fx.tool("start", "feat/thing")
        subprocess.run(["rm", "-rf", str(self.fx.worktree_path("feat/thing"))], check=True)
        out = self.fx.tool("audit")
        self.assertEqual(out.returncode, 0, out.stdout)
        self.assertIn("directory is gone", out.stdout)

    def test_audit_skips_the_merged_check_without_a_base_ref(self) -> None:
        """A shallow CI clone has no origin/main. Unknowable must not mean 'assume merged'."""
        self.fx.tool("start", "feat/thing")
        self.fx.commit_in(self.fx.worktree_path("feat/thing"), "work.txt")
        self.fx.git(self.fx.repo, "remote", "remove", "origin")
        self.fx.git(self.fx.repo, "branch", "-m", "main", "trunk")  # no main to fall back to
        out = self.fx.tool("audit")
        self.assertEqual(out.returncode, 0, out.stdout)
        self.assertIn("skipping the merged check", out.stdout)

    def test_a_freshly_started_worktree_is_not_stale(self) -> None:
        """The regression that made the rule unusable: a new branch IS an ancestor of main,
        so an ancestor test fired before the first commit and the pre-commit hook — which
        runs this audit — could never let that first commit through."""
        self.fx.tool("start", "feat/thing")
        out = self.fx.tool("audit")
        self.assertEqual(out.returncode, 0, out.stdout)
        self.assertIn("none of them merged", out.stdout)

    def test_a_fresh_branch_left_behind_by_main_is_not_stale(self) -> None:
        """Observed for real: main moved while this worktree sat unused, and a tip
        comparison then called an empty branch merged. Nothing was ever committed on it,
        so there is nothing that could have merged."""
        self.fx.tool("start", "feat/thing")
        self.fx.tool("start", "feat/other")
        self.fx.commit_in(self.fx.worktree_path("feat/other"), "other.txt")
        self.fx.merge_to_main("feat/other")           # main advances past feat/thing
        self.fx.tool("finish", "feat/other")
        out = self.fx.tool("audit")
        self.assertEqual(out.returncode, 0, out.stdout)
        self.assertIn("none of them merged", out.stdout)

    def test_resetting_a_branch_onto_a_newer_main_is_not_work(self) -> None:
        """Also observed for real: rebasing an unstarted branch forward moved its ref, and
        a reflog test that counted any entry then called it merged."""
        self.fx.tool("start", "feat/thing")
        self.fx.tool("start", "feat/other")
        self.fx.commit_in(self.fx.worktree_path("feat/other"), "other.txt")
        self.fx.merge_to_main("feat/other")
        self.fx.tool("finish", "feat/other")   # or the audit fails on it, not on feat/thing
        self.fx.git(self.fx.worktree_path("feat/thing"), "reset", "--hard", "origin/main")
        out = self.fx.tool("audit")
        self.assertEqual(out.returncode, 0, out.stdout)

    def test_a_squash_merged_branch_is_stale(self) -> None:
        """How these PRs actually land: nothing on the branch is an ancestor of main."""
        self.fx.tool("start", "feat/thing")
        self.fx.commit_in(self.fx.worktree_path("feat/thing"), "work.txt")
        self.fx.squash_to_main("feat/thing")
        out = self.fx.tool("audit")
        self.assertEqual(out.returncode, 1, out.stdout)
        self.assertIn("merged into", out.stdout)

    def test_audit_leaves_a_locked_worktree_alone(self) -> None:
        """git's own hands-off marker. This machine had a locked entry pointing at a
        directory that no longer exists, so failing on it could never be satisfied."""
        self.fx.tool("start", "feat/thing")
        self.fx.commit_in(self.fx.worktree_path("feat/thing"), "work.txt")
        self.fx.merge_to_main("feat/thing")
        self.fx.git(self.fx.repo, "worktree", "lock", str(self.fx.worktree_path("feat/thing")))
        out = self.fx.tool("audit")
        self.assertEqual(out.returncode, 0, out.stdout)
        self.assertIn("locked", out.stdout)

    def test_audit_ignores_the_primary_worktree(self) -> None:
        """main is merged into origin/main by definition; flagging it would fail always."""
        out = self.fx.tool("audit")
        self.assertEqual(out.returncode, 0, out.stdout)
        self.assertIn("0 linked worktree(s)", out.stdout)

    def test_finish_works_from_inside_the_worktree(self) -> None:
        """git refuses to remove the tree it is standing in; the tool must not."""
        self.fx.tool("start", "feat/thing")
        dest = self.fx.worktree_path("feat/thing")
        self.fx.commit_in(dest, "work.txt")
        self.fx.merge_to_main("feat/thing")
        out = self.fx.tool("finish", cwd=dest)
        self.assertEqual(out.returncode, 0, out.stdout)
        self.assertFalse(dest.exists())


if __name__ == "__main__":
    unittest.main()
