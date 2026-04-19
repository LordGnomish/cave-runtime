# ADR-012: vcluster + Kamaji for Tenant Isolation and PR Environments

**Status:** Accepted (Revised — hybrid model)

**Scope:** Universal

**Category:** Multi-Tenancy

**Related ADRs:** 070 (vcluster CI Policy), 084 (Default-Deny Network Policy), 087 (ResourceQuota/LimitRange)

## Context

CAVE needs strong tenant isolation across two distinct use cases:

1. **Ephemeral PR environments:** CI pipeline creates a short-lived cluster-like environment for integration testing. Must be fast (<60s), cheap, and auto-destroyed after TTL.
2. **Hard/Dedicated tier production:** Enterprise tenants paying for dedicated isolation need real control plane separation — their own API server, etcd, CRD space. Namespace-level isolation is insufficient for compliance-sensitive workloads.

Soft tier tenants (namespace isolation + NetworkPolicy + ResourceQuota) are handled by ADR-084 and ADR-087 — this ADR covers the Hard/Dedicated tier and PR environments only.

---

## Candidates

| Criteria | vcluster | Kamaji | Capsule | Dedicated Clusters |
|---|---|---|---|---|
| **Architecture** | Virtual cluster: syncer proxies namespace as full cluster | Hosted Control Plane: real API server + etcd per tenant | Enhanced namespace with policy engine | Full separate K8s cluster |
| **Isolation level** | Virtual (shared host etcd, syncer-filtered) | Real (separate API server + etcd per tenant) | Namespace (policy-enforced) | Full (dedicated nodes + control plane) |
| **CRD isolation** | ⚠️ CRDs shared on host, syncer filters | ✅ Full CRD isolation (tenant installs own CRDs) | ❌ Shared (host CRDs only) | ✅ Full |
| **etcd isolation** | ❌ Shared host etcd (virtual separation) | ✅ Separate etcd per tenant (or shared with strict RBAC) | ❌ Shared | ✅ Dedicated |
| **Tenant kubeconfig** | Real (points to vcluster API proxy) | Real (points to tenant API server) | Limited (namespace-scoped) | Real (dedicated cluster) |
| **Creation time** | ~30 seconds | ~2-3 minutes | ~5 seconds (namespace only) | 10-20 minutes |
| **Resource overhead** | ~300MB per vcluster | ~500MB-1GB per tenant CP | Minimal | Full cluster cost |
| **Ephemeral use case** | ✅ Excellent (fast create/destroy) | ⚠️ Acceptable but slower (2-3 min) | ✅ Fast but weak isolation | ❌ Too slow, too expensive |
| **Production multi-tenant** | ⚠️ Acceptable but not true CP isolation | ✅ Excellent (real CP per tenant) | ❌ Insufficient for Hard tier | ✅ Best but prohibitive cost |
| **CNCF status** | Sandbox (Loft Labs) | Sandbox (CLASTIX) | N/A | N/A |
| **License** | Apache 2.0 | Apache 2.0 | Apache 2.0 | N/A |
| **OSS vs Pro** | ⚠️ Some features Pro-only (sleep mode, central mgmt) | ✅ Fully open source, no tiered features | N/A | N/A |
| **Community** | Growing (~6K stars, Loft Labs backed) | Growing (~2K stars, CLASTIX backed) | Moderate (~3K stars) | N/A |

---

## Decision

**Hybrid model:**

1. **vcluster** for ephemeral PR environments (fast, cheap, disposable)
   - 30s creation, 4h TTL, max 5 per tenant (ADR-070)
   - 2 CPU / 4Gi RAM cap per vcluster
   - Auto-destroyed after CI pipeline completes or TTL expires
   - Acceptable isolation level for testing (not production data)

2. **Kamaji** for Hard/Dedicated tier production tenants (real isolation)
   - Separate API server + etcd per tenant
   - Full CRD isolation — tenants can install their own operators
   - Real kubeconfig that behaves identically to a standalone cluster
   - Persistent — lifecycle tied to tenant subscription

3. **Capsule** (namespace + policy) remains for Soft tier (ADR-084, ADR-087) — unchanged.

**Runtime implementation:** cave-vcluster crate manages ephemeral vcluster lifecycle. New cave-kamaji crate manages Hosted Control Plane provisioning via Kamaji Tenant Control Plane CRD.

---

## Rejected Options

### vcluster for Everything — Rejected for Production

**Primary:** Shared etcd. All vclusters on a host share the same etcd instance — tenant A's data is in the same etcd as tenant B's. While vcluster syncer provides logical separation, a compromised host etcd exposes all tenants. For Hard/Dedicated tier where tenants pay for isolation guarantees, this is insufficient. Kamaji provides real etcd separation.

**Secondary:** CRD contamination. CRDs installed in a vcluster are actually installed on the host cluster. One tenant's CRD version can conflict with another's. Kamaji gives each tenant their own CRD namespace — no cross-tenant CRD interference.

### Kamaji for Everything — Rejected for Ephemeral

**Primary:** Too slow for PR environments. 2-3 minute creation time vs vcluster's 30 seconds. CI pipelines run dozens of PR environments daily — 2-3 min overhead per PR is unacceptable for developer velocity. vcluster's lightweight syncer model is purpose-built for ephemeral use.

**Secondary:** Resource overhead. Kamaji's real API server + etcd consumes 500MB-1GB per tenant. For ephemeral 4h environments running 5 per tenant, this wastes resources. vcluster at 300MB is 2-3x more efficient for throwaway environments.

### Dedicated Clusters — Rejected

**Primary:** Cost and time. Provisioning a full K8s cluster takes 10-20 minutes and costs a full control plane (3 nodes minimum on Hetzner). Neither ephemeral PR use nor most production tenants justify this cost. Kamaji provides equivalent isolation at ~20% of the cost.

### Capsule for Hard Tier — Rejected

**Primary:** No control plane isolation. Capsule operates at the namespace level — enhanced with policies, quotas, and network rules, but still sharing the cluster API server. Tenants cannot install CRDs, cannot have cluster-scoped resources, and share etcd with all other tenants. Insufficient for compliance-sensitive enterprise workloads.

---

## Consequences

### Positive

- Best-of-both-worlds: fast ephemeral environments (vcluster) + real production isolation (Kamaji)
- Full CRD isolation for Hard/Dedicated tenants — they can install their own operators without affecting others
- Real kubeconfig for both tiers — tenants interact with standard kubectl, no CAVE-specific tooling needed
- Both tools are Apache 2.0 and CNCF Sandbox — aligned with CAVE's OSS-first principle
- Kamaji is fully open source with no tiered/Pro features — no future licensing risk
- Clear tiering: Soft (Capsule/namespace) → Hard (Kamaji) → Ephemeral (vcluster)

### Negative

- Two cluster virtualization technologies to operate (vcluster + Kamaji)
- Kamaji is younger than vcluster — smaller community, fewer production references
- cave-kamaji crate is new implementation effort
- Networking between Kamaji tenant CP and host data plane requires careful Cilium configuration
- Kamaji tenant API server needs TLS certificates (cert-manager integration, ADR-015)

### Risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Kamaji community stalls (CLASTIX pivot) | Low | High | Apache 2.0 license allows fork. vcluster can temporarily serve Hard tier (downgraded isolation) while alternative is evaluated. |
| vcluster Pro features become essential (sleep mode, central mgmt) | Medium | Medium | For ephemeral use (4h TTL), sleep mode is irrelevant. Central management handled by cave-vcluster crate. Evaluate annually if Pro features become blocking. |
| Kamaji + Cilium networking issues | Medium (early) | Medium | Test tenant-to-service connectivity in staging. Cilium ClusterMesh may be needed for cross-vcluster traffic. Document in Runbook. |
| etcd resource consumption (many Kamaji tenants) | Medium | Medium | Shared etcd option in Kamaji (with strict RBAC per tenant) for smaller tenants. Dedicated etcd only for Dedicated tier. Monitor via Prometheus. |
| Kubernetes version skew (host vs tenant CP) | Low | Medium | Kamaji supports N-2 version skew. Pin tenant CP version in Tenant CRD. Automated upgrade via cave-ctl. |

---

## Compliance Mapping

**SOC2 CC6.1:** Logical access controls — Kamaji provides real API server isolation per tenant; vcluster provides virtual isolation for CI.
**SOC2 CC6.4:** Physical and logical access restrictions — etcd separation (Kamaji) prevents cross-tenant data access.
**ISO A.8.22:** Segregation in networks — CiliumNetworkPolicy per vcluster/Kamaji tenant.
**ISO A.8.31:** Separation of development/test from production — vcluster ephemeral environments are destroyed after CI; Kamaji production is persistent and isolated.
**NIS2 Art.21:** Risk management — tiered isolation model matches risk level (Soft < Hard < Dedicated).
