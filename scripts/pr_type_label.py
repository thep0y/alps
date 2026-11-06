#!/usr/bin/env python3
"""Resolve a PR type label from the head branch prefix or PR title.

Branch names follow the repository conventions (feat/, fix/, docs/, ...).
If the branch has no known prefix, fall back to a Conventional Commits title.
"""

from __future__ import annotations

import argparse
import re

BRANCH_LABELS = {
    "feat": "type: feature",
    "feature": "type: feature",
    "fix": "type: bug",
    "hotfix": "type: bug",
    "docs": "type: documentation",
    "ci": "type: ci",
    "refactor": "type: refactor",
    "test": "type: test",
    "chore": "type: chore",
    "release": "type: release",
}

TITLE_LABELS = {
    "feat": "type: feature",
    "feature": "type: feature",
    "fix": "type: bug",
    "bugfix": "type: bug",
    "hotfix": "type: bug",
    "docs": "type: documentation",
    "doc": "type: documentation",
    "ci": "type: ci",
    "build": "type: ci",
    "refactor": "type: refactor",
    "test": "type: test",
    "tests": "type: test",
    "chore": "type: chore",
    "deps": "type: chore",
    "release": "type: release",
}

TITLE = re.compile(r"^(?P<kind>[A-Za-z]+)(?:\([^)]*\))?!?:", re.IGNORECASE)


def resolve(branch: str, title: str) -> str | None:
    prefix = branch.split("/", 1)[0].lower()
    if prefix in BRANCH_LABELS:
        return BRANCH_LABELS[prefix]

    match = TITLE.match(title.strip())
    if match:
        return TITLE_LABELS.get(match.group("kind").lower())
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--branch", required=True)
    parser.add_argument("--title", required=True)
    args = parser.parse_args()
    print(resolve(args.branch, args.title) or "")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
