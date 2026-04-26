# Cave Runtime

**Sovereign, self-healing, self-improving Cloud OS. Multi-tenant by construction. Line-by-line TDD reimplementation of best-of-breed upstream projects in Rust.**

> Status: pre-v1. OSS launch target: 21 May 2026. See [docs/adr/ADR-CHARTER-001.md](docs/adr/ADR-CHARTER-001.md) for the mission, [docs/adr/](docs/adr/) for the full decision record.

---

## What this is

Cave Runtime is a single Rust binary that reimplements the Kubernetes control-plane and surrounding ecosystem — etcd, kube-apiserver, kubelet, kube-scheduler, containerd, Cilium CNI, Istio, Kong, Keycloak, OpenBao, Harbor, ArgoCD, Backstage, and ~60 more — under one unified runtime with shared primitives (Raft consensus, WAL, EventBus, Labels, SPIFFE identity).

Because every upstream is reimplemented under one roof:
- Duplicated concerns (state replication, identity, rate-limiting, backoff) live in `cave-kernel` and every module uses the same battle-tested implementation.
- Cross-module capabilities that no single upstream CLI can express (federated watch, consistent cross-module snapshot, tenant-scoped policy) ship as first-class verbs.
- A single upgrade path rolls the whole platform forward with zero user-visible downtime.

**This is not a thin wrapper over existing projects.** It is a from-scratch Rust implementation that passes the upstream's own tests. See [ADR-GOLDEN-001](docs/adr/ADR-GOLDEN-001-upstream-parity.md) for the rule.

## Charter

1. **World's most performant** — Rust + io_uring + eBPF + cgroup v2 + Linux 7.1+; tail-latency and p99 beat upstream.
2. **Fully featured** — not a "core path" MVP; every upstream flag, edge case, and error mode is ported.
3. **Sovereign** — Linux 7.1 kernel and nothing else; no external SaaS dependency at the core.
4. **Self-healing** — reconcile loops with automatic drift correction at every layer.
5. **Self-improving** — in-cluster `cave-agent` reads observability, proposes tuned changes, canary-deploys, rolls back on regression. See [ADR-SELF-IMPROVE-001](docs/adr/ADR-SELF-IMPROVE-001.md).
6. **Multi-region** — federated control plane + regional data planes.
7. **HA** — 3-replica minimum for every critical component, Raft quorum consensus (one implementation, shared via `cave-kernel`).
8. **DR** — cross-region async replication + point-in-time recovery.
9. **Zero-downtime upgrade** — rolling, blue-green, version-skew tolerant.
10. **HA/DR latency hiding** — replication delays never leak to the client SLA.
11. **Multi-tenant by construction** — every module carries `tenant_id` as first-class attribute; default-deny between tenants; per-tenant quota, SLO, billing. See [ADR-MULTI-TENANT-001](docs/adr/ADR-MULTI-TENANT-001.md).
12. **Post-quantum crypto ready** — ML-KEM / ML-DSA / SLH-DSA at the primitives layer, no classical-only paths. See [ADR-GOLDEN-003](docs/adr/ADR-GOLDEN-003-no-backcompat-pqc.md).

## Project layout

```
cave-runtime/
├── crates/
│   ├── cave-kernel/        shared primitives (Raft, WAL, EventBus, Labels, watch, mvcc)
│   ├── cave-core/          common types, error, hardened I/O
│   ├── cave-auth/          Keycloak-parity OIDC + RBAC + ABAC + SPIFFE identity
│   ├── cave-apiserver/     kube-apiserver parity
│   ├── cave-etcd/          etcd v3 parity (KV, watch, lease, auth, txn, MVCC)
│   ├── cave-scheduler/     kube-scheduler parity
│   ├── cave-kubelet/       kubelet parity
│   ├── cave-cri/           containerd + runc/crun parity
│   ├── cave-net/           Cilium CNI parity, eBPF dataplane
│   ├── cave-mesh/          Istio Ambient parity
│   ├── cave-gateway/       Kong parity, xDS, plugins
│   ├── cave-vault/         OpenBao parity (KV, PKI, transit, SSH, TOTP)
│   ├── cave-registry/      Harbor parity, OCI image + Helm
│   ├── cave-metrics/       Prometheus parity
│   ├── cave-trace/         Tempo / OTel parity
│   ├── cave-scan/          Trivy + Semgrep parity
│   ├── cave-portal/        unified admin portal (two personas per ADR-PORTAL-PERSONAS-001)
│   ├── cave-portal-api/    portal backend
│   ├── cave-local-llm/     build-time dev agent (Qwen3-assisted drafts)
│   ├── cave-agent/         runtime self-improvement agent (per ADR-SELF-IMPROVE-001)
│   ├── cave-cli/           cavectl — unified CLI with upstream parity + cave-native verbs
│   └── ...                  (~60 crates total, see Cargo.toml workspace members)
├── docs/adr/               every architectural decision with context + rationale
├── docs/chain/             module handoff records
├── docs/synergy/           refactor sweep progress
└── deploy/                 launchd plists + systemd units + helm charts
```

## Quickstart (developer)

```bash
# Prerequisites: Rust 1.85+, protoc, Docker (for integration tests), Ollama (optional, for cave-local-llm)

git clone https://github.com/cave-runtime/cave-runtime
cd cave-runtime

# Build everything
cargo build --workspace --release

# Run unit tests
cargo test --workspace

# Start the portal (requires CAVE_JWT_SECRET; any string for dev)
CAVE_JWT_SECRET=dev ./target/release/cave-runtime
# → http://localhost:8080

# Use the CLI
./target/release/cave --help
./target/release/cave etcd get /foo   # cave-native shortcut
./target/release/cave etcdctl get /foo  # upstream etcdctl parity (see ADR-CLI-HYBRID-001)
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Short version: read [ADR-GOLDEN-001](docs/adr/) first — all contributions follow line-by-line upstream TDD parity. No stubs, no `todo!()`, no behavioral approximation.

## Security

See [SECURITY.md](SECURITY.md) for disclosure policy. Cave Runtime is post-quantum-crypto ready; report classical-crypto fallbacks as bugs.

## License

Apache License 2.0 — see [LICENSE](LICENSE).

## Status

Pre-v1. Module completion tracked live at `/admin/parity` on a running portal, sourced from `git log` + test counts + endpoint counts. For a pragmatic snapshot read [docs/audit/2026-04-23-devils-advocate.md](docs/audit/2026-04-23-devils-advocate.md).
