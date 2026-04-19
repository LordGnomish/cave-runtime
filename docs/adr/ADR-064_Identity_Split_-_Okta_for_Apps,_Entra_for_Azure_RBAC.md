# ADR-064: Identity Split — Okta for Apps, Entra for Azure RBAC

**Status:** Accepted

**Scope:** Azure

**Category:** Identity

**Related ADRs:** 007

Status:

Category:

Identity

Related ADRs:

007

Back to Index:

## Context

Azure profile has two identity systems: Okta (Workforce Identity) and Entra ID (native Azure). Without clear boundaries, both could handle app authentication, creating overlap, confusion, and security gaps.


## Candidates

| Single IdP (Okta only) | Single IdP (Entra only) | Split (Chosen) |
|---|---|---|
| Cannot do Azure resource RBAC natively | OIDC for non-MS apps is weaker | Clean separation |
| Would need custom Azure RBAC bridge | Would lose Okta MFA/SCIM quality | Best of both |
| Okta has no Azure Subscription role concept | Entra app registration per app = sprawl | Each does what it's best at |


## Decision

**Strict, no-overlap separation:**
- **Okta** = OIDC/SAML for ALL applications: Backstage, ArgoCD, Grafana, Harbor, Kong, LiteLLM, LibreChat, Teleport, all tenant apps.
- **Entra ID** = Azure resource RBAC ONLY: Subscription-level roles, Resource Group permissions, Managed Identity assignments, Key Vault access policies.

No Entra ID App Registrations for platform applications. No Okta for Azure resource-level access.


## Rejected Options

- **Okta only (no Entra):** Okta cannot do Azure resource-level RBAC (Subscription roles, Resource Group permissions, Managed Identity assignments). Would need custom Azure RBAC bridge.
- **Entra only (no Okta):** Entra ID OIDC for non-Microsoft apps is weaker than Okta. App Registration sprawl (one per platform app). Keycloak parity impossible.
- **Overlapping (both do apps):** Conflicting auth policies, confusing for developers (which IdP for which app?), security gaps in overlap zones.


## Consequences

(+) Single source of truth for app identity (Okta). Clean RBAC for Azure resources (Entra). No overlapping policies. SCIM sync keeps user lifecycle consistent.
(-) Two identity systems. Developers must understand which handles what. SCIM sync adds latency. Debugging auth issues requires checking both systems.

Compliance Mapping

SOC2 CC6.1-6.3 (clear access control boundaries), ISO A.5.15 (access control policy).

