#!/usr/bin/env python3
"""
Augment crates/*/parity.manifest.toml with the OSS-launch upstream metadata
that the audit doc + dashboard require:

  - Add `license`, `url`, `name`, `language` to the existing `[upstream]`
    block (when missing).
  - Insert a top-level `[parity]` block with `ratio` (honest: 0.0 when not
    measured), `last_audit`, and `infra_only`.

The script is idempotent: if a manifest already has the augmented fields,
it is left alone. For each crate name in `--targets`, the augmentation is
driven by the on-disk `[upstream]` org/repo. Crates whose `[upstream]`
block has neither `org` nor `[[upstreams]]` are treated as infra (no
upstream attribution applies).

The data here is hand-curated against the upstream projects' LICENSE
files as of 2026-05-12. Where a license could not be confirmed without
re-checking the source, the script falls back to `"Unknown"` rather than
guessing — operators can edit the manifest by hand.

Honest invariant: `[parity].ratio` is ALWAYS 0.0 unless the per-crate
override in `MEASURED_RATIO` explicitly sets it. Wave3 mechanically
filled `[[files]]`/`[[functions]]` entries from local source; that does
NOT measure upstream parity, so we do not back-derive a ratio from it.
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
CRATES = REPO / "crates"
LAST_AUDIT = "2026-05-12"

# Crates with no upstream project — true infra of the cave-runtime itself.
INFRA_ONLY = {
    "cave-cli",
    "cave-core",
    "cave-changelog",
    "cave-db",
    "cave-docs-site",
    "cave-kernel",
    "cave-ledger",
    "cave-portal-api",
    "cave-portal-web",
    "cave-runbook",
    "cave-runtime",
    "cave-upstream",
}

# Per-upstream license + language metadata. Keyed by `org/repo` lower-cased.
# Sources: each project's LICENSE / README on github at v* of choice. Where
# the upstream relicensed (e.g. Redis SSPL in 2024) we record the license
# that applies to the pinned version in the manifest, not the latest.
UPSTREAM_META: dict[str, dict] = {
    "kubernetes/kubernetes":            {"license": "Apache-2.0", "language": "Go",       "name": "Kubernetes"},
    "etcd-io/etcd":                     {"license": "Apache-2.0", "language": "Go",       "name": "etcd"},
    "containerd/containerd":            {"license": "Apache-2.0", "language": "Go",       "name": "containerd"},
    "cri-o/cri-o":                      {"license": "Apache-2.0", "language": "Go",       "name": "CRI-O"},
    "cilium/cilium":                    {"license": "Apache-2.0", "language": "Go",       "name": "Cilium"},
    "cilium/hubble":                    {"license": "Apache-2.0", "language": "Go",       "name": "Hubble"},
    "cilium/tetragon":                  {"license": "Apache-2.0", "language": "Go",       "name": "Tetragon"},
    "coredns/coredns":                  {"license": "Apache-2.0", "language": "Go",       "name": "CoreDNS"},
    "istio/istio":                      {"license": "Apache-2.0", "language": "Go",       "name": "Istio"},
    "kubernetes-sigs/cluster-api":      {"license": "Apache-2.0", "language": "Go",       "name": "Cluster API"},
    "kubernetes-sigs/karpenter":        {"license": "Apache-2.0", "language": "Go",       "name": "Karpenter"},
    "kubernetes-sigs/network-policy-api":{"license": "Apache-2.0","language": "Go",       "name": "Network Policy API"},
    "kubernetes/kubectl":               {"license": "Apache-2.0", "language": "Go",       "name": "kubectl"},
    "kubevirt/kubevirt":                {"license": "Apache-2.0", "language": "Go",       "name": "KubeVirt"},
    "kedacore/keda":                    {"license": "Apache-2.0", "language": "Go",       "name": "KEDA"},
    "clastix/kamaji":                   {"license": "Apache-2.0", "language": "Go",       "name": "Kamaji"},
    "clastix-labs/kamaji":              {"license": "Apache-2.0", "language": "Go",       "name": "Kamaji"},
    "knative/serving":                  {"license": "Apache-2.0", "language": "Go",       "name": "Knative Serving"},
    "loft-sh/vcluster":                 {"license": "Apache-2.0", "language": "Go",       "name": "vcluster"},

    # Networking + service mesh
    "containernetworking/plugins":      {"license": "Apache-2.0", "language": "Go",       "name": "CNI plugins"},

    # Observability + alerts
    "prometheus/prometheus":            {"license": "Apache-2.0", "language": "Go",       "name": "Prometheus"},
    "prometheus/alertmanager":          {"license": "Apache-2.0", "language": "Go",       "name": "Alertmanager"},
    "grafana/grafana":                  {"license": "AGPL-3.0",   "language": "Go",       "name": "Grafana"},
    "grafana/loki":                     {"license": "AGPL-3.0",   "language": "Go",       "name": "Loki"},
    "grafana/oncall":                   {"license": "AGPL-3.0",   "language": "Python",   "name": "Grafana OnCall"},
    "grafana/pyroscope":                {"license": "AGPL-3.0",   "language": "Go",       "name": "Pyroscope"},
    "jaegertracing/jaeger":             {"license": "Apache-2.0", "language": "Go",       "name": "Jaeger"},
    "openslo/openslo":                  {"license": "Apache-2.0", "language": "Specification","name": "OpenSLO"},
    "louislam/uptime-kuma":             {"license": "MIT",        "language": "JavaScript","name": "Uptime Kuma"},
    "langfuse/langfuse":                {"license": "MIT",        "language": "TypeScript","name": "Langfuse"},

    # Storage / DBs
    "postgres/postgres":                {"license": "PostgreSQL", "language": "C",        "name": "PostgreSQL"},
    "pgbouncer/pgbouncer":              {"license": "ISC",        "language": "C",        "name": "PgBouncer"},
    "cloudnative-pg/cloudnative-pg":    {"license": "Apache-2.0", "language": "Go",       "name": "CloudNativePG"},
    "redis/redis":                      {"license": "BSD-3-Clause","language": "C",       "name": "Redis"},
    "valkey-io/valkey":                 {"license": "BSD-3-Clause","language": "C",       "name": "Valkey"},
    "mongodb/mongo":                    {"license": "SSPL-1.0",   "language": "C++",      "name": "MongoDB"},
    "minio/minio":                      {"license": "AGPL-3.0",   "language": "Go",       "name": "MinIO"},
    "apache/iceberg":                   {"license": "Apache-2.0", "language": "Java",     "name": "Apache Iceberg"},
    "apache/iceberg-rust":              {"license": "Apache-2.0", "language": "Rust",     "name": "Apache Iceberg (Rust)"},
    "apache/datafusion":                {"license": "Apache-2.0", "language": "Rust",     "name": "Apache DataFusion"},
    "apache/kafka":                     {"license": "Apache-2.0", "language": "Java",     "name": "Apache Kafka"},
    "apache/hudi-rs":                   {"license": "Apache-2.0", "language": "Rust",     "name": "Apache Hudi (Rust)"},
    "apache/incubator-devlake":         {"license": "Apache-2.0", "language": "Go",       "name": "Apache DevLake"},
    "delta-io/delta-rs":                {"license": "Apache-2.0", "language": "Rust",     "name": "Delta Lake (Rust)"},

    # Security / supply chain
    "spiffe/spire":                     {"license": "Apache-2.0", "language": "Go",       "name": "SPIRE"},
    "openbao/openbao":                  {"license": "MPL-2.0",    "language": "Go",       "name": "OpenBao"},
    "hashicorp/vault":                  {"license": "BSL-1.1",    "language": "Go",       "name": "HashiCorp Vault"},
    "hashicorp/terraform":              {"license": "BSL-1.1",    "language": "Go",       "name": "HashiCorp Terraform"},
    "aquasecurity/trivy":               {"license": "Apache-2.0", "language": "Go",       "name": "Trivy"},
    "aquasecurity/kube-bench":          {"license": "Apache-2.0", "language": "Go",       "name": "kube-bench"},
    "open-policy-agent/opa":            {"license": "Apache-2.0", "language": "Go",       "name": "Open Policy Agent"},
    "open-policy-agent/gatekeeper":     {"license": "Apache-2.0", "language": "Go",       "name": "OPA Gatekeeper"},
    "falcosecurity/falco":              {"license": "Apache-2.0", "language": "C++",      "name": "Falco"},
    "sigstore/sigstore":                {"license": "Apache-2.0", "language": "Go",       "name": "Sigstore"},
    "trufflesecurity/trufflehog":       {"license": "AGPL-3.0",   "language": "Go",       "name": "TruffleHog"},
    "zaproxy/zaproxy":                  {"license": "Apache-2.0", "language": "Java",     "name": "OWASP ZAP"},
    "dependencytrack/dependency-track": {"license": "Apache-2.0", "language": "Java",     "name": "Dependency-Track"},
    "defectdojo/django-defectdojo":     {"license": "BSD-3-Clause","language": "Python",  "name": "DefectDojo"},
    "microsoft/presidio":               {"license": "MIT",        "language": "Python",   "name": "Microsoft Presidio"},
    "keycloak/keycloak":                {"license": "Apache-2.0", "language": "Java",     "name": "Keycloak"},
    "cert-manager/cert-manager":        {"license": "Apache-2.0", "language": "Go",       "name": "cert-manager"},
    "smallstep/certificates":           {"license": "Apache-2.0", "language": "Go",       "name": "step-ca"},
    "gravitational/teleport":           {"license": "AGPL-3.0",   "language": "Go",       "name": "Teleport"},
    "external-secrets/external-secrets":{"license": "Apache-2.0", "language": "Go",       "name": "External Secrets Operator"},
    "sonarsource/sonarqube":            {"license": "LGPL-3.0",   "language": "Java",     "name": "SonarQube"},

    # GitOps + CD
    "argoproj/argo-cd":                 {"license": "Apache-2.0", "language": "Go",       "name": "Argo CD"},
    "argoproj/argo-rollouts":           {"license": "Apache-2.0", "language": "Go",       "name": "Argo Rollouts"},
    "argoproj/argo-workflows":          {"license": "Apache-2.0", "language": "Go",       "name": "Argo Workflows"},
    "fluxcd/flux2":                     {"license": "Apache-2.0", "language": "Go",       "name": "Flux"},
    "tektoncd/pipeline":                {"license": "Apache-2.0", "language": "Go",       "name": "Tekton Pipelines"},
    "crossplane/crossplane":            {"license": "Apache-2.0", "language": "Go",       "name": "Crossplane"},

    # Backup / DR / chaos
    "vmware-tanzu/velero":              {"license": "Apache-2.0", "language": "Go",       "name": "Velero"},
    "chaos-mesh/chaos-mesh":            {"license": "Apache-2.0", "language": "Go",       "name": "Chaos Mesh"},

    # Cost
    "opencost/opencost":                {"license": "Apache-2.0", "language": "Go",       "name": "OpenCost"},

    # Build / registry / artifacts
    "containers/buildah":               {"license": "Apache-2.0", "language": "Go",       "name": "Buildah"},
    "goharbor/harbor":                  {"license": "Apache-2.0", "language": "Go",       "name": "Harbor"},
    "sonatype/nexus-public":            {"license": "EPL-1.0",    "language": "Java",     "name": "Nexus Repository OSS"},
    "pulp/pulp":                        {"license": "GPL-2.0",    "language": "Python",   "name": "Pulp"},

    # AI / data / ML
    "berriai/litellm":                  {"license": "MIT",        "language": "Python",   "name": "LiteLLM"},
    "ollama/ollama":                    {"license": "MIT",        "language": "Go",       "name": "Ollama"},
    "mlflow/mlflow":                    {"license": "Apache-2.0", "language": "Python",   "name": "MLflow"},
    "kubeflow/spark-operator":          {"license": "Apache-2.0", "language": "Go",       "name": "Kubeflow Spark Operator"},
    "jupyterhub/jupyterhub":            {"license": "BSD-3-Clause","language": "Python",  "name": "JupyterHub"},
    "danny-avila/librechat":            {"license": "MIT",        "language": "TypeScript","name": "LibreChat"},

    # Developer experience
    "kong/kong":                        {"license": "Apache-2.0", "language": "Lua",      "name": "Kong"},
    "unleash/unleash":                  {"license": "Apache-2.0", "language": "TypeScript","name": "Unleash"},
    "backstage/backstage":              {"license": "Apache-2.0", "language": "TypeScript","name": "Backstage"},
    "linear-app/linear":                {"license": "Proprietary","language": "TypeScript","name": "Linear (proprietary; cave-tracker tracks API surface)"},
    "twentyhq/twenty":                  {"license": "AGPL-3.0",   "language": "TypeScript","name": "Twenty CRM"},
    "erpnext/erpnext":                  {"license": "GPL-3.0",    "language": "Python",   "name": "ERPNext"},
    "n8n-io/n8n":                       {"license": "SUSTL-1.0",  "language": "TypeScript","name": "n8n"},
    "towncrier/towncrier":              {"license": "MIT",        "language": "Python",   "name": "towncrier"},
    "zed-industries/zed":               {"license": "GPL-3.0",    "language": "Rust",     "name": "Zed"},

    # CDC / search / techdocs (D2 targets)
    "debezium/debezium":                {"license": "Apache-2.0", "language": "Java",     "name": "Debezium"},
    "opensearch-project/opensearch":    {"license": "Apache-2.0", "language": "Java",     "name": "OpenSearch"},
    "casbin/casbin":                    {"license": "Apache-2.0", "language": "Go",       "name": "Casbin"},
    "authzed/spicedb":                  {"license": "Apache-2.0", "language": "Go",       "name": "SpiceDB"},
}


# Honest ratio overrides for crates where we DO have a measurement.
# Empty by default — we don't claim parity ratios we haven't measured.
# Tier-100 crates from the audit doc already carry ratio=1.0 in
# parity-index.json; they are not in this map.
MEASURED_RATIO: dict[str, float] = {}


# D2 tier crates with no upstream block on disk — we name them explicitly
# so the augment pass can write a fresh [upstream] block for them. Each
# entry MUST correspond to a workspace member; phantoms from the audit
# doc (cave-datafusion, cave-iceberg) are excluded.
D2_TARGETS: dict[str, dict] = {
    "cave-acme":        {"org": "smallstep",         "repo": "certificates", "version": "v0.27.0"},
    "cave-cdc":         {"org": "debezium",          "repo": "debezium",     "version": "v2.7.0"},
    "cave-permission":  {"org": "casbin",            "repo": "casbin",       "version": "v2.103.0"},
    "cave-pki":         {"org": "smallstep",         "repo": "certificates", "version": "v0.27.0"},
    "cave-search":      {"org": "opensearch-project","repo": "opensearch",   "version": "v2.18.0"},
    "cave-techdocs":    {"org": "backstage",         "repo": "backstage",    "version": "v1.32.0"},
    "cave-tracing":     {"org": "open-telemetry",    "repo": "opentelemetry-rust","version": "v0.27.0"},
}

# Phantom crates listed in the audit doc but not workspace members. They
# get reported as "phantom" rather than augmented; the dashboard treats
# them as out-of-scope.
PHANTOMS = {
    "cave-datafusion",
    "cave-iceberg",
    "cave-external-secrets",
    "cave-hubble",
    "cave-pg",
    "cave-spire",
    "cave-vcluster",
}

# open-telemetry/opentelemetry-rust is not in the meta map; add explicitly.
UPSTREAM_META["open-telemetry/opentelemetry-rust"] = {
    "license": "Apache-2.0",
    "language": "Rust",
    "name": "OpenTelemetry Rust",
}

# Additional upstream entries that show up on disk but weren't in the
# first pass — keep adding here as they're discovered.
UPSTREAM_META["pulp/pulpcore"] = {
    "license": "GPL-2.0",
    "language": "Python",
    "name": "Pulp (pulpcore)",
}
UPSTREAM_META["debezium/debezium-server"] = {
    "license": "Apache-2.0",
    "language": "Java",
    "name": "Debezium Server",
}
UPSTREAM_META["frappe/erpnext"] = {
    "license": "GPL-3.0",
    "language": "Python",
    "name": "ERPNext",
}
UPSTREAM_META["distribution/distribution"] = {
    "license": "Apache-2.0",
    "language": "Go",
    "name": "CNCF Distribution (Docker Registry)",
}
UPSTREAM_META["manticoresoftware/manticoresearch"] = {
    "license": "GPL-2.0",
    "language": "C++",
    "name": "Manticore Search",
}
UPSTREAM_META["nobl9/nobl9-go"] = {
    "license": "MIT",
    "language": "Go",
    "name": "Nobl9 SLO SDK (Go)",
}
UPSTREAM_META["makeplane/plane"] = {
    "license": "AGPL-3.0",
    "language": "TypeScript",
    "name": "Plane (Linear-style issue tracking)",
}


def read_manifest(crate: str) -> tuple[Path, str | None]:
    p = CRATES / crate / "parity.manifest.toml"
    if not p.is_file():
        return p, None
    return p, p.read_text(encoding="utf-8")


def existing_upstream(text: str) -> tuple[str | None, str | None, str | None]:
    """Return (org, repo, version) from the first `[upstream]` table."""
    m = re.search(
        r"^\[upstream\][^\[]*",
        text,
        flags=re.MULTILINE,
    )
    if not m:
        return None, None, None
    block = m.group(0)

    def find(key: str) -> str | None:
        mm = re.search(rf'^\s*{key}\s*=\s*"([^"]*)"', block, flags=re.MULTILINE)
        return mm.group(1) if mm else None

    return find("org"), find("repo"), find("version")


def has_parity_block(text: str) -> bool:
    return re.search(r"^\[parity\]", text, flags=re.MULTILINE) is not None


def augment_parity_block(text: str, ratio: float, infra_only: bool) -> tuple[str, bool]:
    """Append `last_audit` (and `ratio`/`infra_only` if absent) to an
    existing `[parity]` block. Existing measured values like
    `fill_ratio` on cave-net are preserved verbatim. The match stops at
    the FIRST table or array-of-tables after `[parity]` so the new
    key=value lines stay inside the parity table, not leaked into a
    following `[[mapped]]` array."""
    m = re.search(
        r"^(\[parity\])([^\[]*)",
        text,
        flags=re.MULTILINE,
    )
    if not m:
        return text, False
    header = m.group(1)
    body = m.group(2)
    lines_to_add: list[str] = []
    if not re.search(r'^\s*(ratio|fill_ratio)\s*=', body, flags=re.MULTILINE):
        lines_to_add.append(
            f'ratio       = {ratio}        # honest: 0.0 means upstream parity NOT yet measured'
        )
    if not re.search(r'^\s*last_audit\s*=', body, flags=re.MULTILINE):
        lines_to_add.append(f'last_audit  = "{LAST_AUDIT}"')
    if not re.search(r'^\s*infra_only\s*=', body, flags=re.MULTILINE):
        lines_to_add.append(f'infra_only  = {"true" if infra_only else "false"}')
    if not lines_to_add:
        return text, False
    stripped = body.rstrip("\n")
    insertion = "\n".join(lines_to_add) + "\n"
    new_block = header + stripped + "\n" + insertion
    if not body.endswith("\n\n"):
        new_block += "\n"
    return text[: m.start()] + new_block + text[m.end():], True


def has_field_in_upstream(text: str, field: str) -> bool:
    m = re.search(
        rf"^\[upstream\][^\[]*?^\s*{field}\s*=",
        text,
        flags=re.MULTILINE | re.DOTALL,
    )
    return m is not None


def augment_upstream(text: str, meta: dict, name: str | None = None) -> tuple[str, list[str]]:
    """Append missing license/url/name/language fields to the first
    `[upstream]` block. Returns (new_text, fields_added)."""
    added: list[str] = []
    # locate the block range
    m = re.search(r"^(\[upstream\])(.*?)(?=^\[|\Z)", text, flags=re.MULTILINE | re.DOTALL)
    if not m:
        return text, added
    header = m.group(1)
    body = m.group(2)
    # body ends just before the next table. Trim trailing blank lines for insertion.
    stripped = body.rstrip("\n")
    lines_to_add: list[str] = []

    if name and not re.search(r'^\s*name\s*=', body, flags=re.MULTILINE):
        lines_to_add.append(f'name    = "{name}"')
        added.append("name")
    if "license" in meta and not re.search(r'^\s*license\s*=', body, flags=re.MULTILINE):
        lines_to_add.append(f'license = "{meta["license"]}"')
        added.append("license")
    if "language" in meta and not re.search(r'^\s*language\s*=', body, flags=re.MULTILINE):
        lines_to_add.append(f'language= "{meta["language"]}"')
        added.append("language")
    if not re.search(r'^\s*url\s*=', body, flags=re.MULTILINE):
        org_repo = meta.get("_org_repo")
        if org_repo:
            lines_to_add.append(f'url     = "https://github.com/{org_repo}"')
            added.append("url")

    if not lines_to_add:
        return text, added

    insertion = "\n".join(lines_to_add) + "\n"
    new_block = header + stripped + "\n" + insertion + ("\n" if not body.endswith("\n\n") else "")
    new_text = text[: m.start()] + new_block + text[m.end():]
    return new_text, added


def insert_parity_block(text: str, ratio: float, infra_only: bool) -> tuple[str, bool]:
    """Insert a `[parity]` block immediately after the first `[upstream]`
    block (or at top of file if no upstream block). If a `[parity]`
    block already exists, augment it with `last_audit` and `ratio` /
    `infra_only` only if missing (preserves existing measured values
    such as `fill_ratio` on cave-net)."""
    if has_parity_block(text):
        return augment_parity_block(text, ratio, infra_only)
    block = (
        "\n[parity]\n"
        f'ratio       = {ratio}        # honest: 0.0 means upstream parity NOT yet measured\n'
        f'last_audit  = "{LAST_AUDIT}"\n'
        f'infra_only  = {"true" if infra_only else "false"}\n'
    )
    # Find end of first [upstream] (or [[upstreams]] sequence) block.
    m = re.search(
        r"^(\[upstream\]|\[\[upstreams\]\]).*?(?=^\[(?!\[upstreams\]\])|\Z)",
        text,
        flags=re.MULTILINE | re.DOTALL,
    )
    if not m:
        # Prepend after the leading comment lines.
        # Find first non-comment line.
        lines = text.splitlines(keepends=True)
        i = 0
        while i < len(lines) and (lines[i].lstrip().startswith("#") or lines[i].strip() == ""):
            i += 1
        prefix = "".join(lines[:i])
        rest = "".join(lines[i:])
        return prefix + block + rest, True
    insert_at = m.end()
    return text[:insert_at] + block + text[insert_at:], True


def write_fresh_d2(crate: str, target: dict) -> str:
    org = target["org"]
    repo = target["repo"]
    version = target["version"]
    org_repo = f"{org}/{repo}"
    meta = UPSTREAM_META.get(org_repo.lower(), {})
    name = meta.get("name", f"{org}/{repo}")
    license_ = meta.get("license", "Unknown")
    language = meta.get("language", "Unknown")
    return (
        f"# parity.manifest.toml — {crate}\n"
        f"# Upstream: {name}  https://github.com/{org_repo}  pinned {version}\n"
        f"# Created by scripts/parity-augment-manifests.py on {LAST_AUDIT}.\n"
        f"# Initial manifest carries upstream attribution; file/function\n"
        f"# mappings still need to be hand-curated.\n"
        f"\n"
        f"[upstream]\n"
        f'org     = "{org}"\n'
        f'repo    = "{repo}"\n'
        f'version = "{version}"\n'
        f'name    = "{name}"\n'
        f'license = "{license_}"\n'
        f'language= "{language}"\n'
        f'url     = "https://github.com/{org_repo}"\n'
        f"\n"
        f"[parity]\n"
        f"ratio       = 0.0        # honest: 0.0 means upstream parity NOT yet measured\n"
        f'last_audit  = "{LAST_AUDIT}"\n'
        f"infra_only  = false\n"
        f"\n"
        f"[module]\n"
        f'name        = "{crate}"\n'
        f'source_root = "src"\n'
    )


def process(crate: str) -> dict:
    p, text = read_manifest(crate)
    infra = crate in INFRA_ONLY
    report = {"crate": crate, "exists": text is not None, "infra_only": infra}

    if crate in PHANTOMS:
        report["action"] = "phantom"
        report["note"] = "audit-doc entry without workspace member; out-of-scope"
        return report

    if text is None:
        # No manifest on disk — only D2 targets have a synthesized body.
        if crate in D2_TARGETS:
            new = write_fresh_d2(crate, D2_TARGETS[crate])
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(new, encoding="utf-8")
            report["action"] = "created-d2"
            report["fields_added"] = ["upstream", "parity"]
            return report
        report["action"] = "skip-no-manifest"
        return report

    org, repo, _version = existing_upstream(text)
    new_text = text
    fields_added: list[str] = []

    if infra:
        # Infra crates: still want a [parity] block with infra_only=true,
        # but no license/url backfill (upstream metadata is intentionally
        # absent).
        new_text, inserted = insert_parity_block(new_text, ratio=0.0, infra_only=True)
        if inserted:
            fields_added.append("parity")
    else:
        if org and repo:
            org_repo = f"{org}/{repo}"
            meta = UPSTREAM_META.get(org_repo.lower())
            if meta is None:
                report["action"] = "skip-no-meta"
                report["upstream"] = org_repo
                return report
            meta = {**meta, "_org_repo": org_repo}
            new_text, added = augment_upstream(new_text, meta, name=meta.get("name"))
            fields_added.extend(added)
        # Always insert a [parity] block if missing.
        ratio = MEASURED_RATIO.get(crate, 0.0)
        new_text, inserted = insert_parity_block(new_text, ratio=ratio, infra_only=False)
        if inserted:
            fields_added.append("parity")

    if new_text != text:
        p.write_text(new_text, encoding="utf-8")
        report["action"] = "augmented"
    else:
        report["action"] = "noop"
    report["fields_added"] = fields_added
    return report


def main(target_list: list[str]) -> dict:
    out = {"touched": [], "skipped": [], "errors": []}
    for c in target_list:
        try:
            r = process(c)
        except Exception as e:
            out["errors"].append({"crate": c, "error": repr(e)})
            continue
        if r["action"] in ("augmented", "created-d2"):
            out["touched"].append(r)
        else:
            out["skipped"].append(r)
    return out


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--from-index":
        idx_path = REPO / "docs" / "parity" / "parity-index.json"
        idx = json.loads(idx_path.read_text())
        # All `manifest_filled` is false-or-null + the infra-only set.
        crates = sorted(
            n
            for n, e in idx["crates"].items()
            if e.get("manifest_filled") is not True
        )
        crates += sorted(c for c in INFRA_ONLY if c not in crates)
        # Dedup, preserve order
        seen = set()
        targets = []
        for c in crates:
            if c not in seen:
                seen.add(c)
                targets.append(c)
    else:
        targets = sys.argv[1:]
        if not targets:
            print("usage: parity-augment-manifests.py --from-index | <crate>...", file=sys.stderr)
            sys.exit(2)

    result = main(targets)
    print(json.dumps(result, indent=2))
    print(f"\n✓ touched={len(result['touched'])}  skipped={len(result['skipped'])}  errors={len(result['errors'])}",
          file=sys.stderr)
