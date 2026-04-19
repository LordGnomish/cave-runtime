# ADR-014: Zero-Trust Network Architecture

**Status:** Accepted

**Scope:** Runtime, Universal

**Category:** Security

**Related ADRs:** 004, 027, 084, 121, 122

## Context

**Minimum Kernel Requirement: Linux 7.0** (released 12 April 2026). CAVE does not support older kernels. Kernel 7.0 provides: official Rust kernel module support, improved eBPF BTF type lookups, enhanced io_uring performance, and the latest eBPF verifier improvements. Talos Linux and Ubuntu 26.04 LTS both ship with kernel 7.0.

CAVE is a multi-tenant platform where workloads from different tenants share cluster infrastructure. Traditional perimeter-based security (firewall at edge, trust inside) is insufficient — any compromised pod could reach any other pod. Zero-trust requires verifying every connection, regardless of source.

## Candidates

| Criteria | Cilium + Istio ambient (chosen) | Cilium only | Istio sidecar only | Calico + Linkerd |
|---|---|---|---|---|
| L3/L4 enforcement | ✅ Cilium eBPF | ✅ Cilium eBPF | ❌ iptables (Istio) | ✅ Calico |
| L7 mTLS (auto) | ✅ Istio ambient ztunnel | ❌ No mTLS | ✅ Istio sidecar | ✅ Linkerd |
| Cryptographic identity | ✅ SPIFFE via Istio | ❌ | ✅ SPIFFE | ✅ |
| Sidecar overhead | ✅ None (ztunnel is per-node, not per-pod) | ✅ None | ❌ Per-pod sidecar (~50MB each) | ❌ Per-pod |
| Network policy | ✅ CiliumNetworkPolicy (L3-L7) | ✅ | ⚠️ AuthorizationPolicy | ✅ Calico NP |
| eBPF observability | ✅ Hubble | ✅ Hubble | ❌ | ❌ |

## Decision

**Cilium** for L3/L4 network enforcement (default-deny, eBPF-based, FQDN egress). **Istio ambient** for L7 mTLS and cryptographic service identity (SPIFFE). Clear boundary: Cilium = network fabric, Istio = service identity and L7 policy. No overlap. *Full evaluation in ADR-004.*

## Rejected Options

### Cilium Only (no mesh) — Rejected

**Primary:** No mTLS between services. Without a mesh, east-west traffic inside the cluster is plaintext. Any compromised pod can sniff traffic to every other pod on the same node. This violates NIST SP 800-207 principle #1 (encrypt all traffic regardless of network location) and SOC2 CC6.6 (encryption in transit).

**Secondary:** No cryptographic workload identity. Cilium identifies pods by IP/label — both are spoofable. SPIFFE SVIDs (via Istio) provide cryptographic proof of workload identity that cannot be forged without compromising the SPIRE CA.

### Istio Sidecar Mode — Rejected

**Primary:** Resource overhead. Envoy sidecar adds ~50-100MB RAM per pod. On a 500-pod cluster = 25-50GB overhead. On Hetzner dev profiles (CX42 = 16GB total), sidecar overhead would consume 50%+ of available memory. Ambient ztunnel runs per-node (~50MB/node × 8 nodes = 400MB total) — 60x more efficient.

**Secondary:** Operational complexity. Sidecar injection, lifecycle management (sidecar must restart when app restarts), sidecar version skew, sidecar-to-sidecar debugging. All eliminated by ambient mode. Ambient ztunnel handles L4 mTLS transparently at node level — zero per-pod configuration.

### Calico + Linkerd — Rejected

**Primary:** No eBPF observability. Calico's eBPF dataplane is available but Hubble (Cilium's L3/L4/L7 flow visibility) has no equivalent in Calico OSS. Calico Enterprise has flow viz but is proprietary/paid. CAVE's forensics (ADR-090) depends on Hubble Prometheus metrics.

**Secondary:** Linkerd has smaller community than Istio. Argo Rollouts integration via SMI TrafficSplit is less mature than Istio's native VirtualService canary. Linkerd still requires per-pod proxy (~20-30MB) — lighter than Envoy but not zero like ambient ztunnel.

## Resource Impact (8-node prod cluster, 500 pods)

| Component | Per-Node | Per-Pod | Total Cluster |
|---|---|---|---|
| Cilium agent | ~150MB | 0 | ~1.2GB |
| Istio ztunnel (ambient) | ~50MB | 0 | ~400MB |
| Istio Waypoint (opt-in L7) | ~100MB per namespace | 0 | ~500MB (5 namespaces) |
| **Total zero-trust overhead** | **~200MB/node** | **0** | **~2.1GB** |

Compare: Istio sidecar mode would cost ~50GB for same cluster (100MB × 500 pods).

## Zero-Trust Maturity Model (NIST SP 800-207 alignment)

| Level | Description | CAVE Status |
|---|---|---|
| **Traditional** | Perimeter-based firewall, trust inside network | ❌ Not acceptable |
| **Advanced** | Micro-segmentation + identity-aware access | ✅ Phase 1 (Cilium default-deny + Istio mTLS) |
| **Optimal** | Continuous verification, risk-based access, full encryption | 🎯 Phase 3 target (add behavioral analysis, adaptive policy) |

CAVE starts at **Advanced** level from day one. Phase 3 targets **Optimal** with:
- Tetragon behavioral baseline per workload (ADR-016) — anomaly detection
- OPAL real-time policy updates (ADR-131) — adaptive authorization
- Sovereign Ledger attestation of every access decision (ADR-093)

## Consequences

**Positive:**
- True zero-trust: every service-to-service connection is mTLS-encrypted with SPIFFE identity verification.
- No sidecar overhead (ambient ztunnel is per-node DaemonSet, shared across all pods on node).
- Cilium eBPF provides kernel-level network enforcement + flow observability (Hubble).
- Clear L3/L4 (Cilium) vs L7 (Istio) boundary — no feature overlap, no conflict.

**Negative:**
- Two networking components (Cilium + Istio) instead of one — higher complexity.
- Istio ambient is newer than sidecar mode — community maturity is catching up but not yet equivalent.
- eBPF kernel compatibility requirements (compatibility matrix triple — ADR-133).
- Debugging network issues requires understanding both Cilium and Istio layers.

### Risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Cilium + Istio interaction bug (L4 allows, L7 denies or vice versa) | Low | High | Cilium authoritative for L3/L4. Istio ztunnel for mTLS only. Document conflict resolution in Runbook. Staging validates policy changes. |
| eBPF kernel compatibility on AKS nodes | Medium | Medium | Pin AKS node image to tested kernel version. Compatibility matrix (ADR-133) tracks Cilium↔kernel↔Istio triple. Test on every AKS upgrade. |
| Istio ambient ztunnel crash (node-level) | Low | High | ztunnel is DaemonSet — K8s auto-restarts. During restart, L4 traffic continues (Cilium), L7 mTLS briefly unavailable. Pod readiness probes detect mTLS loss. |
| SPIFFE identity spoofing | Very Low | Critical | SPIFFE identities issued by Istio CA (not self-issued). CA certificate rotation automated (cert-manager, ADR-015). Mutual verification — both sides must present valid SVID. |
| Zero-trust enforcement blocks legitimate traffic during rollout | Medium | Medium | Gradual rollout: observe-only mode first (Cilium Hubble flow logs), then warn-only (log violations), then enforce. Per-namespace rollout, not cluster-wide. |

## Compliance Mapping

SOC2 CC6.1 (network segmentation). SOC2 CC6.6 (encryption in transit — mTLS). ISO A.8.22 (segregation in networks). ISO A.8.24 (cryptographic controls — mTLS). NIS2 Art.21 (network security). GDPR Art.32 (security of processing — encryption in transit).

**Absorbed Decisions:** The following tool-level decisions are absorbed into this ADR for traceability

**ADR-082**

Platform PKI + mTLS (Control-Plane)

**Decision:** Platform PKI with internal CA for control-plane mTLS. Istio ambient for data-plane mTLS. Two independent mTLS layers for defense-in-depth. Zero plaintext control or data traffic.
