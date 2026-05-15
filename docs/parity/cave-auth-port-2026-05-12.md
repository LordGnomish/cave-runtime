# cave-auth parity — 2026-05-12 audit

**Upstream:** `keycloak/keycloak v22.0.0` (Apache-2.0).

## Methodology

Standard cave-etcd pattern. Keycloak is a ~500K-LOC Java codebase,
much of which is JVM-stack glue (Quarkus, WildFly, FreeMarker
themes, JCE crypto bridges, JEE adapters). The inventory focuses
on Keycloak's **domain packages** — `services/`, `model/`,
`federation/`, `oidc/`, `saml/`, `admin-rest/` — and aggressively
skips Java-stdlib analogs.

## Counts

| Bucket   | Count |
|----------|------:|
| Mapped   | 12 |
| Skipped  | 16 |
| Unmapped | 9 |
| **Total** | **37** |
| **fill_ratio** | **0.7568** |

## What lands in the inventory

* **Mapped (12)** covers the OIDC core (tokens, discovery, JWKS,
  sessions), RBAC/ABAC, the Realm + Client resource models,
  audit-event emission, SCIM 2.0 user/group provisioning, the
  auth/JWT middleware, PAT issuance, multi-tenancy, and the Okta
  adapter.
* **Skipped (16)** covers the JVM runtime (Quarkus, WildFly), UI
  layer (Admin UI + Account UI — cave-portal serves these), themes,
  the Kubernetes operator, JEE/Spring adapters, Java internal SPI
  plumbing, JCE crypto bridge, test suites, Maven build, and docs.
* **Unmapped (9)** covers the honest production blockers: SAML 2.0
  protocol, UMA 2.0 Authorization Services, LDAP federation
  (distinct from auth/ldap.rs), Kerberos / SPNEGO, identity-broker
  for external IdPs, email event listener, JPA persistence (cave-
  auth is in-memory today), uncommon OAuth2 grant types, and
  full WebAuthn / passkey support.

## What this PR does NOT claim

* The 76% fill_ratio is NOT a 76% feature-parity claim against a
  production Keycloak. It's the fraction of Keycloak's top-level
  packages either mapped or honestly skipped — a third (16/37) is
  JVM-stack glue + UI that doesn't belong in a Rust port.
* The 9 unmapped entries are real gaps. SAML and JPA persistence
  in particular are the biggest production blockers for "drop-in
  Keycloak replacement" claims.
