# ADR-015: TLS Certificate Lifecycle via cert-manager

**Status:** Accepted

**Scope:** Azure, Runtime, Universal

**Category:** Security

**Related ADRs:** 014, 027

## Context

CAVE needs automated TLS certificate provisioning and renewal for all external-facing services (Kong ingress, Backstage, Grafana, ArgoCD, Harbor) and internal services (mTLS via Istio uses its own CA — this ADR covers non-mesh TLS).

## Candidates

| Criteria | cert-manager | Manual certificate management | AWS ACM / Azure App Gateway | Caddy auto-TLS |
|---|---|---|---|---|
| K8s native | ✅ CRD-based (Certificate, Issuer) | ❌ | ❌ Cloud-specific | ❌ |
| ACME support | ✅ Let's Encrypt (DNS-01 via Cloudflare) | ❌ Manual CSR | N/A | ✅ |
| Auto-renewal | ✅ 30 days before expiry | ❌ Manual | ✅ | ✅ |
| Multiple issuers | ✅ Let's Encrypt, self-signed, Vault PKI | ❌ | ❌ One per cloud | ❌ |
| Wildcard certs | ✅ via DNS-01 challenge | ❌ Expensive | ⚠️ | ✅ |
| License | Apache 2.0 | N/A | Proprietary | Apache 2.0 |
| Community | Very large (CNCF Graduated, jetstack) | N/A | N/A | Large |

## Decision

**cert-manager** (CNCF Graduated) for all TLS certificate lifecycle. Let's Encrypt ACME with DNS-01 challenge via Cloudflare (ADR-024). 30-day certificate rotation. Wildcard certificates for `*.caveplatform.dev` and `*.tenant.caveplatform.dev`. Internal PKI via OpenBao PKI engine for non-ACME use cases.

## Rejected Options

### Manual Certificate Management — Rejected

**Primary:** Human error. Certificate expiry is consistently a top-3 cause of production outages (Gartner, Ponemon). A single missed renewal takes down all HTTPS endpoints. With 70+ platform components + tenant services, manual tracking is impossible at scale.

**Secondary:** Compliance gap. SOC2 CC6.6 and ISO A.8.24 require documented cryptographic controls. Manual certificate management lacks audit trail, automated rotation evidence, and expiry monitoring — all of which cert-manager provides via K8s events and Prometheus metrics.

### AWS ACM / Azure App Gateway — Rejected

**Primary:** Not portable. ACM certificates only work with AWS services (ALB, CloudFront, API Gateway). Azure App Gateway certificates only work within Azure. CAVE runs on both Hetzner and Azure (ADR-001, ADR-002) — certificate management must be provider-agnostic. cert-manager works identically on both.

**Secondary:** No self-hosted option. On Hetzner there is no managed certificate service — cert-manager with Let's Encrypt ACME is the only automated path. Using cloud-specific on Azure and cert-manager on Hetzner creates two certificate workflows to maintain.

### Caddy Auto-TLS — Rejected

**Primary:** Wrong abstraction layer. Caddy is a web server/reverse proxy with built-in ACME. CAVE already uses Kong as API gateway (ADR-027). Running Caddy alongside Kong solely for certificate management adds an unnecessary component. cert-manager is purpose-built for K8s certificate lifecycle.

**Secondary:** No CRD integration. Caddy manages its own certificates internally. cert-manager's Certificate/Issuer CRDs are GitOps-manageable (ArgoCD reconciles), auditable, and visible in K8s API. Caddy's certificates are opaque to the K8s ecosystem.

## Certificate Inventory

| Certificate | Issuer | Rotation | Scope |
|---|---|---|---|
|  | Let's Encrypt (ACME DNS-01) | 60 days (30-day pre-renewal) | All external ingress (Kong) |
|  | Let's Encrypt (ACME DNS-01) | 60 days | Tenant custom domains |
| Platform internal TLS | OpenBao PKI engine | 24 hours | Inter-component (non-mesh) |
| Istio mesh mTLS | Istio CA (citadel) | 1 hour | Pod-to-pod (ADR-014) |
| etcd encryption | OpenBao Transit | 90 days | etcd KMS (ADR-105) |
| Sovereign Ledger signing | OpenBao PKI | 7 days | WORM attestation (ADR-093) |

## Consequences

**Positive:**
- Automated certificate provisioning and renewal — zero manual certificate management.
- Let's Encrypt = free, trusted CA. DNS-01 challenge works behind firewalls (no HTTP-01 port exposure).
- 30-day rotation reduces certificate exposure window.
- cert-manager CRDs are GitOps-managed (ArgoCD reconciles).
- AI Compliance Officer monitors certificate expiry preemptively (ADR-112).

**Negative:**
- Cloudflare DNS dependency for DNS-01 challenge. Cloudflare outage → new certificate issuance blocked (existing certs remain valid until expiry).
- Let's Encrypt rate limits (50 certs/week per registered domain). Sufficient for CAVE but requires planning during initial deployment.
- cert-manager upgrade path must be validated (CRD changes can break existing Certificate resources).

### Risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Let's Encrypt outage blocks new certificate issuance | Low | Medium | Existing certificates remain valid (90-day lifetime, 30-day rotation). OpenBao PKI as fallback issuer for emergency certificates. |
| Cloudflare DNS-01 solver fails | Low | Medium | Retry logic in cert-manager. Alternative: switch to HTTP-01 solver temporarily (requires port 80 exposure). |
| cert-manager CRD upgrade breaks existing Certificates | Low | High | Pin cert-manager version. Staging validates upgrade before prod. Backup Certificate resources in WORM. |
| Let's Encrypt rate limit hit during initial deploy | Medium | Low | Pre-plan certificate issuance. Use staging ACME server for testing. Wildcard cert reduces individual cert count. |
| Post-Quantum TLS certificates | Low (2028+) | Low | **Watch:** NIST PQC algorithms finalized. PQ TLS certs are years away from browser/CA adoption. cert-manager will support when available. No action now. |

## Compliance Mapping

SOC2 CC6.6 (encryption in transit). ISO A.8.24 (cryptographic controls — certificate lifecycle). ISO A.5.14 (information transfer — TLS). NIS2 Art.21 (encryption). GDPR Art.32 (security of processing — encryption).
