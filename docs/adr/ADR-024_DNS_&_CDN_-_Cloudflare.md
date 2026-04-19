# ADR-024: DNS & CDN — Cloudflare

**Status:** Accepted

**Scope:** Universal

**Category:** Infrastructure

**Related ADRs:** 015, 066 | Absorbs: ADR-065

Status:

Category:

Infrastructure

Related ADRs:

015, 066

Back to Index:

## Context

CAVE needs a DNS provider for caveplatform.dev and all tenant subdomains. Must support: DNS-01 ACME challenge (for cert-manager), geographic failover (for multi-provider), DDoS protection, and fast global resolution.


## Candidates

| Criteria | Cloudflare | AWS Route 53 | Azure DNS | Hetzner DNS |
|---|---|---|---|---|
| DNS-01 ACME | ✅ cert-manager Cloudflare solver | ✅ | ✅ | ⚠️ Limited API |
| Geographic failover | ✅ Load balancing + health checks | ✅ | ✅ Traffic Manager | ❌ |
| DDoS protection | ✅ Enterprise-grade (free tier) | ⚠️ Shield (paid) | ⚠️ DDoS Protection | ❌ |
| API maturity | ✅ Excellent (Terraform/OpenTofu provider) | ✅ | ✅ | ⚠️ |
| Provider independence | ✅ Third-party (not Hz or Az) | ❌ AWS-coupled | ❌ Azure-coupled | ❌ Hz-coupled |
| Free tier | ✅ Generous (unlimited DNS, basic proxy) | ❌ Per-query pricing | ❌ Per-zone pricing | ✅ Basic |


## Decision

**Cloudflare** for DNS, DDoS protection, and geographic failover. Provider-independent (not coupled to Hetzner or Azure). Supports cert-manager DNS-01 challenge. OpenTofu manages DNS records (Day 0). TTL reduced to 60s before cross-provider migration cutover.


## Rejected Options

- **AWS Route 53:** AWS-coupled. Using AWS DNS for a platform that runs on Hetzner and Azure creates unnecessary AWS dependency.
- **Azure DNS:** Azure-coupled. Same reasoning.
- **Hetzner DNS:** Limited API. No geographic failover. No DDoS protection. Insufficient for production multi-provider platform.


## Consequences

**Positive:**
- Provider-independent DNS — no cloud-vendor coupling.
- Free DDoS protection for all CAVE endpoints.
- Geographic health checks enable Hetzner ↔ Azure failover.
- cert-manager integration proven and stable.
- Fast global anycast resolution.

**Negative:**
- Cloudflare dependency (single external provider for DNS). If Cloudflare has major outage, all CAVE endpoints unreachable. Mitigated: Cloudflare 100% SLA, multi-region anycast.
- Cloudflare API token is in Break-glass Kit (ADR-079) — critical credential.
- Cloudflare proxying adds ~10ms latency (optional — can use DNS-only mode).

Compliance Mapping

SOC2 CC7.5 (availability — DDoS protection). ISO A.8.22 (network security). NIS2 Art.21 (network protection). GDPR Art.32 (availability of processing systems).

Absorbed Decisions:

The following tool-level decisions are absorbed into this ADR for traceability

Standard LoadBalancer on Both Providers

Decision:

Ingress parity: standard LoadBalancer on both providers (Hetzner LB + Azure LB). Kong behind standard LB delivers identical routing model regardless of provider.

