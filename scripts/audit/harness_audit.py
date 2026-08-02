#!/usr/bin/env python3
"""Validate the repository's agent-facing harness without third-party dependencies.

An audit that has never been observed to fail is not evidence of anything. Run the
live tree with no args; prove fail-correctly behavior with:

    python3 scripts/audit/test_harness_audit.py
"""

from __future__ import annotations

import argparse
import os
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path


DEFAULT_ROOT = Path(__file__).resolve().parents[2]
MARKDOWN_LINK_RE = re.compile(r"(?<!!)\[[^\]]+\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
PATHISH_RE = re.compile(r"`([^`\n]+)`")
ROUTE_PREFIXES = (".github/", "agentic-docs/", "demos/", "evals/", "packages/", "scripts/", "skills/")
AUTHORITY_DEFAULTS = (
    "- npm publish / GitHub release: **off**",
    "- git push to `main`, force-push, `--no-verify`, self-merge: **never**",
    "- paid or quota-consuming external resources: **off**",
    "- public posts (Show HN, social, docs deploys): **off**",
)
AUTHORITY_LIFT_RE = re.compile(
    r"(?:npm publish / GitHub release|git push to `?main`?[^:]*|force-push[^:]*|--no-verify[^:]*|self-merge[^:]*|paid or quota-consuming external resources|public posts[^:]*)\s*:\s*\*\*(?:on|enabled|true|allowed)\*\*",
    re.IGNORECASE,
)
ISSUE_FORMS = {
    "bug.yml": 'title: "fix(scope): "',
    "feature.yml": 'title: "feat(scope): "',
    "maintenance.yml": 'title: "chore(scope): "',
}
ISSUE_FIELD_IDS = ("motivation", "evidence", "outcome", "scope", "acceptance", "validation")
PR_HEADINGS = (
    "## Motivation",
    "## Impact",
    "## Summary",
    "## Validation",
    "## Evidence freshness",
    "## Review focus",
    "## Gates",
    "## Follow-up after merge",
    "## Agentic process trace",
)
GITHUB_ACTOR = "keunwoochoi"
SKIP_DIR_NAMES = {".git", "fixtures", "node_modules", "target", "dist", ".worktrees"}


@dataclass
class Report:
    failures: list[str] = field(default_factory=list)
    counts: dict[str, int] = field(default_factory=dict)
    checks_run: int = 0

    def require(self, condition: bool, ident: str, message: str) -> None:
        self.checks_run += 1
        if not condition:
            self.failures.append(f"{ident}: {message}")

    def count(self, key: str, value: int) -> None:
        self.counts[key] = value


def text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return ""


def looks_like_path(tok: str) -> bool:
    if " " in tok or tok.startswith(("http", "@", "$", "-", "npm ", "git ", "#")):
        return False
    if tok.endswith("/"):
        return True
    if "/" in tok and re.search(r"\.(md|sh|py|toml|json|rs|ts|js|mjs|yml|yaml)$", tok):
        return True
    if tok.startswith(ROUTE_PREFIXES):
        return True
    return bool(re.fullmatch(r"[A-Z][A-Z0-9_.-]*\.md", tok))


def markdown_files(root: Path) -> list[Path]:
    paths = [root / "AGENTS.md", root / "PRINCIPLES.md", root / ".github" / "pull_request_template.md"]
    for pattern in ("agentic-docs/**/*.md", "skills/**/*.md", ".claude/commands/*.md"):
        paths.extend(root.glob(pattern))
    return sorted({path for path in paths if path.is_file()})


def is_fixture_root(root: Path) -> bool:
    return (root / ".fixture-root").is_file() or "fixtures" in root.resolve().parts


def check_symlink(report: Report, root: Path, relative: str, target: str) -> None:
    path = root / relative
    report.require(path.is_symlink(), "C-SYMLINK", f"{relative} must be a symlink to {target}")
    if path.is_symlink():
        report.require(
            os.readlink(path) == target,
            "C-SYMLINK",
            f"{relative} points to {os.readlink(path)!r}, expected {target!r}",
        )


def parse_frontmatter(path: Path) -> tuple[dict[str, str], str | None]:
    lines = text(path).splitlines()
    if not lines or lines[0].strip() != "---":
        return {}, "missing leading YAML frontmatter"
    try:
        end = next(index for index, line in enumerate(lines[1:], start=1) if line.strip() == "---")
    except StopIteration:
        return {}, "missing closing YAML frontmatter fence"
    values: dict[str, str] = {}
    for line in lines[1:end]:
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if line[0].isspace() or ":" not in line:
            return values, f"malformed top-level frontmatter line: {line!r}"
        key, value = line.split(":", 1)
        values[key.strip()] = value.strip().strip("\"'")
    return values, None


def check_operating_surface_paths(report: Report, root: Path) -> None:
    """C1/C3: every path cited on AGENTS.md exists; cited directories are non-empty."""
    agents = root / "AGENTS.md"
    report.require(agents.is_file(), "C1-PATHS", "AGENTS.md is missing from the operating surface")
    if not agents.is_file():
        return
    body = text(agents)
    exempt = {
        i
        for i, line in enumerate(body.splitlines())
        if re.search(r"does not exist|not exist|never typed|read the tree", line, re.I)
    }
    checked = 0
    for i, line in enumerate(body.splitlines()):
        if i in exempt:
            continue
        for tok in PATHISH_RE.findall(line):
            if not looks_like_path(tok):
                continue
            target = root / tok.rstrip("/")
            checked += 1
            if not target.exists():
                report.require(False, "C1-PATHS", f"AGENTS.md:{i + 1} cites `{tok}` which does not exist")
            elif target.is_dir():
                real = [c for c in target.rglob("*") if c.is_file() and c.name != ".gitkeep"]
                report.require(
                    bool(real),
                    "C3-NONEMPTY",
                    f"AGENTS.md:{i + 1} cites directory `{tok}` but it holds no real files",
                )
    report.count("agents_path_citations", checked)


def check_links(report: Report, root: Path) -> None:
    checked = 0
    root_resolved = root.resolve()
    for path in markdown_files(root):
        for raw_link in MARKDOWN_LINK_RE.findall(text(path)):
            link = raw_link.strip("<>")
            if link.startswith(("#", "http://", "https://", "mailto:")):
                continue
            path_part = link.split("#", 1)[0].split("?", 1)[0]
            if not path_part:
                continue
            target = (path.parent / path_part).resolve()
            checked += 1
            try:
                target.relative_to(root_resolved)
            except ValueError:
                report.require(False, "C2-LINKS", f"{path.relative_to(root)} links outside the repository: {link}")
                continue
            report.require(target.exists(), "C2-LINKS", f"{path.relative_to(root)} has a broken local link: {link}")

    agents = root / "AGENTS.md"
    if agents.is_file():
        agents_text = text(agents)
        for token in PATHISH_RE.findall(agents_text):
            if not token.startswith(ROUTE_PREFIXES) or any(character.isspace() for character in token):
                continue
            checked += 1
            report.require((root / token).exists(), "C2-LINKS", f"AGENTS.md routes to missing path: {token}")
    report.count("local_links", checked)


def check_authority(report: Report, root: Path) -> None:
    agents = root / "AGENTS.md"
    agents_text = text(agents) if agents.is_file() else ""
    for required in AUTHORITY_DEFAULTS:
        report.require(required in agents_text, "C-AUTHORITY", f"AGENTS.md is missing authority default: {required}")

    for path in markdown_files(root):
        for line_number, line in enumerate(text(path).splitlines(), start=1):
            report.require(
                not AUTHORITY_LIFT_RE.search(line),
                "C-AUTHORITY",
                f"{path.relative_to(root)}:{line_number} appears to lift an authority default",
            )


def check_skill_surface(report: Report, root: Path) -> None:
    skills_root = root / "skills"
    report.require(skills_root.is_dir(), "C-SKILLS", "skills/ directory is missing")
    if not skills_root.is_dir():
        return
    skill_dirs = sorted(path for path in skills_root.iterdir() if path.is_dir() and not path.name.startswith("."))
    command_paths = sorted((root / ".claude" / "commands").glob("*.md")) if (root / ".claude" / "commands").is_dir() else []
    report.count("skills", len(skill_dirs))
    report.count("claude_commands", len(command_paths))

    for skill_dir in skill_dirs:
        skill_md = skill_dir / "SKILL.md"
        relative = skill_md.relative_to(root)
        report.require(skill_md.is_file(), "C-SKILLS", f"{relative} is missing")
        if not skill_md.is_file():
            continue
        values, error = parse_frontmatter(skill_md)
        report.require(error is None, "C-SKILLS", f"{relative}: {error}")
        report.require(values.get("name") == skill_dir.name, "C-SKILLS", f"{relative}: frontmatter name must be {skill_dir.name!r}")
        report.require(bool(values.get("description")), "C-SKILLS", f"{relative}: frontmatter description must be non-empty")
        command = root / ".claude" / "commands" / f"{skill_dir.name}.md"
        report.require(command.is_file(), "C-SKILLS", f"missing thin Claude forwarder {command.relative_to(root)}")

    for command in command_paths:
        name = command.stem
        nonblank = [line for line in text(command).splitlines() if line.strip()]
        report.require((root / "skills" / name / "SKILL.md").is_file(), "C-SKILLS", f"orphan Claude command {command.relative_to(root)}")
        report.require(len(nonblank) <= 5, "C-SKILLS", f"{command.relative_to(root)} must stay at or below 5 nonblank lines")
        report.require(
            f"skills/{name}/SKILL.md" in text(command),
            "C-SKILLS",
            f"{command.relative_to(root)} must forward to skills/{name}/SKILL.md",
        )


def check_github_templates(report: Report, root: Path) -> None:
    template_dir = root / ".github" / "ISSUE_TEMPLATE"
    report.count("issue_forms", len(ISSUE_FORMS))
    config = template_dir / "config.yml"
    report.require(config.is_file(), "C-GITHUB-TEMPLATES", ".github/ISSUE_TEMPLATE/config.yml is missing")
    if config.is_file():
        report.require(
            "blank_issues_enabled: false" in text(config),
            "C-GITHUB-TEMPLATES",
            "blank GitHub issues must remain disabled",
        )

    for filename, title_line in ISSUE_FORMS.items():
        path = template_dir / filename
        report.require(path.is_file(), "C-GITHUB-TEMPLATES", f"missing GitHub issue form {path.relative_to(root)}")
        if not path.is_file():
            continue
        content = text(path)
        report.require(title_line in content, "C-GITHUB-TEMPLATES", f"{path.relative_to(root)} must use title prefix {title_line}")
        for field_id in ISSUE_FIELD_IDS:
            report.require(f"id: {field_id}" in content, "C-GITHUB-TEMPLATES", f"{path.relative_to(root)} is missing field id {field_id!r}")

    pr_template = root / ".github" / "pull_request_template.md"
    report.require(pr_template.is_file(), "C-GITHUB-TEMPLATES", ".github/pull_request_template.md is missing")
    if pr_template.is_file():
        content = text(pr_template)
        for heading in PR_HEADINGS:
            report.require(heading in content, "C-GITHUB-TEMPLATES", f"pull request template is missing heading {heading!r}")
        report.require("Closes #" in content, "C-GITHUB-TEMPLATES", "pull request template must link its source issue with `Closes #`")
        report.require(
            "PR title: type(scope): imperative summary" in content,
            "C-GITHUB-TEMPLATES",
            "pull request template must state the title convention",
        )
        report.require(
            "Exact current head SHA" in content,
            "C-GITHUB-TEMPLATES",
            "pull request template must bind evidence to the exact current head SHA",
        )
        report.require(
            "git rev-parse HEAD" in content and "headRefOid" in content,
            "C-GITHUB-TEMPLATES",
            "pull request template must require programmatic local/GitHub head verification",
        )
        report.require(
            "current-head or explicitly labeled historical" in content,
            "C-GITHUB-TEMPLATES",
            "pull request template must gate evidence freshness",
        )


def check_github_identity(report: Report, root: Path) -> None:
    agents_text = text(root / "AGENTS.md")
    wrapper = root / "scripts" / "github.sh"
    report.require(
        f"permits only the `{GITHUB_ACTOR}` account" in agents_text,
        "C-IDENTITY",
        f"AGENTS.md must name {GITHUB_ACTOR} as the only permitted GitHub actor",
    )
    report.require(
        "every GitHub CLI operation through `scripts/github.sh`" in agents_text,
        "C-IDENTITY",
        "AGENTS.md must route GitHub CLI operations through scripts/github.sh",
    )
    report.require("never invoke bare `gh`" in agents_text, "C-IDENTITY", "AGENTS.md must forbid reliance on gh's mutable global account")
    report.require(wrapper.is_file(), "C-IDENTITY", "missing repository-owned GitHub identity wrapper scripts/github.sh")
    if wrapper.is_file():
        report.require(os.access(wrapper, os.X_OK), "C-IDENTITY", "scripts/github.sh must be executable")
        wrapper_text = text(wrapper)
        report.require(f'EXPECTED_GITHUB_ACTOR="{GITHUB_ACTOR}"' in wrapper_text, "C-IDENTITY", f"scripts/github.sh must pin {GITHUB_ACTOR}")
        report.require(
            "auth token" in wrapper_text and '--user "$EXPECTED_GITHUB_ACTOR"' in wrapper_text,
            "C-IDENTITY",
            "scripts/github.sh must select the pinned actor credential explicitly",
        )
        report.require("api user --jq .login" in wrapper_text, "C-IDENTITY", "scripts/github.sh must verify the resolved GitHub actor")
        report.require('exec "$gh_bin" "$@"' in wrapper_text, "C-IDENTITY", "scripts/github.sh must delegate only after identity verification")


def check_owner_boundaries(report: Report, root: Path) -> None:
    todo_files = sorted((root / ".claude").glob("TODO-*.md")) if (root / ".claude").is_dir() else []
    report.require(not todo_files, "C-OWNER", "live work belongs in GitHub; remove tracked .claude/TODO-*.md files")
    for path in markdown_files(root):
        report.require(".claude/TODO-" not in text(path), "C-OWNER", f"{path.relative_to(root)} still routes live work to a local TODO")
    agents_text = text(root / "AGENTS.md")
    docs_org = root / "agentic-docs" / "docs-organization.md"
    report.require(
        "GitHub Issues and pull requests own work state and evidence" in agents_text,
        "C-OWNER",
        "AGENTS.md must assign live work state and evidence to GitHub",
    )
    if docs_org.is_file():
        report.require(
            "Do not create local TODO, backlog, plan-status, or per-PR decision-log files" in text(docs_org),
            "C-OWNER",
            "docs organization must reject parallel local work-state files",
        )


def check_domain_invariants(report: Report, root: Path) -> None:
    agents_text = text(root / "AGENTS.md")
    wrap_path = root / "skills" / "wrap-session" / "SKILL.md"
    finalize_path = root / "skills" / "finalize-pr" / "SKILL.md"
    report.require(wrap_path.is_file(), "C-DOMAIN", "missing canonical wrap-session skill")
    report.require(
        any("Progressive or stacked session wrap" in line and "skills/wrap-session/SKILL.md" in line for line in agents_text.splitlines()),
        "C-DOMAIN",
        "AGENTS.md must route progressive session wrap-up to wrap-session",
    )
    report.require("Evidence is immutable-input-bound" in agents_text, "C-DOMAIN", "AGENTS.md must state the exact-head evidence freshness rule")
    if wrap_path.is_file():
        wrap_text = text(wrap_path)
        for required in ("Reconcile evidence freshness", "Synchronize published stacks without rewriting them", "Debrief in the tracker"):
            report.require(required in wrap_text, "C-DOMAIN", f"wrap-session skill is missing required phase {required!r}")
        report.require("never hand-transcribe" in wrap_text, "C-DOMAIN", "wrap-session must derive exact heads programmatically")
        report.require("Never rebase a published branch" in wrap_text, "C-DOMAIN", "wrap-session must forbid rewriting published branch history")
        report.require("label it historical" in wrap_text, "C-DOMAIN", "wrap-session must preserve stale evidence as explicitly historical")
    report.require(finalize_path.is_file(), "C-DOMAIN", "missing finalize-pr skill")
    if finalize_path.is_file():
        finalize_text = text(finalize_path)
        report.require("Prove evidence freshness" in finalize_text, "C-DOMAIN", "finalize-pr must audit exact-head evidence freshness")
        report.require(
            "git rev-parse HEAD" in finalize_text and "headRefOid" in finalize_text,
            "C-DOMAIN",
            "finalize-pr must verify programmatic local/GitHub head identity",
        )
        report.require("proved byte-identical" in finalize_text, "C-DOMAIN", "finalize-pr must require proof before carrying evidence forward")

    for persona in ("keunwoo", "hayoung", "yotam", "juhan", "jordan", "senior-web-dev", "producer"):
        report.require(
            (root / "skills" / "review-as" / "references" / f"{persona}.md").is_file(),
            "C-DOMAIN",
            f"missing operational persona lens {persona}",
        )
        report.require(
            (root / "agentic-docs" / "personas" / f"{persona}.md").is_file(),
            "C-DOMAIN",
            f"missing full persona profile {persona}",
        )

    design_dir = root / "agentic-docs" / "design"
    if design_dir.is_dir():
        for design_doc in sorted(design_dir.glob("*.md")):
            if design_doc.name == "TEMPLATE.md":
                continue
            report.require(
                re.search(r"^Status:", text(design_doc), re.MULTILINE) is not None,
                "C-DOMAIN",
                f"{design_doc.relative_to(root)} is missing a Status line",
            )

    licensing = root / "agentic-docs" / "licensing.md"
    if licensing.is_file():
        report.require("papers-only" in text(licensing), "C-DOMAIN", "licensing owner is missing the papers-only clean-room policy")


def audit(root: Path) -> Report:
    """Run harness checks against root.

    Fixture roots (marked with `.fixture-root` or living under `scripts/audit/fixtures/`)
    run only the lightweight surface checks that mini trees can express. The live repo
    runs the full suite.
    """
    root = root.resolve()
    report = Report()
    fixture = is_fixture_root(root)

    check_operating_surface_paths(report, root)
    check_links(report, root)
    check_authority(report, root)

    if fixture:
        return report

    check_symlink(report, root, "CLAUDE.md", "AGENTS.md")
    check_symlink(report, root, ".agents/skills", "../skills")
    check_symlink(report, root, ".claude/skills", "../skills")
    check_skill_surface(report, root)
    check_github_templates(report, root)
    check_github_identity(report, root)
    check_owner_boundaries(report, root)
    check_domain_invariants(report, root)
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=None, help="directory to audit (default: repository root)")
    args = parser.parse_args(argv)
    root = Path(args.root).resolve() if args.root else DEFAULT_ROOT
    os.chdir(root if not is_fixture_root(root) else DEFAULT_ROOT)

    report = audit(root)
    if report.failures:
        print("AUDIT FAIL: agent harness validation failed", file=sys.stderr)
        for failure in report.failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    counts = ", ".join(f"{key}={value}" for key, value in sorted(report.counts.items()))
    print(f"harness-audit: OK ({counts}; checks_run={report.checks_run})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
