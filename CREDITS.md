# Credits

Cave Runtime is built by humans and AI agents working together. This file
records who and what produced the v0.1.0 artifact.

The numbers below are pulled from the prototype repo at the moment of the
public-launch orphan commit (see [ADR-148](docs/adr/ADR-148_OSS_Launch_History_Strategy.md)).
Future contributors are appended in chronological order at the bottom.

## Lead

**Burak Tartan** — Architecture, vision, system design, sovereign-cloud
direction. Owner of every cross-cutting decision recorded in `docs/adr/`.

Burak is the committer on every commit in the prototype-era log; the
authorship counts below distinguish *content* contribution where AI agents
generated or co-generated the diff.

## AI co-authors (prototype-era 2026-04-26 → 2026-05-21)

| Agent | Role | Commits where it appears | Notes |
|---|---|---:|---|
| **Claude Opus 4.7 (1M context)** — Anthropic | Pair-programmer for design, refactor, audit, and test work. ADR drafting, sweep-002 cherry-pick + adoption, parity calculator audit, 4-track honest reviews, manifest fills. | 297 (≈22% of prototype-era commits, by `Co-Authored-By` trailer) | Carried the bulk of architectural reasoning + design-doc work alongside Burak. |
| **Qwen3.6 Coder 27B Dense (Q8\_0)** — Alibaba, run locally via Ollama | Test-scaffold generator. Produced `tests/qwen_drafted.rs` files marked `#[ignore = "impl pending"]` across the workspace as TODO markers for behaviour-parity coverage. | 454 (≈34% of prototype-era commits) | Every Qwen-authored test is `#[ignore]`'d by the prompt contract; effective runtime impact on the v0.1.0 artifact is zero. The scaffolds remain as a TODO surface for human-led implementation. |
| **Claude Sonnet 4.6** — Anthropic | Background-agent task helper; small share of early bootstrap commits. | 4 | Listed for completeness. |

The split adds up to >50% of commits because a single commit can carry
multiple co-author trailers (e.g. Burak + Claude on a paired refactor).
What the table reflects is *who participated*, not a competitive
attribution split.

For a per-day / per-crate / per-author live breakdown see the
attribution dashboard in the running portal at
`/api/portal/admin/contributions` once the runtime is live.

## Upstream credits — projects Cave Runtime mirrors

Cave Runtime is a sovereign reimplementation of an OSS stack. We wrote
Rust ports of behaviour, not forks of code; the upstream projects below
are the source of every architectural pattern, wire protocol, and test
case Cave Runtime mirrors. Each is governed by its own license; Cave
Runtime's own code is **AGPL-3.0-or-later** (see [LICENSE](LICENSE) and
[NOTICE](NOTICE)).

### Kubernetes core
- [kubernetes/kubernetes](https://github.com/kubernetes/kubernetes) — apiserver, kubelet, scheduler, controller-manager, cloud-controller-manager, kube-proxy
- [etcd-io/etcd](https://github.com/etcd-io/etcd) — etcd v3 wire compatibility
- [containerd/containerd](https://github.com/containerd/containerd) — CRI compatibility

### Networking & service mesh
- [cilium/cilium](https://github.com/cilium/cilium) — CNI, eBPF dataplane, NetworkPolicy, Hubble, BGP, IPsec, WireGuard
- [istio/istio](https://github.com/istio/istio) — Ambient mesh, ztunnel, Waypoint
- [Kong/kong](https://github.com/Kong/kong) — API gateway data path, plugin pipeline
- [gravitee-io/gravitee-api-management](https://github.com/gravitee-io/gravitee-api-management) — API lifecycle + developer portal

### Data persistence
- [postgres/postgres](https://github.com/postgres/postgres) — Postgres v3 wire protocol
- [cloudnative-pg/cloudnative-pg](https://github.com/cloudnative-pg/cloudnative-pg) — operator pattern
- [pgbouncer/pgbouncer](https://github.com/pgbouncer/pgbouncer) — connection pooling
- [mongodb/mongo](https://github.com/mongodb/mongo) — OP_MSG wire protocol, BSON
- [valkey-io/valkey](https://github.com/valkey-io/valkey) — RESP2/3 protocol
- [apache/iceberg-rust](https://github.com/apache/iceberg-rust) — table format
- [apache/datafusion](https://github.com/apache/datafusion) — query engine

### Streaming
- [apache/kafka](https://github.com/apache/kafka) — wire protocol KIP-482
- [apache/pulsar](https://github.com/apache/pulsar) — binary protocol, persistent topics

### Identity & secrets
- [keycloak/keycloak](https://github.com/keycloak/keycloak) — realm admin, OIDC, JWT
- [openbao/openbao](https://github.com/openbao/openbao) — secrets management (Vault fork)

### Observability
- [prometheus/prometheus](https://github.com/prometheus/prometheus) — metrics
- [grafana/grafana](https://github.com/grafana/grafana) — dashboards
- [grafana/loki](https://github.com/grafana/loki) — logs
- [jaegertracing/jaeger](https://github.com/jaegertracing/jaeger) — traces
- [cilium/hubble](https://github.com/cilium/hubble) — network flow visibility

### Supply chain
- [goharbor/harbor](https://github.com/goharbor/harbor) — container registry
- [pulp/pulpcore](https://github.com/pulp/pulpcore) — package distribution
- [sonatype/nexus-public](https://github.com/sonatype/nexus-public) — artifact repository

### Foundation
- The **Linux kernel** (≥ 7.0 baseline per ADR-014) — eBPF, cgroup v2, namespaces, io_uring
- The **Rust** language and **Tokio** ecosystem — workspace, async runtime, ecosystem deps
- **Sovereign cloud providers** — the existence of independent, non-hyperscaler IaaS motivated the architecture (see public ADR-001)

The full list of tracked upstreams is in
`crates/cave-upstream/src/projects.rs`; the running runtime exposes it at
`/api/portal/upstream`.

## Frameworks & tooling acknowledgements

Built with [tokio](https://tokio.rs), [axum](https://github.com/tokio-rs/axum),
[tower](https://github.com/tower-rs/tower), [hyper](https://hyper.rs),
[serde](https://serde.rs), [thiserror](https://github.com/dtolnay/thiserror),
[anyhow](https://github.com/dtolnay/anyhow), [ring](https://github.com/briansmith/ring),
[rcgen](https://github.com/rustls/rcgen),
[tracing](https://github.com/tokio-rs/tracing), and dozens of crates from
the broader Rust ecosystem. Run `cargo tree` for the exhaustive transitive
list.

Test scaffolds in `tests/qwen_drafted.rs` files were generated by Qwen3.6
Coder 27B running locally on [Ollama](https://github.com/ollama/ollama)
with deterministic prompts.

## Upstream license summary table

The full per-crate mapping is generated from each crate's
`parity.manifest.toml` into `docs/upstream-attribution.md`. The table
below is the headline summary kept in sync by hand for the public
launch; refer to NOTICE for the canonical license-grouped list and
to `crates/cave-upstream/src/projects.rs` for the live runtime view.

| Cave crate                       | Upstream                                    | License                  | Port method                      |
|----------------------------------|---------------------------------------------|--------------------------|----------------------------------|
| cave-apiserver                   | kubernetes/kubernetes                       | Apache-2.0               | line-by-line TDD parity in Rust  |
| cave-etcd                        | etcd-io/etcd                                | Apache-2.0               | line-by-line TDD parity in Rust  |
| cave-kubelet                     | kubernetes/kubernetes (kubelet)             | Apache-2.0               | line-by-line TDD parity in Rust  |
| cave-scheduler                   | kubernetes/kubernetes (kube-scheduler)      | Apache-2.0               | line-by-line TDD parity in Rust  |
| cave-controller-manager          | kubernetes/kubernetes (kube-controller-mgr) | Apache-2.0               | line-by-line TDD parity in Rust  |
| cave-cloud-controller-manager    | kubernetes/kubernetes (cloud-controller)    | Apache-2.0               | line-by-line TDD parity in Rust  |
| cave-kube-proxy                  | kubernetes/kubernetes (kube-proxy)          | Apache-2.0               | line-by-line TDD parity in Rust  |
| cave-cri                         | containerd/containerd                       | Apache-2.0               | clean-room from CRI gRPC spec    |
| cave-net                         | cilium/cilium                               | Apache-2.0               | clean-room from CNI + Hubble API |
| cave-mesh                        | istio/istio (Ambient)                       | Apache-2.0               | clean-room from xDS/HBONE        |
| cave-gateway                     | Kong/kong                                   | Apache-2.0               | clean-room from Kong plugin API  |
| cave-keda                        | kedacore/keda                               | Apache-2.0               | line-by-line TDD parity in Rust  |
| cave-karpenter                   | kubernetes-sigs/karpenter                   | Apache-2.0               | line-by-line TDD parity in Rust  |
| cave-knative                     | knative/serving                             | Apache-2.0               | clean-room from Knative spec     |
| cave-kubevirt                    | kubevirt/kubevirt                           | Apache-2.0               | clean-room from KubeVirt CRDs    |
| cave-kamaji                      | clastix/kamaji                              | Apache-2.0               | line-by-line TDD parity in Rust  |
| cave-rdbms                       | postgres/postgres                           | PostgreSQL (BSD-2-style) | clean-room from v3 wire spec     |
| cave-rdbms-operator              | cloudnative-pg/cloudnative-pg               | Apache-2.0               | clean-room from operator pattern |
| cave-cache                       | valkey-io/valkey (RESP2/3)                  | BSD-3-Clause             | clean-room from RESP spec        |
| cave-streams                     | apache/kafka                                | Apache-2.0               | clean-room from KIP-482          |
| cave-lakehouse                   | apache/iceberg-rust + datafusion            | Apache-2.0               | line-by-line TDD parity in Rust  |
| cave-cdc                         | debezium/debezium-server                    | Apache-2.0               | clean-room from CDC connectors   |
| cave-docdb                       | mongodb/mongo                               | SSPL-1.0 (server)        | clean-room from OP_MSG spec      |
| cave-search                      | manticoresoftware/manticoresearch           | Apache-2.0               | clean-room from query API        |
| cave-vault                       | openbao/openbao                             | Apache-2.0               | line-by-line TDD parity in Rust  |
| cave-auth                        | keycloak/keycloak                           | Apache-2.0               | line-by-line TDD parity in Rust  |
| cave-policy / cave-admission     | open-policy-agent/{opa,gatekeeper}          | Apache-2.0               | clean-room from Rego/admission   |
| cave-pki / cave-acme / cave-certs| smallstep/certificates + cert-manager       | Apache-2.0               | clean-room from ACME/CSR API     |
| cave-sbom / cave-sign            | sigstore/sigstore + cyclonedx               | Apache-2.0               | clean-room from spec             |
| cave-scan / cave-vulns           | aquasecurity/trivy                          | Apache-2.0               | clean-room from scanner API      |
| cave-dast                        | zaproxy/zaproxy                             | Apache-2.0               | clean-room from ZAP API          |
| cave-forensics                   | falcosecurity/falco + cilium/tetragon       | Apache-2.0               | clean-room from eBPF rules       |
| cave-secrets                     | trufflesecurity/trufflehog                  | Apache-2.0               | clean-room from detector model   |
| cave-pii                         | microsoft/presidio                          | MIT                      | clean-room from analyzer/anony.  |
| cave-pam                         | gravitational/teleport                      | Apache-2.0               | clean-room from session-record   |
| cave-container-scan              | aquasecurity/trivy                          | Apache-2.0               | clean-room from CycloneDX out    |
| cave-metrics                     | prometheus/prometheus                       | Apache-2.0               | line-by-line TDD parity in Rust  |
| cave-logs                        | grafana/loki                                | Apache-2.0               | clean-room from LogQL/HTTP       |
| cave-trace / cave-tracing        | jaegertracing/jaeger + OTel                 | Apache-2.0               | clean-room from OTLP spec        |
| cave-dashboard                   | grafana/grafana                             | AGPL-3.0                 | clean-room from datasource API   |
| cave-incidents / cave-oncall     | grafana/oncall                              | Apache-2.0               | clean-room from oncall API       |
| cave-slo                         | nobl9/nobl9-go                              | Apache-2.0               | clean-room from SLO spec         |
| cave-uptime                      | louislam/uptime-kuma                        | Apache-2.0               | clean-room from monitor model    |
| cave-portal / cave-portal-api    | backstage/backstage                         | Apache-2.0               | clean-room from plugin model     |
| cave-techdocs                    | backstage/backstage (TechDocs)              | Apache-2.0               | clean-room from MkDocs flow      |
| cave-permission                  | casbin/casbin                               | Apache-2.0               | clean-room from RBAC/ABAC        |
| cave-runbook                     | grafana/oncall (runbooks)                   | Apache-2.0               | clean-room from runbook DSL      |
| cave-registry                    | distribution/distribution + goharbor/harbor | Apache-2.0               | clean-room from OCI v2 spec      |
| cave-artifacts                   | pulp/pulpcore                               | Apache-2.0               | clean-room from content API      |
| cave-rollouts                    | argoproj/argo-rollouts                      | Apache-2.0               | clean-room from rollout CRDs     |
| cave-pipelines                   | tektoncd/pipeline                           | Apache-2.0               | clean-room from Tekton CRDs      |
| cave-deploy                      | argoproj/argo-cd                            | Apache-2.0               | clean-room from Application CRDs |
| cave-workflows                   | n8n-io/n8n                                  | SSPL-1.0                 | clean-room from node engine      |
| cave-chaos                       | chaos-mesh/chaos-mesh                       | Apache-2.0               | clean-room from CRD model        |
| cave-chat                        | danny-avila/LibreChat                       | Apache-2.0               | clean-room from prompt UX        |
| cave-llm-gateway                 | BerriAI/litellm                             | Apache-2.0               | clean-room from router API       |
| cave-local-llm                   | ollama/ollama                               | Apache-2.0               | clean-room from /api/generate    |
| cave-ai-obs                      | langfuse/langfuse                           | Apache-2.0               | clean-room from trace ingest     |
| cave-erp                         | frappe/erpnext                              | GPL-3.0                  | clean-room from doctype model    |
| cave-crm                         | makeplane/plane                             | AGPL-3.0                 | clean-room from data model       |
| cave-tracker                     | makeplane/plane (issues)                    | AGPL-3.0                 | clean-room from issue model      |
| cave-devlake                     | apache/incubator-devlake                    | Apache-2.0               | clean-room from collector API    |
| cave-flags                       | Unleash/unleash                             | Apache-2.0               | clean-room from flag SDK         |
| cave-cost / cave-cost-alloc      | opencost/opencost                           | Apache-2.0               | clean-room from cost model       |
| cave-backup                      | vmware-tanzu/velero                         | Apache-2.0               | clean-room from BackupController |
| cave-gitops-config               | (custom — first-party)                      | (n/a)                    | first-party design               |
| cave-compliance                  | (custom — first-party)                      | (n/a)                    | first-party design               |
| cave-ledger                      | (custom — first-party)                      | (n/a)                    | first-party Sovereign Ledger     |
| cave-status / cave-changelog     | (custom — first-party)                      | (n/a)                    | first-party design               |

The remaining workspace crates (cave-core / cave-kernel / cave-cli /
cave-runtime / cave-portal-web / cave-cluster / cave-desktop / etc.) are
first-party glue without a single direct upstream and inherit AGPL-3.0
through the workspace.

## How to add yourself

Open a PR that adds a line under "Contributors" with your name and
GitHub handle. Optional: a one-line description of what you contributed.
We do not require Contributor License Agreements; by submitting a PR you
agree to license your contribution under **AGPL-3.0-or-later**.

## Contributors

(Populated post-launch as PRs land. The launch commit on 2026-05-21 starts
this list empty.)
