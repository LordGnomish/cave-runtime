# ADR-007: Okta + Entra ID for Azure Identity

**Status:** Accepted

**Scope:** Azure

**Category:** Identity

**Related ADRs:** 006, 064, 104, 129, 130

## Context

Azure profile needs enterprise identity. The target organization uses Microsoft 365 + Entra ID. Platform needs OIDC for all applications + Azure RBAC for resource access.


## Candidates

| Criteria | Okta + Entra ID | Auth0 | Ping Identity | Entra ID only |
|---|---|---|---|---|
| Enterprise OIDC | Okta (best-in-class) | Strong | Strong | Limited app integration |
| Azure RBAC | Entra ID (native) | Requires bridge | Requires bridge | Native |
| MFA | Okta Verify (push, TOTP, FIDO2) | Auth0 Guardian | PingID | MS Authenticator |
| SCIM provisioning | Okta→Entra, Okta→apps | Auth0→apps | Ping→apps | Entra→apps only |
| Workforce vs customer | Workforce-focused | Customer-focused (CIAM) | Workforce | Workforce (MS ecosystem) |
| License | Per-user, enterprise | Per-user, expensive at scale | Per-user, enterprise | Included with M365 |
| Okta integration depth | Native | Separate product | Separate | Competitor |


## Decision

**Okta Workforce Identity** (master OIDC for all apps) + **Entra ID** (Azure resource RBAC only). No overlap (ADR-064).


## Rejected Options

- **Auth0:** Designed for CIAM (customer identity), not workforce. Overkill for internal platform identity. Expensive at scale.
- **Ping Identity:** Capable but smaller ecosystem than Okta. Less native integration with Entra ID.
- **Entra ID only:** Would work for Azure RBAC but OIDC capabilities for non-Microsoft apps are weaker than Okta. Keycloak replacement parity impossible.


## Consequences

(+) Clean separation (Okta=apps, Entra=Azure RBAC), SCIM sync, Okta MFA, enterprise compliance creds.
(-) Okta licensing cost, two identity systems to manage, SCIM sync latency.

---

Compliance Mapping

SOC2 CC6.1-6.3 (enterprise identity management — Okta OIDC for app authentication). ISO A.5.15-17 (access control — Okta MFA, Entra RBAC for Azure resources). ISO A.5.18 (access rights — SCIM provisioning). NIS2 Art.21 (access control — enterprise-grade identity). GDPR Art.32 (security of processing — centralized authentication).

