#!/usr/bin/env python3
"""Bump the ALPS version in Cargo.toml and Cargo.lock.

Two independent inputs:
  --bump  patch | minor | major   target core version
  --kind  stable | alpha | beta   release type

For stable versions:
  patch -> patch + 1
  minor -> minor + 1
  major -> major + 1

For an existing pre-release, patch targets the current core, so it can
increment/switch a pre-release or (with --kind stable) finalize it:
  0.1.1-alpha.1 + patch/alpha  -> 0.1.1-alpha.2
  0.1.1-alpha.1 + patch/beta   -> 0.1.1-beta.1
  0.1.1-alpha.1 + patch/stable -> 0.1.1
  0.1.1-alpha.1 + minor/beta   -> 0.2.0-beta.1

A stable version never guesses the pre-release core: --kind alpha/beta is
combined with an explicit --bump. This script only edits files; the release
workflow validates with cargo, commits to main, tags and publishes.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

SEMVER = re.compile(
    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-([0-9A-Za-z.-]+))?$"
)
CARGO_VERSION = re.compile(r'(?m)^version\s*=\s*"([^"]+)"')
ALPS_LOCK = re.compile(
    r'(?ms)(\[\[package\]\]\nname = "alps"\nversion = ")[^"]+(")'
)


def parse(version: str) -> tuple[int, int, int, str | None]:
    match = SEMVER.match(version)
    if not match:
        raise ValueError(f"not a supported semantic version: {version!r}")
    return (
        int(match.group(1)),
        int(match.group(2)),
        int(match.group(3)),
        match.group(4),
    )


def target_core(
    major: int, minor: int, patch: int, has_pre: bool, bump: str
) -> tuple[int, int, int]:
    if bump == "patch":
        if has_pre:
            return (major, minor, patch)
        return (major, minor, patch + 1)
    if bump == "minor":
        return (major, minor + 1, 0)
    if bump == "major":
        return (major + 1, 0, 0)
    raise ValueError(f"unsupported bump kind: {bump}")


def bump(version: str, bump: str, kind: str) -> str:
    major, minor, patch, pre = parse(version)
    core = target_core(major, minor, patch, pre is not None, bump)
    core_text = f"{core[0]}.{core[1]}.{core[2]}"

    if kind == "stable":
        return core_text

    if kind not in ("alpha", "beta"):
        raise ValueError(f"unsupported release kind: {kind}")

    if pre is not None:
        identifier, _, suffix = pre.partition(".")
        if identifier == kind and (major, minor, patch) == core:
            number = int(suffix) if suffix.isdigit() else 0
            return f"{core_text}-{kind}.{number + 1}"

    return f"{core_text}-{kind}.1"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bump", required=True, choices=["patch", "minor", "major"])
    parser.add_argument("--kind", required=True, choices=["stable", "alpha", "beta"])
    parser.add_argument("--write", action="store_true", help="write Cargo.toml and Cargo.lock")
    args = parser.parse_args()

    root = Path(__file__).resolve().parents[1]
    cargo_toml = root / "Cargo.toml"
    cargo_lock = root / "Cargo.lock"

    manifest = cargo_toml.read_text(encoding="utf-8")
    match = CARGO_VERSION.search(manifest)
    if not match:
        print("Cargo.toml has no package version", file=sys.stderr)
        return 2

    current = match.group(1)
    new_version = bump(current, args.bump, args.kind)

    if args.write:
        manifest = CARGO_VERSION.sub(
            f'version = "{new_version}"', manifest, count=1
        )
        cargo_toml.write_text(manifest, encoding="utf-8")

        lock = cargo_lock.read_text(encoding="utf-8")
        if len(ALPS_LOCK.findall(lock)) != 1:
            print("Cargo.lock does not contain exactly one alps package", file=sys.stderr)
            return 2
        lock = ALPS_LOCK.sub(rf'\g<1>{new_version}\g<2>', lock, count=1)
        cargo_lock.write_text(lock, encoding="utf-8")

    print(new_version)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValueError as error:
        print(error, file=sys.stderr)
        raise SystemExit(2)
