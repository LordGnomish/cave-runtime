# ADR-007: Okta + Entra ID for Azure Identity

**Status:** Accepted

**Scope:** Azure

**Category:** Identity

**Related ADRs:** 006 (Keycloak/Hetzner), 064 (Identity Split), 104 (Identity Lifecycle), 129 (BYOID Federation), 130 (PAM)

## Context

CAVE's Azure profile needs enterprise-grade identity management for two distinct scopes:

1. **Application identity (OIDC):** All platform and tenant applications need centralized SSO, MFA, and SCIM-based user provisioning. The identity provider must support BYOID federation (ADR-129) for enterprise tenants bringing their own IdP.
2. **Azure resource identity (RBAC):** Kubernetes clusters, managed services (PostgreSQL, Redis, Key Vault), and CI/CD pipelines need Azure-native RBAC for resource access — service principals, managed identities, and role assignments.

The target organization uses Microsoft 365 + Entra ID for corporate identity. Any solution must integrate cleanly with this existing ecosystem.

---

## Candidates

| Criteria | Okta + Entra ID | Auth0 | Ping Identity | Entra ID Only | Keycloak (self-hosted) |
|---|---|---|---|---|---|
| **Enterprise OIDC** | Okta (best-in-class workforce SSO, 7K+ app integrations) | Strong CIAM, weaker workforce | Strong workforce | Limited non-MS app integration | Full OIDC (self-managed) |
| **Azure RBAC** | Entra ID (native — service principals, managed identity, conditional access) | Requires custom bridge to Entra | Requires custom bridge | Native (primary purpose) | No Azure RBAC (separate system) |
| **MFA** | Okta Verify (push, TOTP, FIDO2, WebAuthn, number matching) | Auth0 Guardian (push, TOTP) | PingID (push, TOTP, FIDO2) | MS Authenticator (push, TOTP, FIDO2, passkeys) | Keycloak WebAuthn (FIDO2, TOTP) |
| **Passkey/FIDO2** | ✅ Okta FastPass + FIDO2 (passwordless roadmap active) | ✅ WebAuthn support | ✅ FIDO2 | ✅ Passkeys (strong Microsoft push) | ⚠️ WebAuthn basic (less polished UX) |
| **SCIM provisioning** | Okta→Entra, Okta→apps (bi-directional lifecycle) | Auth0→apps (limited) | Ping→apps | Entra→apps only (no reverse) | SCIM provider (needs custom) |
| **IdP brokering (BYOID)** | Okta brokered SAML/OIDC (enterprise tenants bring their IdP) | Auth0 enterprise connections | Ping federation | Entra external identities (B2B) | Full brokering (ADR-006) |
| **Workforce vs CIAM** | Workforce-focused (core strength) | CIAM-focused (customer identity) | Workforce | Workforce (MS ecosystem locked) | Both (flexible) |
| **Pricing** | ~€8-15/user/mo (Workforce Identity) | ~€23/user/mo (Enterprise) | ~€6-12/user/mo | Included with M365 E3/E5 | Free (self-hosted, ops cost) |
| **Uptime SLA** | 99.99% | 99.99% | 99.99% | 99.99% (with M365) | Self-managed (depends on infra) |
| **Security track record** | ⚠️ 2023 support breach, 2024 credential stuffing incident | Clean (Okta-owned since 2021) | Clean | Clean (Microsoft scale) | Self-hosted (your responsibility) |
| **License** | Commercial (per-user) | Commercial (per-user) | Commercial (per-user) | Included with M365 | Apache 2.0 |

---

## Decision

**Okta Workforce Identity** as the master OIDC provider for all application authentication + **Entra ID** exclusively for Azure resource RBAC. Strict separation per ADR-064: Okta owns app identity, Entra owns Azure infrastructure identity. No overlap.

**Configuration:**
- Okta: OIDC issuer for all platform and tenant applications. SCIM provisioning to downstream apps. BYOID via Okta IdP brokering (SAML 2.0 + OIDC).
- Entra ID: Azure RBAC role assignments, service principals for CI/CD, managed identities for AKS workloads. Conditional Access policies for Azure Portal access.
- SCIM sync: Okta → Entra ID for user lifecycle (create, update, deactivate, deprovision).

---

## Rejected Options

### Auth0 — Rejected

**Primary:** CIAM-focused, not workforce. Auth0 is designed for customer-facing authentication (login boxes for SaaS apps). Workforce SSO features (device trust, desktop SSO, Active Directory integration) are secondary. CAVE needs workforce identity for platform engineers and tenant admins — not customer login pages.

**Secondary:** Pricing. Auth0 Enterprise at ~€23/user/mo is 50-100% more expensive than Okta for equivalent workforce features. Auth0 was acquired by Okta in 2021 — using Auth0 alongside Okta creates redundancy within the same vendor.

### Ping Identity — Rejected

**Primary:** Smaller integration ecosystem. Okta has 7000+ pre-built app integrations vs Ping's ~1800. For a platform that needs to onboard diverse tenant applications quickly, breadth of integrations matters. Ping's Entra ID integration requires more custom configuration than Okta's native connector.

**Secondary:** Less momentum in FIDO2/passkey space compared to Okta FastPass and Microsoft passkeys. Ping is solid but not leading the passwordless transition.

### Entra ID Only — Rejected

**Primary:** Weak non-Microsoft app integration. Entra ID excels at Azure RBAC and M365 SSO but its OIDC support for non-Microsoft applications is limited. Custom app registrations in Entra require manual claim mapping, no SCIM auto-provisioning to non-MS apps, and limited IdP brokering for BYOID (Entra B2B is invite-based, not true brokered federation).

**Secondary:** Vendor lock-in. Using Entra ID for everything couples the identity layer entirely to Microsoft. If CAVE needs to support a non-Azure profile in the future, Okta is portable — Entra is not.

### Keycloak (self-hosted) — Rejected for Azure

**Primary:** Operational burden on a managed cloud profile. CAVE already runs Keycloak on Hetzner (ADR-006) where self-hosting is the only option. On Azure, managed identity services (Okta SaaS + Entra native) eliminate the operational overhead of running, patching, scaling, and backing up a Keycloak cluster. The Azure profile should maximize managed services.

---

## Consequences

### Positive

- Clean identity separation: Okta = apps, Entra = Azure RBAC. No overlap, no confusion.
- 7000+ Okta app integrations accelerate tenant onboarding.
- SCIM lifecycle automation: user created in Okta → provisioned in Entra + all apps → deactivated everywhere on offboard.
- Okta FastPass + FIDO2 enables passwordless roadmap.
- Entra Conditional Access provides Azure-native security policies (device compliance, location-based access, risk-based auth).
- BYOID via Okta brokering: enterprise tenants connect their corporate IdP (SAML/OIDC) without touching CAVE's core identity config.

### Negative

- Okta licensing cost (~€8-15/user/mo). Scales linearly with platform + tenant user count.
- Two identity systems to manage (Okta + Entra). SCIM sync must be monitored for failures/drift.
- Okta's security track record (2023 breach) requires ongoing vendor risk assessment.
- SCIM sync latency: user changes in Okta take 5-40 minutes to propagate to Entra (near-real-time not guaranteed).
- Team needs expertise in both Okta admin console and Entra ID portal.

### Risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Okta security breach (repeat of 2023) | Medium | High | Enable Okta ThreatInsight. Monitor Okta System Log via SIEM. Enforce hardware FIDO2 keys for admin accounts. Okta admin access via PAM (ADR-130). Annual vendor security review. |
| SCIM sync failure (Okta→Entra drift) | Medium | Medium | SCIM sync monitoring in observability stack (ADR-029). Alert on sync errors. Reconciliation job runs daily. Manual override via cave-ctl identity commands. |
| Okta pricing increase | Medium | Medium | Annual contract negotiation. Keycloak on Hetzner is fallback for non-Azure workloads. Evaluate open-source alternatives annually. |
| Passkey ecosystem fragmentation | Low | Low | **Watch:** Passkey/FIDO2 standards are converging but platform implementations differ (Apple vs Google vs Microsoft). Okta FastPass + Entra passkeys provide dual coverage. Re-evaluate passwordless strategy annually as standards mature. |
| Okta acquires/deprecates workforce product | Very Low | High | OIDC is standard protocol — migration to any OIDC provider (Keycloak, Zitadel, Ping) requires reconfiguring clients, not rewriting apps. 90-day migration feasible. |

---

## Compliance Mapping

**SOC2 CC6.1-6.3:** Enterprise identity management — Okta OIDC for centralized authentication, Entra RBAC for Azure resource access control, SCIM for automated provisioning/deprovisioning.
**ISO A.5.15-17:** Access control — Okta MFA (FIDO2, push, TOTP), Entra Conditional Access for risk-based authentication.
**ISO A.5.18:** Access rights — SCIM lifecycle automation ensures timely deprovisioning on employee offboard.
**NIS2 Art.21:** Access control — enterprise-grade identity with MFA and federation.
**GDPR Art.32:** Security of processing — centralized authentication reduces credential sprawl.
