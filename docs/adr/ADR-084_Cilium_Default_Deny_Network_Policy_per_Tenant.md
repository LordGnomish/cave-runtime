# ADR-084: Cilium Default-Deny Network Policy per Tenant

**Status:** Accepted

**Scope:** Runtime, Universal

**Category:** Security

**Related ADRs:** 004, 110

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## Multi-tenant platform must enforce strict network isolation. Default Kubernetes NetworkPolicy is allow-all — unsuitable for multi-tenant.

## Candidates

## | Approach | CiliumNetworkPolicy default-deny | K8s NetworkPolicy | Calico | No policy |
|---|---|---|---|---|
| L3/L4 enforcement | ✅ eBPF-based | ✅ iptables-based | ✅ eBPF/iptables | ❌ |
| L7 FQDN egress | ✅ CiliumNetworkPolicy | ❌ | ⚠️ Limited | ❌ |
| eBPF byte counters | ✅ Per-flow egress metering | ❌ | ⚠️ | ❌ |
| Hubble observability | ✅ Network flow visibility | ❌ | ⚠️ | ❌ |
| Cross-tenant blocking | ✅ Namespace-scoped default-deny | ✅ | ✅ | ❌ |

## Decision

## CiliumNetworkPolicy default-deny applied per tenant namespace per environment. Only explicitly allowed traffic passes. Cross-tenant blocked. Cross-environment blocked (tenant-dev cannot reach tenant-prod). HostNetwork forbidden. OPA validates default-deny exists per namespace. `cave-ctl network test` runs Cilium connectivity suite.

## Rejected

## - **K8s NetworkPolicy only:** Lacks L7 FQDN-based egress rules needed for Safe-Exit Lists (ADR-110). No eBPF byte counters for egress metering. No Hubble flow visibility.
- **Calico:** Capable but CAVE already uses Cilium as CNI (ADR-004). Running both is unnecessary complexity.
- **No network policy:** Unacceptable for multi-tenant platform. Any pod could reach any other pod.

## Consequences

## **Positive:**
- True zero-trust networking: no implicit trust between any services.
- Per-tenant, per-environment isolation enforced at kernel level (eBPF).
- FQDN-based egress enables Safe-Exit Lists during quarantine.
- eBPF byte counters enable per-tenant egress cost attribution.

**Negative:**
- Strict default-deny can break applications during initial deployment if network policy is incomplete.
- Developers must declare explicit network policies for service-to-service communication.
- FQDN egress rules require DNS-aware policy which adds Cilium complexity.

## Compliance Mapping

## SOC2 CC6.1 (network segmentation). ISO A.8.22 (segregation in networks). NIS2 Art.21 (network security). GDPR Art.32 (security of processing — network-level isolation).
