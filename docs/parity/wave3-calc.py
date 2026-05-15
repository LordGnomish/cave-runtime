#!/usr/bin/env python3
"""
Pure-Python mirror of crates/cave-kernel/src/parity/calculator.rs.

Walks crates/*/parity.manifest.toml and reports the same scores the Rust
calculator would produce. Used to prove pre/post deltas without spinning up a
Cargo build.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

try:
    import tomllib  # py3.11+
except ImportError:  # pragma: no cover
    import tomli as tomllib  # type: ignore

ROOT = Path(__file__).resolve().parents[2]
CRATES = ROOT / 'crates'


def file_exists(crate_root: Path, rel: str) -> bool:
    return (crate_root / rel).exists()


def file_contains(crate_root: Path, rel: str, pattern: str) -> bool:
    p = crate_root / rel
    if not p.is_file():
        return False
    try:
        return pattern in p.read_text(encoding='utf-8', errors='replace')
    except OSError:
        return False


def source_contains(crate_root: Path, source_root: str, pattern: str) -> bool:
    base = crate_root / source_root
    if not base.is_dir():
        return False
    for f in base.rglob('*.rs'):
        try:
            if pattern in f.read_text(encoding='utf-8', errors='replace'):
                return True
        except OSError:
            continue
    return False


def count_stubs(crate_root: Path, source_root: str) -> int:
    base = crate_root / source_root
    if not base.is_dir():
        return 0
    n = 0
    for f in base.rglob('*.rs'):
        try:
            for line in f.read_text(encoding='utf-8', errors='replace').splitlines():
                t = line.strip()
                if t.startswith('//'):
                    continue
                if 'todo!' in t or 'unimplemented!' in t:
                    n += 1
        except OSError:
            continue
    return n


def calc_one(crate_root: Path) -> dict:
    manifest_path = crate_root / 'parity.manifest.toml'
    if not manifest_path.exists():
        return {'module': crate_root.name, 'has_manifest': False}
    try:
        m = tomllib.loads(manifest_path.read_text(encoding='utf-8'))
    except Exception as e:
        return {'module': crate_root.name, 'has_manifest': True, 'parse_error': str(e)}
    src_root = (m.get('module') or {}).get('source_root') or 'src'

    files = m.get('files', []) or []
    fns = m.get('functions', []) or []
    tests = m.get('tests', []) or []
    surfaces = m.get('surfaces', []) or []

    def metric(matched: int, total: int) -> dict:
        score = (matched / total) if total > 0 else 0.0
        return {'score': score, 'matched': matched, 'total': total}

    file_match = sum(1 for f in files if file_exists(crate_root, f.get('local', '')))
    fn_match = sum(1 for f in fns if file_contains(crate_root, f.get('file', ''),
                                                    f"fn {f.get('local_name', '')}"))
    test_match = sum(1 for t in tests if source_contains(crate_root, src_root,
                                                         f"fn {t.get('local_test', '')}"))
    surf_match = sum(1 for s in surfaces if source_contains(crate_root, src_root,
                                                            s.get('local_path', '')))

    fp = metric(file_match, len(files))
    fnp = metric(fn_match, len(fns))
    tp = metric(test_match, len(tests))
    sp = metric(surf_match, len(surfaces))

    overall = (fp['score'] + fnp['score'] + tp['score'] + sp['score']) / 4.0
    stubs = count_stubs(crate_root, src_root)

    upstream = m.get('upstream', {}) or {}
    return {
        'module': (m.get('module') or {}).get('name') or crate_root.name,
        'has_manifest': True,
        'upstream': f"{upstream.get('org', '?')}/{upstream.get('repo', '?')} @ {upstream.get('version', '?')}",
        'file_parity': fp,
        'function_parity': fnp,
        'test_parity': tp,
        'surface_parity': sp,
        'overall': overall,
        'stubs': stubs,
    }


def discover() -> list[dict]:
    out = []
    for d in sorted(CRATES.iterdir()):
        if d.is_dir() and d.name.startswith('cave-'):
            out.append(calc_one(d))
    return out


if __name__ == '__main__':
    reports = discover()
    if '--json' in sys.argv:
        print(json.dumps(reports, indent=2))
    else:
        # Compact table
        print(f"{'module':<28} {'overall':>8} {'files':>8} {'fn':>8} {'tests':>8} {'surf':>8} {'stubs':>6}")
        print('-' * 80)
        for r in reports:
            if not r.get('has_manifest'):
                print(f"{r['module']:<28} {'NO-MANIFEST':>8}")
                continue
            if 'parse_error' in r:
                print(f"{r['module']:<28} PARSE-ERROR: {r['parse_error'][:40]}")
                continue
            def fmt(m): return f"{m['matched']}/{m['total']}"
            print(f"{r['module']:<28} {r['overall']:>7.1%}  {fmt(r['file_parity']):>8} {fmt(r['function_parity']):>8} "
                  f"{fmt(r['test_parity']):>8} {fmt(r['surface_parity']):>8} {r['stubs']:>6}")

        n = len(reports)
        zero = sum(1 for r in reports if r.get('has_manifest') and r.get('overall', 0) < 0.001)
        nm = sum(1 for r in reports if not r.get('has_manifest'))
        print('-' * 80)
        print(f"total={n}  no-manifest={nm}  overall=0%={zero}")
