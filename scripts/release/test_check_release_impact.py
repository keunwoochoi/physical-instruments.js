#!/usr/bin/env python3
"""Prove the release-impact gate fails on the cases it exists to catch.

A gate nobody has watched go red is not evidence of anything, so every rule in
check_release_impact.py has a fixture here that must fail — and a neighbouring one that
must pass, because a check that only ever fails is just as useless.

Each fixture is a throwaway git repo shaped like this one: a publishable package, a
private one, a CHANGELOG with an Unreleased section, Rust under crates/, and docs that
ship with nothing.

    python3 scripts/release/test_check_release_impact.py
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GATE = ROOT / "scripts/release/check_release_impact.py"

CHANGELOG = """\
# Changelog

## [Unreleased]

Nothing yet.

## [0.1.0] — 2026-01-01

First public version.
"""


def clean_env() -> dict[str, str]:
    """Do not let a parent hook or an inherited GIT_* redirect fixture commands."""
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    env["GIT_CONFIG_NOSYSTEM"] = "1"
    return env


class Fixture:
    """A minimal repo with one commit of history to diff against."""

    def __init__(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="release-impact-")
        self.root = Path(self.temp.name)
        self.env = clean_env()
        self.write("packages/core/package.json", json.dumps({"name": "pkg", "version": "0.1.0"}))
        self.write("packages/tooling/package.json",
                   json.dumps({"name": "tooling", "version": "0.0.0", "private": True}))
        self.write("CHANGELOG.md", CHANGELOG)
        self.write("README.md", "# pkg\n")
        self.write("crates/dsp/src/lib.rs", "pub fn render() {}\n")
        self.write("agentic-docs/notes.md", "notes\n")
        self.write("packages/core/src/index.js", "export const version = 1;\n")
        self.git("init", "-b", "main")
        self.commit("base")
        self.base = self.git("rev-parse", "HEAD").strip()

    def write(self, rel: str, text: str) -> None:
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def append(self, rel: str, text: str) -> None:
        self.write(rel, (self.root / rel).read_text(encoding="utf-8") + text)

    def git(self, *args: str) -> str:
        proc = subprocess.run(
            ["git", "-c", "core.hooksPath=", "-c", "user.name=fixture",
             "-c", "user.email=fixture@example.invalid", *args],
            cwd=self.root, env=self.env, text=True,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=True,
        )
        return proc.stdout

    def commit(self, message: str) -> None:
        self.git("add", "-A")
        self.git("commit", "-q", "--no-gpg-sign", "-m", message)

    def add_unreleased_entry(self, text: str = "- The oscillator no longer clicks on note-off.") -> None:
        body = (self.root / "CHANGELOG.md").read_text(encoding="utf-8")
        self.write("CHANGELOG.md", body.replace("Nothing yet.", text))

    def set_version(self, version: str) -> None:
        self.write("packages/core/package.json", json.dumps({"name": "pkg", "version": version}))

    def run(self, body: str | None = None) -> subprocess.CompletedProcess[str]:
        body_file = self.root / "pr-body.md"
        args = [sys.executable, str(GATE), "--root", str(self.root), "--base", self.base]
        if body is not None:
            body_file.write_text(body, encoding="utf-8")
            args += ["--body-file", str(body_file)]
        return subprocess.run(args, cwd=self.root, env=self.env, text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)

    def close(self) -> None:
        self.temp.cleanup()


class ReleaseImpactGate(unittest.TestCase):
    def setUp(self) -> None:
        self.fx = Fixture()
        self.addCleanup(self.fx.close)

    # ------------------------------------------------------------------- the declaration
    def test_missing_declaration_fails(self) -> None:
        self.fx.append("agentic-docs/notes.md", "more notes\n")
        self.fx.commit("docs: extend the notes")
        out = self.fx.run(body="## What this changes\n\nNothing much.\n")
        self.assertEqual(out.returncode, 1, out.stdout)
        self.assertIn("no `Release-Impact:` line", out.stdout)

    def test_unedited_template_placeholder_fails(self) -> None:
        """The failure mode that matters most: a template that passes when ignored."""
        self.fx.append("agentic-docs/notes.md", "more notes\n")
        self.fx.commit("docs: extend the notes")
        out = self.fx.run(body="Release-Impact: <none|patch|minor|major> — <one line>\n")
        self.assertEqual(out.returncode, 1, out.stdout)
        self.assertIn("names no level", out.stdout)

    def test_declaration_without_a_reason_fails(self) -> None:
        self.fx.append("agentic-docs/notes.md", "more notes\n")
        self.fx.commit("docs: extend the notes")
        out = self.fx.run(body="Release-Impact: none\n")
        self.assertEqual(out.returncode, 1, out.stdout)
        self.assertIn("carries no reason", out.stdout)

    def test_declaration_may_come_from_a_commit_message(self) -> None:
        """Local runs have no PR body; the trailer has to be enough."""
        self.fx.append("agentic-docs/notes.md", "more notes\n")
        self.fx.commit("docs: extend the notes\n\nRelease-Impact: none — docs only\n")
        out = self.fx.run()
        self.assertEqual(out.returncode, 0, out.stdout)

    # --------------------------------------------------------------------- the changelog
    def test_bump_without_changelog_entry_fails(self) -> None:
        self.fx.append("packages/core/src/index.js", "export const extra = 2;\n")
        self.fx.commit("feat(core): add extra")
        out = self.fx.run(body="Release-Impact: minor — adds a public export\n")
        self.assertEqual(out.returncode, 1, out.stdout)
        self.assertIn("nothing was added under", out.stdout)

    def test_bump_with_changelog_entry_passes(self) -> None:
        self.fx.append("packages/core/src/index.js", "export const extra = 2;\n")
        self.fx.add_unreleased_entry("### Added\n\n- A second export.")
        self.fx.commit("feat(core): add extra")
        out = self.fx.run(body="Release-Impact: minor — adds a public export\n")
        self.assertEqual(out.returncode, 0, out.stdout)

    def test_none_with_a_changelog_entry_fails(self) -> None:
        """The contradiction: nothing to release, yet something worth telling users."""
        self.fx.add_unreleased_entry("### Fixed\n\n- A real user-visible fix.")
        self.fx.commit("fix: something")
        out = self.fx.run(body="Release-Impact: none — internal only\n")
        self.assertEqual(out.returncode, 1, out.stdout)
        self.assertIn("adds a user-facing changelog entry", out.stdout)

    def test_rewording_an_existing_entry_is_not_a_new_entry(self) -> None:
        """A diff-based check would read a reflow of neighbouring text as an entry."""
        body = (self.fx.root / "CHANGELOG.md").read_text(encoding="utf-8")
        self.fx.write("CHANGELOG.md", body.replace("First public version.", "First version."))
        self.fx.append("packages/core/src/index.js", "export const extra = 2;\n")
        self.fx.commit("feat(core): add extra")
        out = self.fx.run(body="Release-Impact: patch — tweak\n")
        self.assertEqual(out.returncode, 1, out.stdout)
        self.assertIn("nothing was added under", out.stdout)

    # ---------------------------------------------------------------- the product surface
    def test_none_on_a_docs_only_change_passes_without_warning(self) -> None:
        self.fx.append("agentic-docs/notes.md", "more notes\n")
        self.fx.commit("docs: extend the notes")
        out = self.fx.run(body="Release-Impact: none — harness docs only\n")
        self.assertEqual(out.returncode, 0, out.stdout)
        self.assertIn("0 warning(s)", out.stdout)

    def test_none_touching_the_published_surface_warns(self) -> None:
        self.fx.append("packages/core/src/index.js", "// a comment\n")
        self.fx.commit("refactor(core): comment")
        out = self.fx.run(body="Release-Impact: none — comment only, identical output\n")
        self.assertEqual(out.returncode, 0, out.stdout)
        self.assertIn("published file(s) changed", out.stdout)

    def test_rust_and_root_readme_count_as_published(self) -> None:
        """Neither appears in the package manifest; both change what ships."""
        for path, text in (("crates/dsp/src/lib.rs", "pub fn extra() {}\n"),
                           ("README.md", "\nA new paragraph.\n")):
            fx = Fixture()
            self.addCleanup(fx.close)
            fx.append(path, text)
            fx.commit("chore: touch " + path)
            out = fx.run(body="Release-Impact: none — no behaviour change\n")
            self.assertIn("published file(s) changed", out.stdout, f"{path}: {out.stdout}")

    def test_private_package_is_not_the_published_surface(self) -> None:
        self.fx.append("packages/tooling/package.json", "")
        self.fx.write("packages/tooling/index.js", "// internal\n")
        self.fx.commit("chore(tooling): internal helper")
        out = self.fx.run(body="Release-Impact: none — private workspace package\n")
        self.assertEqual(out.returncode, 0, out.stdout)
        self.assertIn("0 warning(s)", out.stdout)

    # ------------------------------------------------------------------- the version bump
    def test_version_bump_without_its_changelog_section_fails(self) -> None:
        self.fx.set_version("0.2.0")
        self.fx.add_unreleased_entry("### Added\n\n- A thing.")
        self.fx.commit("chore(release): 0.2.0")
        out = self.fx.run(body="Release-Impact: minor — cuts 0.2.0\n")
        self.assertEqual(out.returncode, 1, out.stdout)
        self.assertIn("no `## [0.2.0]` section", out.stdout)

    def test_version_bump_disagreeing_with_the_declaration_fails(self) -> None:
        self.fx.set_version("0.2.0")
        self.fx.write("CHANGELOG.md",
                      CHANGELOG.replace("## [Unreleased]\n\nNothing yet.",
                                        "## [Unreleased]\n\n## [0.2.0] — 2026-02-02\n\n- A thing."))
        self.fx.commit("chore(release): 0.2.0")
        out = self.fx.run(body="Release-Impact: patch — small fix\n")
        self.assertEqual(out.returncode, 1, out.stdout)
        self.assertIn("the version moved by a minor", out.stdout)

    def test_major_intent_below_one_point_zero_is_allowed_on_a_minor_bump(self) -> None:
        """Semver's 0.x rule: breaking changes ride the minor. Declaring `major` is honest."""
        self.fx.set_version("0.2.0")
        self.fx.write("CHANGELOG.md",
                      CHANGELOG.replace("## [Unreleased]\n\nNothing yet.",
                                        "## [Unreleased]\n\n## [0.2.0] — 2026-02-02\n\n- Removed X."))
        self.fx.commit("chore(release): 0.2.0")
        out = self.fx.run(body="Release-Impact: major — removes a public method\n")
        self.assertEqual(out.returncode, 0, out.stdout)

    # ----------------------------------------------------------------------- degenerate
    def test_no_diff_needs_no_declaration(self) -> None:
        out = self.fx.run(body="")
        self.assertEqual(out.returncode, 0, out.stdout)
        self.assertIn("nothing to declare", out.stdout)


if __name__ == "__main__":
    unittest.main()
