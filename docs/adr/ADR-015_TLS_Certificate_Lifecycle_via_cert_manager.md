# ADR-015: TLS Certificate Lifecycle via cert-manager

**Status:** Accepted

**Category:** Security

**Related ADRs:** 014, 027

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## CAVE needs automated TLS certificate provisioning and renewal for all external-facing services (Kong ingress, Backstage, Grafana, ArgoCD, Harbor) and internal services (mTLS via Istio uses its own CA — this ADR covers non-mesh TLS).

## Candidates

## | Criteria | cert-manager | Manual certificate management | AWS ACM / Azure App Gateway | Caddy auto-TLS |
|---|---|---|---|---|
| K8s native | ✅ CRD-based (Certificate, Issuer) | ❌ | ❌ Cloud-specific | ❌ |
| ACME support | ✅ Let's Encrypt (DNS-01 via Cloudflare) | ❌ Manual CSR | N/A | ✅ |
| Auto-renewal | ✅ 30 days before expiry | ❌ Manual | ✅ | ✅ |
| Multiple issuers | ✅ Let's Encrypt, self-signed, Vault PKI | ❌ | ❌ One per cloud | ❌ |
| Wildcard certs | ✅ via DNS-01 challenge | ❌ Expensive | ⚠️ | ✅ |
| License | Apache 2.0 | N/A | Proprietary | Apache 2.0 |
| Community | Very large (CNCF Graduated, jetstack) | N/A | N/A | Large |

## Decision

## **cert-manager** (CNCF Graduated) for all TLS certificate lifecycle. Let's Encrypt ACME with DNS-01 challenge via Cloudflare (ADR-024). 30-day certificate rotation. Wildcard certificates for `*.caveplatform.dev` and `*.tenant.caveplatform.dev`. Internal PKI via OpenBao PKI engine for non-ACME use cases.

## Rejected

## - **Manual certificate management:** Human error, missed renewals, compliance gaps. Certificate expiry is a top cause of outages.
- **Cloud-specific (ACM/Azure App Gateway):** Not portable across providers. Contradicts zero-vendor-lock-in principle.
- **Caddy auto-TLS:** Caddy is a web server, not a K8s certificate manager. Would require running Caddy alongside Kong — unnecessary complexity.

## Consequences

## **Positive:**
- Automated certificate provisioning and renewal — zero manual certificate management.
- Let's Encrypt = free, trusted CA. DNS-01 challenge works behind firewalls (no HTTP-01 port exposure).
- 30-day rotation reduces certificate exposure window.
- cert-manager CRDs are GitOps-managed (ArgoCD reconciles).
- AI Compliance Officer monitors certificate expiry preemptively (ADR-112).

**Negative:**
- Cloudflare DNS dependency for DNS-01 challenge. Cloudflare outage → new certificate issuance blocked (existing certs remain valid until expiry).
- Let's Encrypt rate limits (50 certs/week per registered domain). Sufficient for CAVE but requires planning during initial deployment.
- cert-manager upgrade path must be validated (CRD changes can break existing Certificate resources).

## Compliance Mapping

## SOC2 CC6.6 (encryption in transit). ISO A.8.24 (cryptographic controls — certificate lifecycle). ISO A.5.14 (information transfer — TLS). NIS2 Art.21 (encryption). GDPR Art.32 (security of processing — encryption).
