# ADR-055: WAF & DDoS Protection — Cloudflare

**Status:** Accepted

**Scope:** Universal

**Category:** Security

**Related ADRs:** 024, 027

## Context

CAVE's external-facing endpoints (Kong API gateway, Backstage portal, Grafana dashboards) are internet-accessible and need protection against DDoS attacks, bot traffic, and common web application attacks (SQLi, XSS, SSRF). Protection must work across both providers without cloud-specific WAF dependencies.


## Candidates

| Criteria | Cloudflare (chosen) | AWS WAF | Azure WAF | ModSecurity (self-hosted) |
|---|---|---|---|---|
| Provider independence | ✅ Third-party (not Hz or Az) | ❌ AWS only | ❌ Azure only | ✅ Self-hosted |
| L3/L4 DDoS | ✅ Enterprise-grade (free tier) | ✅ Shield Standard (free) | ✅ Basic (free) | ❌ |
| L7 WAF rules | ✅ Managed rulesets | ✅ Managed rules | ✅ OWASP CRS | ✅ OWASP CRS |
| Maintenance | ✅ Cloudflare manages rules | ✅ AWS manages | ✅ Azure manages | ❌ Self-managed rules |
| Existing dependency | ✅ Already used for DNS (ADR-024) | ❌ Additional dependency | ❌ Additional dependency | ✅ No external dep |
| Cost | ✅ Free tier generous | ❌ Per-rule pricing | ❌ Per-rule pricing | ✅ Free |


## Decision

**Cloudflare** provides L3/L4 DDoS protection (free tier) + L7 WAF rules. Already used for DNS (ADR-024) — no additional dependency. Kong rate limiting (ADR-027) provides application-level protection. Two layers: Cloudflare (network/transport/application WAF) + Kong (API-level rate limiting, JWT auth, request validation).


## Rejected Options

- **AWS WAF / Azure WAF:** Cloud-specific. Would need different WAF per provider. Cloudflare covers both providers from one configuration.
- **ModSecurity (self-hosted):** Powerful but self-managed rule maintenance is operational burden. OWASP CRS updates require manual application. Cloudflare provides managed, auto-updated rules.
- **No WAF:** Unacceptable for internet-facing services on a multi-tenant platform.


## Consequences

**Positive:**
- Enterprise DDoS protection on free tier — no additional cost.
- Single WAF configuration covers both providers.
- Cloudflare managed rules auto-update — no rule maintenance.
- Already a dependency (DNS) — no new external service.
- Two-layer protection: Cloudflare (L3-L7) + Kong (API-level).

**Negative:**
- Cloudflare as critical path — all external traffic transits Cloudflare. Major Cloudflare outage = all CAVE endpoints unreachable (mitigated: Cloudflare 100% SLA, multi-region anycast).
- Cloudflare proxy adds ~10ms latency (can use DNS-only mode for internal traffic).
- Free tier WAF rules are limited — Pro/Business tier may be needed for advanced rules (evaluated per demand).

Compliance Mapping

SOC2 CC7.5 (availability — DDoS protection). ISO A.8.22 (network security — WAF). ISO A.8.20 (network security controls — application filtering). NIS2 Art.21 (network protection — DDoS mitigation). GDPR Art.32 (availability of processing systems).

