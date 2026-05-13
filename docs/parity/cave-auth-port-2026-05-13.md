# cave-auth parity — 2026-05-13 sweep (SAML 2.0)

**Upstream:** `keycloak/keycloak v22.0.0` (Apache-2.0).
**Delta from previous audit:** `2026-05-12` snapshot at `fill_ratio = 0.7568`.

## What this sweep landed

A new `crates/cave-auth/src/saml/` module porting Keycloak's
`saml-core` + the `services/.../protocol/saml/` endpoint set —
the second federation protocol after OIDC. Seven files,
~1450 LOC + 41 unit tests:

| File | Role |
|------|------|
| `mod.rs`            | `SamlError`, `NameIdFormat`, `SamlSubject`, namespace constants |
| `authn_request.rs`  | `<samlp:AuthnRequest>` builder + parser; tolerant of namespace-prefix variation (`saml2p:` vs `samlp:`) |
| `response.rs`       | `<samlp:Response>` + `<saml:Assertion>` writer + parser; subject extraction with validity-window check (`is_time_valid` w/ 30-second skew tolerance, matching Keycloak's `ASSERTION_SKEW_TIME_SEC`) |
| `metadata.rs`       | `<md:EntityDescriptor>` for both SP and IdP roles, with `<ds:KeyInfo>` / SingleSignOnService / AssertionConsumerService / SingleLogoutService |
| `binding.rs`        | HTTP-Redirect (raw-deflate + base64) and HTTP-POST (base64) bindings |
| `signature.rs`      | RSA-SHA256 sign + verify (ring 0.17) with pluggable canonicalization hook |
| `broker.rs`         | SP-initiated and IdP-initiated flow state machines; in-flight request tracking with TTL-based GC |

**41 new unit tests pass** in `cave-auth --lib`:
- `NameIdFormat` URN round-trip (2)
- AuthnRequest builder, XML round-trip, alternate-NS-prefix tolerance, missing-field rejection, malformed XML (5)
- Response/Assertion XML round-trip, validity-window, `into_subject` filters (status, missing assertion), malformed (7)
- Metadata IdP + SP round-trips, SLO endpoint, missing entityID rejection (4)
- HTTP-Redirect + HTTP-POST encode/decode + compression check + corruption rejection (6)
- RSA-SHA256 sign + verify round-trip, tampered-payload rejection, bad-base64, canonicalization-fn applied (4)
- Broker SP-init redirect URL + in-flight tracking, response consumption, unknown InResponseTo / mismatched issuer / non-success / expired rejection, GC, IdP destination check, mint_response carrying InResponseTo + audience, supported bindings (10)
- Library-level `NameIdFormat` accept/reject (3)

## Counts

| Bucket   | 2026-05-12 | 2026-05-13 |
|----------|-----------:|-----------:|
| Mapped   | 12 | **13** |
| Skipped  | 16 | 16 |
| Unmapped | 9 | **8** |
| **Total** | 37 | 37 |
| **fill_ratio** | 0.7568 | **0.7838** |

## What changed in the inventory

* `[[mapped]]` gained `keycloak:saml-core/ + services/.../protocol/saml/`,
  pointing to the seven new files.
* `[[unmapped]]` SAML entry removed (was: "only OIDC implemented today.
  Tracked but not yet started").

## What this PR does NOT claim

* `fill_ratio = 0.7838` does NOT mean cave-auth is 78% of a
  production Keycloak — it claims 78% of Keycloak's top-level
  packages are either covered or honestly out of scope.
* **XML canonicalization is not fully implemented.** Real SAML
  signatures protect a `<ds:SignedInfo>` block over the
  *canonicalized* form (`exc-c14n`, rfc3741) of the signed
  element. cave-auth's `SignedDocument` accepts a pluggable
  `canonicalize_fn`; default is identity. Real production
  integrations with strict IdPs still need an external c14n
  implementation. This is documented in `signature.rs` and the
  module doc.
* **Encrypted Assertions** (`<saml:EncryptedAssertion>`) are
  parsed-but-not-decrypted. XML-Enc key-transport is its own
  RFC. Tracked as a known limitation, not landed.
* **Artifact Resolution binding** (back-channel) is intentionally
  out of scope — every IdP cave customers federate with supports
  the two front-channel bindings already implemented.
* The new `saml/broker.rs` is **library-level**, not yet wired
  into `auth_middleware`. Routes that accept `SAMLResponse=`
  form posts are a follow-up paket.
