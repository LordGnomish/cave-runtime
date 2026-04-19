# ADR-129: Tenant Identity Federation — BYOID

**Status:** Accepted

**Scope:** Azure, Hetzner, Runtime, Universal

**Category:** Identity

**Related ADRs:** 006, 007, 064

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## Enterprise tenants (Hard/Dedicated tiers) want to authenticate their users via their corporate IdP (Okta, Azure AD, Google Workspace, etc.) rather than creating separate platform accounts.

## Candidates

## | Approach | BYOID via IdP brokering | Platform-managed only | Direct federation passthrough |
|---|---|---|---|
| Tenant UX | ✅ Corporate SSO | ❌ Separate credentials | ✅ Corporate SSO |
| Platform governance | ✅ Keycloak/Okta mediates | ✅ Full control | ❌ No mediation |
| Identity portability | ✅ cave_uid stable | ✅ cave_uid | ❌ IdP-specific sub |

## Decision

## Hard/Dedicated tiers: BYOID via SAML 2.0 or OIDC. Tenant IdP federated into Keycloak (Hetzner) / Okta (Azure) as external identity provider. Soft tier: platform-managed only. Canonical identity: `cave_uid` (UUID). Token claims: sub (IdP-specific), cave_uid (stable), tenant_id, env. Apps MUST use cave_uid, never sub. Federation scope: tenant-scoped roles only — never platform admin.

## Rejected

## - **Platform-managed only:** Enterprise tenants must create/manage separate accounts. Password fatigue. No corporate SSO. Reduces adoption.
- **Direct federation passthrough:** Tenant IdP directly trusted by platform services. No mediation layer. Platform cannot enforce MFA, session policy, or audit. Tenant IdP compromise directly compromises platform.

## Consequences

## **Positive:**
- Enterprise tenants use corporate SSO. No separate credentials.
- Keycloak/Okta mediates — platform enforces MFA, session policy, audit regardless of tenant IdP.
- cave_uid survives IdP migration (tenant changes from Azure AD to Google Workspace → cave_uid unchanged).
- Federation scoped to tenant roles — no cross-tenant or platform-admin access.

**Negative:**
- Federation configuration per tenant IdP (each has different SAML/OIDC quirks).
- cave_uid mapping must be maintained and backed up.
- Tenant IdP outage affects tenant logins (mitigated: Keycloak/Okta session caching).
- SCIM sync adds complexity if tenant wants user provisioning from their IdP.

## Compliance Mapping

## SOC2 CC6.1-6.3 (federated access controls). ISO A.5.16 (identity management). ISO A.5.17 (authentication — federated SSO). GDPR Art.32 (security of processing — SSO reduces credential surface).
