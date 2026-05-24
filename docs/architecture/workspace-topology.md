# Workspace topology — `cave-runtime`

**Generated:** 2026-05-24T20:22:22Z  
**Source:** `docs/refactor-sweep-2.3-workspace-reorg.md` (theme map),
`cargo metadata --no-deps` (cross-theme edges),
`docs/parity/parity-index.json` (per-theme parity).  
**Disk crates:** 112 (all `cave-*`)  
**Themes:** 11

This document is a regenerated read-out of how the workspace
crates group together by responsibility, NOT a description of
an on-disk move — the workspace is still flat under `crates/`
(see `docs/refactor-sweep-2.3-workspace-reorg.md` for why the
mechanical move was scope-cut).

## 11 themes

| Theme | Crates | Mean fill | ≥0.95 |
|-------|-------:|----------:|------:|
| `core` | 4 | 0.000 | 0 |
| `compute` | 15 | 0.786 | 12 |
| `data` | 12 | 0.657 | 8 |
| `security` | 25 | 0.665 | 16 |
| `observability` | 15 | 0.322 | 4 |
| `networking` | 4 | 0.978 | 4 |
| `orchestration` | 12 | 0.405 | 5 |
| `ai` | 4 | 0.988 | 4 |
| `registry` | 2 | 0.479 | 1 |
| `ops` | 15 | 0.128 | 2 |
| `apps` | 4 | 0.250 | 1 |

Themes correspond to the responsibility boundary discussed in
`docs/refactor-sweep-2.3-workspace-reorg.md`. The mean-fill
column averages the `parity_ratio` field from
`docs/parity/parity-index.json` across crates in that theme;
infra-only crates contribute `0.0` and so depress the average
for foundational themes (`core`, `ops`).

### `core`

Primitives. Top inbound crates per chain.md: `cave-kernel` (20 incoming), `cave-db` (14), `cave-core` (9).

- `cave-core` — fill 0.000 / honest 0.000 (infra)
- `cave-db` — fill 0.000 / honest 0.000 (infra)
- `cave-ebpf-common` — fill 0.000 / honest 0.000 (infra)
- `cave-kernel` — fill 0.000 / honest 0.000 (infra)

### `compute`

Kubernetes-shaped control plane + the `cave-runtime` all-in binary that wires every workspace crate together.

- `cave-admission` — fill 0.000 / honest 0.000
- `cave-apiserver` — fill 0.961 / honest 0.941
- `cave-cloud-controller-manager` — fill 1.000 / honest 0.957
- `cave-cluster` — fill 0.000 / honest 0.000
- `cave-controller-manager` — fill 0.956 / honest 0.956
- `cave-cri` — fill 1.000 / honest 0.912
- `cave-crossplane` — fill 0.975 / honest 0.675
- `cave-etcd` — fill 0.958 / honest 0.930
- `cave-kamaji` — fill 1.000 / honest 0.824
- `cave-knative` — fill 1.000 / honest 1.000
- `cave-kube-proxy` — fill 1.000 / honest 0.941
- `cave-kubelet` — fill 0.974 / honest 0.949
- `cave-kubevirt` — fill 1.000 / honest 1.000
- `cave-runtime` — fill 0.000 / honest 0.000 (infra)
- `cave-scheduler` — fill 0.966 / honest 0.966

### `data`

Persistence + analytical engines. RDBMS, document store, KV cache, columnar lakehouse, streams.

- `cave-cache` — fill 1.000 / honest 0.895
- `cave-cdc` — fill 0.000 / honest 0.000
- `cave-datafusion` — fill 1.000 / honest 0.485
- `cave-docdb` — fill 0.981 / honest 0.923
- `cave-iceberg` — fill 1.000 / honest 0.667
- `cave-lakehouse` — fill 0.978 / honest 0.935
- `cave-ledger` — fill 0.000 / honest 0.000 (infra)
- `cave-rdbms` — fill 0.971 / honest 0.913
- `cave-rdbms-operator` — fill 1.000 / honest 1.000
- `cave-search` — fill 0.000 / honest 0.000
- `cave-store` — fill 0.000 / honest 0.000
- `cave-streams` — fill 0.956 / honest 0.956

### `security`

Identity, secrets, scan, supply chain. ADR-RUNTIME-SANDBOX-NO-FFI-001 governs sandboxing posture.

- `cave-acme` — fill 0.000 / honest 0.000 (infra)
- `cave-auth` — fill 1.000 / honest 0.977
- `cave-bench` — fill 1.000 / honest 0.727
- `cave-certs` — fill 0.000 / honest 0.000
- `cave-compliance` — fill 0.000 / honest 0.000
- `cave-container-scan` — fill 0.962 / honest 0.712
- `cave-dast` — fill 1.000 / honest 0.923
- `cave-falco` — fill 1.000 / honest 0.731
- `cave-forensics` — fill 0.958 / honest 0.682
- `cave-gitleaks` — fill 1.000 / honest 0.900
- `cave-identity` — fill 1.000 / honest 0.720
- `cave-pam` — fill 0.000 / honest 0.000
- `cave-permission` — fill 0.000 / honest 0.000
- `cave-pii` — fill 0.000 / honest 0.000 (infra)
- `cave-pki` — fill 0.000 / honest 0.000 (infra)
- `cave-policy` — fill 0.962 / honest 0.577
- `cave-sandbox` — fill 1.000 / honest 0.746
- `cave-sbom` — fill 0.950 / honest 0.733
- `cave-scan` — fill 0.964 / honest 0.917
- `cave-scan-db` — fill 0.952 / honest 0.857
- `cave-secrets` — fill 0.969 / honest 0.438
- `cave-security` — fill 0.000 / honest 0.000
- `cave-sign` — fill 0.949 / honest 0.538
- `cave-vault` — fill 1.000 / honest 0.562
- `cave-vulns` — fill 0.950 / honest 0.900

### `observability`

Telemetry + on-call. cave-logs/metrics/trace form the obs core; cave-oncall is the routing layer.

- `cave-ai-obs` — fill 0.000 / honest 0.000
- `cave-alerts` — fill 0.000 / honest 0.000
- `cave-dashboard` — fill 0.952 / honest 0.809
- `cave-incidents` — fill 0.000 / honest 0.000
- `cave-logs` — fill 0.958 / honest 0.875
- `cave-metrics` — fill 0.967 / honest 0.900
- `cave-oncall` — fill 1.000 / honest 0.889
- `cave-profiler` — fill 0.000 / honest 0.000 (infra)
- `cave-runbook` — fill 0.000 / honest 0.000 (infra)
- `cave-slo` — fill 0.000 / honest 0.000
- `cave-status` — fill 0.000 / honest 0.000 (infra)
- `cave-techdocs` — fill 0.000 / honest 0.000 (infra)
- `cave-trace` — fill 0.947 / honest 0.605
- `cave-tracing` — fill 0.000 / honest 0.000 (infra)
- `cave-uptime` — fill 0.000 / honest 0.000

### `networking`

L3-L7. cave-net (Cilium/Hubble), cave-gateway (Kong+Gravitee), cave-dns (CoreDNS), cave-mesh (Istio).

- `cave-dns` — fill 0.958 / honest 0.750
- `cave-gateway` — fill 0.967 / honest 0.733
- `cave-mesh` — fill 1.000 / honest 0.973
- `cave-net` — fill 0.985 / honest 0.985

### `orchestration`

Deployment + workflow orchestration. ArgoCD/Rollouts/Workflows + Karpenter + KEDA + chaos + backup.

- `cave-backup` — fill 0.000 / honest 0.000
- `cave-chaos` — fill 0.000 / honest 0.000
- `cave-deploy` — fill 0.974 / honest 0.632
- `cave-gitops-config` — fill 0.000 / honest 0.000
- `cave-ha` — fill 0.000 / honest 0.000
- `cave-infra` — fill 0.000 / honest 0.000
- `cave-karpenter` — fill 1.000 / honest 0.864
- `cave-keda` — fill 0.955 / honest 0.750
- `cave-pipelines` — fill 0.000 / honest 0.000
- `cave-rollouts` — fill 0.968 / honest 0.710
- `cave-tracker` — fill 0.000 / honest 0.000
- `cave-workflows` — fill 0.958 / honest 0.667

### `ai`

LLM-facing crates. cave-hermes (agent), cave-llm-gateway (LiteLLM), cave-llm-tracker (model freshness), cave-local-llm (Ollama/llama.cpp/MLX).

- `cave-hermes` — fill 0.953 / honest 0.953
- `cave-llm-gateway` — fill 1.000 / honest 0.500
- `cave-llm-tracker` — fill 1.000 / honest 0.647
- `cave-local-llm` — fill 1.000 / honest 0.926

### `registry`

OCI + artefact storage (Harbor / Nexus).

- `cave-artifacts` — fill 0.957 / honest 0.329
- `cave-registry` — fill 0.000 / honest 0.000 (infra)

### `ops`

Portal + CLI + docs + cost/flag tooling. Internal-facing surfaces.

- `cave-changelog` — fill 0.000 / honest 0.000 (infra)
- `cave-cli` — fill 0.000 / honest 0.000 (infra)
- `cave-cost` — fill 0.000 / honest 0.000
- `cave-cost-alloc` — fill 0.000 / honest 0.000 (infra)
- `cave-desktop` — fill 0.000 / honest 0.000 (infra)
- `cave-docs` — fill 0.000 / honest 0.000 (infra)
- `cave-docs-site` — fill 0.000 / honest 0.000 (infra)
- `cave-flags` — fill 0.969 / honest 0.923
- `cave-lint` — fill 0.000 / honest 0.000 (infra)
- `cave-portal` — fill 0.952 / honest 0.875
- `cave-portal-api` — fill 0.000 / honest 0.000 (infra)
- `cave-portal-web` — fill 0.000 / honest 0.000 (infra)
- `cave-scaffold` — fill 0.000 / honest 0.000 (infra)
- `cave-upstream` — fill 0.000 / honest 0.000 (infra)
- `cave-upstream-watchd` — fill 0.000 / honest 0.000 (infra)

### `apps`

User-facing business apps (CRM/ERP/chat/devlake) layered above the runtime.

- `cave-chat` — fill 0.000 / honest 0.000
- `cave-crm` — fill 1.000 / honest 0.513
- `cave-devlake` — fill 0.000 / honest 0.000
- `cave-erp` — fill 0.000 / honest 0.000

## Cross-theme dependencies

Total non-dev intra-workspace edges: **134** (see
`docs/synergy/chain.md` for the full list). Of these,
**110** edges cross a theme boundary; **24**
are intra-theme.

### Edge counts between themes (source → target)

| From | To | Distinct edges |
|------|----|---------------:|
| `compute` | `security` | 16 |
| `compute` | `observability` | 13 |
| `security` | `core` | 11 |
| `compute` | `ops` | 10 |
| `compute` | `orchestration` | 9 |
| `compute` | `core` | 7 |
| `compute` | `data` | 6 |
| `data` | `core` | 5 |
| `ops` | `core` | 5 |
| `compute` | `apps` | 4 |
| `compute` | `networking` | 4 |
| `networking` | `core` | 4 |
| `orchestration` | `core` | 4 |
| `registry` | `security` | 3 |
| `ai` | `core` | 2 |
| `observability` | `core` | 2 |
| `registry` | `core` | 2 |
| `compute` | `ai` | 1 |
| `compute` | `registry` | 1 |
| `ops` | `security` | 1 |

## Phantom theme entries (in map but not on disk)

None.

## Disk crates without a theme assignment

None — every disk crate is assigned to exactly one theme.

## Programme-track snapshot (2026-05-24)

The runtime is currently driven by four parallel tracks; this
section is a cursor in time, not a long-lived ground truth.

1. **Parity 1.00 uplift** — `adr_justified_ratio = 1.0` sweep over
   the 14 priority crates (cave-gateway / cave-rdbms / cave-docdb /
   cave-iceberg / cave-datafusion / cave-apiserver / cave-controller-manager /
   cave-kubelet / cave-cri / cave-cloud-controller-manager / cave-kube-proxy /
   cave-net / cave-scheduler / cave-etcd). 12 of 14 at 1.00; 2 honest
   sub-1.00 preserved (cave-scheduler 0.966, cave-etcd 0.986). Driven
   by ADR-RUNTIME-PARITY-100-PCT-001.

2. **Charter v2 closures** — security ecosystem (cave-policy /
   cave-vault / cave-identity / cave-bench / cave-sandbox /
   cave-forensics), Argo + Apicurio (cave-deploy / cave-rollouts /
   cave-workflows / cave-knative), Tetragon / Hubble / CoreDNS /
   Crossplane all landed in May 2026. Workspace ≥0.95 count
   now standing at **57** (excl. infra-only).

3. **Refactor sweep (Phase 2.x)** — 8-phase atomic sweep finalized
   on 2026-05-23 (commit `f8f0aa53`). Brought workspace build time
   from 3m17s → 2m29s. Phase 2.5 doc-sync is the surface this
   document lives in. See `docs/refactor-sweep-2026-05-23-report.md`.

4. **OSS launch hygiene** — both repos are public since 2026-05-22
   (`cave-runtime` AGPL-3.0, `cave-home` Apache-2.0). ADR-148 governs
   the squashed history; ADR-151 the phantom-crate cleanup.

## Refresh

```bash
cd $(git rev-parse --show-toplevel)
cargo metadata --no-deps --format-version 1 > /tmp/cargo_meta.json
# regen via the doc-sync ray (see docs/refactor-sweep-2.5-doc-sync.md)
```
