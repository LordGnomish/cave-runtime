# ADR-PORTAL-AUTH-001: portal authentication gate — cave-auth OIDC first, external IdP federation

**Status:** Accepted
**Date:** 2026-04-23
**Author:** Burak Tartan (demanded the gate), Sonnet (scribe)
**Scope:** Universal (charter-binding)
**Related:** ADR-PORTAL-PERSONAS-001, ADR-CHARTER-001, ADR-GOLDEN-003 (no-backcompat + PQC-ready), cave-auth Keycloak parity work

## Context

Today's cave-portal has **zero authentication**. Anyone reaching port 8080 browses the entire platform surface including ADRs, runtime state, LLM daemon telemetry, and (post ADR-PORTAL-PERSONAS-001) tenant-scoped data. This is unacceptable for any stage of the product — not for dev-mode, not for demo, not for OSS v1, not for production.

Burak (2026-04-23): *"portale elini kolunu sallayarak girememen lazım kendi Keycloak reimplementasyonundan ya da external bir identity provider'dan authenticate ve authorized olman lazım."* Correct and urgent.

## Decision

**Portal requires a valid authenticated session before any page renders. Identity is established via cave-auth (Keycloak reimpl, sovereign default) or a federated external IdP. Authorization uses Cave-native RBAC (ADR-PORTAL-PERSONAS-001 role taxonomy) enforced at the route middleware layer.**

### 1. Identity sources

1. **cave-auth (primary)** — Cave's in-tree Keycloak line-by-line reimpl (per ADR-GOLDEN-001). OIDC issuer + authorization endpoint + token endpoint + JWKS. Self-contained; no external dependency; meets charter madde 3.
2. **External IdP federation (optional, enterprise)** — cave-auth configured as OIDC relying-party to an external IdP: Okta, Entra ID / Azure AD, Auth0, Google Workspace, Keycloak upstream, any SAML 2.0 IdP via SAML→OIDC bridge in cave-auth. Federation is additive; cave-auth is always authoritative for Cave-scoped claims (tenant_id, role bindings).

No other identity path. No HTTP basic. No API keys for portal. No anonymous "dev mode" that ships in a release build.

### 2. Session mechanics

- **Login flow:** `GET /` (unauthenticated) → 302 `/auth/login` → OIDC Authorization Code + PKCE → `/auth/callback?code=...` → session created → redirect to originally-requested URL or persona landing (`/admin/parity` for admin, `/t/{home-tenant}` for tenant).
- **Session storage:** signed + encrypted cookie (HttpOnly, Secure, SameSite=Strict), ≤24h lifetime, sliding expiration. No localStorage tokens.
- **CSRF:** double-submit cookie pattern on every state-changing request; GET is safe.
- **Refresh:** silent refresh via OIDC refresh_token with 30-day rotation; rotation invalidates previous refresh_token.
- **Logout:** `/auth/logout` → RP-initiated logout at IdP + local session destroy. Global logout flag available for admin to force-revoke.

### 3. Post-quantum crypto alignment

Token signing and key exchange primitives follow ADR-GOLDEN-003:
- JWT signing: hybrid ML-DSA + Ed25519 during migration; ML-DSA-only post-migration. No RSA/ECDSA in new deployments.
- TLS: hybrid ML-KEM + X25519. cave-auth rejects classic-only TLS handshakes by default.
- Session cookie AE: ChaCha20-Poly1305 with per-session IV; cookie keys rotated weekly.

### 4. RBAC enforcement

- Every HTTP handler wrapped in `require_role(...)` middleware.
- Roles: the taxonomy from ADR-PORTAL-PERSONAS-001 (`system:*` and `tenant:*` sets).
- Permission check is **deny-by-default.** A route with no explicit policy returns 403. There is no "default allow" fallback.
- Policy source: cave-auth RBAC + ABAC engine (already scaffolded per repo inspection). Policies are reloadable without portal restart.
- Audit: every authorised action emits a structured log line to cave-metrics / cave-trace; denied actions likewise (with reason).

### 5. Public surface

The only routes reachable without authentication:
- `GET /` — landing page with sign-in CTA (no platform data)
- `GET /auth/login`, `GET /auth/callback`, `POST /auth/logout`
- `GET /healthz`, `GET /readyz` — Kubernetes-style liveness/readiness, no payload beyond `{ok:true}`
- `GET /static/*` — portal asset bundle
- `GET /.well-known/openid-configuration` — OIDC discovery (when cave-auth is in-process)

Everything else, including `/api/portal/runtime/progress`, requires authentication.

### 6. SPIFFE identity for service-to-service

All in-cluster calls portal→cave-apiserver / portal→cave-gateway / portal→cave-vault use SPIFFE SVIDs (mTLS with cave-auth's Identity service as trust domain authority). User OIDC token authenticates the user to portal; SPIFFE authenticates portal to backends. Two-hop identity with attribution preserved via impersonation header.

## Rationale

**Why not just basic auth for dev?** Because "dev auth" always leaks into demo, staging, and production. A portal that is public by default is a portal that ships public by accident.

**Why cave-auth primary?** Charter madde 3 (sovereign): Cave must work without external dependency. External IdP federation is an option, not a requirement.

**Why PKCE + httpOnly cookies, not SPA + bearer tokens?** localStorage token in a portal that renders untrusted tenant content is an XSS exfiltration vector. HttpOnly cookies are inaccessible to JS; even a successful XSS cannot steal the session.

**Why deny-by-default?** Because the alternative — audit every route to add explicit deny — is how every auth retrofit has ever failed.

## Consequences

**Immediate (this sprint):**
- cave-portal-api gains `auth` middleware layer wrapping all routers.
- `GET /` redirects to `/auth/login` when unauthenticated.
- cave-auth exposes OIDC endpoints (already partial; complete the discovery + JWKS endpoints).
- Dev workflow gets a seed admin (`admin@cave.local`) with a bootstrap password flagged "change on first login" — documented in CONTRIBUTING.md.
- All current "just hit localhost:8080" docs updated to describe login.

**Pre-OSS (28 days):**
- OIDC client for cave-auth (self-referential RP) completed.
- External IdP federation stub (single Okta profile as reference) tested.
- PQC-hybrid signing in production config (not just feature-flagged).
- Penetration test of auth flow (self-administered checklist; external later).

**Post-OSS:**
- SAML 2.0 connector for enterprise.
- Multi-factor enforcement policy per role.
- WebAuthn / passkey support.
- Step-up auth for high-risk admin actions (delete tenant, change billing, rotate cluster-wide secrets).

## Alternatives considered

1. **Public portal with per-route auth.** Rejected — leaks the existence and shape of every endpoint; attackers enumerate endpoints as an unauthenticated recon step.
2. **API keys for portal.** Rejected — keys get committed to scripts, shared in chat, end up in logs.
3. **Ship without auth for OSS v1, "patch it later".** Rejected categorically — this kind of deferral is how auth never ships. Charter doesn't allow it.
4. **Delegate all identity to external IdP (Auth0/Okta only).** Rejected for charter reasons (sovereign).

## References

- ADR-CHARTER-001 — sovereign platform; no external dependency for core
- ADR-GOLDEN-003 — no-backcompat + PQC-ready (crypto choices bound here)
- ADR-PORTAL-PERSONAS-001 — admin vs tenant persona split; RBAC role taxonomy
- ADR-SELF-IMPROVE-001 — cave-agent also gated by this auth layer
- cave-auth Keycloak parity work (in progress)
- 2026-04-23 user demand: *"portale elini kolunu sallayarak girememen lazım kendi Keycloak reimplementasyonundan ya da external bir identity providerdan authenticate ve authorized olman lazım"*
