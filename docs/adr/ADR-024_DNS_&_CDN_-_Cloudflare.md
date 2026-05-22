# ADR-024: DNS & CDN — Cloudflare

**Status:** Accepted

**Scope:** Universal

**Category:** Infrastructure

**Related ADRs:** 015, 066 | Absorbs: ADR-065

## Context

CAVE needs a DNS provider for caveplatform.dev and all tenant subdomains. Must support: DNS-01 ACME challenge (for cert-manager), geographic failover (for multi-provider), DDoS protection, and fast global resolution.


## Candidates

| Criteria | Cloudflare | AWS Route 53 | Azure DNS | sovereign DNS |
|---|---|---|---|---|
| DNS-01 ACME | ✅ cert-manager Cloudflare solver | ✅ | ✅ | ⚠️ Limited API |
| Geographic failover | ✅ Load balancing + health checks | ✅ | ✅ Traffic Manager | ❌ |
| DDoS protection | ✅ Enterprise-grade (free tier) | ⚠️ Shield (paid) | ⚠️ DDoS Protection | ❌ |
| API maturity | ✅ Excellent (Terraform/OpenTofu provider) | ✅ | ✅ | ⚠️ |
| Provider independence | ✅ Third-party (not Hz or Az) | ❌ AWS-coupled | ❌ Azure-coupled | ❌ Hz-coupled |
| Free tier | ✅ Generous (unlimited DNS, basic proxy) | ❌ Per-query pricing | ❌ Per-zone pricing | ✅ Basic |


## Decision

**Cloudflare** for DNS, DDoS protection, and geographic failover. Provider-independent (not coupled to sovereign cloud or hyperscaler). Supports cert-manager DNS-01 challenge. OpenTofu manages DNS records (Day 0). TTL reduced to 60s before cross-provider migration cutover.


## Rejected Options

### AWS Route 53 — Rejected

**Primary:** AWS-coupled. Route 53 requires an AWS account and IAM credentials. CAVE runs on sovereign cloud and hyperscaler — introducing AWS solely for DNS creates a third cloud dependency with its own billing, IAM, and operational surface. Cloudflare is genuinely provider-independent.

**Secondary:** Per-query pricing. Route 53 charges per million DNS queries. Cloudflare's unlimited DNS queries (free tier) is more predictable for a platform with many tenant subdomains.

### Azure DNS — Rejected

**Primary:** Azure-coupled. Same reasoning as Route 53 — DNS should be independent of compute providers so that sovereign ↔ hyperscaler failover is controlled by a neutral third party, not by one of the providers being failed-over from.

### sovereign DNS — Rejected

**Primary:** Limited API and no geographic failover. sovereign DNS has no health check or geographic routing capability. Cannot automatically redirect traffic from the sovereign profile to Azure during a sovereign-cloud outage. Also no DDoS protection — critical for public-facing platform endpoints.


## Consequences

**Positive:**
- Provider-independent DNS — no cloud-vendor coupling.
- Free DDoS protection for all CAVE endpoints.
- Geographic health checks enable sovereign ↔ hyperscaler failover.
- cert-manager integration proven and stable.
- Fast global anycast resolution.

**Negative:**
- Cloudflare dependency (single external provider for DNS). If Cloudflare has major outage, all CAVE endpoints unreachable. Mitigated: Cloudflare 100% SLA, multi-region anycast.
- Cloudflare API token is in Break-glass Kit (ADR-079) — critical credential.
- Cloudflare proxying adds ~10ms latency (optional — can use DNS-only mode).

## Secondary DNS Strategy

Cloudflare is a single point of failure for all CAVE DNS resolution. A secondary DNS provider mitigates total DNS outage risk.

**Architecture:**
- **Primary:** Cloudflare (authoritative nameservers, DDoS protection, geographic routing)
- **Secondary:** NS1 Free Tier (secondary DNS zone transfer via AXFR/IXFR)
- **Failover trigger:** If Cloudflare health check fails for 15 minutes, update registrar NS records to point to NS1

**Implementation:**
1. Configure Cloudflare as primary zone master
2. NS1 as secondary — receives zone transfers automatically
3. Registrar (e.g., Gandi) holds both NS sets: `ns1.cloudflare.com` + `dns1.p01.nsone.net`
4. Normally Cloudflare responds (lower latency, DDoS protection)
5. If Cloudflare fails, NS1 continues serving cached zone data
6. cave-ctl DNS failover command: switches registrar NS priority (automated via registrar API)

**Cost:** NS1 Free Tier supports 1 zone, 500K queries/month — sufficient for CAVE. If exceeded, NS1 Standard ($50/mo).

**Limitations:**
- NS1 secondary does not provide Cloudflare's DDoS protection or geographic routing
- Zone transfer delay: up to 5 minutes for changes to propagate to NS1
- Failover is not instant — DNS TTL (60s) + NS propagation (minutes to hours depending on resolvers)

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

Ingress parity: standard LoadBalancer on both providers (sovereign-cloud LB + Azure LB). Kong behind standard LB delivers identical routing model regardless of provider.

