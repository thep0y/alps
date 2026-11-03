#!/usr/bin/env python3
"""Generate categorized release notes from merged pull requests.

The commit range starts at the previous GitHub release tag (or the repository
root for the first release). Each commit is mapped back to its merged pull
request through the GitHub API, and the PR title is classified using
Conventional Commits prefixes. Commit messages alone are not used because
squash/merge commits do not consistently preserve the PR title.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass
from urllib.parse import quote

CONVENTIONAL = re.compile(
    r"^(?P<kind>[A-Za-z]+)(?:\((?P<scope>[^)]*)\))?(?P<breaking>!)?:\s*(?P<summary>.+)$"
)

CATEGORY_ORDER = [
    ("breaking", "⚠ Breaking Changes"),
    ("feat", "🚀 Features"),
    ("fix", "🐛 Bug Fixes"),
    ("perf", "⚡ Performance"),
    ("refactor", "♻ Refactoring"),
    ("docs", "📝 Documentation"),
    ("test", "🧪 Tests"),
    ("build", "🏗 Build"),
    ("ci", "⚙ CI"),
    ("chore", "🧹 Chores"),
    ("revert", "⏪ Reverts"),
    ("other", "📦 Other Changes"),
]

TYPE_MAP = {
    "feat": "feat",
    "feature": "feat",
    "fix": "fix",
    "bugfix": "fix",
    "hotfix": "fix",
    "perf": "perf",
    "refactor": "refactor",
    "docs": "docs",
    "doc": "docs",
    "test": "test",
    "tests": "test",
    "build": "build",
    "ci": "ci",
    "chore": "chore",
    "deps": "chore",
    "release": "chore",
    "revert": "revert",
}


@dataclass
class Entry:
    number: int
    summary: str
    scope: str | None
    category: str
    url: str
    author: str


def run(args: list[str]) -> str:
    result = subprocess.run(args, capture_output=True, text=True)
    if result.returncode != 0:
        raise SystemExit(f"{' '.join(args)} failed: {result.stderr.strip()}")
    return result.stdout.strip()


def gh_json(args: list[str]) -> object:
    return json.loads(run(["gh", *args]))


def git(*args: str) -> str:
    return run(["git", *args])


def detect_repo(explicit: str | None) -> str:
    if explicit:
        return explicit
    if repo := os.environ.get("GITHUB_REPOSITORY"):
        return repo
    data = gh_json(["repo", "view", "--json", "nameWithOwner"])
    return data["nameWithOwner"]


def previous_release(repo: str, current: str) -> str | None:
    releases = gh_json(
        [
            "release",
            "list",
            "--exclude-drafts",
            "--limit",
            "100",
            "--json",
            "tagName,createdAt,isPrerelease",
        ]
    )
    for release in releases:
        if release["tagName"] != current:
            return release["tagName"]
    return None


def classify(title: str, body: str) -> tuple[str, str, str | None]:
    title = " ".join(title.split())
    match = CONVENTIONAL.match(title)
    kind = "other"
    scope = None
    summary = title

    if match:
        kind = match.group("kind").lower()
        scope = (match.group("scope") or "").strip() or None
        summary = match.group("summary").strip() or title

    body_upper = (body or "").upper()
    breaking = bool(match and match.group("breaking")) or (
        "BREAKING CHANGE:" in body_upper or "BREAKING-CHANGE:" in body_upper
    )
    category = "breaking" if breaking else TYPE_MAP.get(kind, "other")
    return category, summary, scope


def escape_markdown(value: str) -> str:
    value = " ".join(value.split())
    for character in ("\\", "[", "]", "*", "_", "`", "<", ">"):
        value = value.replace(character, "\\" + character)
    return value


def collect_entries(repo: str, previous: str | None, head: str) -> list[Entry]:
    if previous:
        commits = git("rev-list", "--reverse", f"{previous}..{head}").splitlines()
    else:
        commits = git("rev-list", "--reverse", head).splitlines()

    entries: list[Entry] = []
    seen: set[int] = set()

    for sha in commits:
        pulls = gh_json(["api", f"repos/{repo}/commits/{sha}/pulls"])
        for pull in pulls:
            if not pull.get("merged_at"):
                continue
            if (pull.get("base") or {}).get("ref") != "main":
                continue
            number = int(pull["number"])
            if number in seen:
                continue
            seen.add(number)
            category, summary, scope = classify(
                pull.get("title") or "", pull.get("body") or ""
            )
            author = ((pull.get("user") or {}).get("login")) or ""
            entries.append(
                Entry(
                    number=number,
                    summary=summary,
                    scope=scope,
                    category=category,
                    url=pull.get("html_url") or "",
                    author=author,
                )
            )

    entries.sort(key=lambda entry: entry.number)
    return entries


def format_entry(entry: Entry) -> str:
    summary = escape_markdown(entry.summary)
    if entry.scope:
        summary = f"**{escape_markdown(entry.scope)}:** {summary}"
    author = f" @{entry.author}" if entry.author else ""
    return f"- {summary} ([#{entry.number}]({entry.url})){author}"


def render(repo: str, tag: str, previous: str | None, entries: list[Entry]) -> str:
    lines = ["## What's Changed", ""]

    if not entries:
        lines.extend(["No merged pull requests were found in this release.", ""])

    grouped: dict[str, list[Entry]] = {key: [] for key, _ in CATEGORY_ORDER}
    for entry in entries:
        grouped[entry.category].append(entry)

    for key, heading in CATEGORY_ORDER:
        group = grouped[key]
        if not group:
            continue
        lines.extend([f"### {heading}", ""])
        lines.extend(format_entry(entry) for entry in group)
        lines.append("")

    if previous:
        compare = f"https://github.com/{repo}/compare/{quote(previous, safe='')}...{quote(tag, safe='')}"
        lines.append(f"**Full Changelog**: {compare}")
    else:
        lines.append(f"**Full Changelog**: https://github.com/{repo}/commits/{quote(tag, safe='')}")

    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True, help="current release tag, for links")
    parser.add_argument("--repo", help="OWNER/REPO (default: GITHUB_REPOSITORY or gh repo view)")
    parser.add_argument("--previous", help="previous release tag (default: latest release)")
    parser.add_argument("--head", default="HEAD", help="commit to describe (default: HEAD)")
    args = parser.parse_args()

    repo = detect_repo(args.repo)
    previous = args.previous or previous_release(repo, args.tag)
    if previous == args.tag:
        raise SystemExit("previous release must differ from the current tag")
    if previous:
        try:
            git("rev-parse", "--verify", f"{previous}^{{commit}}")
        except SystemExit:
            raise SystemExit(
                f"previous tag {previous!r} is not available locally; run git fetch --tags"
            )

    entries = collect_entries(repo, previous, args.head)
    sys.stdout.write(render(repo, args.tag, previous, entries))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
