# ADR-004: Cilium CNI + Istio Ambient Mesh

**Status:** Accepted

**Scope:** Universal

**Category:** Infrastructure — Networking

**Related ADRs:** 027 (Kong), 031 (Tetragon), 068 (Istio Ambient Mandatory), 084 (Default-Deny), 110 (Egress Governance), 121 (Ambient Multi-Cluster), 122 (Cilium Gateway API)

## Context

CAVE needs a CNI plugin and service mesh for Kubernetes networking across all profiles. Requirements:

- Network policy enforcement (default-deny per tenant per environment, ADR-084)
- mTLS between all services (zero-trust east-west traffic)
- L3/L4/L7 observability (flow visibility for debugging and forensics)
- eBPF-based enforcement (kernel-level, bypass iptables overhead)
- Multi-tenant network isolation (cross-tenant traffic blocked)
- Minimal resource overhead (sidecar-less preferred for cost on dev profiles)
- Egress governance (per-tenant egress quotas, eBPF byte counters, ADR-110)
- Compatible with Talos Linux (ADR-003) and Kong ingress (ADR-027)

---

## Candidates

### 3.1 CNI Comparison

| Criteria | Cilium | Calico | Flannel | AWS VPC CNI | Azure CNI |
|---|---|---|---|---|---|
| **Technology** | eBPF (kernel-level) | iptables (legacy) or eBPF (newer) | VXLAN overlay, iptables | AWS native VPC | Azure native VNET |
| **Network policy** | ✅ CiliumNetworkPolicy (L3/L4/L7, DNS-aware, FQDN-based) + K8s NetworkPolicy | ✅ Calico NetworkPolicy + K8s NetworkPolicy | ❌ No network policy support | ⚠️ K8s NetworkPolicy only (via Calico add-on) | ⚠️ K8s NetworkPolicy only |
| **eBPF** | ✅ Native. eBPF is core architecture. | ⚠️ eBPF dataplane available but not default. Legacy iptables still common. | ❌ No eBPF | ❌ No eBPF | ❌ No eBPF |
| **Hubble (observability)** | ✅ Built-in. L3/L4/L7 flow visibility. Hubble UI. Prometheus metrics. | ❌ Separate tool needed (Calico Enterprise has flow viz) | ❌ No observability | ❌ Separate tools | ❌ Separate tools |
| **Egress gateway** | ✅ CiliumEgressGateway. Per-tenant egress policy. eBPF byte counters for FinOps (ADR-110). | ⚠️ Calico Enterprise (paid) has egress gateway. | ❌ | ❌ NAT Gateway (AWS-specific) | ❌ Azure NAT Gateway |
| **Bandwidth management** | ✅ eBPF-based EDT (Earliest Departure Time) | ❌ | ❌ | ❌ | ❌ |
| **Encryption** | ✅ WireGuard or IPsec (node-to-node) | ✅ WireGuard | ❌ | ❌ (VPC encryption) | ❌ (VNET encryption) |
| **Service mesh** | ⚠️ Cilium Service Mesh exists but less mature than Istio | ❌ Separate mesh needed | ❌ | ❌ | ❌ |
| **Gateway API** | ✅ Full K8s Gateway API support (ADR-122: reserved for future internal routing) | ⚠️ Limited | ❌ | ❌ | ❌ |
| **Performance** | ✅ eBPF bypasses iptables. Near-native kernel performance. | ⚠️ iptables mode has overhead at scale (10K+ rules). eBPF mode improving. | ⚠️ iptables overhead | ✅ Native VPC (no overlay) | ✅ Native VNET |
| **Talos compatible** | ✅ Talos ships without default CNI — Cilium plugs in cleanly | ✅ Works on Talos | ✅ Works on Talos | ❌ AWS-only | ❌ Azure-only |
| **License** | Apache 2.0 | Apache 2.0 (OSS) / Proprietary (Enterprise) | Apache 2.0 | AWS terms | Azure terms |
| **Community** | Very active. CNCF Graduated. ~20K GitHub stars. Isovalent (Cisco acquired 2024). | Active. Tigera backed. ~6K stars. | Active but minimal features. ~9K stars. | AWS-maintained | Azure-maintained |

### 3.2 Service Mesh Comparison

| Criteria | Istio Ambient | Istio Sidecar | Linkerd | Cilium Service Mesh | No Mesh |
|---|---|---|---|---|---|
| **Architecture** | ztunnel (per-node L4) + Waypoint (opt-in L7) | Envoy sidecar per pod | Rust micro-proxy per pod | eBPF (L4) + Envoy (L7) | No mesh |
| **Resource overhead** | Low. ztunnel: ~50MB/node. Waypoint: per-namespace, opt-in. | High. Envoy sidecar: ~50-100MB per pod. 2x pod count. | Medium. ~20-50MB per pod. Less than Envoy. | Low. eBPF is kernel-level. | Zero |
| **mTLS** | ✅ Automatic via ztunnel (L4). No app changes. | ✅ Automatic via sidecar. | ✅ Automatic via proxy. | ✅ eBPF encryption (WireGuard/IPsec). | ❌ No mTLS |
| **L7 policy** | ✅ Waypoint proxies (opt-in per namespace). HTTP routing, retries, timeouts. | ✅ Full Envoy L7 (always-on per pod). | ✅ L7 routing (limited vs Envoy). | ⚠️ Envoy L7 available but less mature than Istio. | ❌ |
| **Traffic shifting (canary)** | ✅ VirtualService / Gateway API HTTPRoute | ✅ Same | ✅ TrafficSplit (SMI) | ⚠️ Via Gateway API | ❌ |
| **Multi-cluster** | ⚠️ Alpha for ambient (ADR-121 evaluates for Phase 4) | ✅ Mature multi-cluster | ✅ Multi-cluster (simpler model) | ⚠️ ClusterMesh (L3/L4 only) | ❌ |
| **CNCF status** | ✅ Graduated (same project as sidecar) | ✅ Graduated | ✅ Graduated | Part of Cilium (Graduated) | N/A |
| **Argo Rollouts integration** | ✅ Native Istio traffic management for canary | ✅ Same | ⚠️ SMI adapter needed | ⚠️ Gateway API adapter | ❌ |
| **Tetragon compatibility** | ✅ Complementary (Istio = L7 traffic, Tetragon = syscall) | ✅ Same | ✅ Same | ✅ Same project ecosystem | ✅ |
| **Maturity** | GA since Istio 1.22 (2024). Production deployments growing. | Very mature. Years of production use. | Mature. CNCF Graduated. | Newer. Evolving. | N/A |

### 3.3 Resource Impact (CAVE Prod Profile, 8 nodes)

| Solution | Per-Node Overhead | Per-Pod Overhead | Total Cluster Overhead (100 pods) |
|---|---|---|---|
| Cilium + Istio Ambient (ztunnel only) | ~150MB (Cilium agent) + ~50MB (ztunnel) = ~200MB/node | 0 (no sidecar) | ~1.6GB (8 nodes) |
| Cilium + Istio Ambient (full, with Waypoint) | ~200MB/node + ~100MB per Waypoint | 0 (no sidecar) | ~2.0GB + Waypoints |
| Cilium + Istio Sidecar | ~150MB/node | ~80MB/pod (Envoy sidecar) | ~1.2GB + 8GB (sidecars) = ~9.2GB |
| Calico + Linkerd | ~100MB/node | ~30MB/pod (Linkerd proxy) | ~0.8GB + 3GB = ~3.8GB |
| Cilium + No Mesh | ~150MB/node | 0 | ~1.2GB |

Istio Ambient saves ~7.5GB RAM vs Istio Sidecar on a 100-pod cluster. On Hetzner dev profiles (CX42 = 16GB), this is nearly 50% of total RAM.

---

## Decision

**Cilium** as CNI for L3/L4 networking, network policy, eBPF observability (Hubble), and egress governance.

**Istio Ambient** (sidecar-less) as service mesh for L4 mTLS (ztunnel) and optional L7 policy (Waypoint proxies).

**Responsibility boundary:** Cilium = network fabric + policy enforcement + observability. Istio ambient = mTLS + traffic management (canary shifting). Kong = north-south API gateway (ADR-027). No overlap.

---

## Rejected

### 4.1 Calico — Rejected (CNI)

**Primary:** No native eBPF observability. Cilium's Hubble provides L3/L4/L7 flow visibility out of the box — critical for CAVE's runtime forensics (ADR-090) and tenant network debugging. Calico's flow visualization is Enterprise-only (proprietary, paid). CAVE's observability stack (ADR-029) depends on Hubble Prometheus metrics for network dashboards.

**Secondary:** Calico's eBPF dataplane is available but not the default or most mature path. Cilium was built eBPF-first. Calico's CiliumNetworkPolicy equivalent (Calico NetworkPolicy) lacks FQDN-based egress rules — critical for tenant egress governance (ADR-110) and Safe-Exit Lists in quarantine protocol. No native egress gateway in OSS — requires Calico Enterprise.

### 4.2 Istio Sidecar — Rejected (Mesh Mode)

**Primary:** Resource overhead. Envoy sidecar per pod doubles the pod count effectively. On Hetzner dev profiles (16GB RAM), sidecar overhead would consume ~50% of available memory for a modest workload. CAVE's ~70 platform components already consume significant resources — adding sidecars to each is unsustainable on smaller profiles.

**Secondary:** Operational complexity. Sidecar injection, sidecar lifecycle management, sidecar-to-sidecar debugging, sidecar resource limits — all eliminated by ambient mode. Ambient's ztunnel handles L4 mTLS at the node level with zero per-pod overhead. L7 policy available via opt-in Waypoint proxies only where needed.

### 4.3 Linkerd — Rejected (Mesh)

**Primary:** Argo Rollouts integration. CAVE uses Argo Rollouts for canary deployments (ADR-036). Argo Rollouts has native Istio traffic management integration — VirtualService weight shifting for canary analysis. Linkerd requires SMI TrafficSplit adapter, which is less mature and has limited community adoption. Switching to Linkerd would degrade progressive delivery capability.

**Secondary:** No Waypoint-equivalent for opt-in L7. Linkerd always injects a proxy per pod (lighter than Envoy but still per-pod overhead). Istio ambient's node-level ztunnel + opt-in Waypoint is architecturally superior for cost optimization across CAVE's 7 profiles.

### 4.4 Cilium Service Mesh (without Istio) — Rejected

**Primary:** Less mature L7 capabilities. Cilium's service mesh provides eBPF-based L4 and basic L7 via Envoy, but traffic management features (canary shifting, fault injection, circuit breaking) are less mature than Istio's. Argo Rollouts integration with Cilium mesh is experimental. Istio's VirtualService/DestinationRule model is battle-tested.

**Secondary:** Cilium Gateway API reserved for future internal platform routing (ADR-122). Using Cilium for both CNI and mesh creates single-vendor risk for the entire networking stack. Separating CNI (Cilium) from mesh (Istio) provides defense-in-depth: if Cilium crashes, Istio's ztunnel failure mode is independent.

### 4.5 No Mesh — Rejected

**Primary:** No automatic mTLS. Zero-trust east-west traffic requires every service-to-service connection to be encrypted and mutually authenticated. Without a mesh, each application must implement TLS — unenforceable across 70+ components and tenant workloads. Compliance gap for SOC2 CC6.7, ISO A.8.24, GDPR Art.32 encryption requirements.

---

## L7 Responsibility Boundary

| Traffic Type | Handler | Scope |
|---|---|---|
| **North-South** (tenant API traffic) | Kong (ADR-027) | Rate limiting, JWT/OAuth2, request transform, OpenAPI validation, versioning, deprecation headers |
| **East-West** (service-to-service) | Istio ambient | mTLS, observability, traffic shifting for canary |
| **Network policy** (L3/L4) | Cilium | Default-deny, cross-tenant/cross-env blocking, egress quotas |
| **Network observability** | Cilium Hubble | Flow visibility, Prometheus metrics, network forensics |
| **Egress governance** | Cilium Egress Gateway | Per-tenant egress quotas, eBPF byte counters, quarantine |

**No overlap:** Kong never handles service-to-service. Istio never handles external API policy. Cilium handles network fabric underneath both.

**Debugging implication:** Tenant API issues → Kong logs + Prometheus. Inter-service issues → Hubble + Istio telemetry. Network policy issues → Cilium flow logs.

---

## Consequences

### Positive

- eBPF-native networking: near-kernel performance, bypasses iptables
- Hubble provides L3/L4/L7 flow visibility without additional tooling
- Istio ambient eliminates sidecar overhead (~7.5GB savings on 100-pod cluster)
- CiliumNetworkPolicy supports FQDN-based egress rules for tenant quarantine
- Clean L7 responsibility boundary: Kong (N-S), Istio (E-W), Cilium (L3/L4)
- Argo Rollouts native Istio integration for canary deployments
- Tetragon (ADR-031) is same ecosystem — Cilium + Tetragon = unified eBPF platform

### Negative

- Two L7 components (Istio + Kong) increases cognitive load vs single gateway
- Cilium + Istio ambient overlap at L4 (both can enforce L4 policy) — requires clear boundary documentation
- Istio ambient relatively new (GA 2024) — fewer production war stories than sidecar mode
- Ambient multi-cluster is alpha — Phase 4 multi-region requires evaluation (ADR-121)
- Cilium agent is critical path — agent crash causes node networking failure

### Risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Istio ambient breaking change | Low | High | Staging validates. cave-ctl upgrade check enforces Cilium→Istio upgrade order. |
| Cilium agent crash on node | Low | High | Node cordoned, pods rescheduled. Talos node rebuild (ADR-003). Section 43 degradation matrix. |
| Isovalent/Cisco changes Cilium direction | Low | Medium | Apache 2.0 license. CNCF Graduated. Community fork viable. |
| L4 policy conflict (Cilium allows, Istio denies) | Low | Medium | Cilium is authoritative for L3/L4. Istio ztunnel handles mTLS only. Document conflict resolution in Runbook Section 5. |

## Compliance Mapping

SOC2 CC6.1 (network segmentation — Cilium default-deny + Istio mTLS). SOC2 CC6.6 (encryption in transit — Istio ambient mTLS). ISO A.8.22 (segregation in networks — eBPF kernel-level enforcement). ISO A.8.24 (cryptographic controls — SPIFFE identity, mTLS). NIS2 Art.21 (network security — zero-trust architecture). GDPR Art.32 (security of processing — encrypted service-to-service communication).

**Absorbed Decisions:** The following tool-level decisions are absorbed into this ADR for traceability

**ADR-068**

Istio Ambient Mesh Mandatory

**Decision:** Istio Ambient Mesh mandatory on all profiles. Rejection: Istio sidecar (resource overhead, upgrade complexity), Linkerd (less feature-rich). Ambient = zero-pod-change mTLS + L7.
