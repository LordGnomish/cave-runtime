# ADR-008: Cache — Valkey (Hetzner) / Azure Redis (Azure)

**Status:** Accepted

**Scope:** Universal, Hetzner, Azure

**Category:** Data

**Related ADRs:** 067, 135

Status:

Category:

Data

Related ADRs:

067, 135

Back to Index:

## Context

CAVE tenants need an in-memory cache/data structure store for session management, rate limiting, pub/sub, and application caching. The solution must be available on both providers via Crossplane XR abstraction and support the Redis protocol for broad application compatibility.


## Candidates

| Criteria | Valkey (Hetzner) | Redis OSS (Hetzner) | Azure Cache for Redis | Dragonfly | KeyDB |
|---|---|---|---|---|---|
| License | BSD 3-Clause (Linux Foundation fork) | RSALv2 + SSPLv1 (post-2024 dual license) | Azure terms (managed) | BSL 1.1 | BSD 3-Clause |
| Redis protocol compatible | ✅ Full (fork of Redis 7.2) | ✅ Native | ✅ Native | ✅ Drop-in | ✅ Drop-in |
| K8s operator | Spotahome operator (community) | Spotahome operator | N/A (managed) | Helm only | Helm only |
| Cluster mode | ✅ Redis Cluster | ✅ Redis Cluster | ✅ Managed clustering | ✅ Single-threaded emulation | ✅ |
| Persistence | ✅ RDB + AOF | ✅ RDB + AOF | ✅ Managed persistence | ✅ | ✅ |
| Community | Growing rapidly (Linux Foundation, ex-Redis contributors) | Fragmented post-license change | Azure-managed | Small (MotherDuck) | Small |
| Self-hosted viability | ✅ Fully self-hostable | ⚠️ License restricts managed service offerings | ❌ Azure only | ✅ | ✅ |
| Multi-tenant support | ✅ ACL per tenant (Redis 6+ ACLs) | ✅ | ✅ | ✅ | ✅ |


## Decision

**Valkey** (self-hosted via Helm on Hetzner) + **Azure Cache for Redis** (managed on Azure). Unified Cache XRD via Crossplane. Valkey chosen over Redis OSS due to Redis Labs' RSALv2/SSPL dual license change — same zero-vendor-lock-in principle as OpenBao over Vault (ADR-020).


## Rejected Options

- **Redis OSS (post-2024 license):** RSALv2 + SSPLv1 dual license. CAVE's zero-vendor-lock-in principle (ADR-066) prohibits restrictive licenses where permissive alternatives exist. Valkey is a direct fork by original Redis contributors under BSD 3-Clause.
- **Dragonfly:** BSL 1.1 license (same concern as Vault/Redis). Despite impressive single-threaded performance claims, BSL is disqualifying.
- **KeyDB:** BSD 3-Clause (acceptable) but smaller community than Valkey. Snap Inc. reduced investment. Valkey has Linux Foundation backing and growing momentum.
- **Memcached:** No persistence, no pub/sub, no data structures beyond key-value. Too limited for CAVE's use cases (session management requires persistence, rate limiting requires sorted sets).


## Consequences

**Positive:**
- BSD 3-Clause license — no vendor lock-in risk.
- Full Redis protocol compatibility — all Redis clients work unmodified.
- Linux Foundation backing provides community governance stability.
- Same Crossplane XR abstracts Valkey (Hz) and Azure Redis (Az) behind unified Cache API.
- ACL-based multi-tenant isolation within shared Valkey instances (Soft tier).

**Negative:**
- Valkey is younger than Redis (forked 2024). Some edge-case compatibility issues may emerge as projects diverge.
- Spotahome operator is community-maintained (not official Valkey project). Operator maturity less than CNPG or Strimzi.
- Azure Cache for Redis uses the original Redis codebase — technical parity with Valkey is high now but may diverge over time. Parity tests (ADR-135) must cover cache behavior.

Compliance Mapping

SOC2 CC6.1 (access controls — ACL per tenant). ISO A.8.24 (encryption — TLS in transit, encryption at rest for persistent data). GDPR Art.32 (security of processing — tenant data isolation in shared cache).

