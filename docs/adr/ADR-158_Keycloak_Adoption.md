# ADR-158 — Keycloak Adoption (cave-keycloak)

- **Status:** Accepted
- **Date:** 2026-05-23
- **Deciders:** Burak Tartan, cave-runtime maintainers
- **Branch:** `claude/cave-keycloak-2026-05-23-deep`
- **Companion crates:** [`cave-auth`] (legacy OIDC middleware — re-points at cave-keycloak issuer URLs), [`cave-portal-ui`] (admin + account console UI), [`cave-vault`] (keychain-backed secrets), [`cave-templates`] (email theme), [`cave-keycloak-ldap-net`] (Phase 2 LDAP wire adapter), [`cave-keycloak-scim`] (Phase 2), [`cave-keycloak-uma`] (Phase 2), [`cave-keycloak-oid4vc`] (Phase 2), [`cave-keycloak-kerberos`] (Phase 2), [`cave-xmlsec`] (Phase 2), [`cave-pqc`] (Phase 2 ML-DSA backend)

## Context

cave-runtime needs an identity provider that:

1. Issues OAuth2 / OIDC tokens for every cave-* HTTP API consumed by the SPA, by service-to-service callers, and by the cave-cli operator.
2. Brokers SSO from enterprise IDPs (Google Workspace, GitHub, Microsoft Entra) and from on-prem LDAP / AD without leaking external secrets into the runtime memory space.
3. Exposes a SAML 2.0 IDP so legacy SaaS apps can keep using cave-keycloak as the corporate login.
4. Stays multi-tenant — one cave deployment must serve many isolated tenants with no cross-tenant data leakage.
5. Carries a PQC-ready signing surface so the JWT alg can rotate to ML-DSA-65 (FIPS 204) without breaking the JWKS publisher or the discovery document.

Pre-port, IAM was split across `cave-auth` (custom OIDC middleware + a `keycloak/` submodule with token + realm + client adapters) and ad-hoc per-service authenticators. That meant:

- No single audit trail for who-did-what across realms.
- No shared admin REST surface — every cave-* crate that needed a user list re-implemented one.
- No PQC story — the JWT signer was hard-coded to RSA + a workspace-local key.
- The brokering scope was Google + GitHub only; Microsoft Entra was a deferred TODO.

## Decision

Adopt **keycloak/keycloak v26.6.2** (Apache-2.0; commit `0a402f777f8985eccbb07556e96d9b386275e048`) as the upstream contract for a new `cave-keycloak` crate. Re-implement the Keycloak IAM control plane in pure Rust against the workspace crypto stack (`ring`, `ed25519-dalek`, `p256`, `sha2`) with a `keychain:`-handle convention for every long-lived secret.

### Module layout (20 src/ modules, ~5929 LOC)

- **Core data:** `models.rs` (Realm + PasswordPolicy + User + Group + Role + Client + GrantType + Protocol + FederatedIdentity + HashAlgorithm), `error.rs`, `store.rs` (multi-tenant in-memory store with `check_tenant` guard).
- **Credentials:** `credentials.rs` (PBKDF2-SHA256/512 password + Argon2-style HKDF + RFC 6238 TOTP with RFC 4648 base32 + WebAuthn assertion verify + magic link), `signer.rs` (ES256 + EdDSA real + ML-DSA-65 placeholder + JWKS shape + compact JWS encode/decode).
- **OAuth2 / OIDC:** `oauth2.rs` (auth code + PKCE + refresh rotation with chain replay revocation + device code + introspection DTO), `discovery.rs` (well-known config), `jwks.rs` (per-realm JWKS), `session.rs` (SSO + offline sessions + token assembly).
- **SAML 2.0:** `saml.rs` (AuthnRequest emit + Response build + SP-side verify with InResponseTo / Destination / Issuer / Audience / time-window / signature-present checks).
- **Federation + brokering:** `ldap.rs` (LdapBackend trait + InMemoryLdap + simple filter parser + search-then-bind authenticate), `brokering.rs` (Google / GitHub / Microsoft OIDC + generic with family-default endpoint table + URL-encoded /authorize builder + JIT user provisioning).
- **Controllers:** `realm.rs`, `user.rs`, `role.rs`, `client_registry.rs`, `auth_flow.rs` (Required / Alternative / Conditional / Disabled flow executor).
- **Policies + audit:** `policies.rs` (password policy + BruteForceTracker + conditional access evaluator), `events.rs` (bounded FIFO audit sink with drop counter + 22 EventKind variants).
- **HTTP + observability:** `routes.rs` (axum router exposing /api/iam/health + discovery + JWKS), `admin_api.rs` + `account_api.rs` (URL builders + DTOs), `metrics.rs` (10 standard panels + 6 standard alerts), `lib.rs` (re-exports + module State).

### CLI surface

`cavectl iam {realm,user,role,client,session,event,health}` — wired in `crates/cave-cli/src/main.rs` against the `/api/iam/*` routes.

### Parity bookkeeping

`parity.manifest.toml` ships subsystem-count bookkeeping under Charter v2:

- `fill_ratio = 0.9574` (45/47) — (mapped + partial + skipped) / total
- `honest_ratio = 0.7660` (36/47) — (mapped + partial) / total
- 33 mapped, 3 partial, 9 skipped (formalised Phase 2 cuts), 2 unmapped (honest gaps)

### Security gates

- **Secrets are keychain references.** LDAP bind credentials and external IDP client secrets validate that the configured string starts with `keychain:`; any inline secret is rejected at config time. Confidential OAuth2 client secrets are stored as PBKDF2-SHA256 hashes; the plaintext is returned exactly once on registration / rotation.
- **Refresh-token replay revokes the whole chain.** `RefreshTokenStore::rotate` marks the predecessor as revoked; if any chain member is presented after a successor has been minted, every token in the chain is revoked (RFC 6749 §10.4 / Keycloak `revokeRefreshToken`).
- **Cross-tenant access denied with structured context.** `check_tenant` returns `CrossTenantDenied { owner_tenant, request_tenant }`; every store accessor pipes through it so cave-oncall can correlate on the failure.
- **PKCE required when the client says so.** `oauth2::authorize` rejects the request when `client.require_pkce && pkce.is_none()`.
- **Brute-force lockout returns retry-after.** `BruteForceTracker::record_failure` flips to `CredentialLocked { account_id, retry_after_seconds }` once the windowed-failure count crosses `max_failures`; the wrong-password path in `UserController::authenticate_password` prefers the lockout error over `InvalidCredentials` so the caller knows the account is now hard-locked.
- **SAML structural defences.** `saml::verify_response` enforces Status / InResponseTo / Destination / Issuer / Audience / NotBefore / NotOnOrAfter / signature-present. The actual XML-DSig bit verify is deferred to the `cave-xmlsec` adapter (formal scope_cut) — the structural checks alone stop the common assertion-injection attacks.
- **ML-DSA-65 fails closed.** The signer registry accepts the alg and exposes the slot in JWKS + discovery, but signing returns an explicit `"ML-DSA-65 signer placeholder — cave-pqc backend not yet wired"` error so RPs can never accidentally trust an unsigned PQC token.

## Alternatives considered

1. **Keep `cave-auth` + glue Keycloak in via container.** Rejected — adds a JVM dependency to every cave deployment, splits the operator story between cave-cli and `kcadm.sh`, and rules out sovereign air-gapped builds where the Quarkus runtime isn't allowed.
2. **Use the `axum-login` + `oauth2-rs` libraries directly.** Rejected — they cover the relying-party side, not the *provider* side; cave-runtime needs to *issue* tokens, broker external IDPs, and host the JWKS, none of which fit those crates.
3. **Bring up FusionAuth or Authentik instead.** Rejected — both are AGPL or BSL with commercial-friendly add-ons; AGPL is fine for us but the codebases are smaller communities than Keycloak's and the spec coverage (especially around SAML IDP + LDAP + UMA) is narrower.
4. **Defer brokering to per-IDP SDKs.** Rejected — would push the JIT-provisioning + federated-identity-link logic into every consumer; the brokering module here keeps that single-sourced and lets cave-keycloak record the AuditEvent::BrokeredLogin.

## Consequences

### Positive

- Full IAM control plane in pure Rust with no JVM, no Quarkus, and no `kcadm.sh` runtime.
- One audit trail for realm / user / role / client / session / token mutations across the whole runtime — cave-oncall + cave-logs subscribe to a single `EventSink`.
- Multi-tenant from day one — every store op + controller op + keychain entry is scoped to a tenant_id.
- PQC story is staged: the discovery document advertises `ML-DSA-65`, JWKS publishes the slot, and the cave-pqc Phase 2 backend will only need to plug into the existing `SigningKeyEntry` enum.
- 132 lib + 9 self-audit + 9 smoke tests catch the high-value invariants (auth-code single-use, refresh-token chain revocation, brute-force lockout, SAML audience-tamper rejection, TOTP RFC 6238 Appendix-B vector, JIT brokered provisioning).

### Negative / trade-offs

- LDAP networking + sync daemon are deferred — the trait surface here lets in-process tests run, but production wiring needs `cave-keycloak-ldap-net` Phase 2.
- XML-DSig canonicalisation + digest verify is deferred to `cave-xmlsec`; cave-keycloak's SAML SP stops the structural attacks but does not yet verify the signature *bits* on the Response.
- Admin + account console UI runtime lives in `cave-portal-ui`; cave-keycloak surfaces the REST shape only.
- Token Exchange (RFC 8693) + UserProfile X.509 mapper are honest unmapped gaps held for Phase 2.

### Migration path

- Existing `cave-auth::keycloak` adapters continue to work against the *external* Keycloak issuer URL while the cave-keycloak crate boots up. Once the operator points the issuer at `https://iam.cave.svc/realms/{tenant}` the same JWT-verify middleware works unchanged (alg + kid lookup via JWKS).
- Phase 2: the LDAP network adapter lands as a separate crate so the cave-keycloak `LdapBackend` trait stays the contract; existing in-memory tests don't break.
- Phase 3: ML-DSA-65 backend lands via `cave-pqc`; `JwsAlg::MlDsa65` flips from placeholder to a real signer with no API change for RPs (the kid + JWKS slot are already there).

## References

- `crates/cave-keycloak/parity.manifest.toml` — full subsystem manifest + source_sha pin.
- `crates/cave-keycloak/PARITY_REPORT.md` — 8-gate close-out evidence.
- `crates/cave-keycloak/tests/parity_self_audit.rs` — the 9 enforcement assertions.
- `crates/cave-keycloak/tests/smoke_end_to_end.rs` — 9 end-to-end smoke flows.
- Keycloak upstream: https://github.com/keycloak/keycloak/tree/26.6.2
