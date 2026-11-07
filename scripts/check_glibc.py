#!/usr/bin/env python3
"""Fail if a Linux ELF binary requires a newer glibc than the supported baseline."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

VERSION = re.compile(r"GLIBC_(\d+)\.(\d+)(?:\.(\d+))?")


def parse_version(value: str) -> tuple[int, int, int]:
    parts = [int(part) for part in value.split(".")]
    return tuple((parts + [0, 0, 0])[:3])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--max", default="2.18", dest="maximum")
    args = parser.parse_args()

    output = subprocess.check_output(
        ["objdump", "-T", str(args.binary)],
        text=True,
        stderr=subprocess.STDOUT,
    )
    versions = {
        tuple(int(part) if part else 0 for part in match.groups())
        for match in VERSION.finditer(output)
    }
    if not versions:
        print("no glibc version references found", file=sys.stderr)
        return 1

    maximum = max(versions)
    allowed = parse_version(args.maximum)
    rendered = ".".join(str(part) for part in maximum)
    print(f"maximum glibc requirement: {rendered} (allowed <= {args.maximum})")
    if maximum > allowed:
        print(
            f"{args.binary} requires glibc {rendered}, newer than {args.maximum}",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
