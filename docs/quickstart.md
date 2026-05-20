# Cave Runtime — Quickstart

This page is the **10-line recipe**. For deeper material see the
runbooks under [`docs/runbook/`](runbook/) and the architecture
overview in [`ARCHITECTURE.md`](../ARCHITECTURE.md).

---

## 1. Local development (single binary, 5 minutes)

```bash
git clone https://github.com/LordGnomish/cave-runtime && cd cave-runtime
rustup default 1.85                                   # toolchain
cargo build --workspace --release
CAVE_JWT_SECRET=dev ./target/release/cave-runtime &   # portal on :8080
./target/release/cave --help                          # cavectl
./target/release/cave etcd put /hello world           # cave-native verb
./target/release/cave etcdctl get /hello              # upstream-parity verb
curl -s http://localhost:8080/api/portal/parity | jq  # live parity index
curl -s http://localhost:8080/healthz                 # liveness
kill %1
```

That is the whole loop: clone, build, run, exercise, stop. Everything
else on this page is detail on top of those ten lines.

## 2. Prerequisites

| Need | Why |
|------|-----|
| Rust **1.85+** | Pinned in `rust-toolchain.toml`. `rustup default 1.85`. |
| `protoc` | Code generation for a handful of crates. `brew install protobuf` / `apt install protobuf-compiler`. |
| Docker (optional) | Integration tests that spin up real upstreams. |
| Ollama (optional) | `cave-hermes` / local-LLM workflows. |
| Linux 7.1+ kernel (for full eBPF / io_uring path) | `cave-net` and `cave-cri` use modern syscalls; macOS dev is fine, production is Linux. |

## 3. Configuration

The minimum-viable env is a single secret:

```bash
export CAVE_JWT_SECRET=$(openssl rand -hex 32)   # required
export CAVE_DATA_DIR=/var/lib/cave-runtime       # default: ./data
export RUST_LOG=info,cave_runtime=debug          # log level
```

A complete reference lives at
[`docs/runbook/CONFIG.md`](runbook/CONFIG.md) (when present) and
inline in `cave-runtime --help`. There are no required external SaaS
dependencies — Cave Runtime is sovereign by construction.

## 4. Test cluster (single node, local dev)

The simplest cluster is one binary serving every module:

```bash
CAVE_JWT_SECRET=dev cargo run -p cave-runtime
# → http://localhost:8080            portal
# → http://localhost:8080/api/...    API surface
# → http://localhost:8080/metrics    Prometheus metrics (cave-metrics)
```

Smoke-test the control plane:

```bash
cave etcd put /demo/foo bar
cave etcd watch /demo --from-rev=0 &
cave apiserver get pods -A
cave scheduler dump-state
```

## 5. Multi-node cluster (3-node, Raft quorum)

Each node runs the same single binary. Raft consensus is provided by
`cave-kernel` and shared across every stateful module.

```bash
# Node 1 (seed)
cave cluster init --node-id=n1 --listen=10.0.0.11 \
                  --advertise=10.0.0.11 --raft-port=2380

# Node 2
cave cluster join --node-id=n2 --listen=10.0.0.12 \
                  --advertise=10.0.0.12 --seed=10.0.0.11:2380

# Node 3
cave cluster join --node-id=n3 --listen=10.0.0.13 \
                  --advertise=10.0.0.13 --seed=10.0.0.11:2380

# Verify
cave cluster status         # should show 3 voters, 1 leader, 2 followers
cave kernel raft snapshot   # cross-module consistent snapshot
```

Three-replica minimum is enforced for every critical component
(per Charter §7: HA). The single-node mode is for development only.

## 6. Sovereign production deployment (outline)

Cave Runtime is designed to run on **your own** sovereign infrastructure
— bare metal, on-prem hypervisor, sovereign cloud — without depending
on any external SaaS in the critical path.

A production deployment typically uses:

1. **3 or 5 control-plane nodes** running `cave-runtime` with
   `--profile=control` (Raft quorum + apiserver + scheduler + etcd
   primitives + portal).
2. **N data-plane nodes** running `cave-runtime` with `--profile=worker`
   (kubelet + cri + net + mesh dataplane).
3. **systemd** unit on Linux (see [`deploy/systemd/`](../deploy/) for
   templates) or **launchd** on macOS dev (see [`scripts/`](../scripts/)).
4. **Storage:** local NVMe per node — Raft handles replication via
   `cave-kernel`. No shared storage required for the control plane.
5. **Network:** plain TCP/IP between nodes; mesh dataplane is `cave-mesh`
   Ambient (no sidecar). Identity is SPIFFE via `cave-auth`.
6. **TLS:** managed by `cave-certs` (ACME) and `cave-vault` (issuing CA).
   PQC-hybrid handshake by default per
   [ADR-GOLDEN-003](adr/ADR-GOLDEN-003-no-backcompat-pqc.md).
7. **Backup:** `cave-backup` runs scheduled snapshots; restore is
   `cave backup restore --to=<timestamp>`.
8. **Observability:** built-in `cave-metrics` (Prometheus parity),
   `cave-logs` (Loki parity), `cave-dashboard` (Grafana parity),
   `cave-oncall` (paging). All four reachable from the portal.

A worked deployment runbook with disk layout, kernel sysctl, and
systemd units lives under [`docs/runbook/`](runbook/) as we publish
them. The single binary is intentional: ship Cave Runtime to a fleet
the same way you ship any other Rust binary — `cargo build --release`,
`scp`, `systemctl restart`. No Helm chart required for the platform
itself (Helm is available for workloads via `cave-registry`).

## 7. Where to go next

- **Architecture deep-dive:** [`ARCHITECTURE.md`](../ARCHITECTURE.md),
  [`ARCHITECTURE-ELASTIC-SCALE.md`](../ARCHITECTURE-ELASTIC-SCALE.md).
- **Charter & golden rules:**
  [`docs/adr/ADR-CHARTER-001.md`](adr/ADR-CHARTER-001.md),
  [`docs/adr/ADR-GOLDEN-001-upstream-parity.md`](adr/ADR-GOLDEN-001-upstream-parity.md).
- **Per-crate parity status:**
  [`docs/parity/parity-index.json`](parity/parity-index.json)
  (or live at `/admin/parity` on a running portal).
- **Contributing:** [`CONTRIBUTING.md`](../CONTRIBUTING.md).
- **Security:** [`SECURITY.md`](../SECURITY.md).
