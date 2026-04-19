# ADR-024: DNS & CDN — Cloudflare

**Status:** Accepted

**Scope:** Universal

**Category:** Infrastructure

**Related ADRs:** 015, 066 | Absorbs: ADR-065

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

### AWS Route 53 — Rejected

**Primary:** AWS-coupled. Route 53 requires an AWS account and IAM credentials. CAVE runs on Hetzner and Azure — introducing AWS solely for DNS creates a third cloud dependency with its own billing, IAM, and operational surface. Cloudflare is genuinely provider-independent.

**Secondary:** Per-query pricing. Route 53 charges per million DNS queries. Cloudflare's unlimited DNS queries (free tier) is more predictable for a platform with many tenant subdomains.

### Azure DNS — Rejected

**Primary:** Azure-coupled. Same reasoning as Route 53 — DNS should be independent of compute providers so that Hetzner ↔ Azure failover is controlled by a neutral third party, not by one of the providers being failed-over from.

### Hetzner DNS — Rejected

**Primary:** Limited API and no geographic failover. Hetzner DNS has no health check or geographic routing capability. Cannot automatically redirect traffic from Hetzner to Azure during a Hetzner outage. Also no DDoS protection — critical for public-facing platform endpoints.


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

### Risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Cloudflare major outage (all DNS down) | Very Low | Critical | Cloudflare anycast = 300+ PoPs, multi-region. Historical uptime >99.99%. Break-glass: pre-cached DNS records at registrar (72h TTL failsafe). |
| Cloudflare API token compromise | Low | Critical | Token stored in OpenBao (ADR-020) + Break-glass Kit (ADR-079). Scoped to DNS-only permissions. Rotate quarterly. |
| Cloudflare pricing change (free → paid for features used) | Low | Medium | CAVE uses DNS + basic proxy (free tier). Enterprise features not required. If pricing changes, migrate to secondary provider (pre-tested annually). |
| Cloudflare WARP/Zero Trust scope creep | Low | Low | CAVE uses Cloudflare ONLY for DNS and DDoS. Zero Trust networking stays Cilium+Istio (ADR-004, ADR-014). No Cloudflare Tunnel, no WARP. Clear boundary. |

Compliance Mapping

SOC2 CC7.5 (availability — DDoS protection). ISO A.8.22 (network security). NIS2 Art.21 (network protection). GDPR Art.32 (availability of processing systems).

Absorbed Decisions:

The following tool-level decisions are absorbed into this ADR for traceability

Standard LoadBalancer on Both Providers

Decision:

Ingress parity: standard LoadBalancer on both providers (Hetzner LB + Azure LB). Kong behind standard LB delivers identical routing model regardless of provider.

