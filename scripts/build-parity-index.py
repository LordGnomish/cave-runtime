#!/usr/bin/env python3
"""
Parse `docs/parity/full-audit-2026-05-01.md` into `docs/parity/parity-index.json`.

Source-of-truth ordering for the per-crate `parity_ratio` (Fix-A,
2026-05-13): **manifest first, audit doc fallback**. Audit doc is
still the source of truth for tier classification, upstream
identity, and the "what's missing" notes; those rarely change. The
ratio is the one number that drifts every time a new file lands or
an audit re-measures a crate, so we read it live from the on-disk
manifest's `[parity] fill_ratio` (or legacy `ratio`).

Output schema:
{
  "generated_from": "docs/parity/full-audit-2026-05-01.md",
  "generated_at": "2026-05-01",
  "crates": {
    "<crate-name>": {
      "tier": "100" | "A" | "B" | "C" | "D1" | "D2" | "E",
      "parity_ratio": float | null,       # null = not measurable
      "parity_ratio_source": "manifest"   # disk manifest fill_ratio/ratio
                          | "audit"       # audit-doc snapshot
                          | "none",       # never measured
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
            "parity_ratio_source": "audit",
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
            "parity_ratio_source": "audit" if ratio is not None else "none",
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
            "parity_ratio_source": "audit" if ratio is not None else "none",
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
            "parity_ratio_source": "audit",
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
            "parity_ratio_source": "audit",
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
            "parity_ratio_source": "none",
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
            "parity_ratio_source": "none",
            "manifest_filled": None,
            "cave_src_loc": None,
            "upstream": None,
            "upstream_version": None,
            "stubs": None,
            "note": row[1] if len(row) > 1 else "infra-only",
        }

    return crates


def disk_manifest_state(crate: str) -> dict:
    """Read crates/<crate>/parity.manifest.toml and surface the disk truth
    about `manifest_filled`, `parity_ratio`, and `infra_only`. The audit
    doc captures a frozen snapshot from 2026-05-01; this function
    reflects what the on-disk manifest now claims, so the dashboard can
    pick up improvements without waiting for the next markdown re-edit.

    `manifest_filled` becomes True iff the `[upstream]` block has a
    `license` field AND a `[parity]` block exists. For infra-only
    crates, `[upstream].license` may be absent (no upstream applies);
    `[parity].infra_only = true` is sufficient to count the manifest
    as filled.
    """
    p = REPO_ROOT / "crates" / crate / "parity.manifest.toml"
    if not p.is_file():
        return {}
    try:
        text = p.read_text(encoding="utf-8")
    except OSError:
        return {}

    # Find [parity] block — scan line-by-line so comments containing
    # `[` (e.g. references to [[mapped]] in inventory manifests) don't
    # truncate the capture.
    lines = text.splitlines()
    block_lines: list[str] = []
    in_block = False
    for line in lines:
        stripped = line.lstrip()
        if stripped.startswith("[parity]"):
            in_block = True
            block_lines.append(line)
            continue
        if in_block:
            # Stop at the next table header (single or array-of-tables).
            # Comments / blank lines stay in the block. Lines that start
            # with `#` may legitimately contain `[`.
            if stripped.startswith("[") and not stripped.startswith("#"):
                break
            block_lines.append(line)
    if not block_lines:
        return {}
    block = "\n".join(block_lines)
    rm = re.search(r'^\s*fill_ratio\s*=\s*([0-9.]+)', block, flags=re.MULTILINE)
    if rm is None:
        rm = re.search(r'^\s*ratio\s*=\s*([0-9.]+)', block, flags=re.MULTILINE)
    ratio = float(rm.group(1)) if rm else None
    # NEW 2026-05-13: honest_ratio = (fully_ported_mapped + skipped) / total
    # — partial blocks excluded. Surfaced as a separate axis on the
    # compliance dashboard.
    hm = re.search(r'^\s*honest_ratio\s*=\s*([0-9.]+)', block, flags=re.MULTILINE)
    honest_ratio = float(hm.group(1)) if hm else None
    # Per-class counts (added 2026-05-13 alongside [[partial]]).
    def _int_field(name: str) -> int | None:
        m = re.search(rf'^\s*{re.escape(name)}\s*=\s*([0-9]+)', block, flags=re.MULTILINE)
        return int(m.group(1)) if m else None
    mapped_count = _int_field("mapped_count")
    partial_count = _int_field("partial_count")
    skipped_count = _int_field("skipped_count")
    unmapped_count = _int_field("unmapped_count")
    total_count = _int_field("total")
    am = re.search(r'^\s*last_audit\s*=\s*"([^"]+)"', block, flags=re.MULTILINE)
    last_audit = am.group(1) if am else None
    im = re.search(r'^\s*infra_only\s*=\s*(true|false)', block, flags=re.MULTILINE)
    infra = im.group(1) == "true" if im else False

    # Check [upstream].license
    upstream_m = re.search(
        r"^\[upstream\][^\[]*",
        text,
        flags=re.MULTILINE,
    )
    has_license = False
    if upstream_m:
        has_license = bool(
            re.search(r'^\s*license\s*=\s*"', upstream_m.group(0), flags=re.MULTILINE)
        )

    # `manifest_filled` = `[upstream]` carries license info (or infra-only)
    # AND `[parity]` exists with `last_audit`.
    filled = (has_license or infra) and last_audit is not None
    return {
        "manifest_filled": filled,
        "parity_ratio_disk": ratio,
        "honest_ratio_disk": honest_ratio,
        "infra_only_disk": infra,
        "last_audit_disk": last_audit,
        "mapped_count": mapped_count,
        "partial_count": partial_count,
        "skipped_count": skipped_count,
        "unmapped_count": unmapped_count,
        "total_count": total_count,
    }


def overlay_disk_state(crates: dict[str, dict]) -> dict[str, int]:
    """For every crate in the index, overlay the disk manifest's
    `manifest_filled` + `parity_ratio` if the disk is newer or
    authoritative. Crates whose name has no corresponding workspace
    directory are marked `phantom = true` so the dashboard can exclude
    them from headline ratios. Workspace crates absent from the
    audit doc (e.g. cave-rdbms-operator, cave-karpenter) get a
    synthesized entry from their on-disk manifest. Returns a delta
    report.
    """
    flipped = 0
    ratio_overrides = 0
    new_filled = 0
    phantoms = 0
    injected = 0
    # Walk every workspace member and inject a stub entry for any
    # crate the audit doc didn't cover. The disk overlay then fills
    # in the measured ratio just like it does for known entries.
    crates_dir = REPO_ROOT / "crates"
    if crates_dir.is_dir():
        for p in sorted(crates_dir.iterdir()):
            if not (p.is_dir() and (p / "Cargo.toml").is_file()):
                continue
            name = p.name
            if name in crates:
                continue
            # Synthesize an audit-unknown entry — the disk overlay below
            # will fill manifest_filled + parity_ratio if the on-disk
            # `[parity]` block carries them.
            crates[name] = {
                "tier": "C",
                "parity_ratio": None,
                "parity_ratio_source": "none",
                "manifest_filled": None,
                "cave_src_loc": None,
                "upstream": None,
                "upstream_version": None,
                "stubs": None,
                "note": "added by disk-overlay; not in audit doc",
            }
            injected += 1
    for name, entry in crates.items():
        crate_dir = REPO_ROOT / "crates" / name
        if not crate_dir.is_dir():
            entry["phantom"] = True
            phantoms += 1
            continue
        disk = disk_manifest_state(name)
        if not disk:
            continue
        before = entry.get("manifest_filled")
        if disk["manifest_filled"] and before is not True:
            entry["manifest_filled"] = True
            new_filled += 1
            flipped += 1
        # Fix-A 2026-05-13: manifest is the **primary** source of
        # truth for parity_ratio. Whenever the on-disk
        # `[parity] fill_ratio` (or legacy `ratio`) is present, it
        # wins over the audit-doc snapshot — including the case where
        # the manifest is older than the doc, because the manifest is
        # what the per-crate parity work is updating and the doc is a
        # frozen Wave-3 capture from 2026-05-01.
        #
        # We surface `parity_ratio_source = "manifest"` so the
        # compliance dashboard can render where the number came from.
        disk_ratio = disk.get("parity_ratio_disk")
        if disk_ratio is not None:
            audit_ratio = entry.get("parity_ratio")
            if disk_ratio != audit_ratio:
                entry["parity_ratio"] = disk_ratio
                ratio_overrides += 1
            entry["parity_ratio_source"] = "manifest"
            entry["last_audit_disk"] = disk.get("last_audit_disk")
        # Honest-parity axis (added 2026-05-13). When the manifest carries
        # an explicit `honest_ratio`, surface it; otherwise fall back to
        # the standard `parity_ratio` so the dashboard sees no NaN.
        honest = disk.get("honest_ratio_disk")
        if honest is not None:
            entry["honest_ratio"] = honest
        elif disk_ratio is not None and "honest_ratio" not in entry:
            # No manifest [[partial]] block authored yet — honest ratio
            # is conservatively the same as the standard one.
            entry["honest_ratio"] = disk_ratio
        # Per-class counts, when the manifest carries them.
        for k in ("mapped_count", "partial_count", "skipped_count",
                  "unmapped_count", "total_count"):
            if disk.get(k) is not None:
                entry[k] = disk[k]
        # Surface infra_only signal from disk for E-tier entries.
        if disk.get("infra_only_disk"):
            entry["infra_only"] = True
    return {
        "flipped": flipped,
        "ratio_overrides": ratio_overrides,
        "new_filled": new_filled,
        "phantoms": phantoms,
        "injected": injected,
    }


def main() -> int:
    if not AUDIT_PATH.exists():
        print(f"error: {AUDIT_PATH} missing", file=sys.stderr)
        return 1
    md = AUDIT_PATH.read_text()
    crates = parse_audit(md)
    overlay = overlay_disk_state(crates)
    out = {
        "generated_from": str(AUDIT_PATH.relative_to(REPO_ROOT)),
        "generated_at": "2026-05-01",
        "disk_overlay_at": "2026-05-12",
        "disk_overlay_stats": overlay,
        "crates": crates,
    }
    OUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    OUT_PATH.write_text(json.dumps(out, indent=2, sort_keys=True))

    # Summary report to stderr
    by_tier: dict[str, int] = {}
    for entry in crates.values():
        by_tier[entry["tier"]] = by_tier.get(entry["tier"], 0) + 1
    filled = sum(1 for e in crates.values() if e.get("manifest_filled") is True)
    not_filled = sum(1 for e in crates.values() if e.get("manifest_filled") is False)
    unknown = sum(1 for e in crates.values() if e.get("manifest_filled") is None)
    print(
        f"Wrote {OUT_PATH.relative_to(REPO_ROOT)}: {len(crates)} crates "
        f"({', '.join(f'{t}={n}' for t, n in sorted(by_tier.items()))})",
        file=sys.stderr,
    )
    print(
        f"manifest_filled: true={filled}  false={not_filled}  null={unknown}",
        file=sys.stderr,
    )
    print(
        f"disk overlay: flipped={overlay['flipped']}  new_filled={overlay['new_filled']}  "
        f"ratio_overrides={overlay['ratio_overrides']}  phantoms={overlay['phantoms']}",
        file=sys.stderr,
    )
    real_total = len(crates) - overlay['phantoms']
    real_filled = sum(
        1 for e in crates.values()
        if e.get('manifest_filled') is True and not e.get('phantom')
    )
    print(
        f"workspace-only: filled={real_filled}/{real_total}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
