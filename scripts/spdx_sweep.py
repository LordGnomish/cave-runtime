#!/usr/bin/env python3
"""SPDX header sweep — prepends AGPL-3.0-or-later SPDX line to .rs files
under crates/*/src and crates/*/tests when missing in the first 8 lines.

Safe to re-run (idempotent). Reports counts.
"""
from __future__ import annotations

import sys
from pathlib import Path

HEADER = "// SPDX-License-Identifier: AGPL-3.0-or-later\n"
MARKER = "SPDX-License-Identifier: AGPL-3.0-or-later"
WINDOW = 8


def gather(root: Path) -> list[Path]:
    files: list[Path] = []
    for crate in sorted((root / "crates").iterdir()):
        if not crate.is_dir():
            continue
        for sub in ("src", "tests"):
            base = crate / sub
            if not base.is_dir():
                continue
            files.extend(base.rglob("*.rs"))
    return files


def has_header(text: str) -> bool:
    head = text.splitlines()[:WINDOW]
    return any(MARKER in line for line in head)


def patch(path: Path) -> bool:
    text = path.read_text(encoding="utf-8")
    if has_header(text):
        return False
    # Preserve existing content verbatim; just prepend the SPDX line.
    path.write_text(HEADER + text, encoding="utf-8")
    return True


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    files = gather(root)
    patched = 0
    skipped = 0
    for f in files:
        try:
            if patch(f):
                patched += 1
            else:
                skipped += 1
        except UnicodeDecodeError:
            print(f"non-utf8: {f}", file=sys.stderr)
    print(f"total={len(files)} patched={patched} already_ok={skipped}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
