#!/usr/bin/env python3
"""
Parse `docs/parity/full-audit-2026-05-01.md` into `docs/parity/parity-index.json`.

Output schema:
{
  "generated_from": "docs/parity/full-audit-2026-05-01.md",
  "generated_at": "2026-05-01",
  "crates": {
    "<crate-name>": {
      "tier": "100" | "A" | "B" | "C" | "D1" | "D2" | "E",
      "parity_ratio": float | null,       # null = not measurable
      "manifest_filled": bool | null,     # null = no manifest
      "cave_src_loc": int | null,         # from audit table for tier C/D
      "upstream": "org/repo" | null,
      "upstream_version": str | null,
      "stubs": int | null,
      "note": str | null
    }
  }
}
"""
import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
AUDIT_PATH = REPO_ROOT / "docs" / "parity" / "full-audit-2026-05-01.md"
OUT_PATH = REPO_ROOT / "docs" / "parity" / "parity-index.json"

# Match a markdown table row "| cell1 | cell2 | ... |" -> [cell1, cell2, ...]
ROW_RE = re.compile(r"^\|(.+)\|$")


def split_row(line: str) -> list[str] | None:
    m = ROW_RE.match(line.strip())
    if not m:
        return None
    cells = [c.strip() for c in m.group(1).split("|")]
    # filter out empty leading/trailing cells from edge pipes
    return cells


def is_separator(cells: list[str]) -> bool:
    return all(re.fullmatch(r":?-+:?", c) for c in cells)


def parse_upstream(s: str) -> tuple[str | None, str | None]:
    """`kubernetes/kubernetes @ v1.28.0` -> ('kubernetes/kubernetes', 'v1.28.0')."""
    if not s or s.startswith("("):  # internal placeholders like "(CAVE internal …)"
        return (None, None)
    parts = s.split("@", 1)
    org_repo = parts[0].strip() or None
    version = parts[1].strip() if len(parts) > 1 else None
    return (org_repo, version)


def parse_int_loc(s: str) -> int | None:
    """`36,645` -> 36645."""
    s = s.strip().replace(",", "")
    if not s.isdigit():
        return None
    return int(s)


def parse_overall(s: str) -> float | None:
    """`0.6625 (93.5% items)` -> 0.6625 ;  `1.00` -> 1.0 ;  `0.25` -> 0.25."""
    m = re.match(r"([01]?\.\d+|\d+)", s.strip())
    if not m:
        return None
    try:
        return float(m.group(1))
    except ValueError:
        return None


def find_tables(text: str) -> dict[str, list[list[str]]]:
    """
    Walk the audit, grouping ROW-cell lists by which `## Tier` section we are
    currently in.

    Returns: { "100" | "A" | "B" | "C" | "D1" | "D2" | "E": [ [cells...], ... ] }
    """
    out: dict[str, list[list[str]]] = {}
    section: str | None = None
    skip_header_row = False
    for line in text.splitlines():
        # Pick up section headers
        if line.startswith("## Tier ✅"):
            section = "100"
        elif line.startswith("## Tier A "):
            section = "A"
        elif line.startswith("## Tier B "):
            section = "B"
        elif line.startswith("## Tier C "):
            section = "C"
        elif line.startswith("## Tier D"):
            section = "D"  # subdivided by D1/D2 sub-headers below
        elif line.startswith("## Tier E"):
            section = "E"
        elif line.startswith("### D1"):
            section = "D1"
        elif line.startswith("### D2"):
            section = "D2"
        elif line.startswith("## "):
            section = None  # leave the per-tier capture area
        if section is None:
            continue
        cells = split_row(line)
        if not cells:
            continue
        if is_separator(cells):
            continue
        # First row inside a section is the header — skip it
        # We detect headers by their first cell starting with "Crate" or being one of the known headers
        if cells[0].lower() in ("crate", "bucket"):
            continue
        out.setdefault(section, []).append(cells)
    return out


def parse_audit(md_text: str) -> dict[str, dict]:
    tables = find_tables(md_text)
    crates: dict[str, dict] = {}

    # Tier ✅ — 100% reached
    # Headers: Crate | Upstream | file | fn | test | surface | stubs
    for row in tables.get("100", []):
        if len(row) < 7:
            continue
        name = row[0]
        org_repo, ver = parse_upstream(row[1])
        stubs = parse_int_loc(row[6])
        crates[name] = {
            "tier": "100",
            "parity_ratio": 1.0,
            "manifest_filled": True,
            "cave_src_loc": None,
            "upstream": org_repo,
            "upstream_version": ver,
            "stubs": stubs,
            "note": None,
        }

    # Tier A — Close to 100
    # Headers: Crate | Upstream | overall | gap
    for row in tables.get("A", []):
        if len(row) < 3:
            continue
        name = row[0]
        org_repo, ver = parse_upstream(row[1])
        ratio = parse_overall(row[2])
        crates[name] = {
            "tier": "A",
            "parity_ratio": ratio,
            "manifest_filled": True,
            "cave_src_loc": None,
            "upstream": org_repo,
            "upstream_version": ver,
            "stubs": None,
            "note": row[3] if len(row) > 3 else None,
        }

    # Tier B — partial fill
    # Headers: Crate | Upstream | overall | what's declared / missing
    for row in tables.get("B", []):
        if len(row) < 3:
            continue
        name = row[0]
        org_repo, ver = parse_upstream(row[1])
        ratio = parse_overall(row[2])
        crates[name] = {
            "tier": "B",
            "parity_ratio": ratio,
            "manifest_filled": True,
            "cave_src_loc": None,
            "upstream": org_repo,
            "upstream_version": ver,
            "stubs": None,
            "note": row[3] if len(row) > 3 else None,
        }

    # Tier C — empty manifest, real impl present
    # Headers: Crate | total src | Upstream
    for row in tables.get("C", []):
        if len(row) < 3:
            continue
        name = row[0]
        loc = parse_int_loc(row[1])
        org_repo, ver = parse_upstream(row[2])
        crates[name] = {
            "tier": "C",
            "parity_ratio": 0.0,
            "manifest_filled": False,
            "cave_src_loc": loc,
            "upstream": org_repo,
            "upstream_version": ver,
            "stubs": None,
            "note": None,
        }

    # Tier D1 — true skeleton (<500 LOC)
    for row in tables.get("D1", []):
        if len(row) < 3:
            continue
        name = row[0]
        loc = parse_int_loc(row[1])
        org_repo, ver = parse_upstream(row[2])
        crates[name] = {
            "tier": "D1",
            "parity_ratio": 0.0,
            "manifest_filled": False,
            "cave_src_loc": loc,
            "upstream": org_repo,
            "upstream_version": ver,
            "stubs": None,
            "note": "skeleton — needs real impl",
        }

    # Tier D2 — no manifest
    for row in tables.get("D2", []):
        if len(row) < 3:
            continue
        name = row[0]
        loc = parse_int_loc(row[1])
        likely = row[2]
        crates[name] = {
            "tier": "D2",
            "parity_ratio": None,
            "manifest_filled": None,
            "cave_src_loc": loc,
            "upstream": None,
            "upstream_version": None,
            "stubs": None,
            "note": f"needs manifest; likely upstream: {likely}",
        }

    # Tier E — CAVE-internal (no upstream)
    for row in tables.get("E", []):
        if len(row) < 1:
            continue
        name = row[0]
        crates[name] = {
            "tier": "E",
            "parity_ratio": None,
            "manifest_filled": None,
            "cave_src_loc": None,
            "upstream": None,
            "upstream_version": None,
            "stubs": None,
            "note": row[1] if len(row) > 1 else "infra-only",
        }

    return crates


def main() -> int:
    if not AUDIT_PATH.exists():
        print(f"error: {AUDIT_PATH} missing", file=sys.stderr)
        return 1
    md = AUDIT_PATH.read_text()
    crates = parse_audit(md)
    out = {
        "generated_from": str(AUDIT_PATH.relative_to(REPO_ROOT)),
        "generated_at": "2026-05-01",
        "crates": crates,
    }
    OUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    OUT_PATH.write_text(json.dumps(out, indent=2, sort_keys=True))

    # Summary report to stderr
    by_tier: dict[str, int] = {}
    for entry in crates.values():
        by_tier[entry["tier"]] = by_tier.get(entry["tier"], 0) + 1
    print(
        f"Wrote {OUT_PATH.relative_to(REPO_ROOT)}: {len(crates)} crates "
        f"({', '.join(f'{t}={n}' for t, n in sorted(by_tier.items()))})",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
