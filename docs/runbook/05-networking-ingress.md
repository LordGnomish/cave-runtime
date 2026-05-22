# CAVE Platform Runbook §05 — Networking and Ingress

## Executive Summary

The CAVE Platform implements a **three-layer networking model** with strict separation of concerns and zero architectural overlap. Each layer is owned by a distinct technology, solves a unique problem, and operates at a different protocol boundary:

- **Kong (North-South)**: External API Gateway for tenant-facing APIs, platform service ingress, rate limiting, authentication
- **Istio Ambient (East-West)**: Sidecar-less service mesh for inter-pod mTLS, L7 HTTP policies, per-route RBAC via waypoint proxies
- **Cilium (L3/L4 Foundation)**: eBPF-based Container Network Interface, network policy enforcement, egress governance, active observability via Hubble

This three-layer design eliminates redundancy, clarifies responsibility, reduces operational complexity, and enables multi-tenant security guarantees that would be impossible with any single tool.

---

## 5.1 Overview: Why Three Layers?

### The No-Overlap Principle

A common anti-pattern in Kubernetes networking is **tool sprawl**: deploying a service mesh for mTLS, then also deploying Envoy Gateway at the ingress, then ALSO deploying a CNI with its own policies. This creates three problems:

1. **Conflicting Control Planes**: When multiple tools claim ownership of the same traffic, debugging becomes impossible. Is Istio dropping your packet, or Cilium, or the CNI?
2. **Operational Complexity**: You must maintain three separate security models, three separate observability stacks, three separate upgrade paths.
3. **Performance Tax**: Each layer adds overhead. Istio sidecars + Envoy Gateway + Cilium = 15-20% latency multiplier, which violates SLO budgets.

The CAVE Platform's three-layer model assigns **exactly one tool per responsibility**:

```
External Client
    ↓
[Kong API Gateway] ← North-South (API routing, rate limiting, auth, OpenAPI validation)
    ↓
[Cilium L3/L4 CNI] ← Foundation (network policy, egress governance, eBPF enforcement)
    ↓
[Istio Ambient] ← East-West (mTLS, L7 service routing, waypoint RBAC)
    ↓
Service Pod
```

### Kong: North-South Traffic (Tenant-Facing APIs)

Kong operates at the cluster boundary. Every external request to a tenant API or platform service enters through Kong, which provides:

- **Multi-tenant API routing**: Distinguishes between platform APIs (`*.cave.caveplatform.dev`), tenant APIs (`*.api.caveplatform.dev/<tenant>/...`), and monitoring APIs
- **Rate limiting**: Per-tier quotas (Soft, Hard, Dedicated) with per-user overrides
- **Authentication**: JWT/OAuth2 validation, tenant-scoped claims extraction
- **API governance**: OpenAPI schema validation, request/response transformation, sunset header injection for versioning
- **Observability**: Per-API request volume, latency percentiles, error rates

Kong does **not** handle inter-pod traffic. It does **not** enforce network policies. It does **not** create mTLS connections between services. These are responsibilities of Cilium and Istio.

### Istio Ambient: East-West Traffic (Service-to-Service)

Istio operates inside the cluster, between pods. The **ambient mode** (sidecar-less) replaces Istio's traditional per-pod proxy with:

- **ztunnel DaemonSet**: A lightweight Rust proxy running on each node that intercepts L4 traffic, establishes mTLS using SPIFFE identities, and enforces L4 authorization policies
- **Waypoint Proxies**: Optional Envoy proxies deployed per namespace that handle L7 HTTP policies (retries, circuit breaking, per-route RBAC) only when needed
- **Automatic mTLS**: Every pod gets a cryptographic identity and automatic mutual TLS with any peer, without sidecars cluttering the cluster

Istio ambient does **not** handle external traffic. It does **not** enforce L3/L4 network policies. It does **not** govern egress. These are Kong's and Cilium's responsibilities.

### Cilium: L3/L4 Foundation (CNI + Network Policy)

Cilium replaces the traditional kube-proxy with an eBPF-based implementation that achieves:

- **Network policy enforcement**: CiliumNetworkPolicy rules block traffic at the kernel level before it reaches user-space proxies
- **Egress governance**: Cilium Egress Gateway + eBPF byte counters enforce per-tenant egress quotas and trigger quarantine when exceeded
- **Observability**: Hubble provides real-time network flows and policy violations without sidecar overhead
- **Performance**: eBPF runs in kernel space, eliminating the 10-20% latency overhead of traditional iptables or user-space proxies

Cilium does **not** handle external API traffic (Kong's job). It does **not** manage mTLS certificates (Istio's job). It operates at the network layer, period.

---

## 5.2 ADR Rationale

### ADR-004: CNI Selection — Why Cilium?

**Context**: CAVE Platform requires a Container Network Interface that can enforce multi-tenant network policies, govern egress traffic with per-tenant quotas, provide observability, and maintain performance under high cardinality (thousands of policies).

**Decision**: Adopt Cilium as the primary CNI, replacing kube-proxy.

**Alternatives Evaluated**:

1. **Calico**: Industry-standard network policies with iptables backend
   - Rejection Reason: Iptables scales linearly with policy count. At 10K policies per cluster, rule compilation creates 5-10s latency spikes. eBPF (Cilium) is O(log n) or O(1) in most cases.

2. **Flannel**: Simple overlay network, minimal features
   - Rejection Reason: No network policy support. No egress governance. Inadequate for multi-tenant isolation.

3. **Weave**: Full-featured CNI with encryption
   - Rejection Reason: User-space packet processing introduces 15-20% latency overhead. Weave's network policy layer relies on iptables, not eBPF.

4. **Canal**: Flannel + Calico
   - Rejection Reason: Calico's iptables limitations still apply. No egress quota mechanism.

**Consequences**:
- Cluster requires Linux kernel 5.4+ (eBPF requires >= 5.3 for XDP, >= 5.4 for socket-level hooks)
- Cilium DaemonSet consumes ~80MB memory per node
- Hubble observability becomes standard operational practice
- Network policy evaluation time drops from O(n) to O(log n)
- Egress quotas and quarantine become native platform features

---

### ADR-027: API Gateway Selection — Why Kong?

**Context**: CAVE Platform must route external tenant API requests to internal services, enforce rate limits, validate OpenAPI schemas, and manage authentication across multiple tenants with different SLAs.

**Decision**: Adopt Kong as the API Gateway for all North-South traffic.

**Alternatives Evaluated**:

1. **Envoy Gateway**: CNCF Kubernetes Gateway API implementation
   - Rejection Reason: Plugin ecosystem immature (Q1 2026). No built-in OpenAPI validation. Rate limiting requires manual Wasm extension development. Fewer operators have deep Envoy expertise at scale.

2. **Traefik**: Kubernetes-native reverse proxy
   - Rejection Reason: No plugin ecosystem for business logic. OpenAPI validation not supported. Rate limiting limited to simple token buckets; cannot express per-tier quotas with burst allowances.

3. **Ambassador**: Datagram-based API Gateway (now Emissary-Ingress)
   - Rejection Reason: No longer actively developed. Sunset roadmap in late 2025. Community fragmented.

4. **Istio Gateway**: Istio's ingress gateway
   - Rejection Reason: Design intent is intra-cluster routing, not multi-tenant external APIs. No rate limiting, no OpenAPI validation, no OAuth2 token validation.

5. **NGINX Ingress Controller**: Battle-tested, widely deployed
   - Rejection Reason: No plugin system for rate limiting or auth at scale. Requires Lua scripting for business logic. Not designed for multi-tenant SLA management.

**Consequences**:
- Kong requires PostgreSQL backend for rate limit counters and configuration
- Kong's plugin architecture enables custom business logic (sunset headers, tenant claim extraction)
- License model: Kong Open Source (OSS) is AGPL v3; Konnect (cloud control plane) is proprietary but optional
- Kong DaemonSet consumes ~150MB per instance
- Rate limit data must replicate across Kong instances; requires shared Redis for distributed counters

---

### ADR-054: Network Segmentation — How Tenant Isolation Works

**Context**: CAVE Platform must provide **hard isolation** between tenants such that a compromised tenant workload cannot access another tenant's data, APIs, or infrastructure.

**Decision**: Implement three layers of tenant isolation—namespace boundary (Kubernetes), network policy (Cilium), and mTLS identity (Istio).

**Isolation Mechanisms**:

1. **Namespace Boundary**: Each tenant workload runs in a dedicated namespace `tenant-<tenant-id>`. Kubernetes RBAC prevents cross-tenant operations.

2. **Network Policy Layer (Cilium)**: CiliumNetworkPolicy explicitly denies all ingress and egress between tenant namespaces unless whitelisted.
   ```yaml
   apiVersion: cilium.io/v2
   kind: CiliumNetworkPolicy
   metadata:
     name: tenant-isolation
     namespace: tenant-acme-corp
   spec:
     endpointSelector: {}  # All pods in this namespace
     ingress:
       - fromNamespaces:
           - matchLabels:
               tenant: acme-corp
           - matchLabels:
               io.kubernetes.metadata.name: kube-system
       - fromEndpoints:
           - matchLabels:
               k8s:io.kubernetes.pod.namespace: kube-system
   ```

3. **mTLS Identity Layer (Istio)**: Each pod receives a SPIFFE identity that encodes the tenant ID. Istio's authorization policies verify tenant claims before allowing L7 routing.

**Consequences**:
- Network policies must be updated when new platform services are added
- Cross-tenant traffic is impossible, even through misconfiguration
- Egress from tenant namespaces requires explicit approval and quota management

---

### ADR-084: Multi-Tenant Network Policies

**Context**: CAVE Platform must scale to 500+ tenants, each with distinct network requirements. Tenants should not be able to exfiltrate data via DNS or side-channel attacks.

**Decision**: Implement per-tenant CiliumNetworkPolicy with automatic Safe-Exit List enforcement for critical external dependencies.

**Policy Structure**:

```yaml
apiVersion: cilium.io/v2
kind: CiliumNetworkPolicy
metadata:
  name: tenant-egress
  namespace: tenant-{{ .TenantID }}
spec:
  endpointSelector: {}
  egress:
    # Intra-cluster: allowed
    - toEndpoints:
        - matchLabels:
            k8s:io.kubernetes.pod.namespace: tenant-{{ .TenantID }}
    # Platform control plane: allowed
    - toNamespaces:
        - matchLabels:
            io.kubernetes.metadata.name: kube-system
    # Safe-Exit List: external FQDNs approved by tenant
    - toFQDNs:
        - matchName: "auth.okta.com"
        - matchName: "api.stripe.com"
    # DNS: allowed
    - toEndpoints:
        - matchLabels:
            k8s:io.kubernetes.app: coredns
      toPorts:
        - ports:
            - port: "53"
              protocol: UDP
```

**Consequences**:
- Tenant cannot reach arbitrary external IPs
- Tenant must pre-declare critical external dependencies (auth, CDN, payments)
- Unauthorized egress attempts trigger quarantine (see ADR-110)

---

### ADR-110: Egress Governance & Quarantine

**Context**: A compromised or misconfigured tenant workload may attempt to exfiltrate data or participate in botnet activity. CAVE Platform must detect and halt unauthorized egress without disrupting legitimate traffic.

**Decision**: Implement per-tenant egress quotas (byte/second) via Cilium Egress Gateway + eBPF byte counters. When egress exceeds quota, automatically patch CiliumNetworkPolicy to deny all external traffic except Safe-Exit List.

**Egress Quarantine Workflow**:

1. **Detection**: Cilium eBPF counter tracks egress bytes/second per tenant namespace
2. **Threshold Breach**: When cumulative egress exceeds per-tier daily quota (e.g., 10GB/day for Soft tier)
3. **Automatic Quarantine**: Reflex Engine (async controller) patches CiliumNetworkPolicy to:
   - DENY all egress to external IPs
   - ALLOW intra-namespace traffic (service-to-service)
   - ALLOW platform control plane traffic (metrics, logs)
   - ALLOW Safe-Exit List FQDNs (auth, CDN, payment gateways)
   - ALLOW DNS (for name resolution of Safe-Exit items)
4. **Notification**: Alerts sent to tenant via Grafana OnCall, platform ops team
5. **Manual Recovery**: Ops team must approve re-enablement or quota adjustment
6. **Auto-Recovery**: After 24 hours without further violations, quarantine lifts automatically

**Per-Tier Egress Quotas**:

| Tier | Daily Quota | Burst Allowance | Autonomy Threshold |
|------|-------------|-----------------|-------------------|
| Soft | 10GB | 100MB/s | Any confidence |
| Hard | 50GB | 500MB/s | ≥0.7 (else human review) |
| Dedicated | Custom | Custom | ≥0.9 (else human approval) |

**Autonomy Thresholds**: The "confidence" that an egress spike is legitimate.
- **Any confidence (Soft tier)**: Platform ML model detects spike; can auto-quarantine without human review
- **≥0.7 (Hard tier)**: Model must be 70%+ confident before auto-quarantine
- **≥0.9 (Dedicated tier)**: Model must be 90%+ confident; otherwise escalate to human reviewer

**Consequences**:
- Tenant workload may experience egress interruption if quota exceeded
- Safe-Exit List becomes critical operational dependency (must be maintained)
- Quarantine prevents data exfiltration but may break legitimate external API calls
- 24-hour auto-recovery window allows tenant to investigate and remediate

---

### ADR-121: Service Mesh — Why Istio Ambient?

**Context**: CAVE Platform requires inter-pod mTLS, service discovery, and L7 HTTP routing with fine-grained RBAC. Traditional sidecar-based service meshes add 50-100MB per pod and complicate debugging.

**Decision**: Adopt Istio ambient mode (sidecar-less) for mTLS and L7 policies, avoiding per-pod proxy sidecars.

**Alternatives Evaluated**:

1. **Istio Sidecar**: Traditional model with per-pod Envoy proxy
   - Rejection Reason: 50MB memory per pod. At 500 pods, that's 25GB cluster overhead. Sidecar crashes leak pod traffic. Increased observability surface area (sidecar logs + app logs).

2. **Linkerd**: Lightweight service mesh with per-pod "micro-proxies"
   - Rejection Reason: 30MB per pod (better than Istio). But observability limited to HTTP. No L7 policy at scale (VirtualService equivalent). Smaller community ecosystem.

3. **Cilium Service Mesh**: eBPF-based service mesh (alpha in Cilium 1.19)
   - Rejection Reason: GA timeline uncertain (estimated late 2026). CNI load already high (network policy + egress). Combining both on same eBPF layer risks performance regression.

4. **No service mesh**: Rely on Kubernetes DNS + application-level libraries (gRPC, Istio CRDs without proxies)
   - Rejection Reason: No cryptographic mTLS guarantee. Requires application changes for retry/circuit-break logic. No per-route RBAC.

**Consequences**:
- Istio ambient requires Kubernetes 1.25+ (for native sidecar gatekeepers)
- Waypoint proxies (Envoy) needed only for L7 policies; namespace can opt-out for performance-critical workloads
- ztunnel DaemonSet consumes ~100MB per node (much lighter than sidecars)
- SPIFFE identity (X.509 SAN: `spiffe://cluster.local/ns/<namespace>/sa/<sa>`) is automatic
- mTLS certificates auto-rotated every 24 hours

---

### ADR-122: Three-Layer Networking Architecture

**Context**: CAVE Platform must support 500+ tenants with distinct network policies, rate limits, and egress governance while maintaining sub-100ms p99 latency. No single tool (Istio, Kong, Cilium) can handle all three concerns without performance trade-offs.

**Decision**: Adopt three-layer architecture with strict separation:
- Kong (North-South API routing & rate limiting)
- Istio Ambient (East-West mTLS & L7 policies)
- Cilium (L3/L4 network policy & egress governance)

**Layer Interaction Model**:

```
Layer 1: Kong (External)
  Input: External HTTP/gRPC requests
  Output: Authenticated, rate-limited traffic to internal load balancer
  Responsibility: Tenant API routing, rate limiting, auth, OpenAPI validation
  Rejects: rate limit exceeded, invalid auth, schema violation

Layer 2: Cilium (L3/L4 Enforcement)
  Input: Internal traffic from Kong or pod-to-pod
  Output: Traffic authorized by network policy and egress quota
  Responsibility: Network policy, egress governance, tenant isolation
  Rejects: traffic violates CiliumNetworkPolicy, egress quota exceeded

Layer 3: Istio Ambient (L7 Service Routing)
  Input: Network-policy-approved traffic from Layer 2
  Output: Traffic routed to correct service with mTLS
  Responsibility: mTLS, per-route RBAC, service discovery, resilience
  Rejects: mTLS verification fails, per-route RBAC denies, unauthorized claim in JWT
```

**Why No Overlap**:

- Kong **never** touches pod-to-pod traffic (Istio handles)
- Kong **never** enforces network policies (Cilium handles)
- Istio **never** handles external traffic (Kong handles)
- Istio **never** enforces egress quotas (Cilium handles)
- Cilium **never** validates JWTs (Kong handles)
- Cilium **never** performs L7 routing (Istio handles)

**Consequences**:
- Debugging is straightforward: Kong logs → Cilium flow logs (Hubble) → Istio metrics
- Scaling is independent: Kong can scale without touching Istio or Cilium
- Upgrades are decoupled: Kong minor version update does not require Cilium restart
- Performance is optimized: Each layer uses best-in-class technology for its concern

---

## 5.3 Tool Comparison Matrices

### API Gateway Comparison

| Feature | Kong | Envoy Gateway | Traefik | Ambassador | NGINX Ingress |
|---------|------|---------------|---------|------------|---------------|
| **OpenAPI Validation** | ✅ (plugin) | ❌ | ❌ | ❌ | ❌ |
| **Plugin Ecosystem** | ✅ Excellent (500+) | ⚠️ Early (Wasm) | ❌ Limited | ⚠️ Moderate | ⚠️ Lua only |
| **Rate Limiting** | ✅ Per-tier + burst | ⚠️ Token bucket | ⚠️ Basic | ⚠️ Basic | ⚠️ Basic |
| **JWT/OAuth2** | ✅ Native | ❌ (needs Wasm) | ❌ | ⚠️ Limited | ❌ |
| **K8s Gateway API** | ✅ v1.3 support | ✅ v1.3 support | ✅ v1.0 support | ⚠️ Alpha | ❌ |
| **Multi-tenant APIs** | ✅ Path-based routing | ⚠️ With extensions | ⚠️ Basic | ⚠️ Limited | ⚠️ Limited |
| **Latency (p99)** | 50ms | 45ms | 60ms | 70ms | 40ms |
| **Community Size** | ⭐⭐⭐⭐ (Large) | ⭐⭐⭐⭐ (Growing) | ⭐⭐⭐ (Medium) | ⭐⭐ (Declining) | ⭐⭐⭐⭐ (Huge, legacy) |
| **License** | AGPL/Commercial | Apache 2.0 | MIT | Apache 2.0 | Apache 2.0 |
| **Best For** | Multi-tenant SaaS | Cloud-native APIs | Microservices | (Deprecated) | Legacy K8s |

**Selection Rationale**: Kong wins on plugin ecosystem, OpenAPI validation, and rate limiting—the three pillars of multi-tenant API governance. NGINX has lower latency but lacks business logic extensibility.

---

### Service Mesh Comparison

| Feature | Istio Ambient | Istio Sidecar | Linkerd | Cilium SM (Alpha) | None |
|---------|---------------|---------------|---------|------------------|------|
| **mTLS** | ✅ Automatic (ztunnel) | ✅ Per-pod (sidecar) | ✅ Per-pod | ✅ eBPF | ❌ |
| **L7 Policies** | ✅ Waypoint (optional) | ✅ Per-pod | ⚠️ HTTP only | ⚠️ Limited | ❌ |
| **Per-Route RBAC** | ✅ Via waypoint | ✅ Per-pod | ⚠️ Limited | ❌ | ❌ |
| **Memory Overhead (500 pods)** | 5GB (ztunnel) | 25GB (sidecars) | 15GB | 2GB | 0GB |
| **Observability** | ✅ Prometheus | ✅ Prometheus | ✅ Prometheus | ⚠️ Emerging | ❌ |
| **Multi-cluster** | ✅ Via gateways | ✅ Via gateways | ✅ Via gateways | ❌ | ❌ |
| **CNCF Status** | ⭐ Graduated | ⭐ Graduated | ⭐ Graduated | ⚠️ Incubating | N/A |
| **GA Timeline** | ✅ GA (1.25) | ✅ Mature | ✅ Mature | 2026 Q3 est. | N/A |
| **Sidecar Complexity** | ❌ None | ⚠️ High | ⚠️ Moderate | ❌ None | N/A |
| **Best For** | Multi-tenant, memory-constrained | Enterprise observability | Lightweight ops | (Future) | Simple K8s |

**Selection Rationale**: Istio ambient offers automatic mTLS with 5x lower memory overhead than sidecar deployment. Linkerd is lighter but lacks per-route RBAC. Cilium SM still in alpha; timeline uncertain.

---

### CNI Comparison

| Feature | Cilium | Calico | Flannel | Weave | Canal |
|---------|--------|--------|---------|-------|-------|
| **eBPF Support** | ✅ Full | ❌ (iptables) | ❌ | ❌ | ❌ |
| **Network Policy** | ✅ L3/L4/L7 | ✅ L3/L4 | ❌ | ⚠️ Basic | ✅ (Calico) |
| **Egress Governance** | ✅ Quotas + quarantine | ❌ | ❌ | ❌ | ❌ |
| **Observability (Hubble)** | ✅ Network flows | ❌ | ❌ | ❌ | ❌ |
| **Policy Scale (10K policies)** | O(log n) | O(n) iptables | N/A | N/A | O(n) |
| **Latency Impact** | <1ms | 50-100ms | <1ms | 100-150ms | 50-100ms |
| **Encryption** | ✅ WireGuard | ⚠️ IPSec | ❌ | ✅ Built-in | ✅ (Calico) |
| **Kernel Requirement** | 5.4+ | Any | Any | Any | Any |
| **Best For** | Multi-tenant, high-scale | Enterprise, iptables-familiar | Simple overlays | Encrypted overlays | N/A (deprecated) |
| **Community** | ⭐⭐⭐⭐ (Growing) | ⭐⭐⭐⭐ (Largest) | ⭐⭐⭐ (Stable) | ⭐⭐ (Limited) | ⭐ (Obsolete) |

**Selection Rationale**: Cilium's eBPF backend scales to 10K+ policies without latency penalty. Calico's iptables implementation degrades at scale. Cilium's egress governance (ADR-110) is unique among CNIs.

---

## 5.4 Twenty-Four Month Roadmap Analysis

### Kong (18-24 months)

- **Q2 2026**: Gateway API v1.4 support (matches Kubernetes Gateway v1.4 release)
- **Q3 2026**: Plugin marketplace expansion (OAuth3, GraphQL validation, AI-powered rate limiting)
- **Q4 2026**: KIC (Kong Ingress Controller) v3.0 with workload identity federation
- **2027 Q1-Q2**: Full ingress-to-gateway migration path (deprecated Ingress API removal in K8s 1.32)
- **Risk**: PostgreSQL becomes performance bottleneck at 50K req/s; Kong Konnect (cloud) recommended for >100K req/s

### Istio (18-24 months)

- **Q2 2026**: Istio ambient mesh GA (graduated from alpha)
- **Q3 2026**: Multi-cluster ambient via Envoy proxy gateways (alpha)
- **Q4 2026**: Automated sidecar-to-ambient migration tooling
- **2027 Q1**: Sidecar mode enters maintenance mode; focus shifts to ambient
- **Risk**: Sidecar injection will continue to work indefinitely for legacy deployments

### Cilium (18-24 months)

- **Q2 2026**: Cilium v1.19 release (WireGuard as default encryption)
- **Q3 2026**: Service mesh beta (eBPF-based alternative to Istio)
- **Q4 2026**: Hubble 2.0 with ML-based anomaly detection
- **2027 Q1**: eBPF BTF (Build Time Format) support for runtime policy generation
- **Risk**: Service mesh feature parity with Istio estimated 2026 Q4

### Kubernetes Gateway API (18-24 months)

- **Q2 2026**: Gateway API v1.4 becomes standard (Ingress deprecated but functional)
- **Q3 2026**: NGINX Ingress Controller retires (no longer maintained)
- **Q4 2026**: 100% of cloud-native API gateways (Kong, Traefik, Envoy Gateway) support v1.4
- **2027 Q1**: Kubernetes core removes Ingress API (v1.32+)
- **Implication**: CAVE Platform must migrate from Ingress → Gateway API by 2026 Q3

---

## 5.5 Architecture Deep Dive

### Kong Architecture: North-South API Routing

Kong runs as a Kubernetes Deployment with a 3-2-1 replica strategy (3 replicas in primary region, 2 in secondary, 1 in tertiary). All replicas share a PostgreSQL backend for configuration and rate limit counters.

**Routing Model**:

```
Request to api.caveplatform.dev/acme-corp/v2/widgets
  ↓
Kong routing rule matches: /acme-corp/* → service kong-tenant-acme
  ↓
Kong rate limiting plugin checks: is user under 100 req/s (Soft tier)?
  ↓
Kong JWT plugin validates: Bearer token claims tenant_id == "acme-corp"
  ↓
Kong OpenAPI validation plugin checks: request matches /v2/widgets schema
  ↓
Kong adds headers:
  - X-Tenant-ID: acme-corp
  - X-API-Version: v2
  - X-Request-ID: <uuid>
  ↓
Request forwarded to upstream: http://tenant-acme.tenant-acme.svc.cluster.local:8080
```

**Rate Limiting Per Tier**:

| Tier | Per-User Limit | Burst | Daily Quota | Cost ($/month) |
|------|----------------|-------|-------------|----------------|
| Soft | 100 req/s | 200 (2s burst) | 500K | $100 |
| Hard | 500 req/s | 1K (2s burst) | 2M | $500 |
| Dedicated | Custom (1K-10K) | Custom | Custom | $2K+ |

**API Versioning Strategy**:

Kong injects `Sunset` and `Deprecation` headers for deprecated API versions:
```
HTTP/1.1 200 OK
Deprecation: true
Sunset: Sat, 30 Jun 2026 23:59:59 GMT
Link: </v3/widgets>; rel="successor-version"
```

### Istio Ambient Architecture: East-West mTLS

Istio ambient operates in two modes:

**Mode 1: ztunnel only (Lightweight)**
```
Service A Pod
  ↓
Outbound traffic on port 8080 (to Service B)
  ↓
[ztunnel eBPF program] intercepts (via SO_REUSEPORT)
  ↓
ztunnel negotiates mTLS with ztunnel on Service B's node
  ↓
ztunnel sends plaintext traffic to Service B pod (already inside cluster)
```

Memory overhead: ~100MB per node (1-2 replicas per node)

**Mode 2: Waypoint proxy (Full-Featured)**

For namespaces that need L7 policies, an Envoy proxy is deployed as a Kubernetes Service:

```
Service A Pod → ztunnel
  ↓
ztunnel (reads namespace labels, sees waypoint enabled)
  ↓
ztunnel forwards traffic to waypoint proxy (Envoy)
  ↓
Waypoint applies L7 policies (retries, circuit break, per-route RBAC)
  ↓
Waypoint forwards to Service B
```

Memory overhead: ~400MB per waypoint proxy (typically 1 per namespace with L7 policies)

**Enrollment via Namespace Labeling**:

```yaml
apiVersion: v1
kind: Namespace
metadata:
  name: tenant-acme-corp
  labels:
    istio.io/rev: default  # Opt-in to mesh
```

**SPIFFE Identity Format**:

```
spiffe://cluster.local/ns/tenant-acme-corp/sa/acme-app
        ↑                ↑ ↑             ↑    ↑
        cluster domain   ns tenant      ns   service account
```

Encoded into X.509 certificate Subject Alternative Name (SAN).

### Cilium Architecture: L3/L4 Foundation + Egress Governance

Cilium replaces `kube-proxy` (iptables-based service networking) with eBPF programs installed on every node.

**Architecture Components**:

1. **Cilium DaemonSet** (per node): Loads eBPF programs into kernel, manages policies
2. **Cilium Operator** (central): Watches Kubernetes API, generates eBPF bytecode
3. **Hubble** (observability): Userspace service that collects eBPF telemetry
4. **Cilium CLI** (`cilium` command): Inspection and debugging

**eBPF Enforcement Model**:

```
Packet arrives at pod's veth pair
  ↓
[egress eBPF program] checks CiliumNetworkPolicy
  ↓
Policy: allow from tenant-acme-corp → service-b? → YES
  ↓
[egress meter eBPF program] increments byte counter for tenant-acme-corp
  ↓
Check egress quota exceeded? → NO (currently 2GB of 10GB daily)
  ↓
Packet forwarded to destination
```

**Egress Quarantine Mechanism**:

When egress quota exceeded (byte counter > threshold):

```
Reflex Engine (async controller) triggered
  ↓
Patch CiliumNetworkPolicy in tenant namespace:
  egress:
    - toFQDNs matching Safe-Exit List (auth.okta.com, api.stripe.com)
    - to kube-system (DNS resolution)
    - DENY all other external IP traffic
  ↓
Alert to tenant + ops team via Grafana OnCall
  ↓
Tenant investigates root cause
  ↓
After 24 hours, auto-restore if no further violations
  ↓
Or: ops team manually approves restoration after reviewing logs
```

**Safe-Exit List Management**:

Each tenant provides a manifest:

```yaml
apiVersion: platform.caveplatform.dev/v1
kind: SafeExitList
metadata:
  name: critical-external-deps
  namespace: tenant-acme-corp
spec:
  fqdns:
    - auth.okta.com  # OIDC provider
    - api.stripe.com # Payment processor
    - cdn.acme-corp.com # CDN for static assets
  permanentExemption: false  # Expires if not renewed quarterly
```

---

## 5.6 Use Cases & Developer Scenarios

### Scenario 1: Tenant API Request Through Full Stack

**Preconditions**: Tenant "acme-corp" (Soft tier, 100 req/s limit) requests `/v2/widgets` API

**Execution**:

1. **Kong (North-South)**
   ```
   GET /acme-corp/v2/widgets HTTP/1.1
   Host: api.caveplatform.dev
   Authorization: Bearer eyJhbGc...
   ```

   Kong's rate limiting plugin checks: user `user-123@acme-corp` has sent 98 requests in last second → within limit → ALLOW

   Kong's JWT plugin validates: token claims include `{"tenant_id": "acme-corp", "user": "user-123"}` → ALLOW

   Kong's OpenAPI validation plugin checks: request body matches `/v2/widgets` POST schema → ALLOW

   Kong injects headers and forwards to internal service:
   ```
   GET /v2/widgets HTTP/1.1
   Host: widgets-service.tenant-acme-corp.svc.cluster.local
   X-Tenant-ID: acme-corp
   X-User-ID: user-123
   X-Request-ID: 550e8400-e29b-41d4-a716-446655440000
   ```

2. **Cilium (L3/L4)**
   ```
   Kong pod (default namespace) → widgets-service pod (tenant-acme-corp namespace)
   ```

   Cilium eBPF program evaluates CiliumNetworkPolicy:
   ```yaml
   # From default namespace is denied unless whitelisted
   # Let's check: is "default" namespace in ingress allowlist for tenant-acme-corp?
   # Yes: platform control plane (system:masters) can access all namespaces
   ```

   Cilium allows traffic. Egress byte counter updated for tenant-acme-corp.

3. **Istio Ambient (East-West)**
   ```
   widgets-service pod needs to call inventory-service pod
   (both in tenant-acme-corp namespace)
   ```

   ztunnel on source node detects outbound connection to `inventory-service.tenant-acme-corp.svc.cluster.local`

   ztunnel establishes mTLS with ztunnel on destination node:
   ```
   ClientHello
     SNI: inventory-service.tenant-acme-corp.svc.cluster.local
     ALPN: h2
     Cert: CN=spiffe://cluster.local/ns/tenant-acme-corp/sa/widgets-app
   ```

   mTLS handshake succeeds. Traffic forwarded.

4. **Response Path** (reverse order)
   ```
   inventory-service → widgets-service (mTLS checked by ztunnel)
     → Kong (no policy check needed, Kong only rates inbound)
     → External client
   ```

---

### Scenario 2: Egress Spike Detection & Automatic Quarantine

**Preconditions**: Tenant "malicious-corp" (Soft tier, 10GB/day quota) starts downloading a 50GB dataset from AWS S3

**Timeline**:

- **T+0s**: Tenant workload initiates egress to S3 (not in Safe-Exit List)
- **T+1s**: Cilium eBPF counter logs ~5GB/s egress (normal for S3 download)
- **T+2s**: Cumulative egress = 10GB. Quota exceeded.
- **T+3s**: Reflex Engine patches CiliumNetworkPolicy:
  ```yaml
  egress:
    - toFQDNs: Safe-Exit List (empty for this tenant)
    - toPorts: [port: 53, protocol: UDP] # DNS
    - toNamespaces: [kube-system] # Control plane
    # All other egress DENIED
  ```
- **T+4s**: Existing S3 connections drop (reset by eBPF program)
- **T+5s**: Alert fires in Grafana OnCall: "Egress quarantine activated: malicious-corp"
- **T+10s**: Ops team receives page, checks Hubble flow logs, confirms unauthorized egress to unknown IP range
- **T+30min**: Ops team contacts tenant, learns of misconfiguration (credentials leaked in container image)
- **T+1h**: Tenant fixes vulnerability, requests quota increase (now Hard tier, 50GB/day)
- **T+1h30min**: Ops team approves, updates CiliumNetworkPolicy (manual restore, not auto-restore)

---

### Scenario 3: Adding New Tenant API with Custom Rate Limits

**Use Case**: New SaaS customer "gamma-corp" signs up, negotiates Hard tier (500 req/s, 2M daily quota) with custom per-endpoint limits.

**Steps**:

1. **Create tenant namespace**:
   ```bash
   kubectl create namespace tenant-gamma-corp
   kubectl label namespace tenant-gamma-corp tenant=gamma-corp
   ```

2. **Enroll in Istio ambient**:
   ```bash
   kubectl label namespace tenant-gamma-corp istio.io/rev=default
   ```

3. **Create network policy**:
   ```bash
   cat <<EOF | kubectl apply -f -
   apiVersion: cilium.io/v2
   kind: CiliumNetworkPolicy
   metadata:
     name: tenant-isolation
     namespace: tenant-gamma-corp
   spec:
     endpointSelector: {}
     ingress:
       - fromNamespaces:
           - matchLabels:
               io.kubernetes.metadata.name: default
     egress:
       - toNamespaces:
           - matchLabels:
               io.kubernetes.metadata.name: tenant-gamma-corp
   EOF
   ```

4. **Add Kong rate limiting config**:
   ```bash
   cat <<EOF | kubectl apply -f -
   apiVersion: configuration.konghq.com/v1beta1
   kind: KongConsumer
   metadata:
     name: gamma-corp-api
     namespace: default
   username: gamma-corp
   credentials:
     - name: gamma-corp-jwt
   custom:
     rateLimitTier: hard
     dailyQuota: 2000000
   EOF
   ```

5. **Create Ingress rule** (or Gateway):
   ```bash
   cat <<EOF | kubectl apply -f -
   apiVersion: networking.k8s.io/v1
   kind: Ingress
   metadata:
     name: gamma-corp-api
     namespace: default
     annotations:
       konghq.com/strip-path: "true"
       konghq.com/plugins: "rate-limiting"
   spec:
     ingressClassName: kong
     rules:
       - host: api.caveplatform.dev
         http:
           paths:
             - path: /gamma-corp/
               pathType: Prefix
               backend:
                 service:
                   name: gamma-corp-api-gateway
                   port:
                     number: 8080
   EOF
   ```

6. **Test rate limiting**:
   ```bash
   # First 500 requests/second succeed
   for i in {1..600}; do curl -H "Authorization: Bearer <jwt>" https://api.caveplatform.dev/gamma-corp/v1/data & done

   # Requests 501+ get 429 (Too Many Requests)
   ```

---

### Scenario 4: Emergency Mesh Permissive Mode

**Use Case**: P1 incident—service-to-service traffic blocked, cause unknown. Ops team enables permissive mode to restore availability while investigating.

```bash
# Enable permissive mode on single node (blast radius = 1 node)
cave-ctl mesh permissive --node node-04-prod

# Now mTLS validation is logged but not enforced
# Traffic flows even if certificate is invalid

# Check logs
kubectl logs -n istio-system -l app=ztunnel --tail=100 | grep "mTLS-permissive"

# Identify root cause (e.g., certificate not rotated on pod restart)

# Fix root cause
# Then disable permissive mode
cave-ctl mesh permissive --node node-04-prod --disable
```

---

## 5.7 Configuration Reference

### Kong Plugin Chain

```yaml
apiVersion: configuration.konghq.com/v1
kind: KongClusterPlugin
metadata:
  name: rate-limiting
plugin: rate-limiting
config:
  minute: 100  # Per-user default
  hour: 6000
  policy: redis  # Distributed counter backend
  redis_host: redis-cluster.kube-system.svc.cluster.local
  redis_port: 6379
  redis_timeout: 1000
  redis_ssl: true
---
apiVersion: configuration.konghq.com/v1
kind: KongClusterPlugin
metadata:
  name: jwt-validator
plugin: jwt
config:
  key_claim_name: sub
  secret_is_base64: false
  algorithms:
    - RS256
  issuer: https://auth.caveplatform.dev
---
apiVersion: configuration.konghq.com/v1
kind: KongClusterPlugin
metadata:
  name: openapi-validator
plugin: openapi-schema-validator
config:
  strict_request_validation: true
  api_spec_url: "https://api-specs.internal/acme-corp/v2/openapi.json"
```

### Cilium Network Policy (Tenant Isolation)

```yaml
apiVersion: cilium.io/v2
kind: CiliumNetworkPolicy
metadata:
  name: tenant-egress-quota
  namespace: tenant-acme-corp
spec:
  description: "Tenant isolation + egress governance"
  endpointSelector: {}  # All pods in this namespace

  # Ingress: deny by default
  ingress:
    - fromNamespaces:
        - matchLabels:
            io.kubernetes.metadata.name: tenant-acme-corp
    - fromNamespaces:
        - matchLabels:
            io.kubernetes.metadata.name: kube-system

  # Egress: explicit whitelist
  egress:
    # Intra-tenant traffic
    - toEndpoints:
        - matchLabels:
            k8s:io.kubernetes.pod.namespace: tenant-acme-corp
    # Control plane (metrics, logs)
    - toNamespaces:
        - matchLabels:
            io.kubernetes.metadata.name: kube-system
    # Safe-Exit List (critical external services)
    - toFQDNs:
        - matchName: "auth.okta.com"
        - matchName: "api.stripe.com"
    # DNS
    - toEndpoints:
        - matchLabels:
            k8s-app: coredns
      toPorts:
        - ports:
            - port: "53"
              protocol: UDP
```

### Istio Ambient Waypoint Policy

```yaml
apiVersion: networking.istio.io/v1beta1
kind: RequestAuthentication
metadata:
  name: jwt-validation
  namespace: tenant-acme-corp
spec:
  jwtRules:
    - issuer: "https://auth.caveplatform.dev"
      jwksUri: "https://auth.caveplatform.dev/.well-known/jwks.json"
      audiences:
        - "acme-corp-api"
---
apiVersion: security.istio.io/v1beta1
kind: AuthorizationPolicy
metadata:
  name: per-route-rbac
  namespace: tenant-acme-corp
spec:
  selector:
    matchLabels:
      app: widgets-service
  action: ALLOW
  rules:
    # Only allow requests with valid JWT from inventory-service
    - from:
        - source:
            principals: ["cluster.local/ns/tenant-acme-corp/sa/inventory-app"]
      to:
        - operation:
            methods: ["GET"]
            paths: ["/v1/widgets/*"]
    # Deny everything else
```

---

## 5.8 Operations: Day-2 Activities

### Kong Plugin Management

```bash
# List installed plugins
kong config db --load --database kong-database-prod

# Add rate limiting to new API endpoint
cat <<EOF | kubectl apply -f -
apiVersion: configuration.konghq.com/v1
kind: KongPluginBinding
metadata:
  name: new-api-rate-limit
spec:
  consumer: gamma-corp-api
  plugin: rate-limiting
  config:
    minute: 500
EOF

# Verify plugin loaded in Kong
kubectl exec -it deployment/kong -c kong -- kong plugins list
```

### Istio Ambient Enrollment

```bash
# Check enrollment status
kubectl get namespace -L istio.io/rev

# Enroll namespace
kubectl label namespace tenant-new-co istio.io/rev=default

# Verify ztunnel deployed the workload
kubectl get pods -n tenant-new-co --field-selector status.phase=Running -o wide | grep ztunnel

# Check mTLS status
istioctl analyze -n tenant-new-co
```

### Cilium Policy Updates

```bash
# Reload policies after edit
kubectl apply -f cilium-network-policy.yaml

# Verify policy loaded
cilium policy get -n tenant-acme-corp

# Trace packet flow (debugging)
cilium monitor -n tenant-acme-corp --type drop

# View flow logs via Hubble
hubble observe -n tenant-acme-corp --follow --verdict DROPPED
```

### Certificate Rotation (Istio)

Istio ambient auto-rotates SPIFFE certificates every 24 hours via Envoy's SDS (Secret Discovery Service).

```bash
# Verify certificate age
kubectl exec -it <pod> -c istio-proxy -- openssl s_client -showcerts </dev/null 2>/dev/null | grep -A2 "Issuer:"

# If manual rotation needed (emergency)
kubectl delete secret istiod-ca-secret -n istio-system
# Istiod will regenerate root CA and push new workload certificates
```

---

## 5.9 Troubleshooting

### Issue 1: "429 Too Many Requests" from Kong

**Symptoms**: Tenant complains requests rejected with 429, within expected rate limit.

**Root Cause**: Distributed rate limiting across multiple Kong replicas may show eventual consistency delays.

**Investigation**:
```bash
# Check Kong rate limit counters in Redis
redis-cli -h redis-cluster.kube-system.svc.cluster.local
> KEYS "ratelimit:*"
> GET ratelimit:acme-corp:user-123
# Should show current counter and TTL

# Check Kong configuration
kubectl get KongConsumer gamma-corp-api -o yaml

# Verify Redis backend connectivity
kubectl logs -n kong deployment/kong | grep redis
```

**Fix**:
```bash
# Option 1: Increase per-user limit if legitimate spike
kubectl patch KongConsumer gamma-corp-api --type merge -p '{"custom":{"rateLimitMinute":1000}}'

# Option 2: Check for request amplification (check application logs)
# Option 3: Upgrade to Kong with sticky rate limiting (future)
```

---

### Issue 2: mTLS Handshake Failure Between Services

**Symptoms**: Service A cannot reach Service B; TLS alert "certificate_required"

**Root Cause**: Service B not enrolled in Istio ambient, or SPIFFE identity mismatch.

**Investigation**:
```bash
# Check Istio enrollment
kubectl get namespace -L istio.io/rev

# Verify SPIFFE identity
kubectl get pod -n tenant-acme-corp widgets-service-xyz -o yaml | grep spiffe

# Check ztunnel logs
kubectl logs -n kube-system -l app=ztunnel -c ztunnel --tail=50 | grep "mTLS\|handshake"

# View traffic via Hubble
hubble observe -n tenant-acme-corp --from-pod tenant-acme-corp/widgets-service-xyz --verdict DROPPED
```

**Fix**:
```bash
# Enroll Service B's namespace
kubectl label namespace tenant-acme-corp istio.io/rev=default --overwrite

# Restart Service B pod to trigger ztunnel injection
kubectl rollout restart deployment/inventory-service -n tenant-acme-corp

# Verify mTLS after restart
kubectl logs <pod> -n tenant-acme-corp | grep "mTLS\|SPIFFE"
```

---

### Issue 3: Egress Quarantine Triggered, Tenant Cannot Access External API

**Symptoms**: Tenant reports external API calls failing; ops team sees quarantine logs.

**Investigation**:
```bash
# Verify quarantine status
kubectl get CiliumNetworkPolicy -n tenant-acme-corp -o yaml | grep -A10 "toFQDNs"

# Check Safe-Exit List
kubectl get SafeExitList -n tenant-acme-corp -o yaml

# Review Hubble flows (egress denials)
hubble observe -n tenant-acme-corp --verdict DROPPED --protocol L3_L4 --follow

# Check egress byte counters
cilium map get -n tenant-acme-corp egress-quota
```

**Fix (Option 1: Add to Safe-Exit List)**:
```yaml
apiVersion: platform.caveplatform.dev/v1
kind: SafeExitList
metadata:
  name: critical-external-deps
  namespace: tenant-acme-corp
spec:
  fqdns:
    - auth.okta.com
    - api.stripe.com
    - api.external-service.com  # NEW
```

**Fix (Option 2: Increase Quota)**:
```bash
# Ops team upgrades tenant to Hard tier (50GB/day)
kubectl patch Tenant acme-corp --type merge -p '{"spec":{"tier":"hard"}}'

# Manual restore (immediately)
cave-ctl tenant restore-egress --tenant acme-corp

# Or wait for 24-hour auto-restore
```

---

### Issue 4: Cilium Policy Not Enforced

**Symptoms**: Traffic that should be denied by CiliumNetworkPolicy is still allowed.

**Root Cause**: Policy not yet compiled into eBPF, or syntax error in policy.

**Investigation**:
```bash
# Validate policy syntax
kubectl apply -f cilium-policy.yaml --dry-run=client

# Check policy compilation status
cilium policy validate -f cilium-policy.yaml

# Monitor Cilium Operator logs
kubectl logs -n kube-system -l k8s-app=cilium-operator | grep "policy\|error"

# Check if policy selector matches any endpoints
kubectl get pods -n tenant-acme-corp -L cilium-policy

# Trace enforcement
cilium monitor -c -t drop
```

**Fix**:
```bash
# Recreate policy with verbose logging
kubectl delete CiliumNetworkPolicy -n tenant-acme-corp --all
kubectl apply -f cilium-policy.yaml

# Verify endpoint assignment
cilium endpoint list -n tenant-acme-corp
# Should show IDENTITY and POLICY-ENFORCED columns

# Wait 30 seconds for eBPF recompilation
sleep 30

# Re-test connectivity
kubectl exec -it <pod> -n tenant-acme-corp -- curl <blocked-service>
# Should now be blocked (connection timeout or refused)
```

---

## 5.10 Compliance Mapping

### SOC 2 Type II: Encryption in Transit

- **Requirement**: All traffic must be encrypted in transit.
- **Kong**: TLS 1.3 for external API traffic (terminated at Kong Ingress LB)
- **Cilium + Istio**: Pod-to-pod traffic encrypted via Istio mTLS (TLS 1.3, SPIFFE identity validation)
- **Evidence**: Hubble flow logs show `tls:true` for all east-west traffic

### ISO 27001: Network Segmentation

- **Requirement**: Separate networks or logical segregation per tenant.
- **Implementation**: Each tenant in dedicated namespace with CiliumNetworkPolicy preventing cross-tenant traffic
- **Evidence**: CiliumNetworkPolicy audit logs, Hubble DENIED flows from unauthorized sources

### NIS2 Directive: Access Control

- **Requirement**: Fine-grained access control, RBAC logging.
- **Implementation**: Istio AuthorizationPolicy per-route RBAC + Kong JWT validation
- **Evidence**: Istio AuthorizationPolicy audit, Kong authentication logs

### GDPR: Data Locality & Egress Governance

- **Requirement**: Restrict data exfiltration, audit outbound traffic.
- **Implementation**: Cilium egress quotas + Safe-Exit List + Hubble flow logs
- **Evidence**: Hubble egress logs showing all external traffic (filtered by tenant, FQDN, volume)

---

## 5.11 Related ADRs

- **ADR-004**: CNI Selection (Cilium)
- **ADR-027**: API Gateway Selection (Kong)
- **ADR-054**: Network Segmentation (per-tenant isolation)
- **ADR-084**: Multi-Tenant Network Policies
- **ADR-110**: Egress Governance & Quarantine
- **ADR-121**: Service Mesh Selection (Istio Ambient)
- **ADR-122**: Three-Layer Networking Architecture

---

## 5.12 Related Runbook Sections

- **§02 — Architecture & Core Concepts**: Platform topology, tenant isolation layers
- **§03 — Installation & Day-1 Operations**: Initial Kong, Istio, Cilium deployment
- **§04 — Security & Multi-Tenancy**: Isolation mechanisms, RBAC foundations
- **§06 — Observability & Monitoring**: Hubble, Prometheus, Kong metrics
- **§08 — Incident Response**: Network troubleshooting playbooks
- **§10 — ADR Reference**: Detailed architectural decision records
