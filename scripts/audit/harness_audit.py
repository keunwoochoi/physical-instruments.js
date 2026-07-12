#!/usr/bin/env python3
"""Validate the repository's agent-facing harness without third-party dependencies."""

from __future__ import annotations

import os
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MARKDOWN_LINK_RE = re.compile(r"(?<!!)\[[^\]]+\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
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
    "## Review focus",
    "## Gates",
    "## Follow-up after merge",
    "## Agentic process trace",
)


class Audit:
    def __init__(self) -> None:
        self.failures: list[str] = []
        self.counts: dict[str, int] = {}

    def require(self, condition: bool, message: str) -> None:
        if not condition:
            self.failures.append(message)

    def count(self, key: str, value: int) -> None:
        self.counts[key] = value


def text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def check_symlink(audit: Audit, relative: str, target: str) -> None:
    path = ROOT / relative
    audit.require(path.is_symlink(), f"{relative} must be a symlink to {target}")
    if path.is_symlink():
        audit.require(os.readlink(path) == target, f"{relative} points to {os.readlink(path)!r}, expected {target!r}")


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


def markdown_files() -> list[Path]:
    paths = [ROOT / "AGENTS.md", ROOT / "PRINCIPLES.md", ROOT / ".github" / "pull_request_template.md"]
    for pattern in ("agentic-docs/**/*.md", "skills/**/*.md", ".claude/commands/*.md"):
        paths.extend(ROOT.glob(pattern))
    return sorted({path for path in paths if path.is_file()})


def check_skill_surface(audit: Audit) -> None:
    skill_dirs = sorted(
        path for path in (ROOT / "skills").iterdir() if path.is_dir() and not path.name.startswith(".")
    )
    command_paths = sorted((ROOT / ".claude" / "commands").glob("*.md"))
    audit.count("skills", len(skill_dirs))
    audit.count("claude_commands", len(command_paths))

    for skill_dir in skill_dirs:
        skill_md = skill_dir / "SKILL.md"
        relative = skill_md.relative_to(ROOT)
        audit.require(skill_md.is_file(), f"{relative} is missing")
        if not skill_md.is_file():
            continue
        values, error = parse_frontmatter(skill_md)
        audit.require(error is None, f"{relative}: {error}")
        audit.require(values.get("name") == skill_dir.name, f"{relative}: frontmatter name must be {skill_dir.name!r}")
        audit.require(bool(values.get("description")), f"{relative}: frontmatter description must be non-empty")
        command = ROOT / ".claude" / "commands" / f"{skill_dir.name}.md"
        audit.require(command.is_file(), f"missing thin Claude forwarder {command.relative_to(ROOT)}")

    for command in command_paths:
        name = command.stem
        nonblank = [line for line in text(command).splitlines() if line.strip()]
        audit.require((ROOT / "skills" / name / "SKILL.md").is_file(), f"orphan Claude command {command.relative_to(ROOT)}")
        audit.require(len(nonblank) <= 5, f"{command.relative_to(ROOT)} must stay at or below 5 nonblank lines")
        audit.require(f"skills/{name}/SKILL.md" in text(command), f"{command.relative_to(ROOT)} must forward to skills/{name}/SKILL.md")


def check_links(audit: Audit) -> None:
    checked = 0
    root_resolved = ROOT.resolve()
    for path in markdown_files():
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
                audit.require(False, f"{path.relative_to(ROOT)} links outside the repository: {link}")
                continue
            audit.require(target.exists(), f"{path.relative_to(ROOT)} has a broken local link: {link}")

    agents_text = text(ROOT / "AGENTS.md")
    for token in re.findall(r"`([^`]+)`", agents_text):
        if not token.startswith(ROUTE_PREFIXES) or any(character.isspace() for character in token):
            continue
        checked += 1
        audit.require((ROOT / token).exists(), f"AGENTS.md routes to missing path: {token}")
    audit.count("local_links", checked)


def check_authority(audit: Audit) -> None:
    agents_text = text(ROOT / "AGENTS.md")
    for required in AUTHORITY_DEFAULTS:
        audit.require(required in agents_text, f"AGENTS.md is missing authority default: {required}")

    for path in markdown_files():
        for line_number, line in enumerate(text(path).splitlines(), start=1):
            audit.require(not AUTHORITY_LIFT_RE.search(line), f"{path.relative_to(ROOT)}:{line_number} appears to lift an authority default")


def check_github_templates(audit: Audit) -> None:
    template_dir = ROOT / ".github" / "ISSUE_TEMPLATE"
    audit.count("issue_forms", len(ISSUE_FORMS))
    config = template_dir / "config.yml"
    audit.require(config.is_file(), ".github/ISSUE_TEMPLATE/config.yml is missing")
    if config.is_file():
        audit.require("blank_issues_enabled: false" in text(config), "blank GitHub issues must remain disabled")

    for filename, title_line in ISSUE_FORMS.items():
        path = template_dir / filename
        audit.require(path.is_file(), f"missing GitHub issue form {path.relative_to(ROOT)}")
        if not path.is_file():
            continue
        content = text(path)
        audit.require(title_line in content, f"{path.relative_to(ROOT)} must use title prefix {title_line}")
        for field_id in ISSUE_FIELD_IDS:
            audit.require(f"id: {field_id}" in content, f"{path.relative_to(ROOT)} is missing field id {field_id!r}")

    pr_template = ROOT / ".github" / "pull_request_template.md"
    audit.require(pr_template.is_file(), ".github/pull_request_template.md is missing")
    if pr_template.is_file():
        content = text(pr_template)
        for heading in PR_HEADINGS:
            audit.require(heading in content, f"pull request template is missing heading {heading!r}")
        audit.require("Closes #" in content, "pull request template must link its source issue with `Closes #`")
        audit.require("PR title: type(scope): imperative summary" in content, "pull request template must state the title convention")


def check_owner_boundaries(audit: Audit) -> None:
    todo_files = sorted((ROOT / ".claude").glob("TODO-*.md"))
    audit.require(not todo_files, "live work belongs in GitHub; remove tracked .claude/TODO-*.md files")
    for path in markdown_files():
        audit.require(".claude/TODO-" not in text(path), f"{path.relative_to(ROOT)} still routes live work to a local TODO")
    agents_text = text(ROOT / "AGENTS.md")
    docs_text = text(ROOT / "agentic-docs" / "docs-organization.md")
    audit.require("GitHub Issues and pull requests own work state and evidence" in agents_text, "AGENTS.md must assign live work state and evidence to GitHub")
    audit.require("Do not create local TODO, backlog, plan-status, or per-PR decision-log files" in docs_text, "docs organization must reject parallel local work-state files")


def check_domain_invariants(audit: Audit) -> None:
    for persona in ("keunwoo", "hayoung", "yotam", "juhan", "jordan", "senior-web-dev", "producer"):
        audit.require((ROOT / "skills" / "review-as" / "references" / f"{persona}.md").is_file(), f"missing operational persona lens {persona}")
        audit.require((ROOT / "agentic-docs" / "personas" / f"{persona}.md").is_file(), f"missing full persona profile {persona}")

    for design_doc in sorted((ROOT / "agentic-docs" / "design").glob("*.md")):
        if design_doc.name == "TEMPLATE.md":
            continue
        audit.require(re.search(r"^Status:", text(design_doc), re.MULTILINE) is not None, f"{design_doc.relative_to(ROOT)} is missing a Status line")

    audit.require("papers-only" in text(ROOT / "agentic-docs" / "licensing.md"), "licensing owner is missing the papers-only clean-room policy")


def main() -> int:
    os.chdir(ROOT)
    audit = Audit()
    check_symlink(audit, "CLAUDE.md", "AGENTS.md")
    check_symlink(audit, ".agents/skills", "../skills")
    check_symlink(audit, ".claude/skills", "../skills")
    check_skill_surface(audit)
    check_links(audit)
    check_authority(audit)
    check_github_templates(audit)
    check_owner_boundaries(audit)
    check_domain_invariants(audit)

    if audit.failures:
        print("AUDIT FAIL: agent harness validation failed", file=sys.stderr)
        for failure in audit.failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    counts = ", ".join(f"{key}={value}" for key, value in sorted(audit.counts.items()))
    print(f"harness-audit: OK ({counts})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
