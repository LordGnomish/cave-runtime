#!/usr/bin/env python3
"""
Build `docs/parity/portal-ui-audit-2026-05-11.md` — companion to
`portal-ui-audit-2026-05-12.md` that adds the **upstream-UI URL** +
**P0 / P1 / P2 priority** columns and serves as the source of truth
for the `[portal_ui]` blocks distributed into each crate's
`parity.manifest.toml` by
`scripts/distribute-portal-ui-audit.py`.

For each non-infra crate the script emits one row:

  | Crate | admin/X.rs | Upstream UI | URL | Score | LOC | Priority | Notes |

Where:
  * `admin/X.rs` is "✓" if `crates/cave-portal/src/admin/<short>.rs`
    exists (short = strip "cave-", swap "-" for "_"), else "—".
  * `Score` ∈ {none, scaffold, partial, complete}:
      - `none`     no admin page on disk
      - `scaffold` < 80 lines (page_shell + boilerplate)
      - `partial`  80 ≤ lines < 200 (some forms / tables / handlers)
      - `complete` ≥ 200 lines AND in the curated `COMPLETE` set
        (empty today — promotion is a hand-review)
  * `Priority` ∈ {P0, P1, P2}: from the curated `META` map.
      - P0 release-blocker upstream UI (Grafana / Vault / K8s
        Dashboard / KEDA / Kiali / etcd / kubelet / scheduler /
        apiserver / etc.)
      - P1 important upstream UI (Argo CD/Rollouts, Twenty CRM,
        Tekton, OnCall, Alertmanager, OpenSearch, Linear, …)
      - P2 CLI-first or low-traffic.

The script is deterministic — running it twice on the same tree
produces the same output. It does NOT mutate manifests; that's
`distribute-portal-ui-audit.py`'s job, which re-parses this file.
"""
from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
CRATES = REPO / "crates"
ADMIN = REPO / "crates/cave-portal/src/admin"
OUT = REPO / "docs/parity/portal-ui-audit-2026-05-11.md"

# Infra / scaffold crates excluded from the upstream-UI audit. Aligned
# with `compliance.rs` INFRA_ONLY_FALLBACK plus a few crates whose
# upstream is intentionally headless.
INFRA_ONLY = {
    "cave-cli", "cave-core", "cave-changelog", "cave-types", "cave-utils",
    "cave-cost-alloc", "cave-kernel", "cave-ebpf-common", "cave-runtime",
    "cave-portal", "cave-portal-api", "cave-portal-web", "cave-desktop",
    "cave-scaffold", "cave-docs", "cave-docs-site", "cave-runbook",
    "cave-lint", "cave-pki", "cave-db", "cave-acme", "cave-techdocs",
    "cave-registry", "cave-tracing", "cave-sign", "cave-pii", "cave-flags",
    "cave-status", "cave-profiler", "cave-upstream",
}

# Per-crate metadata (curated).
META: dict[str, dict] = {
    # P0 — release-blocker upstream UIs
    "cave-keda":           {"upstream_ui": "KEDA dashboard (community plugin)", "url": "https://keda.sh/docs/2.16/concepts/", "priority": "P0", "notes": "Scaler / trigger views"},
    "cave-kubelet":        {"upstream_ui": "Kubernetes Dashboard (workloads)", "url": "https://github.com/kubernetes/dashboard", "priority": "P0", "notes": "Per-node Pod / Volume / Lease views"},
    "cave-apiserver":      {"upstream_ui": "Kubernetes Dashboard (resources)", "url": "https://github.com/kubernetes/dashboard", "priority": "P0", "notes": "Generic API resource explorer"},
    "cave-scheduler":      {"upstream_ui": "Kubernetes Dashboard (scheduling)", "url": "https://github.com/kubernetes/dashboard", "priority": "P0", "notes": "Scheduler queue, predicates, priorities"},
    "cave-controller-manager": {"upstream_ui": "Kubernetes Dashboard (controllers)", "url": "https://github.com/kubernetes/dashboard", "priority": "P0", "notes": "Controller status surfaced via k8s_dashboard"},
    "cave-cloud-controller-manager": {"upstream_ui": "Kubernetes Dashboard (cloud)", "url": "https://github.com/kubernetes/dashboard", "priority": "P0", "notes": "Cloud-provider integration status"},
    "cave-etcd":           {"upstream_ui": "etcdctl (CLI-only)", "url": "https://etcd.io/docs/v3.5/op-guide/", "priority": "P0", "notes": "etcd has no canonical UI; cave-side shows revision / KV stats"},
    "cave-vault":          {"upstream_ui": "Vault Web UI (built-in)", "url": "https://developer.hashicorp.com/vault/docs/configuration/ui", "priority": "P0", "notes": "Secret engines, auth methods, policy editor"},
    "cave-dashboard":      {"upstream_ui": "Grafana", "url": "https://grafana.com/grafana/dashboards/", "priority": "P0", "notes": "Cave dashboard renderer; Grafana panel-render parity at admin/grafana.rs"},
    "cave-mesh":           {"upstream_ui": "Kiali (Istio)", "url": "https://kiali.io/", "priority": "P0", "notes": "Service-mesh topology; covered by admin/kiali.rs"},
    "cave-cache":          {"upstream_ui": "RedisInsight (external)", "url": "https://redis.io/insight/", "priority": "P0", "notes": "Key explorer, slow-log"},
    "cave-net":            {"upstream_ui": "Hubble UI (Cilium)", "url": "https://docs.cilium.io/en/stable/observability/hubble/hubble-ui/", "priority": "P0", "notes": "Flow visibility"},

    # P1 — important upstream UIs
    "cave-pg":             {"upstream_ui": "pgAdmin (external)", "url": "https://www.pgadmin.org/", "priority": "P1", "notes": "Postgres admin"},
    "cave-rdbms":          {"upstream_ui": "pgAdmin (external)", "url": "https://www.pgadmin.org/", "priority": "P1", "notes": "Same surface as pg"},
    "cave-rdbms-operator": {"upstream_ui": "CloudNativePG (CRD)", "url": "https://cloudnative-pg.io/", "priority": "P1", "notes": "Cluster lifecycle UI"},
    "cave-streams":        {"upstream_ui": "AKHQ / kafdrop (external)", "url": "https://akhq.io/", "priority": "P1", "notes": "Kafka topic browser"},
    "cave-lakehouse":      {"upstream_ui": "Spark UI (per-app)", "url": "https://spark.apache.org/docs/latest/web-ui.html", "priority": "P1", "notes": "Iceberg snapshot + Spark job views"},
    "cave-docdb":          {"upstream_ui": "MongoDB Compass (external)", "url": "https://www.mongodb.com/products/tools/compass", "priority": "P1", "notes": "Collection browser"},
    "cave-incidents":      {"upstream_ui": "Grafana OnCall", "url": "https://grafana.com/docs/oncall/latest/", "priority": "P1", "notes": "Schedules, escalations"},
    "cave-oncall":         {"upstream_ui": "Grafana OnCall", "url": "https://grafana.com/docs/oncall/latest/", "priority": "P1", "notes": "Duplicate of incidents — TODO collapse"},
    "cave-alerts":         {"upstream_ui": "Alertmanager UI", "url": "https://prometheus.io/docs/alerting/latest/clients/", "priority": "P1", "notes": "Active alerts, silences"},
    "cave-deploy":         {"upstream_ui": "Argo CD UI", "url": "https://argo-cd.readthedocs.io/en/stable/user-guide/", "priority": "P1", "notes": "Application sync graph"},
    "cave-rollouts":       {"upstream_ui": "Argo Rollouts UI", "url": "https://argo-rollouts.readthedocs.io/en/stable/dashboard/", "priority": "P1", "notes": "Canary progress"},
    "cave-pipelines":      {"upstream_ui": "Tekton Dashboard", "url": "https://tekton.dev/docs/dashboard/", "priority": "P1", "notes": "PipelineRun graph"},
    "cave-workflows":      {"upstream_ui": "n8n editor", "url": "https://docs.n8n.io/", "priority": "P1", "notes": "Visual workflow editor — huge scope"},
    "cave-crm":            {"upstream_ui": "Twenty CRM", "url": "https://twenty.com/", "priority": "P1", "notes": "Full CRM React app"},
    "cave-auth":           {"upstream_ui": "Keycloak Admin Console", "url": "https://www.keycloak.org/documentation", "priority": "P1", "notes": "Realm / client / user management"},
    "cave-policy":         {"upstream_ui": "OPA Rego Playground", "url": "https://play.openpolicyagent.org/", "priority": "P1", "notes": "Policy editor"},
    "cave-chaos":          {"upstream_ui": "Chaos Dashboard", "url": "https://chaos-mesh.org/docs/", "priority": "P1", "notes": "Experiment timeline"},
    "cave-trace":          {"upstream_ui": "Jaeger UI", "url": "https://www.jaegertracing.io/", "priority": "P1", "notes": "Trace search + flamegraph"},
    "cave-metrics":        {"upstream_ui": "Prometheus expr browser", "url": "https://prometheus.io/docs/", "priority": "P1", "notes": "Targets, alerts, query; cave-side concept; admin/prometheus.rs ports upstream-UI shape"},
    "cave-logs":           {"upstream_ui": "Grafana Explore (Loki)", "url": "https://grafana.com/docs/loki/", "priority": "P1", "notes": "LogQL query; admin/loki.rs ports upstream-UI shape"},
    "cave-search":         {"upstream_ui": "OpenSearch Dashboards", "url": "https://opensearch.org/docs/", "priority": "P1", "notes": "Discover, visualize"},
    "cave-store":          {"upstream_ui": "MinIO Console", "url": "https://min.io/docs/minio/linux/operations/minio-console.html", "priority": "P1", "notes": "Bucket browser"},
    "cave-vulns":          {"upstream_ui": "DefectDojo", "url": "https://www.defectdojo.org/", "priority": "P1", "notes": "Finding triage"},
    "cave-artifacts":      {"upstream_ui": "Pulp Web UI", "url": "https://pulpproject.org/", "priority": "P1", "notes": "Repository browser"},
    "cave-sbom":           {"upstream_ui": "Dependency-Track", "url": "https://dependencytrack.org/", "priority": "P1", "notes": "Component / vuln correlation"},
    "cave-tracker":        {"upstream_ui": "Linear / Plane", "url": "https://linear.app/", "priority": "P1", "notes": "Issue browser"},
    "cave-erp":            {"upstream_ui": "ERPNext", "url": "https://erpnext.com/", "priority": "P1", "notes": "Full ERP UI"},
    "cave-iceberg":        {"upstream_ui": "Tabular / Nessie (external)", "url": "https://iceberg.apache.org/", "priority": "P1", "notes": "Iceberg has no canonical UI"},

    # P2 — CLI-first or low-traffic upstreams
    "cave-admission":      {"upstream_ui": "(CRD-only)", "url": "https://kubernetes.io/docs/reference/access-authn-authz/admission-controllers/", "priority": "P2", "notes": "Validating/mutating webhook lifecycle"},
    "cave-ai-obs":         {"upstream_ui": "Langfuse", "url": "https://langfuse.com/", "priority": "P2", "notes": "LLM observability"},
    "cave-backup":         {"upstream_ui": "Velero (limited UI)", "url": "https://velero.io/", "priority": "P2", "notes": "Mostly CLI"},
    "cave-cdc":            {"upstream_ui": "Debezium UI (deprecated)", "url": "https://debezium.io/", "priority": "P2", "notes": "Historical UI"},
    "cave-certs":          {"upstream_ui": "(CRD-only)", "url": "https://cert-manager.io/", "priority": "P2", "notes": "cert-manager has no UI"},
    "cave-chat":           {"upstream_ui": "LibreChat", "url": "https://www.librechat.ai/", "priority": "P2", "notes": "Chat client UI"},
    "cave-cluster":        {"upstream_ui": "(CRD-only)", "url": "https://cluster-api.sigs.k8s.io/", "priority": "P2", "notes": "Cluster-API CLI/CRD"},
    "cave-compliance":     {"upstream_ui": "(cave-original)", "url": "(internal)", "priority": "P2", "notes": "The /admin/compliance dashboard itself"},
    "cave-container-scan": {"upstream_ui": "Trivy (CLI)", "url": "https://trivy.dev/", "priority": "P2", "notes": "CLI-first"},
    "cave-cost":           {"upstream_ui": "OpenCost UI", "url": "https://www.opencost.io/", "priority": "P2", "notes": "Cost allocation panels"},
    "cave-cri":            {"upstream_ui": "(CLI-only)", "url": "https://containerd.io/", "priority": "P2", "notes": "containerd is CLI"},
    "cave-crossplane":     {"upstream_ui": "(CRD-first)", "url": "https://www.crossplane.io/", "priority": "P2", "notes": "Minimal UI"},
    "cave-dast":           {"upstream_ui": "OWASP ZAP (Swing)", "url": "https://www.zaproxy.org/", "priority": "P2", "notes": "Desktop Java UI — not portable"},
    "cave-devlake":        {"upstream_ui": "Apache DevLake UI", "url": "https://devlake.apache.org/", "priority": "P2", "notes": "Engineering metrics"},
    "cave-dns":            {"upstream_ui": "(config-only)", "url": "https://coredns.io/", "priority": "P2", "notes": "CoreDNS is corefile-only"},
    "cave-forensics":      {"upstream_ui": "Tetragon (CLI)", "url": "https://tetragon.io/", "priority": "P2", "notes": "CLI-first"},
    "cave-gateway":        {"upstream_ui": "Kong Manager", "url": "https://docs.konghq.com/", "priority": "P2", "notes": "Plugin / route browser"},
    "cave-gitops-config":  {"upstream_ui": "Flux (CLI)", "url": "https://fluxcd.io/", "priority": "P2", "notes": "CLI-first"},
    "cave-ha":             {"upstream_ui": "(CRD-only)", "url": "https://etcd.io/docs/v3.5/op-guide/", "priority": "P2", "notes": "etcd HA is CLI"},
    "cave-infra":          {"upstream_ui": "Terraform Cloud (proprietary)", "url": "https://www.hashicorp.com/products/terraform", "priority": "P2", "notes": "OSS Terraform is CLI"},
    "cave-kamaji":         {"upstream_ui": "(CRD-only)", "url": "https://kamaji.clastix.io/", "priority": "P2", "notes": "kamaji is CRD/CLI"},
    "cave-karpenter":      {"upstream_ui": "(CRD-only)", "url": "https://karpenter.sh/", "priority": "P2", "notes": "karpenter is CRD"},
    "cave-knative":        {"upstream_ui": "(CRD-only)", "url": "https://knative.dev/", "priority": "P2", "notes": "knative is CRD"},
    "cave-kube-proxy":     {"upstream_ui": "(iptables-only)", "url": "https://kubernetes.io/docs/concepts/services-networking/", "priority": "P2", "notes": "no UI"},
    "cave-kubevirt":       {"upstream_ui": "KubeVirt UI (limited)", "url": "https://kubevirt.io/", "priority": "P2", "notes": "Mostly CLI"},
    "cave-ledger":         {"upstream_ui": "(cave-original)", "url": "(internal)", "priority": "P2", "notes": "Internal audit ledger"},
    "cave-llm-gateway":    {"upstream_ui": "LiteLLM admin UI", "url": "https://docs.litellm.ai/", "priority": "P2", "notes": "Newer admin UI"},
    "cave-local-llm":      {"upstream_ui": "(CLI-only)", "url": "https://ollama.com/", "priority": "P2", "notes": "Ollama is CLI"},
    "cave-pam":            {"upstream_ui": "Teleport Web UI", "url": "https://goteleport.com/docs/", "priority": "P2", "notes": "Access proxy UI"},
    "cave-permission":     {"upstream_ui": "Casbin (CLI)", "url": "https://casbin.org/", "priority": "P2", "notes": "RBAC primitive"},
    "cave-scan":           {"upstream_ui": "SonarQube", "url": "https://www.sonarsource.com/products/sonarqube/", "priority": "P2", "notes": "Code-quality UI"},
    "cave-secrets":        {"upstream_ui": "TruffleHog (CLI)", "url": "https://trufflesecurity.com/trufflehog", "priority": "P2", "notes": "CLI-first"},
    "cave-security":       {"upstream_ui": "Falco (limited)", "url": "https://falco.org/", "priority": "P2", "notes": "Falcosidekick UI is partial"},
    "cave-slo":            {"upstream_ui": "OpenSLO (spec)", "url": "https://openslo.com/", "priority": "P2", "notes": "Spec-first, no canonical UI"},
    "cave-spire":          {"upstream_ui": "(CRD/CLI)", "url": "https://spiffe.io/", "priority": "P2", "notes": "SPIRE is CLI"},
    "cave-uptime":         {"upstream_ui": "Uptime Kuma", "url": "https://uptime.kuma.pet/", "priority": "P2", "notes": "Status page builder"},
    "cave-vcluster":       {"upstream_ui": "vCluster Platform", "url": "https://www.vcluster.com/", "priority": "P2", "notes": "Virtual cluster UI"},

    # cave-side helpers that have admin pages but no upstream-UI counterpart
    "cave-contributions":  {"upstream_ui": "(cave-original)", "url": "(internal)", "priority": "P2", "notes": "Internal contributions dashboard"},
    "cave-iam":            {"upstream_ui": "(cave-original)", "url": "(internal)", "priority": "P2", "notes": "Internal IAM browser"},
    "cave-tenant-dashboard":{"upstream_ui": "(cave-original)", "url": "(internal)", "priority": "P2", "notes": "Internal tenant UI"},
}

# Promotion list — admin pages hand-reviewed and confirmed as a
# faithful upstream-UI port. Promoted on 2026-05-12 after the
# 11-crate P0 expansion batch (see commits
# `feat(portal): expand <N> admin pages …`).
#
# * `cave-vault` — folder-split (mod / secrets_engines / auth_methods /
#   policies / kv_browser / audit) mirroring Vault's four UI tabs
#   plus the secrets-engine mount list. Plaintext-protection
#   invariant preserved.
# * `cave-mesh` — Kiali-faithful aggregations: workloads (by source)
#   + services (by destination, with health classification) + authz
#   + flows. 484 LOC, 13 tests.
#
# 2026-05-12 second batch (feat/portal-5-p0-complete-promotion):
#
# * `cave-kubelet` — admin/kubelet/ folder split (pods / nodes /
#   volumes / events / metrics) mirroring Kubernetes Dashboard
#   per-node view. ~750 LOC, 30 tests.
# * `cave-dashboard` — admin/grafana/ folder split (dashboards / panels /
#   datasources / explore / alerts) — see PORTAL_UI_OVERRIDE.
#   ~900 LOC, 25 tests.
# * `cave-metrics` — admin/prometheus/ folder split (targets / rules /
#   tsdb / flags / status) — see PORTAL_UI_OVERRIDE. ~650 LOC, 23
#   tests.
# * `cave-apiserver` — admin/k8s_dashboard/ folder split (workloads /
#   services / config / storage / cluster) — see PORTAL_UI_OVERRIDE.
#   ~700 LOC, 24 tests.
#
# (`cave-mesh` already complete keeps its mesh.rs; the bonus
# admin/kiali/ folder ships under the kiali URL but does not change
# cave-mesh's audit row.)
COMPLETE: set[str] = {
    "cave-vault",
    "cave-mesh",
    "cave-kubelet",
    "cave-dashboard",
    "cave-metrics",
    "cave-apiserver",
    # 2026-05-12 third batch (feat/portal-6-more-p0-complete):
    # six P0 admin pages promoted via folder split + 5 tabs each.
    # * `cave-cache` — keyspace + commands + clients + replication + pubsub (26 tests)
    # * `cave-net` — flows + policies + services + nodes + identities (23 tests)
    # * `cave-cloud-controller-manager` — node/route/service/volume/instance-metadata (22 tests)
    # * `cave-controller-manager` — controllers/leader-election/events/queues/reconciler-metrics (22 tests)
    # * `cave-etcd` — members/keyspace/leases/alarms/metrics (20 tests)
    # * `cave-scheduler` — queue/plugins/bindings/nodescores/events (23 tests)
    "cave-cache",
    "cave-net",
    "cave-cloud-controller-manager",
    "cave-controller-manager",
    "cave-etcd",
    "cave-scheduler",
    # 2026-05-13 fourth batch (feat/portal-cli-streams-auth-batch4):
    # five P1 admin pages promoted via folder split + 5 tabs each.
    # * cave-auth     → admin/auth/    — realms / clients / users / sessions / events
    # * cave-streams  → admin/streams/ — topics / brokers / consumer_groups / partitions / acls
    # * cave-erp      → admin/erp/     — invoices / inventory / accounting / hr / projects
    # * cave-crm      → admin/crm/     — contacts / deals / activities / workflows / reports
    # * cave-trivy    → admin/container_scan/ — vulnerabilities / images / policies / history / reports
    "cave-auth",
    "cave-streams",
    "cave-erp",
    "cave-crm",
    "cave-container-scan",
}

# Some admin pages live under a URL/short-name that differs from the
# crate's short-name (e.g. cave-dashboard's portal UI lives at
# admin/grafana/, not admin/dashboard.rs). This map tells the audit
# script which folder to measure for those crates.
PORTAL_UI_OVERRIDE: dict[str, str] = {
    "cave-dashboard": "grafana",
    "cave-metrics": "prometheus",
    "cave-apiserver": "k8s_dashboard",
    "cave-container-scan": "container_scan",
}

SCORE_VALUE = {"none": 0, "scaffold": 25, "partial": 60, "complete": 100}


def admin_short(crate: str) -> str:
    """Resolve the admin URL short-name for `crate`. PORTAL_UI_OVERRIDE
    takes precedence so a crate whose portal page lives under a
    differently-named folder is still measured correctly."""
    if crate in PORTAL_UI_OVERRIDE:
        return PORTAL_UI_OVERRIDE[crate]
    return crate.removeprefix("cave-").replace("-", "_")


def admin_path(crate: str) -> Path | None:
    """Return the admin entry point for `crate`. Some admin views are
    a single `.rs` file, others (vault, keda) live under a folder
    with a `mod.rs`. This helper picks whichever shape exists, or
    returns `None` when neither does."""
    short = admin_short(crate)
    single = ADMIN / f"{short}.rs"
    if single.is_file():
        return single
    folder_mod = ADMIN / short / "mod.rs"
    if folder_mod.is_file():
        return folder_mod
    return None


def admin_loc(crate: str) -> int:
    """Total `.rs` LOC for `crate`'s admin view. For folder-shaped
    views, sums every `.rs` under `admin/<short>/`."""
    short = admin_short(crate)
    single = ADMIN / f"{short}.rs"
    if single.is_file():
        try:
            return sum(1 for _ in single.open(encoding="utf-8"))
        except OSError:
            return 0
    folder = ADMIN / short
    if folder.is_dir():
        total = 0
        for f in folder.rglob("*.rs"):
            try:
                total += sum(1 for _ in f.open(encoding="utf-8"))
            except OSError:
                continue
        return total
    return 0


def score_for(crate: str, loc: int) -> str:
    if loc == 0:
        return "none"
    if crate in COMPLETE:
        return "complete"
    if loc < 80:
        return "scaffold"
    if loc < 200:
        return "partial"
    return "partial"


def all_crates() -> list[str]:
    out = []
    for p in sorted(CRATES.iterdir()):
        if p.is_dir() and (p / "Cargo.toml").is_file():
            out.append(p.name)
    return out


def emit_rows() -> list[dict]:
    rows = []
    for crate in all_crates():
        if crate in INFRA_ONLY:
            continue
        ap = admin_path(crate)
        loc = admin_loc(crate)
        meta = META.get(crate, {})
        score = score_for(crate, loc)
        rows.append({
            "crate": crate,
            "has_admin": "✓" if ap is not None else "—",
            "upstream_ui": meta.get("upstream_ui", "(unmapped)"),
            "url": meta.get("url", "—"),
            "score": score,
            "score_value": SCORE_VALUE[score],
            "loc": loc,
            "priority": meta.get("priority", "P2"),
            "notes": meta.get("notes", "needs hand-classification"),
        })
    return rows


def headline(rows: list[dict]) -> dict:
    by_score: dict[str, int] = {}
    by_priority: dict[str, int] = {}
    for r in rows:
        by_score[r["score"]] = by_score.get(r["score"], 0) + 1
        by_priority[r["priority"]] = by_priority.get(r["priority"], 0) + 1
    total_score = sum(r["score_value"] for r in rows)
    avg_score = total_score // len(rows) if rows else 0
    return {
        "total": len(rows),
        "by_score": by_score,
        "by_priority": by_priority,
        "avg_score": avg_score,
    }


NEW_PAGES_BLOCK = """
## Five new admin pages added in this paket

These pages bring upstream-UI parity for the highest-traffic
dashboards. They live alongside existing cave-side pages
(`dashboard.rs`, `metrics.rs`, `logs.rs`, `mesh.rs`) which cover the
*cave-side* concept; the new pages mirror the *upstream-UI* shape.

| Page | Upstream UI | URL | Backed cave crate(s) | Priority |
|---|---|---|---|---|
| `admin/grafana.rs` | Grafana panel-render | https://grafana.com/grafana/dashboards/ | cave-dashboard | P0 |
| `admin/prometheus.rs` | Prometheus targets / alerts | https://prometheus.io/docs/ | cave-metrics | P0 |
| `admin/loki.rs` | Loki LogQL query | https://grafana.com/docs/loki/ | cave-logs | P0 |
| `admin/k8s_dashboard.rs` | Kubernetes Dashboard | https://github.com/kubernetes/dashboard | cave-kubelet, cave-apiserver, cave-scheduler, cave-controller-manager | P0 |
| `admin/kiali.rs` | Istio Kiali topology | https://kiali.io/ | cave-mesh | P0 |
"""


HONEST_FOOTER = """
---

**Honest invariants:**

- **`score` is derived from LOC** (a heuristic; see `score_for()` in
  `scripts/build-portal-ui-priority-audit.py`). A rich page that is
  all `page_shell + table()` lands in `partial` until hand-promoted
  to `complete` via the `COMPLETE` set.
- **No row claims `complete` today.** Promotion is a hand-review and
  a follow-up. The five new pages added in this paket are explicit
  scaffolds (Backstage-pattern; see commit
  `feat(portal): 5 P0 admin pages …`).
- **`priority` is a curated label** from the script's `META` map. P0
  is reserved for release-blocker upstream UIs (Grafana / Vault /
  K8s Dashboard / KEDA / Kiali / etcd / kubelet / scheduler /
  apiserver / etc.). Adjustments must edit `META` and re-run the
  script.
- **`portal_ui_avg_score` is computed live by the dashboard** from the
  `[portal_ui]` blocks in each `parity.manifest.toml`, NOT from this
  file directly. Run `scripts/distribute-portal-ui-audit.py` after
  editing this audit to keep the manifests + dashboard in sync.
"""


def render(rows: list[dict], data: dict) -> str:
    parts: list[str] = []
    parts.append("# Portal UI Audit — 2026-05-11")
    parts.append("")
    parts.append("Companion to `portal-ui-audit-2026-05-12.md` (size + density")
    parts.append("heuristic). This file adds **upstream-UI URL** + **P0 / P1 /")
    parts.append("P2 priority** columns and is the source of truth for the")
    parts.append("`[portal_ui]` blocks distributed into each crate's")
    parts.append("`parity.manifest.toml` by")
    parts.append("`scripts/distribute-portal-ui-audit.py`. The")
    parts.append("`/admin/compliance` dashboard reads those blocks back and")
    parts.append("renders the **Portal UI Parity** grade alongside the")
    parts.append("existing Structural and Upstream Parity grades.")
    parts.append("")
    parts.append("## Headline")
    parts.append("")
    parts.append("| Bucket | Count |")
    parts.append("|---|--:|")
    parts.append(f"| Total crates in audit (non-infra) | {data['total']} |")
    for s in ("none", "scaffold", "partial", "complete"):
        parts.append(f"| `{s}` | {data['by_score'].get(s, 0)} |")
    parts.append("")
    parts.append("**Priority distribution**")
    parts.append("")
    parts.append("| Priority | Count |")
    parts.append("|---|--:|")
    for p in ("P0", "P1", "P2"):
        parts.append(f"| {p} | {data['by_priority'].get(p, 0)} |")
    parts.append("")
    parts.append(f"**Portal UI average score:** **{data['avg_score']} / 100**")
    parts.append("")
    parts.append("Score values: `none = 0`, `scaffold = 25`, `partial = 60`,")
    parts.append("`complete = 100`. Average is the arithmetic mean across")
    parts.append("non-infra crates.")
    parts.append("")
    parts.append("## Full per-crate table")
    parts.append("")
    parts.append("| Crate | admin/X.rs | Upstream UI | URL | Score | LOC | Priority | Notes |")
    parts.append("|---|:-:|---|---|---|--:|:-:|---|")
    for r in rows:
        url = r["url"]
        url_md = f"[link]({url})" if url.startswith("http") else url
        parts.append(
            f"| `{r['crate']}` | {r['has_admin']} | {r['upstream_ui']} | {url_md} "
            f"| `{r['score']}` | {r['loc']} | {r['priority']} | {r['notes']} |"
        )
    parts.append("")
    parts.append(NEW_PAGES_BLOCK.strip())
    parts.append("")
    parts.append(HONEST_FOOTER.strip())
    return "\n".join(parts) + "\n"


def main() -> int:
    rows = emit_rows()
    data = headline(rows)
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(render(rows, data), encoding="utf-8")
    print(
        f"Wrote {OUT.relative_to(REPO)}: {data['total']} crates "
        f"(P0={data['by_priority'].get('P0', 0)} "
        f"P1={data['by_priority'].get('P1', 0)} "
        f"P2={data['by_priority'].get('P2', 0)}) "
        f"avg_score={data['avg_score']}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
