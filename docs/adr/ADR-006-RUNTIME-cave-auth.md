<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ADR-006-RUNTIME — cave-auth: Sovereign Identity Provider (Keycloak parity) (Runtime variant)

**Status:** Accepted
**Scope:** Cave Runtime (sovereign, self-hosted — provider-agnostic)
**Category:** Identity
**Decided:** 2026-06-07 (Burak Tartan)
**Variant-of:** Platform ADR-006 (Keycloak for Hetzner Identity Provider)
**License:** AGPL-3.0-or-later

## Executive Summary

Platform ADR-006 **Keycloak**'u "Hetzner identity provider" olarak seçti —
self-hosted OIDC/SAML, realm-per-tenant, IdP brokering, SCIM. Karar **universal**;
"Hetzner" çerçevesi tesadüfi. Cave Runtime bu kararı **cave-auth** ile materialize
eder: Keycloak'ın realm/user/token/OIDC/JWKS/RBAC yüzeyinin Rust reimpl'i
([[cave-auth-cont3-2026-05-31]], honest 1.0 manifest-authored).

**cave-auth provider-agnostic'tir.** Hetzner sadece bir *örnek* deployment
hedefidir; AWS/GCP/Azure/bare-metal provider'larda aynı cave-auth çalışır
(ADR-001 provider-agnostic charter). cave-auth ek olarak Platform variant'ta
olmayan **iki sovereign yetenek** sunar:

1. **ABAC** (RBAC'a ek attribute-based access control) — cave-permission ile.
2. **SPIFFE workload identity** — cave-identity ([[cave-identity-honest-cont2-2026-05-31]])
   ile entegre; insan kimliği (OIDC) + workload kimliği (SPIFFE SVID) tek düzlemde.

## Context

### Neden bir Runtime variant gerekli
Platform variant Keycloak'ı **upstream JVM container** olarak (Keycloak Operator +
CNPG Postgres) deploy ediyordu — ~1-2GB RAM/instance, Java SPI bağımlılığı, ve
"Hetzner" özelinde çerçevelenmiş. Cave Runtime sovereign Cloud OS'tür: identity
provider **in-binary** (cave-auth Rust crate), provider-agnostic, ve workload
identity (SPIFFE) ile birleşik olmalı. Ayrıca insan-kimliği + makine-kimliği tek
sovereign düzlemde toplanır (cave-auth OIDC + cave-identity SPIFFE).

### Korunan değer
Keycloak'ın realm-per-tenant izolasyonu, OIDC + SAML 2.0 federation/IdP brokering,
SCIM lifecycle, custom token mapper (cave_uid eşdeğeri) — hepsi cave-auth'ta port
edilmiş (mapped 27, partial 1, honest 0.9773).

## Candidates

| Kriter | **Keycloak** (→ cave-auth) | Authentik | Zitadel | Authelia |
|---|---|---|---|---|
| Protokol | OIDC, SAML 2.0, LDAP, SCIM | OIDC, SAML, LDAP, SCIM | OIDC, SAML | OIDC only |
| Multi-tenancy | Realms (native, izole config) | Tenants | Organizations | ❌ yok |
| Custom extension | Java SPI (cave-auth: Rust mapper) | Python blueprint | yok | yok |
| Federation / IdP brokering | Full (SAML/OIDC/social/LDAP) | OIDC/SAML | sınırlı | sınırlı |
| K8s operator | Official Keycloak Operator | Helm | Helm | Helm |
| Community | Çok büyük (Red Hat, 23K★, 10+ yıl) | ~2K★ | ~5K★ | orta |
| Üretim olgunluğu | Çok yüksek (Fortune 500) | orta | orta | düşük |
| License | Apache-2.0 | MIT-like (+ commercial) | Apache-2.0 | Apache-2.0 |

## Decision

**cave-auth** (Keycloak parity, Rust reimpl) Cave Runtime'ın sovereign identity
provider'ıdır. Realm-per-tenant multi-tenant izolasyon; OIDC discovery + JWKS;
RBAC + **ABAC**; SAML 2.0 / OIDC IdP brokering (BYOID); SCIM lifecycle; PKCE;
cave_uid eşdeğeri stable cross-IdP token mapper (Rust, Java SPI yerine).

**Provider-agnostic:** cave-auth Postgres backend'i **cave-rdbms** (PostgreSQL
wire engine, [[cave-rdbms-cont2-2026-05-31]]) üzerinden veya herhangi bir provider
Postgres'iyle çalışır. Hetzner *bir* deployment örneğidir, AWS/GCP/Azure eşit.

### Runtime yükseltmeleri

- **ABAC** — RBAC'ın üstünde attribute-based policy (cave-permission); tenant
  attribute'larına göre fine-grained authorization.
- **SPIFFE workload identity** — cave-identity ile insan (OIDC) + workload
  (SPIFFE SVID) tek kimlik düzlemi; mesh mTLS (cave-mesh, ADR-004-RUNTIME) ve CI
  runner kimliği (ADR-010-RUNTIME) bununla bağlanır.
- **PQC JWS keys** — cave-auth token imzası PQC-hazır anahtarlarla
  (cave-vault hierarchy, ADR-RUNTIME-CERT-LIFECYCLE-001).
- **Single-binary** — cave-auth ayrı JVM değil; cave-runtime'a mount'lu router.

## Rejected

- **Authentik** — güçlü modern alternatif ama küçük community (2K vs 23K★);
  enterprise SAML federation senaryolarında daha az battle-tested; Java SPI yok
  (Python blueprint custom token mapping için daha zayıf). Keycloak community
  durağanlaşırsa yedek.
- **Zitadel** — mükemmel modern tasarım ama **SAML IdP brokering yok** → BYOID
  (enterprise SAML 2.0 federation) için hard blocker. Organization modeli realm'den
  daha az esnek.
- **Authelia** — OIDC-only (SAML yok), multi-tenancy yok, IdP brokering yok.
  Authentication proxy, full IdP değil. Enterprise BYOID için fazla sınırlı.

> Platform variant Keycloak'ı **"Hetzner identity"** olarak çerçeveliyordu;
> Runtime bunu **provider-agnostic cave-auth**'a genelleştirir.

## Consequences

### Olumlu
- Enterprise-ölçek kanıtlı parity (Keycloak 10+ yıl, Fortune 500).
- Zengin federation (SAML 2.0 + OIDC brokering) — tüm enterprise IdP tipleri.
- Realm-per-tenant tam izolasyon (users, clients, roles, IdP config).
- **ABAC + SPIFFE** — insan + workload kimliği tek sovereign düzlem.
- Rust reimpl → JVM'in ~1-2GB RAM yükü yok; single-binary.
- SCIM lifecycle + stable cross-IdP token mapper.

### Olumsuz / maliyet
- Keycloak'ın derin federation edge-case'leri (karmaşık claim mapping) cave-auth'ta
  honest gap olabilir (partial 1; OID4VC JCS substitution gibi).
- Realm migration/upgrade testi disiplin gerektirir (export→upgrade→import→validate).
- SPIFFE entegrasyonu cave-identity olgunluğuna bağlı (partial→mapped yolculuğu
  devam ediyor, [[cave-identity-honest-cont2-2026-05-31]]).

### Riskler & azaltım
- **cave-auth parity drift** → cave-runtime-tracker Keycloak v22.0.0 upstream'ini
  izler; Charter v2 self-audit honest_ratio'yu zorlar.
- **PQC JWS anahtar yönetimi** → cave-vault hierarchy + rotation
  (ADR-RUNTIME-CERT-LIFECYCLE-001).

## Compliance Mapping

- **SOC2 CC6.1-6.3** — identity management, authentication, access provisioning.
- **ISO A.5.15-18** — access control policy, identity/authentication, access rights.
- **GDPR Art.32** — security of processing (merkezi sovereign identity).
- **NIS2 Art.21** — access control policies.

## Charter v2 8-gate linkage

Strict TDD: cave-auth `parity.manifest.toml` (Keycloak v22.0.0, mapped 27 /
total 44, honest 0.9773) Charter v2 self-audit gate'lerine bağlı. ABAC/SPIFFE
yükseltmeleri cave-permission + cave-identity manifestleriyle çapraz-doğrulanır.
Bu ADR fabricated parity iddia etmez (gate_1). `last_audit == 2026-06-07`.

## Related ADRs

- [ADR-001](ADR-001-sovereign-bare-metal-hosting.md) — provider-agnostic charter (Hetzner = örnek)
- [ADR-004-RUNTIME](ADR-004-RUNTIME-cilium-istio.md) — cave-mesh mTLS (SPIFFE SVID consumer)
- [ADR-RUNTIME-CERT-LIFECYCLE-001](ADR-RUNTIME-CERT-LIFECYCLE-001-sovereign-cert-hierarchy-pqc-acme.md) — PQC anahtarlar (JWS)
- [ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001](ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001-multi-upstream-data-layer.md) — cave-rdbms (Postgres backend)
- [ADR-PORTAL-AUTH-001](ADR-PORTAL-AUTH-001.md) — portal authentication (cave-auth consumer)
- **Platform ADR-006** — Keycloak for Hetzner Identity Provider (JVM reference variant)

---

*Bu ADR Platform ADR-006'nın **Runtime sovereign variant**'ıdır. Keycloak →
cave-auth Rust reimpl ile materialize edilmiş; provider-agnostic (Hetzner = örnek),
ABAC + SPIFFE workload identity + PQC JWS eklenmiştir. Cave Runtime AGPL-3.0-or-later.*
