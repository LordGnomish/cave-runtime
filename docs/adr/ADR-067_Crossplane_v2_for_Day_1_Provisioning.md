# ADR-067: Crossplane v2 for Day 1+ Provisioning

**Status:** Accepted

**Scope:** Universal

**Category:** Platform / Infrastructure

**Related ADRs:** 119, 124

## Context

CAVE needs a mechanism for Day 1+ infrastructure provisioning (databases, caches, message queues, search, storage) that provides continuous reconciliation (not one-shot apply) and works identically across Hetzner and Azure.

## Candidates

| Criteria | Crossplane v2 | Terraform/OpenTofu | Pulumi | ACK/ASO (AWS/Azure) |
|---|---|---|---|---|
| Reconciliation | ✅ Continuous (K8s controller loop) | ❌ One-shot (plan/apply) | ❌ One-shot | ✅ Continuous (K8s) |
| Multi-provider abstraction | ✅ XRDs + Compositions (same API, different backends) | ⚠️ Modules (different HCL per provider) | ⚠️ Same language, different SDKs | ❌ Single provider |
| Namespace-first (v2) | ✅ Namespaced XRs for natural tenant isolation | ❌ Global state | ❌ Global state | ⚠️ Namespace possible |
| Day-2 operations | ✅ CronOperation/WatchOperation (ADR-119) | ❌ Separate tooling | ❌ Separate tooling | ❌ |
| MRAP (CRD reduction) | ✅ ManagedResourceActivationPolicy (ADR-124) | N/A | N/A | N/A |
| Composition Functions | ✅ Programmable compositions (Go, Python) | N/A (HCL modules) | ✅ (full language) | N/A |
| K8s native | ✅ CRDs, kubectl, ArgoCD-managed | ❌ Separate state + CLI | ❌ Separate state + CLI | ✅ |
| License | Apache 2.0 | MPL 2.0 (OpenTofu) | Apache 2.0 | Apache 2.0 |

## Decision

**Crossplane v2** (namespace-first model) for all Day 1+ provisioning. **OpenTofu** for Day 0 only (cluster creation, VNet, DNS — one-shot resources that don't need reconciliation). ArgoCD orchestrates both. Crossplane owns infrastructure reconciliation; ArgoCD owns workload deployment.

## Rejected

- **Terraform/OpenTofu for everything:** OpenTofu is imperative (plan/apply). Day 2+ infrastructure needs continuous reconciliation — if someone manually changes a DB setting, Crossplane auto-reverts. OpenTofu wouldn't detect drift until next apply. OpenTofu is perfect for Day 0 (cluster, network) but wrong for Day 1+ (application infrastructure).
- **Pulumi:** Powerful language SDKs but creates heterogeneous IaC (TypeScript + Go + Python). Crossplane's declarative K8s-native model is more consistent with ArgoCD GitOps. No namespace-first model.
- **ACK/ASO:** Single provider only. Cannot abstract Hetzner + Azure behind same API.

## Consequences

(+) Continuous reconciliation catches drift automatically. Namespace-first provides natural RBAC per tenant. Same XR API across providers. MRAP reduces CRD footprint. Operations handle Day-2 maintenance. ArgoCD manages Crossplane XRs like any other K8s resource.
(-) Crossplane learning curve (XRDs, Compositions, Functions). Provider version pinning required (ADR-108). Crossplane v2 Operations still alpha — fallback to Reflex Engine (ADR-119). Debugging failed compositions requires understanding provider + Crossplane + ArgoCD layers.

## Compliance Mapping

SOC2 CC8.1 (infrastructure changes via Git/Crossplane, not manual).

**Absorbed Decisions:** The following tool-level decisions are absorbed into this ADR for traceability

**ADR-071**

kuttl for Crossplane Composition Testing

**Decision:** kuttl gates every Crossplane Composition change. Rejection: custom Go tests (slower iteration), manual kubectl verification (no regression safety).
