# ADR-014: Zero-Trust Network Architecture

**Status:** Accepted

**Scope:** Runtime, Universal

**Category:** Security

**Related ADRs:** 004, 027, 084, 121, 122

## Context

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

## Rejected

- **Cilium only (no mesh):** No mTLS between services. No cryptographic identity. Insufficient for zero-trust (traffic in cluster is plaintext).
- **Istio sidecar mode:** Per-pod sidecar adds ~50MB RAM per pod. At 500+ pods = 25GB overhead. Ambient mode eliminates this with per-node ztunnel.
- **Calico + Linkerd:** Calico lacks eBPF observability (Hubble). Linkerd is lighter but smaller community and less enterprise adoption than Istio.

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
