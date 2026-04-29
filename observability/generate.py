#!/usr/bin/env python3
"""
Generator for the observability catalog.

Emits a Grafana dashboard JSON + Prometheus alert YAML per runtime module.
Dashboards target Grafana 10+ schema (schemaVersion 39); alerts use the
standard `groups: [{name, rules: [...]}]` form Prometheus / Loki / Mimir
expect.

Module specs are intentionally compact: each entry just names the metric
prefix used by the runtime so that the standard ten panels and eight
alerts compose directly. Modules that don't yet emit any metrics still
get the catalog entry — the panels light up when the runtime starts
producing the named series.

Run:
    python3 observability/generate.py

Output:
    observability/dashboards/<module>.json    (10 panels)
    observability/alerts/<module>.yml         (8 rules)

A summary table is printed to stdout at the end.
"""

import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent
DASH_DIR = ROOT / "dashboards"
ALERT_DIR = ROOT / "alerts"
DASH_DIR.mkdir(parents=True, exist_ok=True)
ALERT_DIR.mkdir(parents=True, exist_ok=True)

# ─── Module specs ──────────────────────────────────────────────────────────
# (module, metric_prefix, summary)
#
# Library/binary crates (cave-core, cave-kernel, cave-cli, cave-runtime,
# cave-tracing SDK, cave-db, cave-ebpf-common, cave-docs-site, cave-pki,
# cave-ledger, cave-permission, cave-gitops-config, cave-portal-web,
# cave-cost-alloc, cave-changelog, cave-status, cave-techdocs, cave-flags,
# cave-lint, cave-docs) are intentionally NOT in this list — they don't
# expose a metrics endpoint of their own.

MODULES = [
    # ─── Kubernetes control plane ────────────────────────────────────────
    ("cave-apiserver",                "apiserver_request",      "Kubernetes API server parity"),
    ("cave-etcd",                     "etcd_server",            "etcd KV store"),
    ("cave-cri",                      "cri_operations",         "Container runtime interface"),
    ("cave-kubelet",                  "kubelet_pod",            "Node-side kubelet"),
    ("cave-scheduler",                "scheduler_schedule",     "Pod scheduler"),
    ("cave-controller-manager",       "controller_manager",     "Built-in controllers"),
    ("cave-cloud-controller-manager", "ccm",                    "Cloud reconciler"),
    ("cave-kube-proxy",               "kube_proxy",             "Service-VIP data-plane"),
    ("cave-net",                      "cilium",                 "Cilium-parity CNI"),
    ("cave-gateway",                  "gateway",                "Ingress / API gateway"),

    # ─── Data layer ──────────────────────────────────────────────────────
    ("cave-pg",                       "pg",                     "Postgres-parity OLTP"),
    ("cave-rdbms",                    "rdbms",                  "RDBMS proxy"),
    ("cave-cache",                    "cache",                  "DragonflyDB-parity cache"),
    ("cave-store",                    "store",                  "Generic KV store"),
    ("cave-docdb",                    "docdb",                  "Document database"),
    ("cave-search",                   "search",                 "Search engine"),
    ("cave-iceberg",                  "iceberg",                "Iceberg lake"),
    ("cave-datafusion",               "datafusion",             "Query engine"),
    ("cave-streams",                  "kafka",                  "Kafka-parity streams"),
    ("cave-cdc",                      "cdc",                    "Change-data capture"),
    ("cave-artifacts",                "artifacts",              "Artifact store"),
    ("cave-backup",                   "backup",                 "Backup / restore"),

    # ─── Observability self-stack ────────────────────────────────────────
    ("cave-metrics",                  "prom",                   "Metrics store"),
    ("cave-logs",                     "loki",                   "Log aggregation"),
    ("cave-trace",                    "trace",                  "Distributed tracing store"),
    ("cave-alerts",                   "alertmanager",           "Alert manager"),
    ("cave-dashboard",                "grafana",                "Dashboard engine"),
    ("cave-oncall",                   "oncall",                 "On-call rotation"),
    ("cave-uptime",                   "uptime",                 "Synthetic monitoring"),
    ("cave-profiler",                 "profiler",               "Continuous profiler"),
    ("cave-slo",                      "slo",                    "SLO evaluator"),
    ("cave-incidents",                "incidents",              "Incident management"),
    ("cave-runbook",                  "runbook",                "Runbook executor"),

    # ─── Security ────────────────────────────────────────────────────────
    ("cave-vault",                    "vault",                  "Vault-parity secrets"),
    ("cave-secrets",                  "secrets",                "External Secrets Operator"),
    ("cave-auth",                     "auth",                   "Authentication"),
    ("cave-admission",                "admission",              "Admission webhooks"),
    ("cave-policy",                   "policy",                 "OPA-parity policy"),
    ("cave-pam",                      "pam",                    "Privileged access mgmt"),
    ("cave-certs",                    "certs",                  "Cert lifecycle"),
    ("cave-acme",                     "acme",                   "ACME issuance"),
    ("cave-vulns",                    "vulns",                  "Vulnerability tracking"),
    ("cave-sbom",                     "sbom",                   "SBOM generation"),
    ("cave-scan",                     "scan",                   "Security scan"),
    ("cave-container-scan",           "cscan",                  "Container image scan"),
    ("cave-sign",                     "sign",                   "Image / artifact signing"),
    ("cave-forensics",                "forensics",              "Forensic capture"),
    ("cave-compliance",               "compliance",             "Compliance evaluator"),
    ("cave-pii",                      "pii",                    "PII detection"),
    ("cave-dast",                     "dast",                   "DAST scanner"),
    ("cave-chaos",                    "chaos",                  "Chaos injector"),
    ("cave-security",                 "security",               "Security event bus"),

    # ─── Service mesh / networking ───────────────────────────────────────
    ("cave-mesh",                     "mesh",                   "Service mesh"),
    ("cave-dns",                      "dns",                    "DNS service"),

    # ─── Platform / cluster mgmt ─────────────────────────────────────────
    ("cave-knative",                  "knative",                "Knative-parity serverless"),
    ("cave-keda",                     "keda",                   "KEDA-parity event scaler"),
    ("cave-kamaji",                   "kamaji",                 "Hosted control planes"),
    ("cave-crossplane",               "crossplane",             "Crossplane-parity providers"),
    ("cave-cluster",                  "cluster",                "Cluster mgmt"),
    ("cave-ha",                       "ha",                     "High-availability layer"),
    ("cave-infra",                    "infra",                  "Infra provisioner"),

    # ─── DevX / apps ─────────────────────────────────────────────────────
    ("cave-deploy",                   "deploy",                 "Deployer"),
    ("cave-pipelines",                "pipeline",               "CI/CD pipelines"),
    ("cave-rollouts",                 "rollout",                "Argo Rollouts parity"),
    ("cave-workflows",                "workflow",               "Argo Workflows parity"),
    ("cave-scaffold",                 "scaffold",               "Service scaffolder"),
    ("cave-registry",                 "registry",               "Container registry"),
    ("cave-llm-gateway",              "llm",                    "LLM gateway"),
    ("cave-local-llm",                "local_llm",              "Local LLM runtime"),
    ("cave-chat",                     "chat",                   "Chat / messaging"),
    ("cave-erp",                      "erp",                    "ERP layer"),
    ("cave-cost",                     "cost",                   "FinOps cost"),
    ("cave-tracker",                  "tracker",                "Issue tracker"),
    ("cave-upstream",                 "upstream",               "Upstream-tracking"),
    ("cave-devlake",                  "devlake",                "DORA / DevLake"),
    ("cave-portal",                   "portal",                 "Developer portal"),
    ("cave-portal-api",               "portal_api",             "Portal API"),
    ("cave-ai-obs",                   "ai_obs",                 "AI/LLM observability"),
]

# ─── Dashboard generation ──────────────────────────────────────────────────

def grid_pos(idx):
    """Two columns of 12 wide × 8 tall, stacking down."""
    col = idx % 2
    row = idx // 2
    return {"x": col * 12, "y": row * 8, "w": 12, "h": 8}


def panel_timeseries(idx, title, expr, unit, legend, description=""):
    return {
        "id": idx + 1,
        "title": title,
        "type": "timeseries",
        "datasource": {"type": "prometheus", "uid": "${DS_PROMETHEUS}"},
        "gridPos": grid_pos(idx),
        "targets": [{
            "datasource": {"type": "prometheus", "uid": "${DS_PROMETHEUS}"},
            "expr": expr,
            "legendFormat": legend,
            "refId": "A",
        }],
        "fieldConfig": {"defaults": {"unit": unit}, "overrides": []},
        "options": {"legend": {"showLegend": True}},
        "description": description,
    }


def panel_multi_query(idx, title, queries_legends, unit, description=""):
    targets = []
    for i, (expr, legend) in enumerate(queries_legends):
        targets.append({
            "datasource": {"type": "prometheus", "uid": "${DS_PROMETHEUS}"},
            "expr": expr,
            "legendFormat": legend,
            "refId": chr(ord("A") + i),
        })
    return {
        "id": idx + 1,
        "title": title,
        "type": "timeseries",
        "datasource": {"type": "prometheus", "uid": "${DS_PROMETHEUS}"},
        "gridPos": grid_pos(idx),
        "targets": targets,
        "fieldConfig": {"defaults": {"unit": unit}, "overrides": []},
        "options": {"legend": {"showLegend": True}},
        "description": description,
    }


def panel_stat(idx, title, expr, unit, description=""):
    return {
        "id": idx + 1,
        "title": title,
        "type": "stat",
        "datasource": {"type": "prometheus", "uid": "${DS_PROMETHEUS}"},
        "gridPos": grid_pos(idx),
        "targets": [{
            "datasource": {"type": "prometheus", "uid": "${DS_PROMETHEUS}"},
            "expr": expr,
            "legendFormat": "",
            "refId": "A",
        }],
        "fieldConfig": {"defaults": {"unit": unit}, "overrides": []},
        "options": {"reduceOptions": {"calcs": ["lastNotNull"]}},
        "description": description,
    }


def panel_table(idx, title, expr, description=""):
    return {
        "id": idx + 1,
        "title": title,
        "type": "table",
        "datasource": {"type": "prometheus", "uid": "${DS_PROMETHEUS}"},
        "gridPos": grid_pos(idx),
        "targets": [{
            "datasource": {"type": "prometheus", "uid": "${DS_PROMETHEUS}"},
            "expr": expr,
            "format": "table",
            "instant": True,
            "refId": "A",
        }],
        "fieldConfig": {"defaults": {}, "overrides": []},
        "options": {},
        "description": description,
    }


def render_dashboard(module, prefix, summary):
    job = '"' + module + '"'
    panels = [
        panel_timeseries(
            0, "Request rate by route",
            f'sum(rate({prefix}_total{{job={job}}}[5m])) by (route)',
            "reqps", "{{route}}",
            description=f"{summary} — request throughput.",
        ),
        panel_timeseries(
            1, "Error rate (4xx + 5xx)",
            f'sum(rate({prefix}_total{{job={job},code=~"4..|5.."}}[5m])) by (code)',
            "reqps", "{{code}}",
            description="Errors broken down by HTTP class.",
        ),
        panel_multi_query(
            2, "Latency p50 / p95 / p99",
            [
                (f'histogram_quantile(0.50, sum(rate({prefix}_duration_seconds_bucket{{job={job}}}[5m])) by (le))', "p50"),
                (f'histogram_quantile(0.95, sum(rate({prefix}_duration_seconds_bucket{{job={job}}}[5m])) by (le))', "p95"),
                (f'histogram_quantile(0.99, sum(rate({prefix}_duration_seconds_bucket{{job={job}}}[5m])) by (le))', "p99"),
            ],
            "s",
            description="RED triad — duration percentiles.",
        ),
        panel_stat(
            3, "Inflight requests (saturation)",
            f'sum({prefix}_inflight_requests{{job={job}}})',
            "short",
            description="Concurrent in-progress requests.",
        ),
        panel_timeseries(
            4, "CPU utilization",
            f'rate(process_cpu_seconds_total{{job={job}}}[5m])',
            "percentunit", "{{instance}}",
            description="Per-instance CPU.",
        ),
        panel_timeseries(
            5, "Memory (RSS)",
            f'process_resident_memory_bytes{{job={job}}}',
            "bytes", "{{instance}}",
            description="Resident set size.",
        ),
        panel_timeseries(
            6, "Active goroutines / tasks",
            f'go_goroutines{{job={job}}}',
            "short", "{{instance}}",
            description="Goroutine count proxies for backpressure.",
        ),
        panel_stat(
            7, "Tenant cardinality",
            f'count(count by (tenant_id) ({prefix}_total{{job={job}}}))',
            "short",
            description="Number of distinct tenants emitting traffic.",
        ),
        panel_table(
            8, f"Top-10 tenants by request volume",
            f'topk(10, sum(rate({prefix}_total{{job={job}}}[5m])) by (tenant_id))',
            description="Hot-tenant table — feeds throttle / inhibit decisions.",
        ),
        panel_stat(
            9, "Health probe (up)",
            f'min(up{{job={job}}})',
            "short",
            description="0 = at least one instance failing the scrape.",
        ),
    ]
    dashboard = {
        "title": f"Cave / {module}",
        "uid": module,
        "tags": ["cave", "observability", module],
        "timezone": "browser",
        "schemaVersion": 39,
        "version": 1,
        "refresh": "30s",
        "time": {"from": "now-1h", "to": "now"},
        "templating": {
            "list": [
                {
                    "name": "DS_PROMETHEUS",
                    "type": "datasource",
                    "query": "prometheus",
                    "current": {"text": "Prometheus", "value": "Prometheus"},
                    "hide": 0,
                },
                {
                    "name": "tenant",
                    "type": "query",
                    "datasource": {"type": "prometheus", "uid": "${DS_PROMETHEUS}"},
                    "query": f"label_values({prefix}_total{{job=\"{module}\"}}, tenant_id)",
                    "current": {"text": "All", "value": "$__all"},
                    "includeAll": True,
                    "multi": True,
                    "refresh": 2,
                },
            ],
        },
        "panels": panels,
        "annotations": {"list": []},
        "editable": True,
        "graphTooltip": 1,
        "description": summary,
    }
    return dashboard


# ─── Alert generation ──────────────────────────────────────────────────────

def alert_yaml_lines(module, prefix, summary):
    """Render the Prometheus alert YAML for one module.

    Eight rules: SLO burn rate fast / slow, error budget, latency p99,
    saturation, memory, cardinality, health probe.
    """
    pascal = "".join(part.capitalize() for part in module.split("-")[1:])  # cave-pg → Pg, cave-cri → Cri
    runbook = "https://docs.cave.dev/runbooks/" + module
    job = '"' + module + '"'
    rules = [
        # 1. SLO burn rate fast (1h, 14.4× of a 99.9% target)
        f"""      - alert: {pascal}SloBurnRateFast
        expr: |
          (
            sum(rate({prefix}_total{{job={job},code=~"5.."}}[1h]))
            /
            sum(rate({prefix}_total{{job={job}}}[1h]))
          ) > (14.4 * 0.001)
        for: 2m
        labels:
          severity: critical
          tenant_scoped: "true"
        annotations:
          summary: "{module} fast burn-rate (1h)"
          description: "5xx fraction is {{{{ $value | humanizePercentage }}}} — burning the 99.9% budget 14.4× faster than allowed."
          runbook_url: "{runbook}/slo-burn-fast.md"
""",
        # 2. SLO burn rate slow (6h, 6×)
        f"""      - alert: {pascal}SloBurnRateSlow
        expr: |
          (
            sum(rate({prefix}_total{{job={job},code=~"5.."}}[6h]))
            /
            sum(rate({prefix}_total{{job={job}}}[6h]))
          ) > (6 * 0.001)
        for: 15m
        labels:
          severity: warning
          tenant_scoped: "true"
        annotations:
          summary: "{module} slow burn-rate (6h)"
          description: "Sustained 5xx fraction over 6h."
          runbook_url: "{runbook}/slo-burn-slow.md"
""",
        # 3. Error budget low (7d)
        f"""      - alert: {pascal}ErrorBudgetLow
        expr: |
          (
            1 -
            (sum(rate({prefix}_total{{job={job},code=~"5.."}}[7d]))
              / sum(rate({prefix}_total{{job={job}}}[7d])))
          ) < 0.999
        for: 30m
        labels:
          severity: warning
        annotations:
          summary: "{module} 7-day availability < 99.9%"
          description: "Trailing 7d availability = {{{{ $value | humanizePercentage }}}}."
          runbook_url: "{runbook}/error-budget.md"
""",
        # 4. Latency p99 high
        f"""      - alert: {pascal}LatencyP99High
        expr: |
          histogram_quantile(
            0.99,
            sum(rate({prefix}_duration_seconds_bucket{{job={job}}}[5m])) by (le)
          ) > 1
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "{module} p99 latency > 1s"
          description: "p99 = {{{{ $value }}}}s."
          runbook_url: "{runbook}/latency.md"
""",
        # 5. Saturation high
        f"""      - alert: {pascal}SaturationHigh
        expr: 'sum({prefix}_inflight_requests{{job={job}}}) > 400'
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "{module} inflight requests > 400"
          description: "Concurrent requests = {{{{ $value }}}}."
          runbook_url: "{runbook}/saturation.md"
""",
        # 6. Memory pressure
        f"""      - alert: {pascal}MemoryPressure
        expr: 'process_resident_memory_bytes{{job={job}}} > 4 * 1024 * 1024 * 1024'
        for: 10m
        labels:
          severity: warning
        annotations:
          summary: "{module} RSS > 4 GiB"
          runbook_url: "{runbook}/memory.md"
""",
        # 7. Cardinality explosion
        f"""      - alert: {pascal}CardinalityExplosion
        expr: 'count(count by (tenant_id) ({prefix}_total{{job={job}}})) > 5000'
        for: 30m
        labels:
          severity: warning
        annotations:
          summary: "{module} tenant cardinality > 5000"
          description: "Count = {{{{ $value }}}}."
          runbook_url: "{runbook}/cardinality.md"
""",
        # 8. Health probe failing
        f"""      - alert: {pascal}HealthProbeFailing
        expr: 'min(up{{job={job}}}) < 1'
        for: 1m
        labels:
          severity: critical
        annotations:
          summary: "{module} health probe failing"
          description: "At least one instance reports up == 0."
          runbook_url: "{runbook}/health.md"
""",
    ]
    body = (
        f"# {summary}\n"
        f"groups:\n"
        f"  - name: {module}-slo\n"
        f"    interval: 30s\n"
        f"    rules:\n"
        + "".join(rules)
    )
    return body


# ─── Driver ────────────────────────────────────────────────────────────────

def main():
    summary = []
    for module, prefix, desc in MODULES:
        dash = render_dashboard(module, prefix, desc)
        dash_path = DASH_DIR / f"{module}.json"
        with open(dash_path, "w") as f:
            json.dump(dash, f, indent=2)
            f.write("\n")
        alert_path = ALERT_DIR / f"{module}.yml"
        with open(alert_path, "w") as f:
            f.write(alert_yaml_lines(module, prefix, desc))
        summary.append((module, len(dash["panels"]), 8))

    width = max(len(m[0]) for m in summary) + 2
    print(f"{'module':<{width}} {'panels':>8} {'alerts':>8}")
    print("-" * (width + 18))
    for m, p, a in summary:
        print(f"{m:<{width}} {p:>8} {a:>8}")
    total_p = sum(p for _, p, _ in summary)
    total_a = sum(a for _, _, a in summary)
    print("-" * (width + 18))
    print(f"{'TOTAL':<{width}} {total_p:>8} {total_a:>8}")
    print(f"\n{len(summary)} modules covered.")


if __name__ == "__main__":
    main()
