#!/usr/bin/env python3
"""Honest re-audit pass (2026-05-13).

For each top-tier crate (parity_ratio > 0.6, manifest-sourced), walk every
`[[mapped]]` block in `parity.manifest.toml` and demote those whose own
`note` field already self-flags the port as partial / scope-cut / MVP.
The result is a new `[[partial]]` block class that the parity-index +
compliance dashboard surface as a fifth grade ("honest parity").

Also reclassifies known shared-placeholder mappings in cave-net
(`src/cilium/idiom_map.rs`, `src/cilium/binary_cites.rs`) as
`[[skipped]]` with the appropriate charter reason — those entries are
documentation tables for stdlib-analog / CLI-entrypoint upstreams, not
real ports.

Schema change (manifests):

    [parity]
    mapped_count   = N         # fully-ported (line-by-line behaviour)
    partial_count  = M         # NEW — shape-only / scope-cut / MVP
    skipped_count  = K
    unmapped_count = U
    total          = N+M+K+U
    fill_ratio     = (N+M+K) / total   # unchanged semantics (partial counts)
    honest_ratio   = (N+K) / total     # NEW — partial excluded

The script is idempotent: running it twice on the same manifest is a
no-op (entries are matched by `upstream_pkg` so re-runs detect the
already-demoted shape).
"""
from __future__ import annotations

import re
import sys
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
CRATES_DIR = REPO_ROOT / "crates"

# Crates to audit — top 14 manifest-sourced ratios > 0.6 (per
# `docs/parity/parity-index.json` as of 2026-05-13).
TOP_TIER_CRATES = [
    "cave-cache", "cave-cri", "cave-net", "cave-etcd", "cave-scheduler",
    "cave-apiserver", "cave-kubelet", "cave-mesh", "cave-rdbms-operator",
    "cave-streams", "cave-vault", "cave-controller-manager", "cave-auth",
    "cave-karpenter",
]

# Self-flag patterns that indicate a [[mapped]] block is really a partial port.
# These are phrases the original manifest authors used to admit scope cuts.
PARTIAL_PATTERNS = re.compile(
    r"scope cut|deferred|not implemented|not yet implemented|"
    r"no on-the-wire|no real |without (real|the |a )|"
    r"\bsubset\b|honest scope|caller bridges|caller drives|"
    r"out of scope|out-of-scope|MVP only|\bMVP[;.,]|\bMVP$|"
    r"shape-only|\bplaceholder\b|\bskeleton\b|stub-only|"
    r"first cut|first pass|TODO:|FIXME:|"
    r"is the prerequisite|wire .* not implemented|\blacks \b|\bmisses \b|"
    r"\bTBD\b|\bWIP\b",
    re.IGNORECASE,
)

# Files that are known shared placeholders (one file referenced by many
# upstream packages). Mapping to such a file counts as a stdlib-analog /
# CLI-entrypoint skip per the manifest schema, not a real port.
SHARED_PLACEHOLDER_RECLASSIFY = {
    "src/cilium/idiom_map.rs": ("stdlib-analog", "Cilium micro-pkg covered by Rust stdlib/well-known crate (see idiom_map.rs)"),
    "src/cilium/binary_cites.rs": ("CLI", "Cilium standalone-binary entrypoint; agent-side logic ported elsewhere"),
}


@dataclass
class Block:
    """One [[<header>]] table entry parsed from a manifest."""
    header: str
    raw_start: int   # offset of the `[[<header>]]` line
    raw_end: int     # offset of the next header (or EOF)
    body: str        # text between header line and next header

    def field(self, name: str) -> str | None:
        m = re.search(rf'^\s*{re.escape(name)}\s*=\s*"((?:[^"\\]|\\.)*)"', self.body, re.MULTILINE)
        return m.group(1) if m else None

    def local_files(self) -> list[str]:
        m = re.search(r'^\s*local_files\s*=\s*\[(.*?)\]', self.body, re.MULTILINE | re.DOTALL)
        if not m:
            return []
        return re.findall(r'"((?:[^"\\]|\\.)*)"', m.group(1))


HEADER_RE = re.compile(r"^\[\[(?P<h>[a-z_]+)\]\]\s*$", re.MULTILINE)


def parse_blocks(text: str) -> list[Block]:
    matches = list(HEADER_RE.finditer(text))
    out: list[Block] = []
    for i, m in enumerate(matches):
        body_end = matches[i + 1].start() if i + 1 < len(matches) else len(text)
        out.append(
            Block(
                header=m.group("h"),
                raw_start=m.start(),
                raw_end=body_end,
                body=text[m.end():body_end],
            )
        )
    return out


@dataclass
class CrateReport:
    crate: str
    total: int
    old_mapped: int
    new_mapped: int
    new_partial: int
    new_skipped: int
    new_unmapped: int
    old_fill_ratio: float
    new_fill_ratio: float
    new_honest_ratio: float
    demoted_to_partial: list[tuple[str, str]]      # (upstream_pkg, reason)
    demoted_to_skipped: list[tuple[str, str]]      # (upstream_pkg, reclass-reason)


def detect_demotions(blocks: list[Block]) -> tuple[set[int], set[int]]:
    """Walk the mapped blocks and pick out indices to demote.

    Returns (partial_indices, skipped_indices) — disjoint sets.
    """
    partials: set[int] = set()
    skipped_reclass: set[int] = set()
    for i, blk in enumerate(blocks):
        if blk.header != "mapped":
            continue
        local_files = blk.local_files()
        # Shared-placeholder reclassification (stdlib-analog / CLI).
        # Any local file that's a known placeholder demotes the entry.
        if any(lf in SHARED_PLACEHOLDER_RECLASSIFY for lf in local_files):
            skipped_reclass.add(i)
            continue
        note = blk.field("note") or ""
        if PARTIAL_PATTERNS.search(note):
            partials.add(i)
    return partials, skipped_reclass


def build_partial_block(blk: Block) -> str:
    """Render a [[mapped]] block as a [[partial]] block (header rename only)."""
    return f"[[partial]]{blk.body}"


def build_skipped_block(blk: Block, reason: str, reclass_note: str) -> str:
    """Render a [[mapped]] block as a [[skipped]] block with stdlib-analog/CLI reason."""
    upstream_pkg = blk.field("upstream_pkg") or "unknown"
    original_note = (blk.field("note") or "").strip()
    return (
        f'[[skipped]]\n'
        f'upstream_pkg = "{upstream_pkg}"\n'
        f'reason       = "{reason}"\n'
        f'note         = "{reclass_note}"\n'
        f'original_note = "{original_note}"\n'
    )


COUNT_RE = re.compile(
    r"^(?P<key>mapped_count|partial_count|skipped_count|unmapped_count|total|fill_ratio|honest_ratio)\s*=\s*[0-9.]+",
    re.MULTILINE,
)


def update_parity_counts(
    text: str,
    new_mapped: int,
    new_partial: int,
    new_skipped: int,
    new_unmapped: int,
) -> str:
    """Rewrite the `[parity]` block's mapped/partial/skipped/unmapped counts
    and recompute fill_ratio + honest_ratio. Inserts the keys if missing.
    """
    total = new_mapped + new_partial + new_skipped + new_unmapped
    fill_ratio = round((new_mapped + new_partial + new_skipped) / total, 4) if total else 0.0
    honest_ratio = round((new_mapped + new_skipped) / total, 4) if total else 0.0

    # Find the [parity] block boundaries.
    parity_m = re.search(r"^\[parity\]\s*$", text, re.MULTILINE)
    if not parity_m:
        return text
    block_start = parity_m.end()
    # Block ends at the next top-level table header or array-of-tables.
    next_header = re.search(r"^\[[^\]]+\]\s*$", text[block_start:], re.MULTILINE)
    block_end = block_start + next_header.start() if next_header else len(text)
    parity_text = text[block_start:block_end]

    new_lines: list[str] = []
    have_keys: set[str] = set()
    for line in parity_text.splitlines(keepends=True):
        stripped = line.lstrip()
        m = COUNT_RE.match(stripped)
        if not m:
            new_lines.append(line)
            continue
        key = m.group("key")
        have_keys.add(key)
        if key == "mapped_count":
            new_lines.append(f"mapped_count   = {new_mapped}\n")
        elif key == "partial_count":
            new_lines.append(f"partial_count  = {new_partial}\n")
        elif key == "skipped_count":
            new_lines.append(f"skipped_count  = {new_skipped}\n")
        elif key == "unmapped_count":
            new_lines.append(f"unmapped_count = {new_unmapped}\n")
        elif key == "total":
            new_lines.append(f"total          = {total}\n")
        elif key == "fill_ratio":
            new_lines.append(f"fill_ratio     = {fill_ratio}\n")
        elif key == "honest_ratio":
            new_lines.append(f"honest_ratio   = {honest_ratio}\n")

    # Insert any missing keys right after mapped_count (or at top of block
    # if even that's missing) so the block stays human-readable.
    # Find anchor — first count line in new_lines.
    def insert_after(anchor_key: str, key_line: str) -> None:
        nonlocal new_lines
        for i, l in enumerate(new_lines):
            if l.lstrip().startswith(anchor_key):
                new_lines.insert(i + 1, key_line)
                return
        # Fallback: prepend.
        new_lines.insert(0, key_line)

    if "partial_count" not in have_keys:
        insert_after("mapped_count", f"partial_count  = {new_partial}\n")
    if "honest_ratio" not in have_keys:
        insert_after("fill_ratio", f"honest_ratio   = {honest_ratio}\n")

    return text[:block_start] + "".join(new_lines) + text[block_end:]


def transform_manifest(text: str) -> tuple[str, CrateReport | None]:
    blocks = parse_blocks(text)
    partials, skipped_reclass = detect_demotions(blocks)

    # Tallies for the report.
    old_mapped = sum(1 for b in blocks if b.header == "mapped")
    new_mapped = sum(
        1 for i, b in enumerate(blocks)
        if b.header == "mapped" and i not in partials and i not in skipped_reclass
    )
    new_partial = sum(1 for b in blocks if b.header == "partial") + len(partials)
    new_skipped = sum(1 for b in blocks if b.header == "skipped") + len(skipped_reclass)
    new_unmapped = sum(1 for b in blocks if b.header == "unmapped")
    total = new_mapped + new_partial + new_skipped + new_unmapped
    if total == 0:
        return text, None
    old_fill_ratio = round(
        (old_mapped + new_skipped - len(skipped_reclass) + new_unmapped * 0) / total, 4
    )
    # Old fill ratio = (old_mapped + old_skipped) / total. old_skipped = new_skipped - len(reclass).
    old_skipped = new_skipped - len(skipped_reclass)
    old_fill_ratio = round((old_mapped + old_skipped) / total, 4) if total else 0.0
    new_fill_ratio = round((new_mapped + new_partial + new_skipped) / total, 4) if total else 0.0
    new_honest_ratio = round((new_mapped + new_skipped) / total, 4) if total else 0.0

    # Collect demotion details for the audit report.
    demoted_partial: list[tuple[str, str]] = []
    demoted_skipped: list[tuple[str, str]] = []
    for i in partials:
        blk = blocks[i]
        note = (blk.field("note") or "").strip()
        flag = PARTIAL_PATTERNS.search(note)
        demoted_partial.append((blk.field("upstream_pkg") or "?", flag.group(0) if flag else "self-flagged"))
    for i in skipped_reclass:
        blk = blocks[i]
        for lf in blk.local_files():
            if lf in SHARED_PLACEHOLDER_RECLASSIFY:
                demoted_skipped.append((blk.field("upstream_pkg") or "?", lf))
                break

    # Walk the source text once, rewriting demoted [[mapped]] blocks in place.
    if not partials and not skipped_reclass:
        # Still update [parity] counts so honest_ratio gets added even if
        # nothing demotes. Idempotent.
        new_text = update_parity_counts(text, new_mapped, new_partial, new_skipped, new_unmapped)
        return new_text, CrateReport(
            crate="?",
            total=total,
            old_mapped=old_mapped,
            new_mapped=new_mapped,
            new_partial=new_partial,
            new_skipped=new_skipped,
            new_unmapped=new_unmapped,
            old_fill_ratio=old_fill_ratio,
            new_fill_ratio=new_fill_ratio,
            new_honest_ratio=new_honest_ratio,
            demoted_to_partial=[],
            demoted_to_skipped=[],
        )

    # Build the new text by stitching the unchanged prefix, then each
    # block (possibly rewritten), then any trailing content.
    out_pieces: list[str] = []
    if blocks:
        out_pieces.append(text[: blocks[0].raw_start])
        for i, blk in enumerate(blocks):
            if i in partials:
                out_pieces.append(build_partial_block(blk))
            elif i in skipped_reclass:
                reason, reclass_note = SHARED_PLACEHOLDER_RECLASSIFY[
                    next(lf for lf in blk.local_files() if lf in SHARED_PLACEHOLDER_RECLASSIFY)
                ]
                out_pieces.append(build_skipped_block(blk, reason, reclass_note))
            else:
                out_pieces.append(text[blk.raw_start:blk.raw_end])
    new_text = "".join(out_pieces)
    new_text = update_parity_counts(new_text, new_mapped, new_partial, new_skipped, new_unmapped)
    return new_text, CrateReport(
        crate="?",
        total=total,
        old_mapped=old_mapped,
        new_mapped=new_mapped,
        new_partial=new_partial,
        new_skipped=new_skipped,
        new_unmapped=new_unmapped,
        old_fill_ratio=old_fill_ratio,
        new_fill_ratio=new_fill_ratio,
        new_honest_ratio=new_honest_ratio,
        demoted_to_partial=demoted_partial,
        demoted_to_skipped=demoted_skipped,
    )


def main(argv: list[str]) -> int:
    dry_run = "--dry-run" in argv
    crates = TOP_TIER_CRATES
    if "--all" in argv:
        crates = sorted(
            p.name for p in CRATES_DIR.iterdir()
            if (p / "parity.manifest.toml").is_file()
        )

    reports: list[CrateReport] = []
    for crate in crates:
        manifest = CRATES_DIR / crate / "parity.manifest.toml"
        if not manifest.is_file():
            print(f"  skip {crate}: no manifest", file=sys.stderr)
            continue
        text = manifest.read_text(encoding="utf-8")
        new_text, report = transform_manifest(text)
        if report is None:
            print(f"  skip {crate}: no counts", file=sys.stderr)
            continue
        report.crate = crate
        reports.append(report)
        if not dry_run and new_text != text:
            manifest.write_text(new_text, encoding="utf-8")

    # Honest report
    print("=" * 70)
    print("Honest re-audit summary")
    print("=" * 70)
    print(f"{'crate':<28} {'old':>6} {'fill':>6} {'honest':>7}  partial / reclass")
    for r in reports:
        delta = r.new_honest_ratio - r.old_fill_ratio
        print(
            f"{r.crate:<28} {r.old_fill_ratio:>6.4f} {r.new_fill_ratio:>6.4f} "
            f"{r.new_honest_ratio:>7.4f}  partial={len(r.demoted_to_partial):>2}  "
            f"reclass={len(r.demoted_to_skipped):>2}  Δhonest={delta:+.4f}"
        )
    total_demoted_partial = sum(len(r.demoted_to_partial) for r in reports)
    total_reclass = sum(len(r.demoted_to_skipped) for r in reports)
    print(f"\nTotal demotions: {total_demoted_partial} mapped→partial, {total_reclass} mapped→skipped (stdlib-analog/CLI)")
    if dry_run:
        print("(dry run — no files written)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
