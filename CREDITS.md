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
Runtime's own code is Apache-2.0 (see [LICENSE](LICENSE)).

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
- **Hetzner Cloud** — sovereign infrastructure provider that motivated the architecture (ADR-001)

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

## How to add yourself

Open a PR that adds a line under "Contributors" with your name and
GitHub handle. Optional: a one-line description of what you contributed.
We do not require Contributor License Agreements; by submitting a PR you
agree to license your contribution under Apache-2.0.

## Contributors

(Populated post-launch as PRs land. The launch commit on 2026-05-21 starts
this list empty.)
