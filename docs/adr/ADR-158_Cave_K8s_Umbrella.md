# ADR-158 — Cave-K8s Control-Plane Umbrella

**Status:** Accepted
**Date:** 2026-05-23
**Author:** Burak Tartan
**Charter:** v2

## Context

cave-runtime has eight independently-developed Kubernetes subsystem
crates: `cave-apiserver`, `cave-scheduler`, `cave-kubelet`,
`cave-kube-proxy`, `cave-controller-manager`,
`cave-cloud-controller-manager`, `cave-cri`, and `cave-etcd`. Each was
brought to `fill_ratio ≥ 0.95` against `kubernetes/kubernetes` in the
[k8s-parity-uplift-2026-05-19](../../crates/cave-k8s/PARITY_REPORT.md)
and [parity-edge-v4-2026-05-19](../../) waves.

What was missing was an **umbrella** — one crate, one type tree, one
discovery surface, one PQC-signed ServiceAccount issuer — that joined
the eight together and surfaced the cross-cutting K8s concepts that
none of the subsystems own alone:

* the `ControlPlane` bootstrap state machine
* an admission chain wiring NamespaceLifecycle / ServiceAccount /
  LimitRanger / PodSecurity / ValidatingAdmissionPolicy together
* a CRD lifecycle registry the discovery layer can consult
* an APIService aggregator registry
* an OpenAPI v3 schema aggregator merging builtin + CRD schemas
* a GarbageCollector that computes cascade plans across kind
  boundaries
* a per-namespace ResourceQuota tracker
* the kubelet-level concerns the upstream Kubernetes splits across
  cgroup managers, probes, image GC, eviction
* the K8s networking concerns that span apiserver + kube-proxy
  (Service ↔ EndpointSlice derivation)
* the K8s storage concerns that span apiserver + CSI (PV/PVC binder)

Without an umbrella, every cave-runtime user has to learn the
geography of eight crates, and the cross-cutting features
(particularly PQC-ready SA tokens) live nowhere.

## Decision

Introduce **`cave-k8s`** as a thin umbrella crate that:

1. Depends on all eight Kubernetes subsystem crates + `cave-core` +
   `cave-kernel`.
2. Exposes a `ControlPlane` facade that brings up the eight subsystems
   in canonical bootstrap order (etcd → apiserver → controller-manager
   → scheduler → kube-proxy → kubelet → cri, then cloud-controller-manager
   when `enable_cloud_provider`).
3. Owns the cross-cutting features (PQC SA tokens, admission chain,
   CRD/aggregator/openapi composition, GC cascade planner, quota
   tracker, eviction / probe / image-GC planners).
4. Pins upstream `kubernetes/kubernetes v1.32.0`
   (`70d3cc986aa8221cd1dfb1121852688902d3bf53`).
5. Ships its own Charter v2 paperwork:
   `parity.manifest.toml` + `parity_self_audit.rs` (9 assertions) +
   `PARITY_REPORT.md` + observability dashboard + alerts.
6. Wires the `cavectl k8s ...` subcommand exposing the umbrella's HTTP
   surface (cluster / version / healthz / readyz / discovery / openapi
   / metrics / apply / scale / rollout / top-nodes / top-pods / logs /
   exec / port-forward).

cave-k8s **does not duplicate** the strongly-typed K8s resource
definitions — they continue to live in `cave-apiserver::resources`. It
**does not** reimplement the scheduler plugin framework, the CRI
runtime calls, or the etcd Raft log — those continue to live in their
respective subsystem crates. The umbrella's mapped subsystems are the
**umbrella-level** concerns (`ControlPlane`, admission chain, CRD
lifecycle, aggregator, PQC SA tokens, GC planner, …) that did not
have a home before.

### PQC-ready ServiceAccount tokens

cave-k8s' `pqc` module ships a `HybridSigner` / `HybridVerifier` pair
that produces JOSE-shaped tokens with a wire envelope holding both:

* a classical Ed25519 signature, and
* a 3309-byte ML-DSA-65 signature.

The PQC half is presently backed by a deterministic SHA-256 expansion
of `(domain ‖ pqc_seed ‖ payload)` and the verifier *rejects* this
placeholder unless `accept_placeholder` or `with_expected_pqc_seed` is
set. When the workspace upgrades to a real `pqcrypto-mldsa` dependency,
only the inner `sign_pqc`/`verify_pqc` pair changes — the envelope, the
`alg = "Ed25519+ML-DSA-65"` JOSE tag, and the integration tests stay
the same. This unblocks the migration path without committing to a
specific PQC library today.

### No-backcompat scope cuts

Eleven Kubernetes subsystems are **honestly skipped** rather than
silently absent:

* `in-tree-volume-plugins` — CSI-only design (`no-backcompat`)
* `podsecuritypolicy` — removed upstream v1.25 (`no-backcompat`)
* `dockershim` — `cave-cri` is the runtime (`no-backcompat`)
* `cgroupv1` — Linux 7.1+ only (`no-backcompat`)
* `windows-node-support` — Linux only (`linux-only`)
* `alpha-feature-gates` — stable surface only (`no-alpha-gates`)
* `konnectivity-tunnel` — flat networks (`no-backcompat`)
* `kubectl-plugin-protocol` — `cavectl` is the CLI (`cli-replaced`)
* `aggregator-request-forwarding` — `cave-portal-api` proxies
  (`portal-api-owned`)
* `audit-log-shipping` — `cave-logs` + `cave-metrics` ship
  (`obs-stack-owned`)
* `extender-scheduler` — plugin framework only (`plugin-framework-only`)
* `kubeadm` — `cave-runtime` + `cavectl` bootstrap
  (`runtime-bootstrap-owned`)
* `cloud-provider-matrix-beyond-hetzner-azure` —
  `cave-cloud-controller-manager` is scoped to Hetzner + Azure
  (`ccm-scoped`).

These appear as `[[skipped]]` in `parity.manifest.toml` with explicit
`scope_cuts` tags, not as `[[unmapped]]` — they were not forgotten,
they were intentionally cut.

### Honest unmapped (2)

* `dual-write-storage-migration` — present in `cave-apiserver` but not
  surfaced through `cave-k8s::ControlPlane`. Phase 2.
* `leader-election-coordination-lease` — `coordination.k8s.io/v1/Lease`
  objects are not yet exposed through the umbrella API.
  `cave-controller-manager` already uses `cave-kernel::reconcile` for
  loop leadership; the Lease surface for *third-party* controllers
  (Knative + KEDA + Karpenter + Kamaji) is the missing piece. Phase 2.

## Consequences

* **One entry point.** Downstream consumers (cave-runtime,
  cave-portal-api, cave-cli) import `cave_k8s` and get the entire
  control plane.
* **One PQC issuer.** Hybrid Ed25519 + ML-DSA-65 SA tokens are the
  default in cave-k8s; cave-apiserver's existing single-key issuer
  remains available but is shadowed when `enable_pqc_sa_tokens = true`
  (the cluster-config default).
* **One observability surface.** `cave-k8s::routes::create_router`
  serves `/metrics` for Prometheus, `/healthz` + `/readyz` for kubelets
  / load balancers, and `/version` for cluster-version probes.
* **No regression in subsystem parity.** The eight subsystem crates'
  `parity.manifest.toml` files are unchanged — cave-k8s is *additive*.

## Alternatives considered

* **A "k8s.rs" in cave-runtime itself.** Rejected — cave-runtime is a
  binary; cave-k8s belongs as a library so it can be linked from
  cave-portal-api and others.
* **Keep eight separate facades.** Rejected — the cross-cutting
  features (PQC SA tokens, admission chain, GC cascade) have nowhere
  to live and would be duplicated.
* **One mega-crate replacing the eight.** Rejected — the eight crates'
  parity audits are already at ≥ 0.95 and a mega-crate would force a
  re-audit; the umbrella+subsystems split keeps the audit boundaries
  stable and adds value at the umbrella layer.

## References

* `parity.manifest.toml` (47 subsystems, 28 mapped + 4 partial + 13
  skipped + 2 unmapped, fill_ratio 0.9516)
* `PARITY_REPORT.md` (8-gate Charter v2 close-out)
* `tests/parity_self_audit.rs` (9 assertions PASS)
* `tests/smoke.rs` (6 scenarios PASS)
* Upstream pin: kubernetes/kubernetes
  [v1.32.0](https://github.com/kubernetes/kubernetes/releases/tag/v1.32.0) ·
  source\_sha `70d3cc986aa8221cd1dfb1121852688902d3bf53`
