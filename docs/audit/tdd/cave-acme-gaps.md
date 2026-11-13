# cave-acme — TDD coverage gap report

- **Crate:** `crates/security/cave-acme` (theme: security)
- **Upstream:** smallstep/certificates `@ v0.30.2` (Go) — `acme/` package
- **Upstream test-symbol count (whole repo inventory):** 774 across 142 files; ~120 in the ACME-relevant test files
- **cave test-fn count:** 32 `#[test]` total — 13 real behavioral tests, 4 generic proptest-smoke, 15 `qwen_drafted` type-existence assertions

cave-acme is a deliberately small RFC 8555 model + in-memory multi-tenant
state machine (account/challenge/order/authorization). It does **not** ship the
HTTP/JWS API layer, the nosql persistence layer, or device/TPM/Wire
attestation that dominate the upstream test suite — those are legitimate
scope-cuts. The high-value gaps are **portable-coverage**: server-side state
transitions cave already implements in `server.rs` but never tests.

## Gap table

| Behavioral unit | Upstream test | cave impl? | cave test? | gap type | suggested cave test name |
|---|---|---|---|---|---|
| Order finalize: Ready→Processing→Valid + stamps cert URL | `TestDB_UpdateOrder` / `TestHandler_FinalizeOrder` | yes — `AcmeServer::finalize_order` | no | portable-coverage | `finalize_order_promotes_ready_to_valid` |
| Finalize rejects non-Ready order (`OrderNotReady`) | `TestHandler_FinalizeOrder` (status guard) | yes — `finalize_order` status guard | no | portable-coverage | `finalize_order_rejects_non_ready_order` |
| Challenge valid → authz valid → order Ready promotion | `TestAuthorization_UpdateStatus` / `TestChallenge_Validate` | yes — `AcmeServer::mark_challenge_valid` | no | portable-coverage | `mark_challenge_valid_promotes_authz_and_order` |
| Order stays Pending until ALL authz valid | `TestDB_UpdateOrder` (all-valid gate) | yes — `mark_challenge_valid` all-valid check | no | portable-coverage | `order_not_ready_until_all_authz_valid` |
| Deactivated account blocks new orders (`unauthorized`) | `TestHandler_GetOrUpdateAccount` (deactivate) + `acme_test` | yes — `deactivate_account` + `new_order` status guard | no | portable-coverage | `deactivated_account_cannot_create_order` |
| new_order builds one authz per identifier, 3 challenge types each | `TestHandler_NewOrder` / `TestHandler_newAuthorization` | yes — `AcmeServer::new_order` | no | portable-coverage | `new_order_creates_authz_per_identifier_with_three_challenges` |
| Lookup unknown account → `accountDoesNotExist` | `TestDB_GetAccount` (not-found) | yes — `AcmeServer::account` → `AccountNotFound` | no | portable-coverage | `account_lookup_unknown_id_is_not_found` |
| EAB key-id binding to account | `TestExternalAccountKey_BindTo` | partial — only `validate_algorithm` (no bind/MAC verify) | partial (`validate_algorithm` only) | missing-impl | n/a (MAC verification not implemented) |
| JWS parse / verify / extract-or-lookup-JWK middleware | `TestHandler_parseJWS`, `validateJWS`, `extractOrLookupJWK` | no — no HTTP/JWS layer in cave | no | scope-cut | gateway parses JWS; cave drives state only |
| Nonce issue / consume | `TestDB_CreateNonce`, `TestHandler_addNonce` | no | no | scope-cut | replay-nonce is HTTP-layer (cave-gateway) |
| Directory / metadata object | `TestHandler_GetDirectory`, `Test_createMetaObject` | no | no | scope-cut | directory JSON is HTTP-layer concern |
| HTTP-01 / DNS-01 / TLS-ALPN-01 live validation (network probe) | `TestHTTP01Validate`, `TestDNS01Validate`, `TestTLSALPN01Validate` | partial — derives key-auth/record value, no network probe | derivation tested | missing-impl | live validation = network I/O, deferred |
| device-attest-01 / TPM / step / apple attestation | `Test_deviceAttest01Validate`, `Test_doTPMAttestationFormat` | no | no | scope-cut | hardware attestation out of launch scope |
| Wire DPoP / OIDC challenge | `Test_wireDPOP01Validate`, `Test_wireOIDC01Validate` | no | no | scope-cut | Wire identity protocol out of scope |
| Cert revocation (`validateReasonCode`, `RevokeCert`) | `Test_validateReasonCode`, `TestHandler_RevokeCert` | no | no | missing-impl | revocation handled by cave-pki, not cave-acme |
| URL linker (account/order/authz/challenge links) | `TestLinker_*` (linker_test.go) | no (hardcoded path formats only) | no | scope-cut | link construction is gateway routing concern |

## Recommended TDD fills (portable-coverage first)

These exercise behavior already present in `crates/security/cave-acme/src/server.rs`
and `order.rs`; each needs only an in-memory `AcmeServer` and no new source.

1. **`finalize_order_promotes_ready_to_valid`** — drive an order to `Ready` via
   `mark_challenge_valid`, call `finalize_order`, assert status `Valid` and
   `certificate_url` is `Some`. Exercises `AcmeServer::finalize_order`.
2. **`finalize_order_rejects_non_ready_order`** — call `finalize_order` on a
   freshly created (`Pending`) order, assert `AcmeError::OrderNotReady`.
   Exercises the status guard in `finalize_order`.
3. **`mark_challenge_valid_promotes_authz_and_order`** — single-identifier
   order, mark its one challenge valid, assert the authz is `Valid` and the
   order advances to `Ready`. Exercises `AcmeServer::mark_challenge_valid`.
4. **`order_not_ready_until_all_authz_valid`** — two-identifier order, mark only
   the first authz's challenge valid, assert order is still `Pending`; mark the
   second, assert `Ready`. Exercises the all-valid gate in `mark_challenge_valid`.
5. **`deactivated_account_cannot_create_order`** — `deactivate_account`, then
   `new_order` with that account, assert `AcmeError::Unauthorized`. Exercises
   `deactivate_account` + the `status != Valid` guard in `new_order`.
6. **`new_order_creates_authz_per_identifier_with_three_challenges`** — order
   with 2 DNS identifiers, assert `authorization_count == 2` and each authz
   carries exactly 3 challenges (http-01, dns-01, tls-alpn-01). Exercises
   `AcmeServer::new_order` authorization fan-out.
7. **`account_lookup_unknown_id_is_not_found`** — call `account()` with a random
   id, assert `AcmeError::AccountNotFound`. Exercises `AcmeServer::account`.

## Scope-cut justifications (one line each)

- **JWS / nonce / directory middleware** — cave-acme is the state machine; JWS
  parsing and nonce replay-protection live in the cave-gateway HTTP layer.
- **Live HTTP-01/DNS-01/TLS-ALPN-01 probes** — require outbound network I/O;
  cave tests the deterministic key-authorization/record derivation only.
- **device-attest-01 / TPM / Wire DPoP+OIDC** — hardware/identity attestation
  protocols outside the launch scope of the RFC 8555 core.
- **Cert revocation** — owned by cave-pki, not the ACME order state machine.
- **URL linker** — link construction is a cave-gateway routing concern.
