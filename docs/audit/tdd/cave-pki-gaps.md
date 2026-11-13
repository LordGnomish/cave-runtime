# cave-pki TDD coverage gap report

- **Crate:** `crates/security/cave-pki`
- **Upstream:** [smallstep/certificates](https://github.com/smallstep/certificates) @ `v0.30.2` (Go)
- **Upstream test-symbol count:** 774 test functions across 142 `*_test.go` files
- **cave test-fn count:** 12 (`#[test]`, excluding 4 generic proptest smokes)

## Scope note

Upstream `smallstep/certificates` is a full ACME/SCEP CA server: ACME challenges
(HTTP-01 / DNS-01 / TLS-ALPN-01 / device-attest-01 / TPM / Wire), NoSQL DB layers,
admin REST API, provisioners, KMS/HSM backends, SSH CA, JOSE, and X.509 generation.
cave-pki is a deliberately small **PKI core**: a 3-tier in-memory CA hierarchy
(`Ca`), RFC 5280 chain validation (`ChainValidator`), a CRL responder
(`CrlResponder`), and an OCSP responder (`OcspResponder`). It stores metadata
handles only — real DER lives in cave-vault. The vast majority of upstream test
symbols therefore map to behavior cave does not implement and does not intend to
(scope-cut). The portable gaps below are the handful of behaviors cave **already
implements** but does not yet assert.

## Gap table

| Behavioral unit | Upstream test | cave impl? | cave test? | gap type | suggested cave test name |
|---|---|---|---|---|---|
| Chain rejects child whose issuer ref ≠ parent serial | `authority/authority_test.go::TestAuthority_GetTLSOptions` / path-validation (RFC 5280 §6.1) | yes — `ChainValidator::validate` issuer-match branch (chain.rs:89) | no | portable-coverage | `chain_rejects_issuer_reference_mismatch` |
| Chain rejects illegal issuer kind (e.g. Root issuing Tenant directly) | path-validation basic-constraints (`authority/...` cert checks) | yes — `legal_issuer` / validate (chain.rs:96) | no | portable-coverage | `chain_rejects_illegal_issuer_kind` |
| Chain rejects when trust anchor is not a known Root | path-validation trust-anchor (RFC 5280 §6.1) | yes — `validate` anchor branch (chain.rs:106) | no | portable-coverage | `chain_rejects_non_root_trust_anchor` |
| Cert not-yet-valid (`notBefore` in future) fails validation | cert validity-window checks | yes — `validate` not_before branch (chain.rs:65) | no | portable-coverage | `chain_rejects_not_yet_valid_cert` |
| Revocation reason-code round-trip incl. reserved code 7 | `acme/api/revoke_test.go::Test_validateReasonCode`, `Test_reason` | yes — `RevocationReason::from_code` (already partly covered) | partial — `revocation_reason_codes_match_rfc5280_table` covers code/from_code but not `code()→from_code()` round-trip over the full enum | portable-coverage | `revocation_reason_code_roundtrip_total` |
| OCSP: root / unissued serial returns `Unknown` (no-authority semantic) | OCSP responder status table (RFC 6960 §2.2) | yes — `OcspResponder::check` known_serials miss (ocsp.rs:58) | partial — Unknown is asserted for one literal; the "not in known_serials ⇒ Unknown even though valid" semantic isn't isolated | portable-coverage | `ocsp_unknown_for_serial_outside_authority` |
| `chain_for` on an unknown serial returns `ParentNotFound` | path lookup error | yes — `Ca::chain_for` (ca.rs:217) | no | portable-coverage | `chain_for_unknown_serial_is_parent_not_found` |
| CRL accessors `is_revoked` / `len` / `is_empty` | CRL build/list (`api/crl_test.go::Test_CRL`) | yes — `CrlResponder::is_revoked` (crl.rs:112) | no | portable-coverage | `crl_is_revoked_reflects_membership` |
| ACME order/account/authz lifecycle | `acme/order_test.go`, `acme/account_test.go`, `acme/db/nosql/*` | no | — | missing-impl | — |
| ACME challenge validation (HTTP-01/DNS-01/TLS-ALPN-01/device-attest) | `acme/challenge_test.go` (18 tests) | no | — | missing-impl | — |
| X.509 certificate generation / signing / profiles | `authority/authorize_test.go`, `db/x509util` | no (metadata handles only; DER in cave-vault) | — | scope-cut: real X.509 issuance is delegated to cave-vault PKI engine | — |
| SSH CA (sign/host/user certs, federation, bastion) | `api/ssh_test.go` (14 tests) | no | — | scope-cut: SSH CA is not in cave-pki launch scope | — |
| Admin REST API (provisioners, policy, webhooks, EAB) | `authority/admin/api/*` (40+ tests) | no | — | scope-cut: admin surface lives in cave-portal-api | — |
| KMS / HSM key backends, JOSE, SCEP | `kms/*`, `scep/*` | no | — | scope-cut: key custody delegated to cave-vault / HSM | — |
| Name constraints engine | `authority/internal/constraints/constraints_test.go` | no | — | missing-impl (not yet modelled; cave has no SAN/name-constraint check) | — |

## Recommended TDD fills (portable-coverage first)

These exercise code paths that **already exist** in `cave-pki/src` but have no
asserting test. Each is a cheap, honest coverage win.

1. **`chain_rejects_issuer_reference_mismatch`** — build a 3-tier CA, then construct
   a chain where a child's `issuer_serial` does not equal its parent's `serial`;
   assert `ValidationResult::Invalid` mentioning "issuer reference". Exercises
   `ChainValidator::validate` (chain.rs:89-95). (Requires a constructor path or
   handle mutation to force the mismatch.)
2. **`chain_rejects_illegal_issuer_kind`** — drive `legal_issuer` to `false`
   (e.g. a Root sitting directly above a TenantIntermediate); assert `Invalid`
   "cannot issue". Exercises `ChainValidator::validate` (chain.rs:96-101).
3. **`chain_rejects_non_root_trust_anchor`** — validate a serial whose chain
   terminates at a non-Root handle; assert `Invalid` "is not a Root CA".
   Exercises `ChainValidator::validate` (chain.rs:106-111).
4. **`chain_rejects_not_yet_valid_cert`** — set the validator clock with `.at(...)`
   to a moment before the leaf's `not_before`; assert `Invalid` "not yet valid".
   Exercises `ChainValidator::validate` (chain.rs:65-70).
5. **`revocation_reason_code_roundtrip_total`** — for every `RevocationReason`
   variant, assert `from_code(r.code()) == Some(r)`, and that codes 7/11..=255
   are `None`. Exercises `RevocationReason::code`/`from_code` (crl.rs:31-49)
   more completely than the existing literal-by-literal table test.
6. **`ocsp_unknown_for_serial_outside_authority`** — a serial that is neither in
   the CRL nor in `known_serials` returns `OcspStatus::Unknown` (no authority),
   distinct from `Good`. Exercises `OcspResponder::check` (ocsp.rs:51-63).
7. **`chain_for_unknown_serial_is_parent_not_found`** — call `Ca::chain_for` with
   a serial that was never issued; assert `PkiError::ParentNotFound`. Exercises
   `Ca::chain_for` (ca.rs:217-223).
8. **`crl_is_revoked_reflects_membership`** — assert `is_revoked` / `len` /
   `is_empty` track `revoke` and `unrevoke`. Exercises `CrlResponder` accessors
   (crl.rs:112-121).

## Honesty notes

- No stubs were counted as coverage. Items 1-3 require either a test-only handle
  constructor or a small `pub(crate)` insert path to *forge* an invalid chain,
  because the public `Ca` builders always produce well-formed hierarchies — flag
  this as a prerequisite, not as hidden coverage.
- ACME, SSH, admin API, KMS, SCEP, JOSE, and X.509 generation are genuine
  scope-cuts (delegated to cave-vault / cave-portal-api or out of launch scope),
  not coverage gaps. Name-constraints is a real `missing-impl` if cave later adds
  SAN handling.
