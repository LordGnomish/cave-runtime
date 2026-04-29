#!/usr/bin/env python3
"""
Generate manifest-fill payloads for Qwen pump.

Sources:
1. crates/cave-upstream/src/projects.rs (52 cave_modules auto-mapped)
2. Manual mappings for crates not covered by projects.rs
3. Cave-native skip list (no upstream parity)

Output:
- ~/Library/Application Support/cave-qwen-pump/queue/manifest-fill-<module>-<ts>.json
- ~/Library/Application Support/cave-runtime/manifest-fill-progress.json
"""
import json
import os
import re
import time
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path

REPO = Path('/Users/gnomish/Code/cave-runtime/.claude/worktrees/eager-proskuriakova-fe8ccf')
QUEUE_DIR = Path.home() / 'Library/Application Support/cave-qwen-pump/queue'
STATE_DIR = Path.home() / 'Library/Application Support/cave-runtime'
PROGRESS_FILE = STATE_DIR / 'manifest-fill-progress.json'

# Modules with no upstream parity (Cave-native) — explicitly skipped per Burak's brief
CAVE_NATIVE = {
    'cave-runtime',         # delta engine
    'cave-upstream',        # the watchd itself
    'cave-kernel',          # parity calculator
    'cave-changelog',       # internal changelog
    'cave-core',            # umbrella core types
    'cave-db',              # generic db abstraction
    'cave-erp',             # internal ERP
    'cave-ledger',          # internal ledger
    'cave-lint',            # internal lint
    'cave-pam',             # internal PAM
    'cave-permission',      # internal permission
    'cave-runbook',         # internal runbook
    'cave-status',          # internal status
    'cave-tracker',         # internal tracker
    'cave-scaffold',        # internal scaffolder (could map to backstage scaffolder later)
    'cave-portal-api',      # internal API to portal
}

# UI/docs-only modules — skipped per Burak's exception "upstream UI port edilmez"
UI_ONLY = {
    'cave-docs',
    'cave-docs-site',
    'cave-techdocs',
    'cave-dashboard',  # primary surface is UI; backend portion already lives in cave-portal
}

# Manual upstream mappings for modules not in projects.rs
MANUAL = {
    'cave-alerts':         {'github_repo': 'prometheus/alertmanager',           'name': 'Alertmanager',          'category': 'observability'},
    'cave-artifacts':      {'github_repo': 'pulp/pulp',                         'name': 'Pulp',                  'category': 'artifacts'},
    'cave-cli':            {'github_repo': 'kubernetes/kubectl',                'name': 'kubectl/cavectl',       'category': 'cli',
                            'note': 'cavectl bundles surface from kubectl, etcdctl, helm, argocd, vault, etc. Start with kubectl core verbs.'},
    'cave-compliance':     {'github_repo': 'open-policy-agent/gatekeeper',      'name': 'OPA Gatekeeper',        'category': 'security'},
    'cave-container-scan': {'github_repo': 'aquasecurity/trivy',                'name': 'Trivy',                 'category': 'security'},
    'cave-cost-alloc':     {'github_repo': 'opencost/opencost',                 'name': 'OpenCost',              'category': 'operations'},
    'cave-crossplane':     {'github_repo': 'crossplane/crossplane',             'name': 'Crossplane',            'category': 'gitops'},
    'cave-datafusion':     {'github_repo': 'apache/datafusion',                 'name': 'Apache DataFusion',     'category': 'data-platform'},
    'cave-docdb':          {'github_repo': 'mongodb/mongo',                     'name': 'MongoDB',               'category': 'database'},
    'cave-external-secrets': {'github_repo': 'external-secrets/external-secrets', 'name': 'External Secrets Operator', 'category': 'secrets'},
    'cave-gitops-config':  {'github_repo': 'argoproj/argo-cd',                  'name': 'ArgoCD (config)',       'category': 'gitops'},
    'cave-hubble':         {'github_repo': 'cilium/hubble',                     'name': 'Hubble',                'category': 'networking'},
    'cave-iceberg':        {'github_repo': 'apache/iceberg',                    'name': 'Apache Iceberg',        'category': 'data-platform'},
    'cave-kamaji':         {'github_repo': 'clastix/kamaji',                    'name': 'Kamaji',                'category': 'kubernetes'},
    'cave-keda':           {'github_repo': 'kedacore/keda',                     'name': 'KEDA',                  'category': 'kubernetes'},
    'cave-knative':        {'github_repo': 'knative/serving',                   'name': 'Knative Serving',       'category': 'serverless',
                            'note': 'Also covers knative/eventing — list both upstream paths.'},
    'cave-local-llm':      {'github_repo': 'ollama/ollama',                     'name': 'Ollama',                'category': 'ai'},
    'cave-net':            {'github_repo': 'cilium/cilium',                     'name': 'Cilium',                'category': 'networking',
                            'note': 'projects.rs maps cave-net to flannel; effective upstream is cilium per Burak step 8.'},
    'cave-oncall':         {'github_repo': 'grafana/oncall',                    'name': 'Grafana OnCall',        'category': 'operations'},
    'cave-pipelines':      {'github_repo': 'argoproj/argo-workflows',           'name': 'Argo Workflows',        'category': 'data-platform',
                            'note': 'Could also extend with apache/airflow per ADR-031 — start with argo-workflows.'},
    'cave-rdbms':          {'github_repo': 'postgres/postgres',                 'name': 'PostgreSQL',            'category': 'database'},
    'cave-secrets':        {'github_repo': 'trufflesecurity/trufflehog',        'name': 'TruffleHog',            'category': 'security'},
    'cave-security':       {'github_repo': 'aquasecurity/kube-bench',           'name': 'kube-bench',            'category': 'security',
                            'note': 'Umbrella — primary upstream kube-bench (CIS). Also extend with falcosecurity/falco runtime later.'},
    'cave-spire':          {'github_repo': 'spiffe/spire',                      'name': 'SPIRE',                 'category': 'identity'},
    'cave-vcluster':       {'github_repo': 'loft-sh/vcluster',                  'name': 'vcluster',              'category': 'kubernetes'},
}

# Override projects.rs mapping when Burak's brief says otherwise
OVERRIDE = {
    # cave-net in projects.rs maps to flannel; Burak step 8 says cilium
    'cave-net': MANUAL['cave-net'],
}

# ── Parse projects.rs ────────────────────────────────────────────────
src = (REPO / 'crates/cave-upstream/src/projects.rs').read_text()
blocks = re.findall(r'TrackedProject\s*\{([^}]*?)\}', src, re.DOTALL)

mod_to_upstreams: dict[str, list[dict]] = defaultdict(list)
for b in blocks:
    def f(name):
        m = re.search(rf'{name}:\s*"([^"]*)"', b)
        return m.group(1) if m else None
    cat_m = re.search(r'category:\s*"([^"]*)"', b)
    p = {
        'name': f('name'),
        'github_repo': f('github_repo'),
        'cave_module': f('cave_module'),
        'features': f('track_features'),
        'category': cat_m.group(1) if cat_m else None,
    }
    if p['cave_module']:
        mod_to_upstreams[p['cave_module']].append(p)

# ── Crate inventory ─────────────────────────────────────────────────
crates_dir = REPO / 'crates'
on_disk = sorted(d.name for d in crates_dir.iterdir() if d.is_dir() and d.name.startswith('cave-'))

# ── Build payload set ───────────────────────────────────────────────
payloads = []
skipped = []

for crate in on_disk:
    if crate in CAVE_NATIVE:
        skipped.append({'module': crate, 'reason': 'cave-native (no upstream parity)'})
        continue
    if crate in UI_ONLY:
        skipped.append({'module': crate, 'reason': 'UI-only / docs-only (per upstream-UI-port-edilmez exception)'})
        continue

    # 1. projects.rs auto-mapping (may have multiple upstreams)
    upstreams = mod_to_upstreams.get(crate, [])
    primary = None
    additional = []

    if crate in OVERRIDE:
        primary = OVERRIDE[crate]
    elif upstreams:
        primary = upstreams[0]
        additional = upstreams[1:]
    elif crate in MANUAL:
        primary = MANUAL[crate]
    else:
        skipped.append({'module': crate, 'reason': 'no clear upstream (needs Burak decision)'})
        continue

    payloads.append({
        'crate': crate,
        'primary': primary,
        'additional': additional,
    })

print(f'Generating payloads for {len(payloads)} modules')
print(f'Skipped: {len(skipped)} modules')
for s in skipped:
    print(f'  SKIP {s["module"]:<25s} — {s["reason"]}')

# ── Write payloads ──────────────────────────────────────────────────
QUEUE_DIR.mkdir(parents=True, exist_ok=True)
STATE_DIR.mkdir(parents=True, exist_ok=True)

run_id = f'manifest-fill-sweep-{datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")}'
ts_ms_base = int(time.time() * 1000)

written = []
for i, p in enumerate(sorted(payloads, key=lambda x: x['crate'])):
    crate = p['crate']
    primary = p['primary']
    additional = p['additional']
    ts_ms = ts_ms_base + i

    payload = {
        'schema_version': 1,
        'kind': 'manifest-fill',
        'cave_module': crate,
        'upstream_repo': primary['github_repo'],
        'upstream_name': primary.get('name'),
        'category': primary.get('category'),
        'additional_upstream_repos': [a['github_repo'] for a in additional],
        'target_manifest': f'crates/{crate}/parity.manifest.toml',
        'instructions': (
            f"Fill {crate}'s parity.manifest.toml against {primary['github_repo']}. "
            "Extract upstream public surface (exported fn / route / type / RPC). "
            "For each upstream symbol: write a [[functions]] (or [[surfaces]]) entry "
            "with the fields the existing schema expects (upstream_name, local_name, file). "
            "Verify local Rust equivalent via rg 'fn <name>' under crates/" + crate + "/src/. "
            "If found → status='mapped'. If absent → status='unmapped'. "
            "Mark UI files (templates/, web/, frontend/) as status='skipped (UI)'. "
            "Mark CLI binaries (cmd/cli, bin/...ctl) as status='skipped (CLI)'. "
            "Mark v1beta1/Deprecated/Legacy as status='skipped (backcompat)'. "
            "Final test gate: cargo test -p " + crate + " must compile (no new failing tests added). "
            "Reference: crates/cave-etcd/parity.manifest.toml is the gold standard format."
        ),
        'priority': 'normal',
        'created_at': datetime.now(timezone.utc).isoformat(),
        'origin': {
            'daemon': 'manifest-fill-sweep',
            'run_id': run_id,
        },
    }
    if 'features' in primary and primary['features']:
        payload['feature_hint'] = primary['features']
    if 'note' in primary:
        payload['note'] = primary['note']

    fname = f'manifest-fill-{crate}-{ts_ms}.json'
    fpath = QUEUE_DIR / fname
    tmp_path = QUEUE_DIR / f'{fname}.tmp'
    tmp_path.write_text(json.dumps(payload, indent=2, ensure_ascii=False))
    tmp_path.rename(fpath)
    written.append({'crate': crate, 'file': fname, 'upstream': primary['github_repo']})

# ── Write progress tracker ──────────────────────────────────────────
progress = {
    'schema_version': 1,
    'run_id': run_id,
    'generated_at': datetime.now(timezone.utc).isoformat(),
    'queue_dir': str(QUEUE_DIR),
    'total_payloads': len(written),
    'modules': {
        w['crate']: {
            'status': 'queued',
            'queue_file': w['file'],
            'upstream_repo': w['upstream'],
            'queued_at': datetime.now(timezone.utc).isoformat(),
        } for w in written
    },
    'skipped': skipped,
}

# Atomic write
tmp = PROGRESS_FILE.with_suffix('.tmp')
tmp.write_text(json.dumps(progress, indent=2, ensure_ascii=False))
tmp.rename(PROGRESS_FILE)

print()
print(f'✓ Wrote {len(written)} payloads to {QUEUE_DIR}')
print(f'✓ Wrote progress tracker to {PROGRESS_FILE}')
print(f'✓ Run ID: {run_id}')

