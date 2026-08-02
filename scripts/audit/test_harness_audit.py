#!/usr/bin/env python3
"""Prove the harness audit fails when it should.

METR's task standard requires submitting an invalid, a partially-correct, and a best
solution and confirming each scores as expected before a task is accepted. The same
logic applies to an audit script: one that has never been observed to fail is not
evidence of anything, and a green audit tells you nothing until you know it can go red.

Each fixture below is a deliberately broken minimal harness. The audit MUST reject it,
and MUST do so via the named check — failing for an unrelated reason would mean the
check we care about is silently dead.

    python3 scripts/audit/test_harness_audit.py
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from harness_audit import DEFAULT_ROOT, audit  # noqa: E402

FIXTURES = Path(__file__).resolve().parent / "fixtures"
COMMIT_MSG_HOOK = DEFAULT_ROOT / ".githooks" / "commit-msg"

# fixture directory -> the check id that must appear among the failures
MUST_REJECT = {
    "missing-referent": "C1-PATHS",
    "broken-link": "C2-LINKS",
    "empty-dir-claimed": "C3-NONEMPTY",
    "authority-lift": "C-AUTHORITY",
}


class TestAuditFailsCorrectly(unittest.TestCase):
    def test_good_fixture_passes(self) -> None:
        """The audit is not merely always-red: a clean harness must go green."""
        rep = audit(FIXTURES / "good")
        self.assertEqual(rep.failures, [], f"clean fixture was rejected: {rep.failures}")
        self.assertGreater(rep.checks_run, 0, "clean fixture ran no checks at all")

    def test_live_repo_passes(self) -> None:
        rep = audit(DEFAULT_ROOT)
        self.assertEqual(rep.failures, [], f"live repo was rejected: {rep.failures}")
        self.assertGreater(rep.checks_run, 0)

    def test_broken_fixtures_are_rejected(self) -> None:
        for name, expected in MUST_REJECT.items():
            with self.subTest(fixture=name):
                rep = audit(FIXTURES / name)
                self.assertTrue(rep.failures, f"{name} was accepted but must be rejected")
                ids = {f.split(":", 1)[0] for f in rep.failures}
                self.assertIn(
                    expected,
                    ids,
                    f"{name} was rejected, but not by {expected} — got {sorted(ids)}. "
                    f"The check we rely on may be dead. Failures: {rep.failures}",
                )

    def test_every_fixture_is_covered(self) -> None:
        """A fixture nobody asserts against is decoration."""
        on_disk = {p.name for p in FIXTURES.iterdir() if p.is_dir()}
        self.assertEqual(
            on_disk - {"good"},
            set(MUST_REJECT),
            "fixtures on disk and asserted fixtures disagree",
        )


class TestCommitMsgHookFailsCorrectly(unittest.TestCase):
    """Issue #100 also names a missing commit-message section as a broken input.
    The commit-msg hook is the enforcer; prove it can reject and accept."""

    @classmethod
    def setUpClass(cls) -> None:
        if not COMMIT_MSG_HOOK.is_file():
            raise unittest.SkipTest("commit-msg hook not installed yet (#101)")

    def _run(self, body: str) -> subprocess.CompletedProcess[str]:
        with tempfile.NamedTemporaryFile("w", encoding="utf-8", delete=False) as handle:
            handle.write(body)
            path = handle.name
        try:
            return subprocess.run(
                [str(COMMIT_MSG_HOOK), path],
                capture_output=True,
                text=True,
                check=False,
            )
        finally:
            os.unlink(path)

    def test_short_body_is_rejected(self) -> None:
        result = self._run("chore: x\n\nshort\n")
        self.assertNotEqual(result.returncode, 0, "short body was accepted")

    def test_full_body_passes(self) -> None:
        result = self._run(
            "chore: x\n\n"
            "Defect found by the fail-correctly suite. Before: unenforced. "
            "After: hook rejects thin bodies. Root cause: prose is not a gate. "
            "Tried relying on AGENTS.md alone and abandoned it. Cost: zero runtime.\n"
        )
        self.assertEqual(result.returncode, 0, result.stderr or result.stdout)


if __name__ == "__main__":
    raise SystemExit(unittest.main(verbosity=2))
