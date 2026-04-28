# ADR-CHARTER-002: Deployment Profiles Trinity

**Status:** Accepted (2026-04-28)
**Scope:** Charter (foundational)
**Category:** Deployment Architecture
**Decided:** 2026-04-28 (Burak Tartan)

## Context

Cave is consumed by tenants with materially different requirements: enterprises that want managed-cloud convenience, sovereignty-conscious teams that need OSS-only on a sovereign substrate, and forward-leaning operators who want a clean-slate sovereign Cloud OS. A single profile cannot serve all three without compromising one of the three postures.

ADR review surfaced a recurring pattern: contributors flag "SaaS dependency in the Azure profile" as a sovereignty break, even though Azure profile's stated posture explicitly trades sovereignty for managed convenience. The audit framework needs an explicit charter that names what each profile is and is not.

## Decision

Cave ships **three independent deployment profiles**. Each profile has a distinct posture, a distinct default component set, and a distinct contract with the tenant.

### Profile 1 — Azure

- **Underlying:** Azure cloud infrastructure (managed PaaS / IaaS).
- **Default components:** Azure-native services where they exist (PostgreSQL Flexible Server, AI Search, Key Vault, Entra ID, Azure DNS) plus selected vendor products acceptable in this posture (Cloudflare DNS+WAF, Confluent Cloud, Okta).
- **OSS opt-in:** Every component has an OSS alternative selectable per tenant (cave-dns, Strimzi, Keycloak, OpenSearch, etc.). Tenants who want partial sovereignty inside Azure can compose freely.
- **Posture:** Managed enterprise convenience with explicit flexibility. CLOUD Act and equivalent legal regimes apply; tenants accept that trade in exchange for managed SLAs and ecosystem alignment.

### Profile 2 — Hetzner

- **Underlying:** Hetzner cloud infrastructure (sovereign EU jurisdiction).
- **Components:** OSS-only, mandatory. CNPG, OpenSearch, MinIO, Strimzi, Keycloak, OpenBao, Cilium, ESO. No third-party SaaS dependency is permitted in this profile — DNS, WAF, IdP, streaming, all run on the cluster or on adjacent OSS infrastructure.
- **Posture:** Sovereign-cloud-OSS. Full data ownership, EU jurisdictional posture, no vendor cloud lock-in. Tenants accept the operational cost of running everything themselves.

### Profile 3 — Cave Runtime

- **Underlying:** Linux kernel 7.1 directly on VM or bare metal — no underlying managed cloud.
- **Components:** Sovereign single-binary Rust reimplementation of every upstream product. cave-pg, cave-streams, cave-search, cave-blob, cave-net, cave-vault, cave-portal, cavectl, etc. Multi-upstream consolidation is permitted (e.g. cave-streams unifies Kafka and Pulsar; cave-gateway unifies Kong proxy with Gravitee portal).
- **No backward compatibility:** Clean slate. ML-KEM-768 / ML-DSA-65 PQC-ready by default. Wire-protocol parity is honored where it serves tenants; legacy POSIX/glibc/systemd compromises are not.
- **UX:** Single portal + cavectl. Upstream UIs (pgAdmin, Compass, Redis Insight, etc.) are never ported — all admin surface lives in cave-portal panels (ADR-RUNTIME-UI-CONSOLIDATION-001, ADR-048).
- **Posture:** Future-of-platform sovereign Cloud OS.

## Rationale

A single profile cannot satisfy enterprise convenience, sovereign OSS, and clean-slate sovereign Cloud OS at once. Forcing one to serve all three either bloats it (managed + sovereign + reimpl all in one config) or paralyses decisions (every PR debates trade-offs that the profile's posture should have already settled). The trinity makes the posture explicit and pushes the trade-off decision to the **profile selection** point, not the **per-component** point.

## Implementation rule (binding for future ADRs)

Every future product-selection ADR MUST declare a profile-conditional decision in this shape:

```
Hetzner:      <OSS choice — sovereign>
Azure:        <default vendor or Azure-native> + <OSS opt-in>
Cave Runtime: cave-<name> (single-binary reimpl)
```

ADRs that name only one profile's choice are reviewed as incomplete and bounced back for the missing rows.

## Audit framework

Findings of the form "SaaS dependency in the Azure profile" are **not** automatic sovereignty breaks. Azure profile's posture explicitly accepts vendor managed services as a contract with the tenant. Such findings are valid input for an Azure-profile-specific tenant-portability or vendor-exit discussion, but they do not by themselves trigger a "modify-hard" recommendation under devils-advocate review.

Findings of the form "SaaS dependency in the Hetzner profile" or "SaaS dependency in the Cave Runtime profile" remain real charter violations and are surfaced as `critical`.

## Long-term vision

Cave Runtime is the long-arc destination. Each sprint moves more of the stack into single-binary Rust reimplementation. The endgame absorbs the Linux kernel itself (cave-kernel refactor track) so that Profile 3 ships as a self-contained sovereign Cloud OS that boots directly to bare metal — no Linux underneath, no containers around, just cave-runtime serving every layer.

Profiles 1 and 2 do not disappear in that endgame — they remain available for tenants who want managed cloud or sovereign OSS without committing to the Cave Runtime stack.

## Related

- **ADR-001** — Hetzner Cloud as Sovereign Infrastructure Provider (Profile 2 underlying).
- **ADR-002** — Azure as Enterprise Infrastructure Provider (Profile 1 underlying).
- **ADR-CHARTER-001** — Golden Rule foundation (process invariants this trinity slots into).
- **ADR-RUNTIME-STACK-001** — Cave Runtime stack architecture (Profile 3 layer model).
- **ADR-RUNTIME-CONSOLIDATION-PRINCIPLE-001** — multi-upstream consolidation rule (Profile 3 component sourcing).
- **ADR-RUNTIME-NO-BACKCOMPAT-PQC** — clean-slate + PQC posture (Profile 3 posture).
- **ADR-RUNTIME-UI-CONSOLIDATION-001** — single portal mandate (Profile 3 UX rule).
- **ADR-RUNTIME-CLI-CONSOLIDATION-001** — single CLI mandate (Profile 3 UX rule).
- **ADR-048** — Data persistence portal wraps (Profile 3 UI specialization).

## Compliance

- **GDPR Art.32** — each profile carries a distinct data-residency posture; tenants select per data-classification needs.
- **NIS2 Art.21** — sovereignty posture is an explicit deployment-time decision, auditable.
- **SOC2 CC6.1** — single AuthN per profile; no cross-profile credential leakage.

---
*Decided by Burak Tartan, recorded by Sonnet, 2026-04-28 ADR session.*
