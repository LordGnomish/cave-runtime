# ADR-110: Egress Governance — Quarantine + Safe-Exit List

**Status:** Accepted

**Category:** Security/FinOps

**Related ADRs:** 084, 096

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## Egress cost overruns and data exfiltration are major risks in multi-tenant platforms. Egress must be monitored, metered, and controllable — including automated quarantine.

## Candidates

## | Control | Mechanism | Layer |
|---|---|---|
| Monitoring | Cilium eBPF byte counters → Prometheus | Network |
| Metering | Per-tenant egress attribution | FinOps |
| Threshold alert | Prometheus alert (quota or anomaly) | Observability |
| Quarantine | Reflex Engine patches CiliumNetworkPolicy | Network |
| Safe-Exit List | Tenant-declared critical FQDNs preserved during quarantine | Network |
| Auto-restore | 24h timeout without human confirmation | Network |

## Decision

## Two enforcement layers: (1) Cilium + eBPF byte counters → Prometheus → threshold alert → Reflex Engine quarantine. (2) Kong rate-limiting per tenant per API. Quarantine preserves: cluster DNS, intra-namespace traffic, platform control plane, Safe-Exit List (tenant-declared critical FQDNs). Auto-restore 24h. Autonomy per tier: Soft=any confidence, Hard≥0.7, Dedicated≥0.9.

## Rejected

## - **No egress control:** Cost explosion (Azure egress ~€0.087/GB). Data exfiltration undetectable.
- **Kong-only:** Insufficient for infrastructure-level egress (direct pod-to-internet traffic bypasses Kong).
- **Full network isolation during quarantine:** Breaks tenant internal services. Safe-Exit List preserves critical functionality.
- **No per-tier autonomy:** Dedicated tenants should have higher threshold before automated quarantine (higher cost tolerance, more sensitive to false positives).

## Consequences

## **Positive:**
- Egress cost explosion contained automatically.
- Data exfiltration attempts trigger quarantine + alert.
- Safe-Exit List preserves tenant core functionality during quarantine.
- Per-tier autonomy thresholds prevent false-positive disruption of high-value tenants.

**Negative:**
- False positives quarantine legitimate high-egress tenants (mitigated: Safe-Exit List, 24h auto-restore).
- Safe-Exit List maintenance by tenants — incomplete lists may block critical services.
- Cilium eBPF byte counters add small CPU overhead per node.

## Compliance Mapping

## SOC2 CC6.1 (network controls). SOC2 CC7.2 (anomaly detection). ISO A.8.22 (network controls). ISO A.8.23 (web filtering). NIS2 Art.21 (network security). GDPR Art.32 (security of processing).
