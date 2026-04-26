# ADR-RUNTIME-CERT-LIFECYCLE-001 — Sovereign Cert Hierarchy + PQC-Ready + Multi-DNS ACMEv2

Status: Accepted (2026-04-26)
Scope: Cave Runtime (overrides ADR-015 for Runtime context)
Category: Charter / Architecture (Layer 4 Security — Cert Lifecycle)
Related: ADR-015 (Platform — kept intact), ADR-014 (Zero-Trust Network), ADR-RUNTIME-UPSTREAM-MIRROR-001 (override pattern), ADR-RUNTIME-STACK-001 (Cloud OS Layers 1-4)

## Override Notice

ADR-015 (TLS Certificate Lifecycle via cert-manager) **Platform context'te aynen geçerli kalır** — cert-manager + Let's Encrypt ACME + Cloudflare DNS-01 30-day rotation. Bu ADR Cave Runtime context'inde ADR-015'i override eder, sovereign + no-backcompat + PQC-ready charter prensiplerine göre.

ADR-RUNTIME-UPSTREAM-MIRROR-001 override pattern'ine göre yazılır.

## Context

ADR-015 cert-manager-based çözüm Platform deployment'larında (Hetzner SaaS + Azure SaaS) yeterli, mature ekosistem. Ama Cave Runtime sovereign Cloud OS:

1. **Sovereign mandate** — Let's Encrypt + Cloudflare external dependency yok (ya da tenant opt-in only)
2. **PQC-ready** — Charter binding (no-backcompat + PQC-ready). ML-KEM/ML-DSA day-one, hybrid cert chains
3. **Multi-tenant CA isolation** — Her tenant kendi CA chain'ine sahip, cross-tenant trust path yok
4. **Multi-DNS provider** — Cloudflare hard dep yok, tenant DNS provider seçer
5. **Internal ACME server** — `cave-acme` Cave Runtime'ın kendi RFC 8555 ACMEv2 sunucusu

## Decision

### Per-tenant Sovereign CA Hierarchy

```
cave-runtime root CA (offline, hardware-backed key — HSM/TPM)
  └─ cave-platform intermediate CA (cave-vault PKI engine)
       └─ tenant-<id> intermediate CA (per-tenant, automated provisioning)
            ├─ workload leaf certs (1h rotation, mTLS via cave-mesh)
            ├─ ingress leaf certs (24h internal / 60d external opt-in)
            └─ application leaf certs (per-app config)
```

Default: sovereign internal CA. Let's Encrypt path opt-in (sadece public-facing tenant custom domains).

### PQC-Ready Hybrid Certificates

- **Signature**: ML-DSA-65 + Ed25519 hybrid (RFC 8410 + draft-ietf-lamps-pq-composite-sigs)
- **Key exchange**: TLS 1.3 + X25519 + ML-KEM-768 hybrid
- **No-backcompat**: Pre-classical (RSA/ECDSA-only) cert atılır internal endpoints'te
- **External opt-in (Let's Encrypt)**: Pure-classical cert public CA trust isteyen tenant için

### Multi-DNS Provider

`cave-dns` provider trait:
- Cloudflare (mevcut)
- Route53 (AWS multi-cloud)
- Azure DNS (ADR-002 Azure path)
- Hetzner DNS (sovereign Hetzner)
- PowerDNS / BIND (self-hosted full sovereign)
- Plugin trait — yeni provider implementation = trait impl

Tenant config'den DNS provider seçer. Hard dependency yok.

### `cave-acme` — Sovereign RFC 8555 ACMEv2 Server

Yeni crate `cave-acme` Rust reimpl:
- Account management (JWK + EAB)
- Order workflow (newOrder → finalize → cert + revoke)
- Challenge types: DNS-01, HTTP-01, TLS-ALPN-01
- Multi-tenant ACME (per-tenant accounts + orders + rate limits)
- Cert chain from cave-vault PKI engine (per-tenant CA)
- ACMEv2 (RFC 8555) full compliance + RFC 8737 ALPN extensions

### Cert Lifetime Strictness

| Cert type | Platform v1 | Runtime v2 | Reason |
|-----------|-------------|------------|--------|
| Workload mTLS (Istio CA) | 1h | 1h | Aynı |
| Platform internal | 24h | 24h | Aynı |
| Ingress internal CA | 60d (LE) | **24h** | Sovereign cheap rotation |
| Tenant external (opt-in) | 60d | 60d | LE rate limit |
| etcd KMS | 90d | **30d** | PQC adoption pace |
| Sovereign Ledger | 7d | 7d | Aynı |

## Implementation crates

- `cave-certs` — Certificate/Issuer CRD reconciler + multi-DNS solver + PQC hybrid signer
- `cave-acme` (yeni) — RFC 8555 ACMEv2 server reimpl
- `cave-vault` (extension) — root + platform + per-tenant CA hierarchy via PKI engine
- `cave-dns` — pluggable DNS provider trait (Cloudflare/Route53/Azure/Hetzner/PowerDNS)
- `cave-gateway` — Kong cert wiring + dual-cert (PQC hybrid internal + classical Let's Encrypt opt-in)

## Reddedilen Alternatifler (Runtime-specific)

### Pure cert-manager + Let's Encrypt (ADR-015'i tutarsız uygula)
- Cloudflare hard dependency sovereign promise zayıflığı
- Let's Encrypt rate limit (50 cert/hafta) per-tenant intermediate CA için yetersiz
- PQC hybrid Let's Encrypt henüz desteklemiyor — charter binding ihlali
- ADR-015 Platform için doğru, Runtime için değil

### Pure-classical cert chains (Ed25519/ECDSA only)
- Charter PQC-ready binding ihlali
- 2028+ post-quantum threat horizon: pre-deployed certs PQC-vulnerable
- Hybrid maliyeti küçük (~30% cert size, ~5% TLS handshake CPU), benefit kritik

### Single DNS provider hard-coded
- Multi-cloud support kaybolur (Azure path Cloudflare zorlanmamalı)
- Self-hosted DNS + sovereign promise çakışır
- Trait-based design yıllık provider ekleme cost'u sıfır

### `cave-acme`'ı drop, sadece external ACME (Let's Encrypt) kullan
- Sovereign Cloud OS internal endpoint için public CA gerekli değil
- Multi-tenant ACME per-tenant rate limit + isolation external CA'da yok
- Cave kendi CA hiyerarşisinde root-of-trust kontrol etmeli

## Consequences

### Positive
- **Sovereign promise net** — internal endpoints external CA'ya bağlı değil
- **PQC-ready day-one** — ML-DSA + ML-KEM hybrid cert chain'leri shipped
- **Multi-DNS** — tenant cloud preference'a göre çalışır, vendor lock-in yok
- **Multi-tenant CA isolation** — cross-tenant trust path yok, compliance promise net
- **Cheap rotation** — sovereign CA ile ingress 24h, etcd KMS 30d, security postür güçlü
- **External opt-in path** — public CA trust isteyen tenant Let's Encrypt'i opt-in alır

### Negative
- **`cave-acme` impl complexity** — RFC 8555 + multi-tenant + PQC integration
- **PQC adoption browser side** — public-facing endpoints pure-classical fallback (Let's Encrypt) gerekir
- **HSM/TPM bağımlılığı** root CA için (offline + hardware-backed key)
- **CRL + OCSP responder** Cave Runtime kendi infra'sında

### Risks
- **PQC algoritma deprecation (ileride)** — NIST PQC daha sonraki rounds'ta ML-DSA replaced olabilir → cert rotation policy + dual-algorithm transition
- **HSM unavailability** root CA için → break-glass + offline backup procedure (ADR-079)
- **External ACME dependency** Let's Encrypt opt-in path için → Multi-CA fallback (Buypass, ZeroSSL)

## Compliance

- **NIST PQC FIPS 203 (ML-KEM)** + **FIPS 204 (ML-DSA)** — PQC-ready charter binding
- **NIST 800-207 Zero-Trust** — workload identity (cert-based) + sovereign CA chain
- **SOC2 CC6.6 + ISO A.8.24** — encryption in transit + cryptographic controls
- **NIS2 Art.21** — encryption + identity verification
- **GDPR Art.32** — security of processing (encryption)

## Implementation Phases

**v0.1 (this OSS launch — 21 May 2026):**
- `cave-certs` MVP: Certificate/Issuer CRD reconciler + DNS-01 (Cloudflare + Hetzner DNS)
- `cave-vault` extension: per-tenant CA chain provisioning
- PQC hybrid signer: ML-DSA-65 + Ed25519 (basic)
- Cert lifetime enforcement (1h workload, 24h ingress)

**v0.2 (post-launch):**
- `cave-acme` full RFC 8555 server
- Additional DNS providers (Route53, Azure DNS, PowerDNS)
- HSM/TPM root CA
- TLS-ALPN-01 challenge support

**v0.3:**
- CRL + OCSP responder
- Multi-CA fallback for external opt-in
- PQC algorithm transition tooling (cert-rotation when ML-DSA replaced)
