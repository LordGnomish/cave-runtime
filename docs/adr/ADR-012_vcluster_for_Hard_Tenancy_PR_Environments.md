# ADR-012: vcluster for Hard Tenancy + PR Environments

**Status:** Accepted

**Scope:** Universal

**Category:** Multi-Tenancy

**Related ADRs:** 070, 084

## Context

CAVE needs strong tenant isolation for Hard/Dedicated tiers (production) and ephemeral environments for CI PR validation. Namespace-level isolation (Soft tier) is insufficient for tenants requiring dedicated control plane semantics.

## Candidates

| Criteria | vcluster | Capsule | Dedicated Clusters | Kata Containers |
|---|---|---|---|---|
| Isolation level | Virtual cluster (own API server, etcd) | Namespace with enhanced policies | Full cluster | Pod-level VM isolation |
| Resource overhead | ~300MB per vcluster (lightweight) | Minimal (namespace-only) | Full cluster cost | Per-pod VM overhead |
| K8s API parity | Full (tenant gets real kubeconfig) | Limited (namespace-scoped) | Full | N/A (runtime, not cluster) |
| Ephemeral creation time | ~30 seconds | ~5 seconds (just namespace) | 10-20 minutes | N/A |
| Multi-tenant scheduling | Shares host cluster nodes | Shares cluster | Dedicated nodes | Shares nodes |
| License | Apache 2.0 (Loft Labs) | Apache 2.0 | N/A | Apache 2.0 |

## Decision

**vcluster** for: (1) Hard/Dedicated tier production (persistent vclusters), (2) PR environments (ephemeral, capped 2CPU/4Gi/4h TTL/max 5 per tenant — ADR-070).

## Rejected

- **Capsule:** Namespace-level only. No virtual control plane. Tenant cannot get a real kubeconfig. Insufficient for Hard tier isolation where tenant needs cluster-admin-like experience within their scope.
- **Dedicated clusters:** Too expensive for PR environments. Each cluster takes 10-20 min to provision. vcluster gives cluster semantics at namespace cost (~30s creation).
- **Kata Containers:** Runtime-level isolation (pod sandboxing), not cluster-level. Doesn't address the kubeconfig/control-plane isolation requirement.

## Consequences

(+) Cluster-level isolation at namespace cost. 30s creation for ephemeral PR envs. Full kubeconfig for Hard/Dedicated tenants. Apache 2.0.
(-) Additional component to manage. vcluster version must track host K8s version. Syncer overhead. Networking between vcluster and host cluster requires careful CiliumNetworkPolicy.

## Compliance Mapping

SOC2 CC6.1 (logical access controls per tenant), ISO A.8.22 (segregation in networks).
