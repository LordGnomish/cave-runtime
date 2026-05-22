# Cave Runtime

[![License: AGPL v3+](https://img.shields.io/badge/License-AGPL_v3+-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)
[![Rust: 1.85+](https://img.shields.io/badge/Rust-1.85+-orange.svg)](https://www.rust-lang.org/)
[![Charter: v2 8-gate](https://img.shields.io/badge/Charter-v2_8--gate-green.svg)](docs/adr/ADR-CHARTER-001.md)
[![Honest fill-ratio](https://img.shields.io/badge/parity-honest_measured-brightgreen.svg)](docs/parity/parity-index.json)

**Sovereign, self-healing, self-improving Cloud OS. Multi-tenant by
construction. Line-by-line TDD reimplementation of best-of-breed upstream
projects in Rust — under one unified runtime, under one license.**

> Status: pre-v1. OSS launch: **21 May 2026**.
> Mission: [docs/adr/ADR-CHARTER-001.md](docs/adr/ADR-CHARTER-001.md).
> Full decision record: [docs/adr/](docs/adr/).

---

## What this is

Cave Runtime is a single Rust binary that reimplements the Kubernetes
control plane and the surrounding cloud-native ecosystem — etcd,
kube-apiserver, kubelet, kube-scheduler, containerd, Cilium CNI, Istio,
Kong, Keycloak, OpenBao, Harbor, ArgoCD, Backstage, Prometheus, Loki,
Grafana, Tempo, KEDA, Karpenter, Kamaji, Knative, and ~80 more — into a
single workspace of 100+ Rust crates sharing primitives (Raft consensus,
WAL, EventBus, Labels, SPIFFE identity, MVCC store) from `cave-kernel`.

Because every upstream is reimplemented under one roof:

- **Duplicated concerns** (state replication, identity, rate-limiting,
  backoff, watch, lease, MVCC) live once in `cave-kernel` and every module
  uses the same battle-tested implementation.
- **Cross-module verbs** that no single upstream CLI can express
  (federated watch, consistent cross-module snapshot, tenant-scoped policy,
  one-shot drain across mesh + gateway + apiserver) ship as first-class
  capabilities.
- **A single upgrade path** rolls the whole platform forward with
  zero user-visible downtime.

**This is not a thin wrapper over existing projects.** It is a from-scratch
Rust implementation that passes the upstream's own test vectors. See
[ADR-GOLDEN-001](docs/adr/ADR-GOLDEN-001-upstream-parity.md) for the rule.

## Charter (v2, 8 gates per crate)

Every crate in this repo must pass — and continue to pass — eight gates:

1. **TDD-strict** — RED → GREEN → REFACTOR; failing upstream test ported
   before any implementation lands.
2. **SPDX coverage** — 100% of `.rs` files carry the `AGPL-3.0-or-later`
   SPDX header.
3. **Source pinned** — `parity.manifest.toml` records `source_sha` and
   `upstream_version` against a real upstream commit/tag.
4. **No stubs** — `todo!()`, `unimplemented!()`, silent `Ok(())` are
   forbidden. Either ship it or write a `[[scope_cuts]]` entry justifying
   the deferral.
5. **No backwards-compat shims** — Cave is Linux 7.1+ and PQC-ready;
   no legacy kernel branches, no classical-only crypto paths.
6. **Always-latest upstream** — pin to the most recent stable release,
   re-pin per `last_audit` cycle.
7. **4-track ship** — every user-visible capability lands in all four
   tracks in the same PR: **backend + portal + cavectl + observability**.
8. **Honest measured `fill_ratio`** — see below.

See [docs/adr/](docs/adr/) for the full ADR record. The Charter is
machine-enforced via `tests/parity_self_audit.rs` (9 assertions per crate)
and `PARITY_REPORT.md`.

## Honest measured `fill_ratio`

We refuse to report self-graded percentages. Every crate's `parity.manifest.toml`
carries a `fill_ratio` derived from a literal subsystem inventory of the
named upstream version — `mapped + partial + skipped` (with `scope_cuts`
justification) over total entities. The aggregate is recomputed by
[`scripts/build-parity-index.py`](scripts/build-parity-index.py) into
[`docs/parity/parity-index.json`](docs/parity/parity-index.json) and surfaced
live in the portal at `/admin/parity`.

> If a crate claims 0.95, that means 95% of the upstream's named subsystems
> are either mapped (real code + tests) or declared skipped with rationale
> — not 95% of an abstract "feature list". Look at the manifest, look at
> the report, look at the tests. No self-graded numbers.

## Project layout

```
cave-runtime/
├── crates/                     100+ crates, one workspace
│   ├── cave-kernel/            shared primitives (Raft, WAL, EventBus, Labels, watch, MVCC)
│   ├── cave-core/              common types, error, hardened I/O
│   ├── cave-auth/              Keycloak parity (OIDC + RBAC + ABAC + SPIFFE)
│   ├── cave-apiserver/         kube-apiserver parity
│   ├── cave-etcd/              etcd v3 parity
│   ├── cave-scheduler/         kube-scheduler parity
│   ├── cave-kubelet/           kubelet parity
│   ├── cave-cri/               containerd + runc/crun parity
│   ├── cave-net/               Cilium CNI parity, eBPF dataplane
│   ├── cave-mesh/              Istio Ambient parity
│   ├── cave-gateway/           Kong parity, xDS, plugins
│   ├── cave-vault/             OpenBao parity (KV, PKI, transit, SSH, TOTP)
│   ├── cave-registry/          Harbor parity, OCI image + Helm
│   ├── cave-metrics/           Prometheus parity
│   ├── cave-logs/              Loki parity
│   ├── cave-dashboard/         Grafana parity
│   ├── cave-trace/             Tempo / OTel parity
│   ├── cave-oncall/            Grafana OnCall parity
│   ├── cave-streams/           Kafka + Pulsar parity (consolidated)
│   ├── cave-rdbms/             PostgreSQL wire-protocol parity
│   ├── cave-docdb/             MongoDB wire-protocol parity (FerretDB v2)
│   ├── cave-cache/             Valkey 8 (Redis 7.2 compat) parity
│   ├── cave-keda/              KEDA parity (event-driven autoscaling)
│   ├── cave-karpenter/         Karpenter parity (node lifecycle)
│   ├── cave-kamaji/            Kamaji parity (managed control planes)
│   ├── cave-knative/           Knative Serving + Eventing parity
│   ├── cave-flags/             Unleash parity (feature flags)
│   ├── cave-scan/              SonarQube + SAST parity
│   ├── cave-gitleaks/          Gitleaks parity
│   ├── cave-dast/              ZAP parity (dynamic app security)
│   ├── cave-sbom/              Dependency-Track parity
│   ├── cave-vulns/             DefectDojo parity
│   ├── cave-hermes/            Hermes-agent parity (local LLM gateway)
│   ├── cave-portal/            unified admin portal (two-persona, WCAG AA)
│   ├── cave-cli/               cavectl — upstream-parity + cave-native verbs
│   └── ...                     full list: Cargo.toml `[workspace.members]`
├── docs/adr/                   architectural decisions (context + rationale)
├── docs/parity/                live parity-index.json + per-crate reports
├── docs/runbook/               operations runbooks
├── docs/synergy/               refactor sweep progress
└── deploy/                     launchd plists, systemd units, Helm charts
```

Full per-crate matrix: [`docs/parity/parity-index.json`](docs/parity/parity-index.json)
(112 entries, regenerated daily by [`scripts/build-parity-index.py`](scripts/build-parity-index.py)).
Per-crate detail: [`crates/*/PARITY_REPORT.md`](crates/).
Architecture deep-dive: [ARCHITECTURE.md](ARCHITECTURE.md) and
[ARCHITECTURE-ELASTIC-SCALE.md](ARCHITECTURE-ELASTIC-SCALE.md).

## Quickstart

```bash
git clone https://github.com/LordGnomish/cave-runtime && cd cave-runtime
cargo build --workspace --release
cargo test  --workspace
CAVE_JWT_SECRET=dev ./target/release/cave-runtime    # portal at http://localhost:8080
./target/release/cave --help                          # cavectl
```

Prerequisites: Rust 1.85+, protoc, Docker (for integration tests),
Ollama (optional, for `cave-hermes` / local-LLM workflows).

For a 10-line deployment recipe (single-node dev → sovereign multi-node
production) see [docs/quickstart.md](docs/quickstart.md).

## The Cave ecosystem

Cave Runtime is the foundation layer. The full sovereign stack:

| Layer | Project | Role |
|-------|---------|------|
| Foundation | **cave-runtime** (this repo) | Sovereign Cloud OS — control plane, data plane, observability |
| Home | **cave-home** | Sovereign personal-cloud distribution built on cave-runtime |
| Build | **MuleForge** | Build/CI mule — pre-cached toolchains for cave-runtime contributors |
| Platform | **Pipeline Platform** | Opinionated SaaS workflow on top of cave-runtime |

Each component is independently licensed and independently usable;
they compose top-to-bottom without lock-in.

## Contributing

We welcome contributions. The bar is higher than most repos, but the
rules are simple and repeatable. See [CONTRIBUTING.md](CONTRIBUTING.md)
for the full workflow.

Short version:

1. Read [ADR-CHARTER-001](docs/adr/ADR-CHARTER-001.md), [ADR-GOLDEN-001](docs/adr/ADR-GOLDEN-001-upstream-parity.md),
   and [ADR-GOLDEN-003](docs/adr/ADR-GOLDEN-003-no-backcompat-pqc.md).
2. Port a failing upstream test first; then implement until it passes.
3. Update `parity.manifest.toml` (with `source_sha`, `last_audit`,
   `parity_ratio_source = "manifest"`).
4. All four tracks (backend + portal + cavectl + observability) ship in
   the same PR.
5. PR template will walk you through the 8-gate checklist.

First-timers: look for issues labelled `good-first-issue` and
`parity-gap`. The `parity-gap` template helps you scope a single upstream
subsystem into a single PR.

Code of Conduct: [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
Security: [SECURITY.md](SECURITY.md).

## License

**GNU Affero General Public License v3 or later** (AGPL-3.0-or-later) —
see [LICENSE](LICENSE), [NOTICE](NOTICE), and [CREDITS.md](CREDITS.md)
for upstream attribution and the per-crate license inventory.

We chose AGPL-3.0-or-later deliberately to **prevent hyperscalers and
other large SaaS vendors from re-selling Cave Runtime as a closed managed
service without contributing back**. AGPL §13 ("Remote Network
Interaction") closes the SaaS loophole that Apache-2.0 leaves open:
anyone who runs a modified Cave Runtime as a network-accessible service
must make the corresponding source code available to every user of that
service, on the same terms.

Operators self-hosting Cave Runtime — including commercial operators
serving paying customers — keep all the freedoms the license grants.
The only obligation is that modifications surface to their users on the
same terms.

> Sovereign-by-default. *insanlığa hizmet* — service to humanity.

By submitting a pull request you agree to license your contribution
under AGPL-3.0-or-later. We do not require a Contributor License
Agreement.

## Status

Pre-v1. Live module status: `/admin/parity` on a running portal, sourced
from `parity.manifest.toml` + `tests/parity_self_audit.rs` + test counts.
Snapshot: [docs/parity/parity-index.json](docs/parity/parity-index.json).
