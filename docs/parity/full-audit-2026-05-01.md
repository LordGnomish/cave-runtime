# Full Parity Audit — 2026-05-01

Tier-grouped roadmap for all 103 crates in `crates/*`, generated against the
`feat(parity): expand calculator to discover all crates with parity.manifest.toml`
landing.

Reproduce with:
```sh
cargo run -p cave-kernel --example parity_audit > audit.csv
```

## Headline numbers

| Bucket                                            | Count | Notes |
|---------------------------------------------------|------:|-------|
| Total crates in `crates/*`                        |   103 | up from the "85" estimate |
| Manifest present                                  |    89 | one `parity.manifest.toml` per crate root |
| Manifest absent                                   |    14 | 5 are CAVE-internal infra (no upstream) |
| Manifest with **declared items** (file/fn/test/surface) | 13 | the only crates the calculator can score against |
| Manifest empty (declared file but `[]` everywhere)|    76 | calculator returns `overall=0.0` regardless of impl size |
| **Reaches 100% parity**                           | **5** | apiserver / cri / etcd / kubelet / scheduler |
| Manifest items, ≥50% overall                      |     3 | vault, streams, cloud-controller-manager |
| Manifest items, <50% overall (partial-fill)       |     5 | local-llm, portal, mesh, controller-manager, kube-proxy |

## Constraint reminders for this audit

- **Stub yasak (golden rule 3):** crates with `todo!` / `unimplemented!` are
  flagged in the calculator's `stubs_detected` field and listed below. The
  calculator never substitutes a green check for a stub.
- **Empty manifest ≠ empty crate.** `cave-net` ships 36k src lines but its
  `parity.manifest.toml` declares zero file/fn/test/surface mappings, so the
  calculator scores it 0/0 (overall 0.0). Filling the manifest is a
  bookkeeping task — the impl is already there.
- **CAVE-internal crates** (cave-runtime, cave-cli, cave-kernel, cave-portal-api,
  cave-portal-web, cave-tracing) have no upstream counterpart. They legitimately
  have no manifest and no parity score.

---

## Tier ✅ — 100% parity reached (5 crates)

These are the only crates currently green on the calculator. The k8s control
plane is the proof-of-concept; everything else needs to follow this playbook.

| Crate              | Upstream                          | file | fn | test | surface | stubs |
|--------------------|-----------------------------------|-----:|---:|-----:|--------:|------:|
| cave-apiserver     | kubernetes/kubernetes @ v1.28.0   | 49/49 | 22/22 | 12/12 | 16/16 | 0 |
| cave-cri           | containerd/containerd @ v1.7.0    | 31/31 | 83/83 | 87/87 | 35/35 | 0 |
| cave-etcd          | etcd-io/etcd @ v3.5.13            | 13/13 | 41/41 | 87/87 | 34/34 | 0 |
| cave-kubelet       | kubernetes/kubernetes @ v1.28.0   | 30/30 | 24/24 | 118/118 | 3/3 | 0 |
| cave-scheduler     | kubernetes/kubernetes @ v1.28.0   | 33/33 | 19/19 | 107/107 | 3/3 | 0 |

---

## Tier A — Close to 100% (3 crates, OSS-launch realistic)

Manifest is fleshed out; one or two missing items each. Each lands in <1 day.

| Crate                          | Upstream | overall | gap |
|--------------------------------|----------|--------:|-----|
| cave-vault                     | hashicorp/vault @ v1.15.0 | 0.6625 (93.5% items) | 1 fn missing, 1 surface missing, **0 tests declared** — needs test mapping pass |
| cave-streams                   | apache/kafka @ 3.6.0      | 0.7159 (90.7% items) | 3 fn missing, 2 surfaces missing |
| cave-cloud-controller-manager  | kubernetes/kubernetes @ v1.28.0 | 0.4444 (86.4% items) | 6 fn missing, **0 tests / 0 surfaces declared** |

**Action**: port the missing functions, add the missing surface routes, populate
`[[tests]]` for vault and cloud-controller-manager. Realistic for OSS launch.

---

## Tier B — Partial-fill manifests (5 crates, medium work)

Manifest declares some dimensions in full but leaves others empty, dragging
overall down. None are "real misses" — they need additional dimensions
declared (typically tests + surfaces) or implementations of declared items.

| Crate              | Upstream | overall | what's declared / missing |
|--------------------|----------|--------:|---------------------------|
| cave-local-llm     | huggingface/transformers @ v4.36 | 0.50 | 5/5 fn, 3/3 surf — **3 stubs** in src; needs file + test mappings |
| cave-portal        | backstage/backstage @ v1.20.0 | 0.25 | 10/10 files only — needs fn/test/surface mappings |
| cave-mesh          | istio/istio @ 1.20.0 | 0.25 | 8/8 files only — needs fn/test/surface mappings |
| cave-controller-manager | kubernetes/kubernetes @ v1.28.0 | 0.25 | 10/10 files only, **3 stubs** in src |
| cave-kube-proxy    | kubernetes/kubernetes @ v1.28.0 | 0.25 | 10/10 files; **8 fn missing, 4 surfaces missing** |

**Action**: kube-proxy has real misses (port the 8 functions and 4 routes).
The other four are "manifest fill" tasks similar to Tier C below.

---

## Tier C — Empty manifest, real implementation already present (53 crates)

> 2026-05-19 cleanup (ADR-151): the prior count was 58; five entries were
> phantom directories that no longer exist on disk and have been removed
> from this table — `cave-pg` (renamed to `cave-rdbms-operator` per
> ADR-147) and the four 5d6a067b orphan-dir deletions (`cave-spire`,
> `cave-external-secrets`, `cave-hubble`, `cave-vcluster`). Their
> upstream surfaces are absorbed elsewhere: PgBouncer pool →
> `cave-rdbms-operator`, External Secrets → `cave-vault`, Hubble →
> `cave-forensics`, vcluster → `cave-kamaji`, SPIRE → cave-pki (future).

`parity.manifest.toml` exists with `[upstream]` and `[module]` blocks but no
`[[files]]` / `[[functions]]` / `[[tests]]` / `[[surfaces]]` entries. The
calculator has nothing to score, so they show 0%, but the impl is real.

**This is the single biggest lever for the parity dashboard.** Each of these
needs a manifest-fill pass: walk the upstream repo, enumerate ported items,
add mappings. No new code; just bookkeeping.

Sorted by total src LoC (largest first — biggest dashboard wins per hour spent):

| Crate | total src | Upstream |
|-------|----------:|----------|
| cave-net | 36,645 | cilium/cilium @ v1.19.3 |
| cave-gateway | 12,187 | Kong/kong @ v3.5.0 |
| cave-policy | 11,624 | open-policy-agent/opa @ v0.58.0 |
| cave-cache | 11,602 | redis/redis @ 7.2.0 |
| cave-store | 10,711 | minio/minio |
| cave-metrics | 9,530 | prometheus/prometheus @ v2.48.0 |
| cave-trace | 8,923 | jaegertracing/jaeger @ v1.52.0 |
| cave-auth | 8,236 | keycloak/keycloak @ v22.0.0 |
| cave-dashboard | 8,092 | grafana/grafana @ v10.2.0 |
| cave-dns | 7,796 | coredns/coredns @ v1.11.0 |
| cave-logs | 7,285 | grafana/loki @ v2.9.0 |
| cave-security | 6,350 | falcosecurity/falco @ v0.36.0 |
| cave-ha | 5,882 | etcd-io/etcd @ v3.5.13 |
| cave-registry | 5,540 | goharbor/harbor @ v2.10.0 |
| cave-erp | 5,298 | erpnext/erpnext @ v15.0.0 |
| cave-rdbms | 5,073 | postgres/postgres @ 16.0 |
| cave-artifacts | 4,994 | sonatype/nexus-public @ v3.0.0 |
| cave-tracker | 4,898 | linear-app/linear @ v1.0.0 |
| cave-cluster | 4,873 | kubernetes-sigs/cluster-api @ v1.6.0 |
| cave-deploy | 4,743 | argoproj/argo-cd @ v2.9.0 |
| cave-infra | 4,648 | hashicorp/terraform @ v1.6.0 |
| cave-pipelines | 4,211 | tektoncd/pipeline @ v0.55.0 |
| cave-alerts | 3,829 | prometheus/alertmanager @ v0.26.0 |
| cave-llm-gateway | 3,689 | BerriAI/litellm @ v1.0.0 |
| cave-docdb | 3,622 | mongodb/mongo @ 7.0.0 |
| cave-upstream | 3,519 | (CAVE internal — re-target) |
| cave-runbook | 3,237 | (CAVE internal — re-target) |
| cave-flags | 2,948 | Unleash/unleash @ v5.0.0 |
| cave-backup | 2,662 | vmware-tanzu/velero @ v1.12.0 |
| cave-container-scan | 2,340 | aquasecurity/trivy @ v0.48.0 |
| cave-crossplane | 2,328 | crossplane/crossplane @ v1.14.0 |
| cave-compliance | 2,289 | open-policy-agent/gatekeeper @ v3.14.0 |
| cave-docs-site | 2,185 | (CAVE internal — re-target) |
| cave-gitops-config | 2,182 | fluxcd/flux2 @ v2.1.0 |
| cave-core | 1,990 | (CAVE internal — re-target) |
| cave-cost | 1,965 | opencost/opencost @ v1.108.0 |
| cave-rollouts | 1,898 | argoproj/argo-rollouts @ v1.6.0 |
| cave-oncall | 1,846 | grafana/oncall @ v1.4.0 |
| cave-cost-alloc | 1,737 | opencost/opencost @ v1.108.0 |
| cave-admission | 1,620 | kubernetes/kubernetes @ v1.28.0 |
| cave-scaffold | 1,398 | backstage/backstage @ v1.20.0 |
| cave-scan | 1,189 | SonarSource/sonarqube @ v10.3.0 |
| cave-knative | 1,167 | knative/serving @ v1.12.0 |
| cave-keda | 1,145 | kedacore/keda @ v2.12.0 |
| cave-chaos | 1,125 | chaos-mesh/chaos-mesh @ v2.6.0 |
| cave-certs | 914 | cert-manager/cert-manager @ v1.13.0 |
| cave-incidents | 836 | grafana/oncall @ v1.4.0 |
| cave-ledger | 821 | (CAVE internal — re-target) |
| cave-db | 707 | (CAVE internal — re-target) |
| cave-workflows | 666 | n8n-io/n8n @ v1.0.0 |
| cave-secrets | 648 | trufflesecurity/trufflehog @ v3.63.0 |
| cave-lint | 625 | SonarSource/sonarqube @ v10.3.0 |
| cave-devlake | 526 | apache/incubator-devlake @ v0.19.0 |

**Note**: a handful of these manifests point at `cave-runtime/cave-runtime @
v0.1.0` as their "upstream" — that's a placeholder and should be re-targeted
to the actual OSS project the crate mirrors before manifest-fill.

---

## Tier D — Skeleton crates (18 + 9, real impl gerek)

### D1: empty manifest, <500 src lines (true skeletons, 18 crates)

| Crate | total src | Upstream |
|-------|----------:|----------|
| cave-slo | 476 | OpenSLO/OpenSLO @ v0.1.0 |
| cave-changelog | 463 | towncrier/towncrier @ 23.0.0 |
| cave-docs | 460 | backstage/backstage @ v1.20.0 |
| cave-status | 450 | louislam/uptime-kuma @ v1.23.0 |
| cave-sbom | 434 | DependencyTrack/dependency-track @ v4.9.0 |
| cave-chat | 419 | danny-avila/LibreChat @ v0.7.0 |
| cave-ai-obs | 408 | langfuse/langfuse @ v2.0.0 |
| cave-vulns | 384 | DefectDojo/django-DefectDojo @ v2.28.0 |
| cave-pam | 353 | gravitational/teleport @ v14.0.0 |
| cave-dast | 304 | zaproxy/zaproxy @ v2.14.0 |
| cave-uptime | 299 | louislam/uptime-kuma @ v1.23.0 |
| cave-forensics | 210 | cilium/tetragon @ v1.0.0 |
| cave-pii | 204 | microsoft/presidio @ v2.2.0 |
| cave-kamaji | 199 | clastix/kamaji @ v1.0.0 |
| cave-sign | 183 | sigstore/sigstore @ v1.8.0 |
| cave-profiler | 180 | grafana/pyroscope @ v1.3.0 |
| cave-desktop | 168 | zed-industries/zed @ main |
| cave-ebpf-common | 127 | cilium/cilium @ v1.14.0 |

### D2: no manifest, has upstream counterpart (9 crates)

These need both a `parity.manifest.toml` written from scratch *and* the impl
fleshed out / expanded.

| Crate | total src | Likely upstream (verify before manifest write) |
|-------|----------:|------------------------------------------------|
| cave-tracing | 2,502 | needs target — possibly opentelemetry-rust or jaeger client |
| cave-cdc | 1,456 | likely debezium / pg-logical-replication |
| cave-permission | 586 | likely casbin / authzed |
| cave-pki | 607 | likely smallstep/certificates or hashicorp/vault PKI |
| cave-techdocs | 835 | backstage/backstage techdocs plugin |
| cave-acme | 662 | likely smallstep/certificates ACME |
| cave-iceberg | 1,986 | apache/iceberg |
| cave-datafusion | 1,921 | apache/arrow-datafusion |
| cave-search | 172 | likely opensearch-project/OpenSearch |

---

## Tier E — CAVE-internal infrastructure (no manifest expected)

These have no upstream counterpart. They are the platform itself.

| Crate              | Role |
|--------------------|------|
| cave-runtime       | Single-binary orchestrator hosting all module routers |
| cave-cli           | Operator CLI |
| cave-kernel        | Shared parity calculator, ratelimiter, retry, reconcile primitives |
| cave-portal-api    | Portal HTTP API attribution endpoints |
| cave-portal-web    | Static web bundle for the portal UI |

---

## OSS-launch (20 days) realism cut

**Realistic to land before launch (parity dashboard goes from 5 → ~25 modules):**

1. **Tier A close-out** (vault, streams, cloud-controller-manager) — 3 modules
   to 100%.
2. **Tier B real-impl** (kube-proxy 8 fn + 4 surfaces) — 1 module to 100%.
3. **Tier B manifest-fill** (portal, mesh, controller-manager, local-llm) — 4
   modules potentially to 100% just by populating tests + surfaces.
4. **Top-of-Tier-C manifest-fill** for the largest 12 crates (cave-net,
   cave-gateway, cave-policy, cave-cache, cave-store, cave-metrics, cave-trace,
   cave-auth, cave-dashboard, cave-dns, cave-logs, cave-security). Each should
   reach >50% overall once mappings are populated, given how much real impl is
   already present.

**Post-OSS launch (Wave 2 push targets):**

- Remaining 46 Tier C crates (manifest-fill only, no new code).
- All 27 Tier D crates (manifest write + impl extension).
- Re-target the ~6 Tier C manifests pointing at `cave-runtime/cave-runtime` to
  their actual upstream OSS projects.

## Stub watch (golden rule 3)

`stubs_detected` is non-zero in 2 crates with declared manifests:
- `cave-local-llm` — 3 stubs
- `cave-controller-manager` — 3 stubs

Tier C/D crates are not stub-scanned automatically because their manifest does
not declare a source root with mappings, but a separate `grep -rn 'todo!\|unimplemented!'`
sweep should be run before OSS launch.
