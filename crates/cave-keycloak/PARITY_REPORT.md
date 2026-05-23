# cave-keycloak — Charter v2 8-gate close-out

**Date:** 2026-05-23
**Branch:** `claude/cave-keycloak-2026-05-23-deep`
**Upstream pin:** keycloak/keycloak `v26.6.2` (`0a402f777f8985eccbb07556e96d9b386275e048`) — Apache-2.0
**Parity:** `fill_ratio = 0.9574` (45/47) · `honest_ratio = 0.7660` (36/47)

| # | Gate | Status | Evidence |
| - | --- | --- | --- |
| 1 | **Upstream pinned** (always-latest) | PASS | `parity.manifest.toml::[upstream].version = "v26.6.2"` (Keycloak latest stable, 2026-05-19). `assertion_1_keycloak_version_pinned`. |
| 2 | **source_sha pinned** | PASS | Keycloak `0a402f7…275e048` (annotated-tag commit). `assertion_2_source_sha_matches_version`. |
| 3 | **fill_ratio ≥ 0.65** | PASS | `0.9574` = (33 mapped + 3 partial + 9 skipped) / 47. `assertion_3_fill_ratio_meets_floor`. |
| 4 | **parity_ratio_source = "manifest"** | PASS | `[parity].parity_ratio_source = "manifest"`. `assertion_4_parity_ratio_source_is_manifest`. |
| 5 | **last_audit = 2026-05-23** | PASS | `[parity].last_audit = "2026-05-23"`. `assertion_5_last_audit_is_today`. |
| 6 | **counts sum to total + ≥ 15 mapped** | PASS | 33 + 3 + 9 + 2 = 47 total; 33 mapped ≥ 15 floor. `assertion_6_counts_sum_to_total`. |
| 7 | **AGPL SPDX header coverage 100%** | PASS | All 22 `.rs` files in `src/` + `tests/` carry `SPDX-License-Identifier: AGPL-3.0-or-later`. `assertion_7_agpl_spdx_header_coverage`. |
| 8 | **no stub macros in src/** | PASS | No `todo!()` / `unimplemented!()` / `panic!("stub")` / `panic!("todo")` in `src/**/*.rs`. `assertion_8_no_stub_macros_in_src`. |

Bonus gate 9 (Charter v2 surface integrity): the full realm / user / role / client / credential / OAuth2 / OIDC / SAML / LDAP / brokering / signer / session / flow / events / metrics surface reachable through `cave_keycloak` crate-root re-exports. `assertion_9_keycloak_surface_intact`.

## Subsystem counts

| Bucket | Count | Examples |
| --- | --- | --- |
| Mapped | 33 | realm-crd, user-crd, group-crd-and-hierarchy, role-crd-composite, role-mapping, client-crd, in-memory-multitenant-store, password-credential, totp-credential, webauthn-credential, magic-link-credential, oauth2-authorization-code-grant, oauth2-pkce-s256-and-plain, oauth2-client-credentials-grant, oauth2-refresh-token-rotation, oauth2-device-authorization-grant, oauth2-introspection-rfc7662, oauth2-revocation-rfc7009, oidc-discovery, oidc-jwks-publish, oidc-userinfo, jwt-signer-es256-eddsa, sso-user-session, oidc-token-assembly, saml2-idp-response-build, saml2-sp-response-verify, ldap-federation-trait, idp-brokering, password-policy, brute-force-detection, conditional-access-evaluator, event-listener-audit, authentication-flow-executor |
| Partial | 3 | pqc-mldsa-hybrid-signer, saml-xmlsig-signature-verify, required-actions-runtime |
| Skipped | 9 | admin-console-ui + account-console-ui (cave-portal-ui), email-theme-rendering (cave-templates), ldap-network-runtime + ldap-sync-daemon (cave-keycloak-ldap-net), kerberos-gssapi-runtime, vault-secret-store-adapter (cave-vault), uma2-authorization-services, scim2-provisioning, xmlsec-c14n-verify, oid4vc-{issuer,holder} |
| Unmapped (honest gaps) | 2 | token-exchange-rfc8693, user-profile-x509-mapper |

## Test totals

| Suite | Pass | Fail | Skip |
| --- | ---: | ---: | ---: |
| Lib unit tests | 132 | 0 | 0 |
| `tests/parity_self_audit.rs` | 9 | 0 | 0 |
| `tests/smoke_end_to_end.rs` | 9 | 0 | 0 |
| **TOTAL** | **150** | **0** | **0** |

## Scope-cuts → Phase 2 owners

| Group | Phase 2 crate(s) | Items |
| --- | --- | --- |
| Admin + account console UI | `cave-portal-ui` | admin-console-ui, account-console-ui |
| Theme rendering | `cave-templates` | email-theme-rendering |
| LDAP wire + sync | `cave-keycloak-ldap-net` | ldap-network-runtime, ldap-sync-daemon |
| Kerberos | `cave-keycloak-kerberos` | kerberos-gssapi-runtime |
| Vault secret store | `cave-vault` | vault-secret-store-adapter |
| Authorization Services | `cave-keycloak-uma` | uma2-authorization-services |
| SCIM 2.0 | `cave-keycloak-scim` | scim2-provisioning |
| XML-DSig canonicalisation | `cave-xmlsec` | xmlsec-c14n-verify |
| Verifiable Credentials | `cave-keycloak-oid4vc` | oid4vc-issuer, oid4vc-holder |

## Smoke evidence

| Scenario | Test | Result |
| --- | --- | --- |
| Auth code + PKCE + signed ID token end-to-end | `smoke_1_auth_code_pkce_to_id_token` | PASS |
| Refresh-token rotation + replay revokes the chain | `smoke_2_refresh_rotation_replay_revokes_chain` | PASS |
| Brute-force lockout after threshold + clear-on-success | `smoke_3_brute_force_locks_then_clears` | PASS |
| TOTP RFC 6238 Appendix-B vector + magic-link verify+expiry | `smoke_4_totp_and_magic_link_credentials` | PASS |
| Brokered Google login JIT-provisions a federated user | `smoke_5_brokered_google_login_jit_user` | PASS |
| SAML SP-side verify happy path + audience-tamper rejection | `smoke_6_saml_sp_verify_happy_then_tamper` | PASS |
| LDAP federation + AuthenticationFlow + discovery + JWKS | `smoke_7_ldap_federation_plus_flow` | PASS |
| Router /health + state round-trip + event sink drain | `smoke_8_router_health_round_trip` | PASS |
| Password policy min-length + PKCE verifier rejection | `smoke_aux_password_policy_min_length` | PASS |

## Security gates honoured

| Gate | Where |
| ---- | ----- |
| Bind credentials never in plaintext | `LdapConfig::validate` rejects any `bind_credential_keychain_handle` that doesn't start with `keychain:` |
| IDP client secret never in plaintext | `ExternalIdp::validate` rejects inline `client_secret`s |
| Confidential client secret stored as PBKDF2 hash | `ClientController::register_confidential` returns the plaintext exactly once + persists only the hash |
| Password is constant-time-equality checked | `PasswordCredential::verify` rebuilds with the recorded salt + iterates the same hash, then `constant_time_eq` compares the full encoded form |
| Refresh-token replay revokes the entire chain | `RefreshTokenStore::rotate` — a replay of any chain token sets `revoked=true` on every token in the chain (RFC 6749 §10.4 / Keycloak `revokeRefreshToken`) |
| Cross-tenant read denied with structured error | `check_tenant` returns `CrossTenantDenied { owner_tenant, request_tenant }` and every store accessor pipes through it |
| PKCE required when the client demands it | `oauth2::authorize` returns `PkceFailed("missing-challenge")` when `client.require_pkce && pkce.is_none()` |
| SAML structural injection-attack defences | `saml::verify_response` enforces Status / InResponseTo / Destination / Issuer / Audience / time-window / signature-present — XML-DSig signature *bit* verify deferred to cave-xmlsec |
| Brute-force lockout | `BruteForceTracker::record_failure` returns `CredentialLocked { account_id, retry_after_seconds }` when the windowed-failure count crosses `max_failures` |
| ML-DSA-65 placeholder fails closed | `SignerRegistry` accepts the alg and exposes it in JWKS + discovery, but the signer returns an explicit "cave-pqc backend not yet wired" error so RPs can't accidentally trust an unsigned token |

## Integration notes (for downstream wiring)

* `cave-vault` → `cave-keycloak`: `LdapConfig.bind_credential_keychain_handle` + `ExternalIdp.client_secret_keychain_handle` + (future) `Client.client_secret_keychain_handle` resolve through the cave-vault keychain client at request time — secrets never live in process memory beyond the call.
* `cave-portal-ui` → `cave-keycloak`: consumes the `/api/iam` REST surface to render the admin + account consoles; cave-keycloak does not ship a UI bundle.
* `cave-mesh` / `cave-net` → `cave-keycloak`: receive `discovery_for(realm, base_url)` + `jwks_for(realm)` payloads to bootstrap RP middleware against the issuer.
* `cave-runtime` → `cave-keycloak`: wires `State::default()` + `router(state)` into the top-level axum app under `/api/iam/*`; per-request tenant resolution attaches `tenant_id` via the cave-auth middleware before the controllers see the call.
* `cave-sign` → `cave-keycloak`: the JWT signer registry's `SigningKeyEntry::to_jwk` shape lines up with the OIDC `id_token_signing_alg_values_supported` list, so the cave-sign trust root + signing config can rotate keys without breaking the JWKS publisher.
* `cave-pqc` (Phase 2) → `cave-keycloak`: closes the `pqc-mldsa-hybrid-signer` partial — the registry already accepts `JwsAlg::MlDsa65` and stages the JWKS slot.

## Closure

ADR-158 (Keycloak Adoption — cave-keycloak) documents the design choice + the Phase 2 split.

2026-05-23 — Charter v2 8-gate close. Workspace ≥ 0.95 count grows by 1.
