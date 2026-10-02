#!/usr/bin/env python3
"""Extract one version section from the Chinese release notes."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


def extract(text: str, version: str) -> str:
    normalized = version.strip().removeprefix("v")
    if not normalized:
        raise ValueError("版本号不能为空")
    heading = re.compile(rf"^##\s+v{re.escape(normalized)}(?:\s|$)", re.MULTILINE)
    start_match = heading.search(text)
    if start_match is None:
        raise ValueError(f"RELEASE.md 中没有 v{normalized} 小节")
    end_match = re.search(r"^##\s+v", text[start_match.end() :], re.MULTILINE)
    end = start_match.end() + end_match.start() if end_match else len(text)
    section = text[start_match.start() : end].strip()
    if not section:
        raise ValueError(f"v{normalized} 小节为空")
    return section + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("notes", type=Path)
    parser.add_argument("version")
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        section = extract(args.notes.read_text(encoding="utf-8"), args.version)
        args.output.write_text(section, encoding="utf-8")
    except (OSError, ValueError) as error:
        print(error, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
