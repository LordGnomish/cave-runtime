# ADR-006: Keycloak for Hetzner Identity Provider

**Status:** Accepted

**Scope:** Hetzner

**Category:** Identity

**Related ADRs:** 007, 064, 104, 129

## Context

CAVE Hetzner profile needs a self-hosted OIDC/SAML identity provider for user authentication, tenant RBAC, and federation. The provider must support multi-tenancy (realm-per-tenant), IdP brokering for BYOID (ADR-129), and SCIM for user lifecycle management.


## Candidates

| Criteria | Keycloak | Authentik | Zitadel | Authelia |
|---|---|---|---|---|
| Protocol support | OIDC, SAML 2.0, LDAP, SCIM | OIDC, SAML, LDAP, SCIM | OIDC, SAML | OIDC only |
| Multi-tenancy | Realms (native, isolated config per tenant) | Tenants (native) | Organizations | No native multi-tenancy |
| Custom extensions | Java SPI (custom auth flows, token mappers) | Python blueprints | No custom | No custom |
| Federation / IdP brokering | Full: SAML, OIDC, social, LDAP | IdP brokering (OIDC, SAML) | Limited federation | Limited |
| K8s operator | Official Keycloak Operator (auto-upgrade, realm import) | Helm only (no operator) | Helm only | Helm only |
| Admin UI | Mature, full-featured (realm management, user management, client config) | Modern, clean (fewer features) | Modern (organization-focused) | Minimal (proxy config) |
| Community | Very large (Red Hat backed, CNCF adjacent, 10+ years, 23K+ GitHub stars) | Growing (~2K stars, 3+ years) | Growing (~5K stars, 3+ years) | Moderate |
| Production maturity | Very high (Fortune 500 deployments) | Moderate (startup/SMB adoption) | Moderate (startup adoption) | Low for enterprise |
| Resource footprint | Heavy (~1-2GB RAM per instance) | Moderate (~512MB-1GB) | Moderate (~512MB) | Light (~128MB) |
| License | Apache 2.0 | MIT-like (authentik-enterprise is commercial) | Apache 2.0 | Apache 2.0 |


## Decision

**Keycloak** (self-hosted on K8s via Keycloak Operator, PostgreSQL backend via CNPG). Realm-per-tenant for multi-tenant isolation. cave_uid custom token mapper via Java SPI. BYOID via SAML 2.0 / OIDC IdP brokering.


## Rejected Options

- **Authentik:** Strong modern alternative but smaller community (2K vs 23K stars). Less battle-tested for enterprise SAML federation scenarios (complex claim mappings, token transformation). No Java SPI — Python blueprints are less powerful for custom authentication flows needed for cave_uid token mapping. Would be acceptable as a backup if Keycloak community stagnated.
- **Zitadel:** Excellent modern design but limited federation capabilities. No SAML IdP brokering — this is a hard blocker for BYOID (ADR-129) where enterprise tenants federate via SAML 2.0. Organization model is less flexible than Keycloak realms for CAVE's isolation requirements.
- **Authelia:** OIDC-only (no SAML). No multi-tenancy. No IdP brokering. Designed as authentication proxy, not full identity provider. Too limited for enterprise platform with BYOID requirement.


## Consequences

**Positive:**
- Proven at enterprise scale (10+ years, Fortune 500 deployments).
- Rich federation for BYOID — SAML 2.0 + OIDC brokering covers all enterprise IdP types.
- Realm-per-tenant provides complete isolation (users, clients, roles, IdP configs).
- Official K8s Operator with automated realm import/export.
- Java SPI enables cave_uid custom token mapper for stable cross-IdP identity.
- SCIM support for user lifecycle management.

**Negative:**
- Resource-intensive (~1-2GB RAM per instance, HA requires 2+ instances = 4GB minimum).
- Java stack — team must understand Java SPI for custom extensions.
- Admin UI learning curve (powerful but complex).
- Upgrade path requires careful realm migration testing (realm export → upgrade → import → validate).
- Red Hat upstream changes (Quarkus rewrite completed, but future direction is Red Hat-influenced).

Compliance Mapping

SOC2 CC6.1-6.3 (identity management, authentication, access provisioning). ISO A.5.15-17 (access control policy, identity management, authentication). ISO A.5.18 (access rights). GDPR Art.32 (security of processing — centralized identity). NIS2 Art.21 (access control policies).

