#!/usr/bin/env python3
"""
Build `docs/parity/parity-index.json` from on-disk `parity.manifest.toml`
files.

Source-of-truth ordering (Fix-A 2026-05-13 → themed-paths 2026-05-27):

1. The `full-audit-2026-05-01.md` markdown is the historical seed for
   tier classification and "what's missing" notes.
2. The per-crate `parity.manifest.toml` on disk is the live source of
   truth for every numeric field (`fill_ratio`, `honest_ratio`,
   `adr_justified_ratio`, per-class counts), upstream identity
   (`org/repo`, `version`, `source_sha`, `license`, `url`), and the
   `last_audit` date.
3. `git log -1 --format=%H -- <crate-dir>` provides `last_commit` so the
   dashboard can render staleness.

Crates are discovered at BOTH the themed path
(`crates/<theme>/<crate>/parity.manifest.toml`) and the legacy flat path
(`crates/<crate>/parity.manifest.toml`) — the theme reorg landed
2026-05-25 (commit `9c15b3fb`) and consumers should not need to re-flow.
"""
import json
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
AUDIT_PATH = REPO_ROOT / "docs" / "parity" / "full-audit-2026-05-01.md"
OUT_PATH = REPO_ROOT / "docs" / "parity" / "parity-index.json"

ROW_RE = re.compile(r"^\|(.+)\|$")


def split_row(line: str):
    m = ROW_RE.match(line.strip())
    if not m:
        return None
    return [c.strip() for c in m.group(1).split("|")]


def is_separator(cells):
    return all(re.fullmatch(r":?-+:?", c) for c in cells)


def parse_upstream_audit(s):
    """`kubernetes/kubernetes @ v1.28.0` -> ('kubernetes/kubernetes', 'v1.28.0')."""
    if not s or s.startswith("("):
        return (None, None)
    parts = s.split("@", 1)
    org_repo = parts[0].strip() or None
    version = parts[1].strip() if len(parts) > 1 else None
    return (org_repo, version)


def parse_int_loc(s):
    s = s.strip().replace(",", "")
    return int(s) if s.isdigit() else None


def parse_overall(s):
    m = re.match(r"([01]?\.\d+|\d+)", s.strip())
    if not m:
        return None
    try:
        return float(m.group(1))
    except ValueError:
        return None


def find_tables(text):
    out = {}
    section = None
    for line in text.splitlines():
        if line.startswith("## Tier ✅"):
            section = "100"
        elif line.startswith("## Tier A "):
            section = "A"
        elif line.startswith("## Tier B "):
            section = "B"
        elif line.startswith("## Tier C "):
            section = "C"
        elif line.startswith("## Tier D"):
            section = "D"
        elif line.startswith("## Tier E"):
            section = "E"
        elif line.startswith("### D1"):
            section = "D1"
        elif line.startswith("### D2"):
            section = "D2"
        elif line.startswith("## "):
            section = None
        if section is None:
            continue
        cells = split_row(line)
        if not cells or is_separator(cells):
            continue
        if cells[0].lower() in ("crate", "bucket"):
            continue
        out.setdefault(section, []).append(cells)
    return out


def parse_audit(md_text):
    tables = find_tables(md_text)
    crates = {}

    for row in tables.get("100", []):
        if len(row) < 7:
            continue
        name = row[0]
        org_repo, ver = parse_upstream_audit(row[1])
        crates[name] = {
            "tier": "100",
            "parity_ratio": 1.0,
            "parity_ratio_source": "audit",
            "manifest_filled": True,
            "cave_src_loc": None,
            "upstream": org_repo,
            "upstream_version": ver,
            "stubs": parse_int_loc(row[6]),
            "note": None,
        }
    for row in tables.get("A", []):
        if len(row) < 3:
            continue
        name = row[0]
        org_repo, ver = parse_upstream_audit(row[1])
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
    for row in tables.get("B", []):
        if len(row) < 3:
            continue
        name = row[0]
        org_repo, ver = parse_upstream_audit(row[1])
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
    for row in tables.get("C", []):
        if len(row) < 3:
            continue
        name = row[0]
        org_repo, ver = parse_upstream_audit(row[2])
        crates[name] = {
            "tier": "C",
            "parity_ratio": 0.0,
            "parity_ratio_source": "audit",
            "manifest_filled": False,
            "cave_src_loc": parse_int_loc(row[1]),
            "upstream": org_repo,
            "upstream_version": ver,
            "stubs": None,
            "note": None,
        }
    for row in tables.get("D1", []):
        if len(row) < 3:
            continue
        name = row[0]
        org_repo, ver = parse_upstream_audit(row[2])
        crates[name] = {
            "tier": "D1",
            "parity_ratio": 0.0,
            "parity_ratio_source": "audit",
            "manifest_filled": False,
            "cave_src_loc": parse_int_loc(row[1]),
            "upstream": org_repo,
            "upstream_version": ver,
            "stubs": None,
            "note": "skeleton — needs real impl",
        }
    for row in tables.get("D2", []):
        if len(row) < 3:
            continue
        name = row[0]
        crates[name] = {
            "tier": "D2",
            "parity_ratio": None,
            "parity_ratio_source": "none",
            "manifest_filled": None,
            "cave_src_loc": parse_int_loc(row[1]),
            "upstream": None,
            "upstream_version": None,
            "stubs": None,
            "note": f"needs manifest; likely upstream: {row[2]}",
        }
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


def discover_workspace_crates():
    """Return {crate_name: crate_dir_relative_to_repo_root} for every
    workspace crate that has a `Cargo.toml`, scanning BOTH themed paths
    (`crates/<theme>/<crate>/`) and the legacy flat layout
    (`crates/<crate>/`)."""
    out = {}
    crates_dir = REPO_ROOT / "crates"
    if not crates_dir.is_dir():
        return out
    for p in sorted(crates_dir.iterdir()):
        if not p.is_dir():
            continue
        if (p / "Cargo.toml").is_file():
            out[p.name] = p
            continue
        for sub in sorted(p.iterdir()):
            if sub.is_dir() and (sub / "Cargo.toml").is_file():
                out[sub.name] = sub
    return out


def parse_section_block(text, section_name):
    """Pull lines under [section_name] up to the next top-level table."""
    lines = text.splitlines()
    block, in_block = [], False
    target = f"[{section_name}]"
    for line in lines:
        stripped = line.lstrip()
        if stripped.startswith(target):
            in_block = True
            block.append(line)
            continue
        if in_block:
            if stripped.startswith("[") and not stripped.startswith("#"):
                break
            block.append(line)
    return "\n".join(block) if block else ""


def _re_str(block, key):
    m = re.search(rf'^\s*{re.escape(key)}\s*=\s*"([^"]+)"', block, flags=re.MULTILINE)
    return m.group(1) if m else None


def _re_float(block, key):
    m = re.search(rf'^\s*{re.escape(key)}\s*=\s*([0-9.]+)', block, flags=re.MULTILINE)
    return float(m.group(1)) if m else None


def _re_int(block, key):
    m = re.search(rf'^\s*{re.escape(key)}\s*=\s*([0-9]+)', block, flags=re.MULTILINE)
    return int(m.group(1)) if m else None


def _re_bool(block, key):
    m = re.search(rf'^\s*{re.escape(key)}\s*=\s*(true|false)', block, flags=re.MULTILINE)
    return m.group(1) == "true" if m else None


def parse_manifest(path: Path):
    """Read a `parity.manifest.toml` and return a structured snapshot."""
    try:
        text = path.read_text(encoding="utf-8")
    except OSError:
        return {}

    upstream_block = parse_section_block(text, "upstream")
    parity_block = parse_section_block(text, "parity")

    out = {}
    if upstream_block:
        org = _re_str(upstream_block, "org")
        repo = _re_str(upstream_block, "repo")
        if org and repo:
            out["upstream"] = f"{org}/{repo}"
        out["upstream_version"] = _re_str(upstream_block, "version")
        out["source_sha"] = _re_str(upstream_block, "source_sha")
        out["upstream_license"] = _re_str(upstream_block, "license")
        out["upstream_url"] = _re_str(upstream_block, "url")
        out["upstream_name"] = _re_str(upstream_block, "name")
        out["upstream_language"] = _re_str(upstream_block, "language")

    if parity_block:
        out["fill_ratio"] = _re_float(parity_block, "fill_ratio")
        if out["fill_ratio"] is None:
            out["fill_ratio"] = _re_float(parity_block, "ratio")
        out["honest_ratio"] = _re_float(parity_block, "honest_ratio")
        out["adr_justified_ratio"] = _re_float(parity_block, "adr_justified_ratio")
        out["adr_justification"] = _re_str(parity_block, "adr_justification")
        out["mapped_count"] = _re_int(parity_block, "mapped_count")
        out["partial_count"] = _re_int(parity_block, "partial_count")
        out["skipped_count"] = _re_int(parity_block, "skipped_count")
        out["unmapped_count"] = _re_int(parity_block, "unmapped_count")
        out["total_count"] = _re_int(parity_block, "total")
        out["last_audit"] = _re_str(parity_block, "last_audit")
        infra = _re_bool(parity_block, "infra_only")
        if infra is not None:
            out["infra_only"] = infra

    out["manifest_filled"] = (
        out.get("upstream_license") is not None or out.get("infra_only")
    ) and out.get("last_audit") is not None
    return out


def parse_behavioral(path: Path):
    """Read `[behavioral_parity]` + `[[upstream_test]]` entries and
    return per-status counts. Returns {} when the block is absent."""
    try:
        text = path.read_text(encoding="utf-8")
    except OSError:
        return {}
    ported = partial = missing = total = 0
    audit_scope = audit_at = None
    in_entry = False
    cur_status = None
    in_bp = False
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("[[upstream_test]]"):
            if in_entry and cur_status:
                total += 1
                if cur_status == "ported":
                    ported += 1
                elif cur_status == "partial":
                    partial += 1
                elif cur_status == "missing":
                    missing += 1
            in_entry = True
            cur_status = None
            in_bp = False
            continue
        if stripped.startswith("["):
            if in_entry and cur_status:
                total += 1
                if cur_status == "ported":
                    ported += 1
                elif cur_status == "partial":
                    partial += 1
                elif cur_status == "missing":
                    missing += 1
                in_entry = False
                cur_status = None
            in_bp = stripped.startswith("[behavioral_parity]")
            continue
        if in_entry and stripped.startswith("status"):
            m = re.match(r'status\s*=\s*"(\w+)"', stripped)
            if m:
                cur_status = m.group(1)
        elif in_bp:
            ms = re.match(r'audit_scope\s*=\s*"([^"]+)"', stripped)
            if ms:
                audit_scope = ms.group(1)
            ma = re.match(r'audit_at\s*=\s*"([^"]+)"', stripped)
            if ma:
                audit_at = ma.group(1)
    if in_entry and cur_status:
        total += 1
        if cur_status == "ported":
            ported += 1
        elif cur_status == "partial":
            partial += 1
        elif cur_status == "missing":
            missing += 1
    if total == 0:
        return {}
    return {
        "behavioral_parity": ported / total,
        "behavioral_ported": ported,
        "behavioral_total": total,
        "behavioral_partial": partial,
        "behavioral_missing": missing,
        "behavioral_audit_scope": audit_scope,
        "behavioral_audit_at": audit_at,
    }


def last_commit_for(crate_dir: Path):
    """`git log -1 --format=%H -- <crate-dir>` so the dashboard can show
    staleness. Returns (sha, iso8601) or (None, None)."""
    try:
        rel = crate_dir.relative_to(REPO_ROOT)
        res = subprocess.run(
            [
                "git",
                "-C",
                str(REPO_ROOT),
                "log",
                "-1",
                "--format=%H%x09%cI",
                "--",
                str(rel),
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        if res.returncode != 0 or not res.stdout.strip():
            return (None, None)
        sha, _, when = res.stdout.strip().partition("\t")
        return (sha or None, when or None)
    except Exception:
        return (None, None)


def overlay_workspace(crates):
    """Inject + refresh crates from the live workspace. Returns delta stats."""
    overlay = {
        "flipped": 0,
        "ratio_overrides": 0,
        "new_filled": 0,
        "phantoms": 0,
        "injected": 0,
    }
    workspace = discover_workspace_crates()
    for name in list(crates.keys()):
        if name not in workspace:
            crates[name]["phantom"] = True
            overlay["phantoms"] += 1
    for name, crate_dir in workspace.items():
        if name not in crates:
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
            overlay["injected"] += 1
        entry = crates[name]
        entry.pop("phantom", None)
        manifest_path = crate_dir / "parity.manifest.toml"
        if manifest_path.is_file():
            disk = parse_manifest(manifest_path)
            before = entry.get("manifest_filled")
            if disk.get("manifest_filled") and before is not True:
                entry["manifest_filled"] = True
                overlay["new_filled"] += 1
                overlay["flipped"] += 1
            if disk.get("fill_ratio") is not None:
                if entry.get("parity_ratio") != disk["fill_ratio"]:
                    entry["parity_ratio"] = disk["fill_ratio"]
                    overlay["ratio_overrides"] += 1
                entry["fill_ratio"] = disk["fill_ratio"]
                entry["parity_ratio_source"] = "manifest"
            if disk.get("honest_ratio") is not None:
                entry["honest_ratio"] = disk["honest_ratio"]
            elif disk.get("fill_ratio") is not None and "honest_ratio" not in entry:
                entry["honest_ratio"] = disk["fill_ratio"]
            for key in (
                "adr_justified_ratio",
                "adr_justification",
                "mapped_count",
                "partial_count",
                "skipped_count",
                "unmapped_count",
                "total_count",
                "source_sha",
                "upstream_license",
                "upstream_url",
                "upstream_name",
                "upstream_language",
                "last_audit",
            ):
                if disk.get(key) is not None:
                    entry[key] = disk[key]
            if disk.get("upstream"):
                entry["upstream"] = disk["upstream"]
            if disk.get("upstream_version"):
                entry["upstream_version"] = disk["upstream_version"]
            if disk.get("last_audit"):
                entry["last_audit_disk"] = disk["last_audit"]
            if disk.get("infra_only"):
                entry["infra_only"] = True
            bp = parse_behavioral(manifest_path)
            for k, v in bp.items():
                entry[k] = v
        sha, when = last_commit_for(crate_dir)
        if sha:
            entry["last_commit"] = sha
        if when:
            entry["last_commit_at"] = when
        entry["crate_dir"] = str(crate_dir.relative_to(REPO_ROOT))
    return overlay


def main():
    if AUDIT_PATH.exists():
        md = AUDIT_PATH.read_text()
        crates = parse_audit(md)
    else:
        crates = {}
    overlay = overlay_workspace(crates)

    out = {
        "generated_from": (
            str(AUDIT_PATH.relative_to(REPO_ROOT)) if AUDIT_PATH.exists() else None
        ),
        "generated_at": "2026-05-01",
        "disk_overlay_at": "2026-05-27",
        "disk_overlay_stats": overlay,
        "crates": crates,
    }
    OUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    OUT_PATH.write_text(json.dumps(out, indent=2, sort_keys=True))

    by_tier = {}
    for entry in crates.values():
        by_tier[entry.get("tier", "?")] = by_tier.get(entry.get("tier", "?"), 0) + 1
    filled = sum(1 for e in crates.values() if e.get("manifest_filled") is True)
    print(
        f"Wrote {OUT_PATH.relative_to(REPO_ROOT)}: {len(crates)} crates "
        f"({', '.join(f'{t}={n}' for t, n in sorted(by_tier.items()))})",
        file=sys.stderr,
    )
    print(
        f"manifest_filled: true={filled}  phantoms={overlay['phantoms']}  "
        f"injected={overlay['injected']}  ratio_overrides={overlay['ratio_overrides']}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
