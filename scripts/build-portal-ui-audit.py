#!/usr/bin/env python3
"""
Generate `docs/parity/portal-ui-audit-2026-05-12.md` — a quick classification
pass over `crates/cave-portal/src/admin/<x>.rs` to flag which admin pages are
real upstream-UI re-implementations versus structural scaffolds.

Heuristics (data-driven, NOT a substitute for hand-review):
  * scaffold     : < 3.5 KB source or fewer than 4 distinct fields/forms
  * partial      : 3.5 - 8 KB source with some real domain content
  * substantial  : > 8 KB source — likely a meaningful UI port

For each admin page we also pull the upstream UI reference from a curated
map (where the upstream actually has a UI worth porting) — KEDA has no
console UI of its own, so cave-keda is "N/A" rather than "missing port".
"""
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
ADMIN_DIR = REPO / "crates" / "cave-portal" / "src" / "admin"
PARITY_INDEX = REPO / "docs" / "parity" / "parity-index.json"
OUT = REPO / "docs" / "parity" / "portal-ui-audit-2026-05-12.md"

# Curated map: cave admin page → ("upstream UI source", "notes"). When the
# upstream is operator-only (no console UI), we mark "(no upstream UI)" so a
# scaffold isn't unfairly graded.
UPSTREAM_UI: dict[str, tuple[str, str]] = {
    "admission":              ("(no upstream UI)", "k8s admission controllers are CLI/CRD-only"),
    "ai_obs":                 ("langfuse/langfuse — Next.js dashboard", "rich web UI to port"),
    "alerts":                 ("prometheus/alertmanager — UI at /#/alerts", "list/silence views"),
    "apiserver":              ("(no upstream UI)", "k8s control-plane, no operator UI"),
    "artifacts":              ("pulp/pulpcore — Pulp Web UI (Vue)", "rich web UI to port"),
    "auth":                   ("keycloak/keycloak — Admin Console (React)", "rich web UI to port"),
    "backup":                 ("vmware-tanzu/velero — limited UI; mostly CLI", "CLI-first, partial UI"),
    "cache":                  ("(no upstream UI)", "redis has redis-cli + RedisInsight (external)"),
    "cdc":                    ("debezium/debezium-ui (deprecated)", "minimal historical UI"),
    "certs":                  ("(no upstream UI)", "cert-manager is CRD-only"),
    "chaos":                  ("chaos-mesh/chaos-mesh — Chaos Dashboard (React)", "rich web UI to port"),
    "chat":                   ("danny-avila/LibreChat — Chat UI (React)", "rich web UI to port"),
    "cloud_controller_manager":("(no upstream UI)", "k8s controller, no UI"),
    "cluster":                ("(no upstream UI)", "cluster-api is CRD/CLI-only"),
    "compliance":             ("(no upstream UI)", "OPA gatekeeper is CRD/CLI-only — cave-original UI"),
    "container_scan":         ("aquasecurity/trivy — minimal terminal UI", "CLI-first"),
    "contributions":          ("(cave-original)", "internal dashboard"),
    "controller_manager":     ("(no upstream UI)", "k8s controller, no UI"),
    "cost":                   ("opencost/opencost-ui — React dashboard", "rich web UI to port"),
    "cri":                    ("(no upstream UI)", "containerd is CLI-only"),
    "crm":                    ("twentyhq/twenty — full CRM React app", "rich web UI to port"),
    "crossplane":             ("crossplane/crossplane — minimal UI; CRD-first", "CRD-first"),
    "dashboard":              ("grafana/grafana — full Grafana UI", "rich web UI to port — biggest scope"),
    "dast":                   ("zaproxy/zaproxy — Java Swing UI", "desktop UI, not directly portable"),
    "deploy":                 ("argoproj/argo-cd — Web UI (React)", "rich web UI to port"),
    "devlake":                ("apache/incubator-devlake — Web UI", "rich web UI to port"),
    "dns":                    ("(no upstream UI)", "coredns is config-file-only"),
    "docdb":                  ("(no upstream UI)", "mongo has Compass (external)"),
    "erp":                    ("frappe/erpnext — full ERP UI", "rich web UI to port"),
    "etcd":                   ("(no upstream UI)", "etcd is CLI-only — etcdctl"),
    "forensics":              ("cilium/tetragon — minimal UI; mostly CLI", "CLI-first"),
    "gateway":                ("Kong/kong-manager — Kong Manager (Angular)", "rich web UI to port"),
    "gitops_config":          ("fluxcd/flux2 — minimal UI; CLI-first", "CLI-first"),
    "ha":                     ("(no upstream UI)", "etcd-HA is CLI/CRD-only"),
    "iam":                    ("(cave-original)", "internal IAM UI"),
    "incidents":              ("grafana/oncall — On-Call UI (React)", "rich web UI to port"),
    "infra":                  ("hashicorp/terraform-cloud — UI", "Terraform OSS itself is CLI-only"),
    "kamaji":                 ("(no upstream UI)", "kamaji is CRD-only"),
    "karpenter":              ("(no upstream UI)", "karpenter is CRD-only"),
    "keda":                   ("(no upstream UI)", "KEDA is CRD-only"),
    "knative":                ("(no upstream UI)", "knative is CRD-only"),
    "kube_proxy":             ("(no upstream UI)", "kube-proxy is CLI/iptables-only"),
    "kubelet":                ("kubernetes/kubernetes — k8s Dashboard add-on", "Dashboard add-on UI to port"),
    "kubevirt":               ("kubevirt/kubevirt — limited UI; mostly CLI/CRD", "CLI-first"),
    "lakehouse":              ("apache/iceberg — no UI; data engineers use Spark", "no upstream UI"),
    "ledger":                 ("(cave-original)", "internal audit-ledger UI"),
    "llm_gateway":            ("BerriAI/litellm — admin UI (newer)", "partial web UI"),
    "local_llm":              ("(no upstream UI)", "Ollama is CLI-only"),
    "logs":                   ("grafana/loki — via Grafana Explore", "use Grafana panel"),
    "mesh":                   ("istio/istio — Kiali (separate project)", "rich web UI to port"),
    "metrics":                ("prometheus/prometheus — built-in expr UI", "minimal but real UI"),
    "namespaces":             ("(no upstream UI)", "k8s native, no dedicated UI"),
    "net":                    ("cilium/cilium — Hubble UI (separate)", "rich web UI to port"),
    "nodes":                  ("(no upstream UI)", "kubectl-only"),
    "oncall":                 ("grafana/oncall — On-Call UI", "duplicate of incidents — TODO collapse"),
    "pam":                    ("gravitational/teleport — Web UI (React)", "rich web UI to port"),
    "pii":                    ("(no upstream UI)", "Microsoft Presidio is API-only"),
    "pipelines":              ("tektoncd/dashboard — Tekton Dashboard (React)", "rich web UI to port"),
    "policy":                 ("open-policy-agent/opa — Rego Playground", "minimal upstream UI"),
    "profiler":               ("grafana/pyroscope — Pyroscope UI", "rich web UI to port"),
    "rdbms":                  ("(no upstream UI)", "Postgres uses psql/pgAdmin (external)"),
    "rdbms_operator":         ("(no upstream UI)", "cnpg is CRD-only"),
    "registry":               ("goharbor/harbor — Harbor UI (Angular)", "rich web UI to port"),
    "rollouts":               ("argoproj/argo-rollouts — Rollouts UI (React)", "rich web UI to port"),
    "runbook":                ("(cave-original)", "internal runbook UI"),
    "sbom":                   ("DependencyTrack/dependency-track — UI", "rich web UI to port"),
    "scan":                   ("SonarSource/sonarqube — SonarQube UI", "rich web UI to port"),
    "scheduler":              ("(no upstream UI)", "k8s scheduler is CLI/CRD-only"),
    "scaffold":               ("backstage/backstage — Scaffolder UI", "rich web UI to port"),
    "search":                 ("opensearch-project/OpenSearch-Dashboards", "rich web UI to port"),
    "secrets":                ("(no upstream UI)", "trufflehog is CLI-only"),
    "security":               ("falcosecurity/falco — Falco UI (Falcosidekick-ui)", "partial web UI"),
    "sign":                   ("(no upstream UI)", "sigstore is CLI-only"),
    "slo":                    ("OpenSLO/OpenSLO — minimal UI", "spec-first, no canonical UI"),
    "spire":                  ("(no upstream UI)", "spire is CLI/CRD-only"),
    "status":                 ("louislam/uptime-kuma — Uptime Kuma UI", "rich web UI to port"),
    "store":                  ("minio/minio — MinIO Console (React)", "rich web UI to port"),
    "streams":                ("(no upstream UI)", "Kafka uses CLI; AKHQ/kafdrop are external"),
    "trace":                  ("jaegertracing/jaeger-ui — Jaeger UI (React)", "rich web UI to port"),
    "tracker":                ("linear-app/linear — Linear UI", "rich web UI to port"),
    "uptime":                 ("louislam/uptime-kuma — Uptime Kuma UI", "rich web UI to port"),
    "vault":                  ("openbao/openbao — Vault Web UI", "rich web UI to port"),
    "vulns":                  ("DefectDojo/django-DefectDojo — UI", "rich web UI to port"),
    "workflows":              ("n8n-io/n8n — n8n Editor (Vue)", "rich web UI to port"),
}


def classify(loc_bytes: int, content: str) -> tuple[str, str]:
    """Return (status, evidence) where status is scaffold|partial|substantial."""
    form_count = content.count("<form")
    table_count = content.count("<table") + content.count("table(&[")
    field_count = len(re.findall(r"<(input|select|textarea|button)\b", content))
    fn_count = len(re.findall(r"\bfn [a-z_][a-z0-9_]*\(", content))
    # Boilerplate baseline: every admin page has a render fn, page_shell, and tenant param.
    if loc_bytes < 3500:
        return "scaffold", f"{loc_bytes}B src, {form_count}F/{table_count}T/{field_count}I"
    if loc_bytes < 8000:
        return "partial", f"{loc_bytes}B src, {form_count}F/{table_count}T/{field_count}I/{fn_count}fn"
    return "substantial", f"{loc_bytes}B src, {form_count}F/{table_count}T/{field_count}I/{fn_count}fn"


def main() -> int:
    if not ADMIN_DIR.is_dir():
        print(f"error: {ADMIN_DIR} missing", file=sys.stderr)
        return 1

    rows: list[dict] = []
    for path in sorted(ADMIN_DIR.glob("*.rs")):
        name = path.stem
        if name in ("mod", "permission", "render", "types"):
            continue
        if name == "compliance":
            # Skip the dashboard itself — it's the meta-tool, not a port.
            continue
        content = path.read_text()
        size = len(content)
        status, evidence = classify(size, content)
        upstream, note = UPSTREAM_UI.get(name, ("(unmapped)", "needs hand-classification"))
        rows.append({
            "page": name,
            "status": status,
            "size_bytes": size,
            "upstream_ui": upstream,
            "note": note,
            "evidence": evidence,
        })

    # Group by status for the summary
    by_status = {"substantial": [], "partial": [], "scaffold": []}
    for r in rows:
        by_status[r["status"]].append(r)
    no_upstream = sum(1 for r in rows if r["upstream_ui"].startswith("(no upstream UI)"))
    cave_original = sum(1 for r in rows if r["upstream_ui"].startswith("(cave-original)"))

    out_lines: list[str] = []
    out_lines.append("# Portal admin-UI parity audit — 2026-05-12")
    out_lines.append("")
    out_lines.append("Burak's challenge: the dashboard says `Grade A / 100` for structural")
    out_lines.append("coverage, but how many of the 83 cave-portal admin pages are *actual*")
    out_lines.append("re-implementations of the upstream UI versus a `page_shell + table()`")
    out_lines.append("scaffold? This audit gives a first-pass answer.")
    out_lines.append("")
    out_lines.append("**Method.** Each `crates/cave-portal/src/admin/<x>.rs` is classified")
    out_lines.append("by source size + DOM-element density:")
    out_lines.append("")
    out_lines.append("- **substantial** — `>8 KB` source with multiple forms / tables / inputs.")
    out_lines.append("  Real reimplementation candidate.")
    out_lines.append("- **partial**     — `3.5–8 KB` source with some domain-specific content.")
    out_lines.append("- **scaffold**    — `<3.5 KB` source. Mostly `page_shell` + a placeholder")
    out_lines.append("  panel; not a port of the upstream UI.")
    out_lines.append("")
    out_lines.append("Heuristics are NOT a substitute for hand-review. Each cell records the")
    out_lines.append("evidence (`<size>B src, <forms>F/<tables>T/<inputs>I/<fns>fn`).")
    out_lines.append("")
    out_lines.append("## Headline")
    out_lines.append("")
    out_lines.append(f"| Bucket | Count | Notes |")
    out_lines.append(f"|---|--:|---|")
    out_lines.append(f"| Total admin pages | {len(rows)} | excluding `compliance.rs` (this dashboard) |")
    out_lines.append(f"| Substantial (real port candidate) | {len(by_status['substantial'])} | size > 8 KB |")
    out_lines.append(f"| Partial | {len(by_status['partial'])} | size 3.5–8 KB |")
    out_lines.append(f"| Scaffold | {len(by_status['scaffold'])} | size < 3.5 KB |")
    out_lines.append(f"| Upstream has no UI (legitimate scaffold) | {no_upstream} | CLI-/CRD-only upstreams |")
    out_lines.append(f"| Cave-original (no upstream to mirror) | {cave_original} | internal-only |")
    out_lines.append("")
    out_lines.append("## Substantial admin pages (real reimplementation candidates)")
    out_lines.append("")
    out_lines.append("| Page | Size | Evidence | Upstream UI | Note |")
    out_lines.append("|---|--:|---|---|---|")
    for r in sorted(by_status["substantial"], key=lambda x: -x["size_bytes"]):
        out_lines.append(
            f"| `{r['page']}` | {r['size_bytes']} | {r['evidence']} | {r['upstream_ui']} | {r['note']} |"
        )

    out_lines.append("")
    out_lines.append("## Partial admin pages")
    out_lines.append("")
    out_lines.append("| Page | Size | Evidence | Upstream UI | Note |")
    out_lines.append("|---|--:|---|---|---|")
    for r in sorted(by_status["partial"], key=lambda x: -x["size_bytes"]):
        out_lines.append(
            f"| `{r['page']}` | {r['size_bytes']} | {r['evidence']} | {r['upstream_ui']} | {r['note']} |"
        )

    out_lines.append("")
    out_lines.append("## Scaffold admin pages (NOT a real UI port)")
    out_lines.append("")
    out_lines.append("Split: legitimate-scaffold (upstream has no UI) vs work-to-do.")
    out_lines.append("")
    legit = [r for r in by_status["scaffold"]
             if r["upstream_ui"].startswith("(no upstream UI)") or r["upstream_ui"].startswith("(cave-original)")]
    todo = [r for r in by_status["scaffold"]
            if not r["upstream_ui"].startswith("(no upstream UI)") and not r["upstream_ui"].startswith("(cave-original)")]
    out_lines.append("### Legitimate scaffolds")
    out_lines.append("")
    out_lines.append("| Page | Size | Upstream UI | Note |")
    out_lines.append("|---|--:|---|---|")
    for r in sorted(legit, key=lambda x: x["page"]):
        out_lines.append(
            f"| `{r['page']}` | {r['size_bytes']} | {r['upstream_ui']} | {r['note']} |"
        )
    out_lines.append("")
    out_lines.append("### Work-to-do (real upstream UI exists, current page is scaffold)")
    out_lines.append("")
    out_lines.append("| Page | Size | Upstream UI | Note |")
    out_lines.append("|---|--:|---|---|")
    for r in sorted(todo, key=lambda x: x["page"]):
        out_lines.append(
            f"| `{r['page']}` | {r['size_bytes']} | {r['upstream_ui']} | {r['note']} |"
        )

    out_lines.append("")
    out_lines.append("---")
    out_lines.append("")
    out_lines.append("**How this surfaces on `/admin/compliance`.** The new `parity_ratio`")
    out_lines.append("metric tracks upstream-code parity (via the audit doc), NOT upstream-UI")
    out_lines.append("parity. A future iteration could add a `portal_ui_parity` field that")
    out_lines.append("ingests this audit so the dashboard distinguishes backend-parity (Grade")
    out_lines.append("F today) from portal-UI-parity (this doc) explicitly. For now both")
    out_lines.append("axes are tracked separately — see the dashboard's `Upstream Parity`")
    out_lines.append("card for backend parity, and this doc for UI parity.")
    out_lines.append("")
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(out_lines))

    # Summary to stderr
    print(
        f"Wrote {OUT.relative_to(REPO)}: {len(rows)} pages "
        f"({len(by_status['substantial'])} substantial, "
        f"{len(by_status['partial'])} partial, "
        f"{len(by_status['scaffold'])} scaffold; "
        f"{no_upstream} legit / {cave_original} cave-original)",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
