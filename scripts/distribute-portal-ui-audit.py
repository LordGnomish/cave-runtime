#!/usr/bin/env python3
"""
Distribute `docs/parity/portal-ui-audit-2026-05-11.md` rows into each
crate's `parity.manifest.toml` as a `[portal_ui]` block.

For every row in the audit's "Full per-crate table" we write:

  [portal_ui]
  upstream_url = "..."
  status       = "none" | "scaffold" | "partial" | "complete"
  loc          = N
  priority     = "P0" | "P1" | "P2"
  notes        = "..."
  last_audit   = "2026-05-11"

The block is idempotent — re-running the script after editing the
audit updates the existing block in place. Infra crates without a
row (cave-cli, cave-kernel, etc.) are NOT touched.

The compliance dashboard reads these blocks at runtime to compute
`portal_ui_avg_score` and the third dashboard grade. This script is
the bridge from the audit doc (human-edited) to the manifest blocks
(machine-read).
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
AUDIT = REPO / "docs/parity/portal-ui-audit-2026-05-11.md"
CRATES = REPO / "crates"
LAST_AUDIT = "2026-05-11"

ROW_RE = re.compile(
    r"^\|\s*`(?P<crate>cave-[^`]+)`\s*"
    r"\|\s*(?P<has_admin>[✓—])\s*"
    r"\|\s*(?P<upstream_ui>[^|]+?)\s*"
    r"\|\s*(?P<url>[^|]+?)\s*"
    r"\|\s*`(?P<score>none|scaffold|partial|complete)`\s*"
    r"\|\s*(?P<loc>\d+)\s*"
    r"\|\s*(?P<priority>P0|P1|P2)\s*"
    r"\|\s*(?P<notes>[^|]*?)\s*\|$"
)


def parse_audit() -> list[dict]:
    rows = []
    for line in AUDIT.read_text(encoding="utf-8").splitlines():
        m = ROW_RE.match(line)
        if not m:
            continue
        url = m.group("url").strip()
        # The audit renders URLs as `[link](https://...)`; pull the
        # bare URL out.
        link_m = re.match(r"\[[^\]]+\]\((?P<u>https?://[^)]+)\)", url)
        if link_m:
            url = link_m.group("u")
        rows.append({
            "crate": m.group("crate"),
            "upstream_ui": m.group("upstream_ui").strip(),
            "url": url,
            "status": m.group("score"),
            "loc": int(m.group("loc")),
            "priority": m.group("priority"),
            "notes": m.group("notes").strip(),
        })
    return rows


def render_block(row: dict) -> str:
    notes_escaped = row["notes"].replace('"', '\\"')
    upstream_ui_escaped = row["upstream_ui"].replace('"', '\\"')
    return (
        "\n[portal_ui]\n"
        f'upstream_ui  = "{upstream_ui_escaped}"\n'
        f'upstream_url = "{row["url"]}"\n'
        f'status       = "{row["status"]}"\n'
        f'loc          = {row["loc"]}\n'
        f'priority     = "{row["priority"]}"\n'
        f'notes        = "{notes_escaped}"\n'
        f'last_audit   = "{LAST_AUDIT}"\n'
    )


def replace_block(text: str, new_block: str) -> str:
    """Replace an existing [portal_ui] block, or append a new one to
    end-of-file. Block end is detected by the next single-table
    header at column 0 (a `[name]` line that's not `[[name]]`)."""
    # Find [portal_ui] header
    lines = text.splitlines()
    start = end = None
    for i, line in enumerate(lines):
        stripped = line.lstrip()
        if stripped.startswith("[portal_ui]"):
            start = i
            # Scan forward to the next table header (single or array-of-tables).
            j = i + 1
            while j < len(lines):
                s = lines[j].lstrip()
                if s.startswith("[") and not s.startswith("#"):
                    break
                j += 1
            end = j
            break
    if start is not None and end is not None:
        # Replace. Preserve leading blank line if any.
        kept = lines[:start] + new_block.lstrip("\n").splitlines() + lines[end:]
        out = "\n".join(kept)
        if not out.endswith("\n"):
            out += "\n"
        return out
    # Append. Ensure newline separation.
    if not text.endswith("\n"):
        text += "\n"
    return text + new_block


def process(row: dict) -> dict:
    crate_dir = CRATES / row["crate"]
    manifest = crate_dir / "parity.manifest.toml"
    if not manifest.is_file():
        return {"crate": row["crate"], "action": "skip-no-manifest"}
    text = manifest.read_text(encoding="utf-8")
    new_block = render_block(row)
    updated = replace_block(text, new_block)
    if updated == text:
        return {"crate": row["crate"], "action": "noop"}
    manifest.write_text(updated, encoding="utf-8")
    # Distinguish first-write from update by detecting whether the
    # original text contained the block.
    had_block = "[portal_ui]" in text
    return {
        "crate": row["crate"],
        "action": "updated" if had_block else "written",
        "status": row["status"],
        "priority": row["priority"],
    }


def main() -> int:
    if not AUDIT.is_file():
        print(f"error: audit doc missing: {AUDIT}", file=sys.stderr)
        return 1
    rows = parse_audit()
    if not rows:
        print("error: parsed 0 rows from audit table — table syntax mismatch?", file=sys.stderr)
        return 2
    written = updated = skipped = noop = 0
    for row in rows:
        r = process(row)
        a = r.get("action")
        if a == "written":
            written += 1
        elif a == "updated":
            updated += 1
        elif a == "skip-no-manifest":
            skipped += 1
        else:
            noop += 1
    print(
        f"rows={len(rows)}  written={written}  updated={updated}  "
        f"noop={noop}  skipped={skipped}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
