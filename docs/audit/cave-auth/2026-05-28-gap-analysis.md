# cave-auth — Keycloak parity gap analysis

<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
<!-- Upstream: keycloak/keycloak — Apache-2.0 — see NOTICE -->

**Date:** 2026-05-30
**Upstream baseline:** keycloak/keycloak **v26.6.2** (`0a402f777f8985eccbb07556e96d9b386275e048`), Apache-2.0
**cave-auth manifest pin (unchanged):** v22.0.0 — this audit measures against the current stable 26.6.2.
**Branch:** `claude/cave-auth-honest-2026-05-30` (pushed, **not merged**)

> This is a *report*. It does not edit `parity.manifest.toml` or the parity-index —
> the manifest remains the count-based source of truth. The honest LOC ratio below
> is recorded here only.

## Phase 1 — LOC inventory

| Scope | Upstream (Java, non-test) | cave-auth (Rust, src) |
|-------|--------------------------:|----------------------:|
| Whole Keycloak repo | 615,639 | 40,223 |
| Domain packages cave-auth targets (7 components below) | 151,837 | 31,220¹ |

¹ cave LOC attributable to the seven mapped components; the remaining ~9k cave LOC
is glue/adapters (`abac`, `rbac`, `okta`, `scim`, middleware, tenancy) without a 1:1
Keycloak package.

## Phase 4 — Honest LOC ratio (`cave / upstream`)

| Denominator | honest_ratio |
|-------------|-------------:|
| Whole Keycloak non-test Java (615,639) | **0.0653** |
| Domain target packages (151,837, full cave src 40,223) | **0.2649** |
| Component-matched (151,837 vs 31,220) | **0.2056** |

**Verdict:** under the LOC definition the ratio is **0.065–0.265 ≪ 0.95**. Consistent with
the count-based manifest (`fill 1.0 / honest 0.9773`) being a *coverage-of-mapped-surface*
metric, not a line-for-line ratio. **No merge** is warranted on the LOC gate; the branch
is pushed and held. The value delivered this session is genuine behavioural coverage
(below), not a ratio bump.

## Phase 3 — strict-TDD cycles completed this session

Each cycle is a `test`-only commit (RED, build/assert fail proven) followed by a
`src`-only commit (GREEN). Test vectors are the published RFC vectors, so GREEN proves
the port is byte-accurate to the upstream algorithm.

| Cycle | Component | Upstream source (Apache-2.0) | RED (test) | GREEN (impl) | Tests |
|-------|-----------|------------------------------|-----------|--------------|------:|
| 1 | OTP (HOTP/TOTP) | `HmacOTP.java`, `TimeBasedOTP.java` | `4c279045` | `6a551473` | +4 (RFC 4226/6238) |
| 2 | Brute-force | `DefaultBruteForceProtector.java` | `40b3ca3a` (+fix `347cecdc`) | `814352a5` | +5 |
| 3 | PBKDF2 hashing | `Pbkdf2PasswordHashProvider.java` | `7b711ee6` | `cb1de44f` | +4 (RFC 6070/7914) |

New modules: `src/otp.rs`, `src/brute_force.rs`, `src/password_hash.rs` (+436 src LOC,
+264 test LOC). Suite **956 → 969 passing, 0 failed**. Self-audit still green; manifest untouched.

> Cycle-2 note: the first brute-force RED produced one failing assertion at GREEN because the
> test used `failure_time = 0`, which collides with the `last_failure == 0` "never failed"
> sentinel — so the `last > 0` quick-login guard (faithfully ported) correctly did not fire.
> Real `Time.currentTimeMillis()` is never 0; the **test vector** was corrected (`347cecdc`),
> the impl was already accurate. (Cf. prior audits: such REDs are wrong test-expectations, not cave bugs.)

## Remaining work (priority order, not started)

Highest-value, most-unit-testable next cycles drawn from the matrix below:
1. **Domain model** — composite roles (`addCompositeRole`/`getComposites`), user→role
   mappings (`grantRole`/`hasRole`), group membership (`joinGroup`/`isMemberOf`). All
   land in `src/persistence/backend.rs` (async trait, 2 backends → larger cycles).
2. **OAuth/OIDC** — refresh-token rotation, token introspection (RFC 7662), PKCE `plain`
   parity, device-code polling/`slow_down`.
3. **JWT/JOSE** — `PS256/384/512`, key rotation/`kid` selection, real ML-DSA (currently placeholder).
4. **LDAP/AD** — attribute/group/role mappers, no-import sync mode, AD `userAccountControl`.
5. **Admin REST + events** — `EventListenerProvider` SPI, admin-event auditing, partial export/import.
6. **SAML 2.0** — encryption, artifact binding, single-logout (largest gap: 60k upstream LOC).

## Phase 2 — gap matrix (per component)

### Keycloak Domain Model (Realm/User/Group/Role/Client)

*upstream ≈ 12,439 LOC · cave ≈ 7,217 LOC*

**Present (verified in cave):**

- Realm CRUD (create, read, update, delete) with display_name, enabled, ssl_required, registration_allowed, access_token_lifespan, sso_session_idle_timeout
- User CRUD per-realm with username, email, email_verified, first_name, last_name, enabled, attributes (BTreeMap), created_timestamp, credentials
- Client CRUD per-realm with client_id, name, enabled, public_client, secret, redirect_uris, web_origins, protocol, attributes
- Role CRUD per-realm with name, description, attributes, composite flag
- Group CRUD with parent-child hierarchy (parent_id), attributes, role_ids vector
- Realm-scoped isolation: all User, Client, Role, Group entities keyed by realm_id
- UserCredential with credential_type, secret_data (JSON), credential_data, priority
- Soft-delete audit trail (created_at, updated_at, deleted_at) on all entities
- IdentityProvider and IdpMapper entities for federation
- AuthenticationFlow entity for auth flow configuration

**Stubbed / partial:**

- Tenant settings enforcement: TenantSettings struct exists with max_members, sso_enforced, mfa_required, session_ttl_minutes, but only max_members is checked on add_member; session TTL and MFA are not enforced anywhere
- RBAC role hierarchy: cave-auth::rbac.rs implements parent-pointer role hierarchy with role_allows() recursion, but this is CAVE-specific and not aligned with Keycloak's explicit-edge composition model

**Missing vs Keycloak 26.6.2:**

- Role composition relationships: add_composite_role, remove_composite_role, get_composites (with search/pagination)
- User-role mappings: grant_role_to_user, revoke_role_from_user, get_user_roles, has_role
- User-group membership: join_group, leave_group, get_user_groups, is_member_of
- Group-role inheritance: add_role_to_group, remove_role_from_group, list_group_roles
- Realm default roles and default groups
- Required actions on users (VERIFY_EMAIL, UPDATE_PROFILE, CONFIGURE_TOTP, UPDATE_PASSWORD, etc.)
- Group subgroup pagination and search (fuzzy match by name with first/max)
- Role attribute stream queries (getAttributeStream)
- ClientScope entity and default/optional scope assignment
- Realm-level default client scopes
- Group role inheritance from parent groups
- Password policies and WebAuthn/OTP policies
- Authentication flow execution model
- Client protocol mappers
- Brute force protection enforcement
- Event listeners and admin event logging (audit beyond soft-delete)


### Authentication (Password Hashing, OTP/TOTP, WebAuthn, Brute-Force Protection)

*upstream ≈ 26,093 LOC · cave ≈ 3,722 LOC*

**Present (verified in cave):**

- WebAuthn registration ceremony (W3C L2 §7.1): attestation parsing, attestation statement verification (none, packed, TPM, android-key), authenticator data parsing, rpIdHash/UP/UV/AT flag verification, credential public key extraction, excludeCredentials check (registration.rs:150+ verified lines)
- WebAuthn authentication ceremony (W3C L2 §7.2): assertion verification, credential lookup, allowCredentials filtering, clientDataJSON parsing, rpIdHash check, UP/UV flag validation, signature verification via COSE, sign-count monotonicity check, backup-state-change detection (authentication.rs:64-147 verified)
- COSE signature verification: ES256, EdDSA, RS256 with public key validation (cose.rs verified implementations)
- WebAuthn credential store abstraction with sign-count persistence (credential_store.rs)
- Resident key (discoverable credential) validation via resident_key.rs
- Session management with idle timeout, revocation, TTL tracking (session.rs:1-81 verified)
- Audit logging for auth events with decision tracking (allowed/denied), audit event structs (audit.rs verified)

**Stubbed / partial:**

- TOTP configuration required action (admin_flows/required_actions.rs references 'CONFIGURE_TOTP' as a string constant but no implementation)
- OTP enrollment event dispatch (email_listener/dispatcher.rs maps 'totp_enrolled' to AuthEvent::MfaEnrollment enum variant but no TOTP code generation/verification logic)
- Credential type enumeration (persistence/entities.rs: UserCredential.credential_type field supports 'password' | 'otp' | 'webauthn' as strings, but only password and webauthn are functional; otp is unreachable)
- Brute-force realm config (persistence/entities.rs: RealmEntity.brute_force_protected boolean field and SQL migrations declare the column, but no enforcement logic reads or uses it)

**Missing vs Keycloak 26.6.2:**

- Password hashing algorithms (PBKDF2/Argon2): cave-auth stores plaintext password comparison in-memory (keycloak/user.rs:164-170 plaintext equality check), no PBKDF2 or Argon2 key derivation functions
- OTP/TOTP verification (RFC 6238): no TimeBasedOTP or HmacOTP implementations for TOTP code validation or HMAC computation
- Brute-force protection: no login-failure tracking, no configurable lockout thresholds, no wait-time escalation, no quick-login detection, no failure-count reset logic (contrast: Keycloak DefaultBruteForceProtector.java:85-150 implements all these)
- Secondary authentication failure tracking (OTP-specific lockout)
- Password policy enforcement (iteration counts, hash algorithm validation)
- HOTP (counter-based OTP) — only referenced in data models, not implemented


### OAuth2/OIDC Token Endpoints (authorization_code, client_credentials, device_code, refresh_token, introspection, discovery, JWKS)

*upstream ≈ 10,355 LOC · cave ≈ 5,280 LOC*

**Present (verified in cave):**

- authorization_endpoint GET/POST with login_hint, prompt=none support
- authorization code generation with TTL enforcement
- authorization code storage with UUID keys
- PKCE S256 and plain method support with RFC 7636 validation
- PKCE code_verifier length bounds (43-128 chars) and charset validation
- PKCE constant-time challenge comparison
- client_credentials grant type with client secret validation
- refresh_token grant type with session state rotation
- refresh token generation and JWT decode
- access token JWT encoding (HS256) with exp/iat claims
- token introspection endpoint (RFC 7662) supporting active/inactive response
- token introspection with issuer validation per realm
- OpenID Discovery endpoint (.well-known/openid-configuration)
- discovery document with standard endpoints advertised
- discovery grant_types_supported claim advertising authorization_code, refresh_token, password, client_credentials
- discovery response_types_supported including 'code'
- discovery scopes_supported (openid, profile, email, offline_access)
- device authorization grant (RFC 8628) with device_code + user_code issuance
- device flow approval endpoint
- device polling with exp_unix and interval support
- device authorization status management (pending/approved/denied/expired)
- CIBA (OpenID Client-Initiated Backchannel Authentication) endpoint
- PAR (RFC 9126) Pushed Authorization Request storage and resolution
- token revocation endpoint (RFC 7009)
- JWKS fetching and caching (for external JWKS URIs)
- JWKS background refresh every 5 minutes

**Missing vs Keycloak 26.6.2:**

- authorization_code exchange at token endpoint (grant_type=authorization_code code= handling)
- authorization code PKCE verification in token endpoint
- authorization code expiry validation in token endpoint
- authorization code client_id validation in token endpoint
- authorization code redirect_uri validation in token endpoint
- JWKS publication endpoint serving /protocol/openid-connect/certs
- ID token generation for authorization_code flows
- nonce handling for ID tokens
- token_type_hint support in introspection (introspection only handles access tokens)
- DPoP (Demonstration of Proof-of-Possession) support
- jti (JWT ID) claim tracking for token revocation
- JWT Bearer Token assertion grant (RFC 7523)
- resource_owner_password_credentials grant type (password grant exists, but not full ROPC semantics)
- token exchange grant (RFC 8693)
- client assertion authentication methods (client_secret_jwt, private_key_jwt)
- revocation_endpoint_auth_methods_supported in discovery
- requestObjectSigningAlgValuesSupported in discovery
- claimsLocalesSupported in discovery
- ui_locales_supported in discovery
- display parameter support in authorization request
- max_age enforcement for re-authentication
- acr_values support
- ui_locales parameter support
- login_hint_locale support


### SAML 2.0 Protocol Implementation

*upstream ≈ 60,500 LOC · cave ≈ 5,379 LOC*

**Present (verified in cave):**

- AuthnRequest builder and parser (SP→IdP message)
- Response parser with Assertion extraction
- Assertion signing/verification (RSA-SHA256)
- ECDSA-SHA256/384/512 signature support (XMLDSig RFC 4051)
- XML canonicalization (exc-c14n RFC 3741, subset)
- HTTP-Redirect binding (DEFLATE+base64)
- HTTP-POST binding (base64)
- HTTP-Artifact binding (44-byte type-0x0004 artifacts)
- Metadata EntityDescriptor generation (both SP/IdP roles)
- SingleLogoutService endpoint declarations
- NameID format URNs (EmailAddress, Persistent, Transient, Unspecified)
- NameIDPolicy (Format, AllowCreate, SPNameQualifier)
- Assertion Conditions validation (NotBefore/NotOnOrAfter window)
- AuthnContextClassRef parsing (5 classes: PPT, Password, Kerberos, PreviousSession, Unspecified)
- SubjectConfirmation method URNs (Bearer, SenderVouches, HolderOfKey)
- Status codes (Success, Responder)
- In-flight request state tracking and TTL management (5min default)
- SP-initiated login flow state machine
- IdP-initiated login flow state machine
- Audience restriction validation
- Session index correlation (for SLO)
- Attribute statement flattening (multi-value maps)
- SAML error discrimination (Parse, MissingField, InvalidSignature, Expired, WrongDestination, Binding)
- Backward-compatible free-function binding API

**Stubbed / partial:**

- Encryption key-transport (XML-Enc library wrapper missing)
- Full c14n interop with strict third-party IdPs (pluggable via `canonicalize_fn` but default subset only)
- Artifact resolution store/retrieval (back-channel ArtifactResolve SOAP dispatch not implemented)

**Missing vs Keycloak 26.6.2:**

- LogoutRequest builder (SAML 2.0 Bindings §3.7)
- LogoutResponse parser
- ArtifactResolve SOAP back-channel resolution (client-side)
- Assertion encryption/decryption (XML-Enc, RFC 3394)
- EncryptedAssertion parsing+decryption
- RelayState parameter round-trip (HTTP parameter binding)
- Extensions element nesting (SamlProtocolExtensionsAwareBuilder pattern)
- ProxyRestriction element (SAML 2.0 §2.5.1.4)
- AuthnStatement delegation (via <saml:BaseID> or <saml:NameID>)
- Proxy-restricted assertion chains
- Attribute Authority queries (AttributeQuery protocol)
- Authorization Decision queries
- SAML 1.1 compatibility
- AssertionIDRequest / AssertionIDRequestService


### LDAP/AD User Federation + Kerberos/SPNEGO

*upstream ≈ 15,878 LOC · cave ≈ 4,579 LOC*

**Present (verified in cave):**

- LDAP bind/unbind (simple auth + SASL frame builder)
- RFC 4511 BER frame encoding for BindRequest/BindResponse
- LDAP filter AST + RFC 4515 string serialization
- Filter parsing (=, ~=, >=, <=, *, &, |, !)
- In-memory filter matching (for tests, offline mode)
- LdapSearchSpec + LdapQueryBuilder fluent API
- User attribute mapping (uid/mail/givenName/sn/cn/memberOf)
- Group membership sync: memberOf (user back-ref) and member (group listing) modes
- Group DN → CN extraction and projection
- Active Directory UAC flag parsing (AccountDisable, Lockout, PasswordExpired, DontExpirePassword)
- AD objectSid binary → S-1-5-21-... string parsing
- AD pwdLastSet FILETIME → Unix epoch conversion
- Result code enum (Success, InvalidCredentials, NoSuchObject, UnwillingToPerform)
- LdapError enum (Protocol, BindFailed, SearchFailed, MissingAttribute, FilterParse, Mapper)
- Kerberos GSSAPI InitialContextToken tag parsing (RFC 2743 §3.1)
- SPNEGO NegTokenInit/NegTokenResp ASN.1 DER parsing (RFC 4178 §4)
- HTTP Negotiate 401 + WWW-Authenticate: Negotiate challenge handler
- Keytab file format v0x0502 parser (kadmin ktadd format)
- Comprehensive unit tests for all parsing layers

**Stubbed / partial:**

- Kerberos ticket payload decryption (parsed to tag level only, RFC 4120 §5)
- SPNEGO mutual auth follow-up token (token frame built but not cryptographically signed)

**Missing vs Keycloak 26.6.2:**

- SearchRequest BER encoding (RFC 4511 §4.5)
- SearchResultEntry parsing (DN + attributes stream)
- SearchResultDone parsing and result-code handling
- Modify / ModifyRequest (RFC 4511 §4.6) for password updates
- AD password update via unicodePwd attribute (UTF-16LE encoding)
- LDAP password update via Modify or extended PasswordModify op
- User creation (add user to LDAP via Modify)
- User deletion from LDAP
- Persistent LDAP connection pooling + lifecycle mgmt
- Password validation with error code branch (weak password, policy constraints)
- Proxy user model wrapping (CredentialInputValidator, CredentialInputUpdater)
- User import / sync-on-demand logic
- LDAP mapper plugin system + LDAPStorageMapper base class
- Hardcoded attribute mappers
- Hardcoded role/group mappers
- MSAD password policy hint mapper (lock/expire detection)
- MSAD LDS account control mapper variant
- Full Kerberos Domain SID parsing (domain SID + RID decode)
- Live KDC integration (krb5_kt_get_entry call via libgssapi)
- Ticket cryptography (ENC-TGS-REP decryption, EncTicketPart verification)
- Mutual auth token generation + signing
- StartTLS upgrade (RFC 4513 §3)
- SASL EXTERNAL bind (SPNEGO + client-cert auth over TLS)


### JWT/JOSE Signing & Key Management

*upstream ≈ 9,642 LOC · cave ≈ 2,344 LOC*

**Present (verified in cave):**

- RS256 validation via jsonwebtoken crate (JWKS-backed)
- HS256 signing for dev tokens (cave_auth/src/auth_routes.rs:80)
- JWKS fetch and caching with 5-minute background refresh
- JWE RSA-OAEP key management (RFC 7518 §4.3)
- JWE ECDH-ES+A256KW key agreement (RFC 7518 §4.6)
- JWE content encryption: A256GCM and A128CBC-HS256 (RFC 7518 §5)
- Token introspection and revocation denylist (RFC 7662/7009)
- PAT (Personal Access Token) management with SHA-256 hashing
- Service-to-service token management
- Claims extraction for Okta and Keycloak formats (realm_access.roles + groups)
- PQC hybrid ML-DSA-65+Ed25519 signing (ADR-PORTAL-AUTH-001) with test coverage
- Token expiry validation and JTI denylist
- Role-to-permission mapping (platform-admin, tenant-admin, developer, auditor)
- Multi-key fallback during JWKS rotation (try_decode_jwt iterates all cached keys)

**Stubbed / partial:**

- ML-DSA-65 signing uses deterministic XOR-derivation instead of NIST FIPS 204 (derive_mldsa_key in pqc.rs:73-80)
- ML-DSA-65 verification compares XOR-derived expected sig rather than cryptographic verification (pqc.rs:145-149)

**Missing vs Keycloak 26.6.2:**

- PS256, PS384, PS512 (RSA-PSS) signing algorithms
- ES256, ES384, ES512 (ECDSA) signing algorithms
- EdDSA signature generation (only verification via test stubs)
- ML-DSA-44 and ML-DSA-87 (only ML-DSA-65 stub present)
- Explicit kid lookup and rotation policy (kid in header but no explicit-key-selection logic visible)
- JWE JSON serialization (RFC 7516 §7.2)
- JWE compression (zip parameter)
- HMAC signing/verification for HS384, HS512
- ECDH-ES+A128KW and ECDH-ES+A192KW (only A256KW implemented)
- Key provider abstraction (Keycloak has RSA/EC/EdDSA/HMAC/AES providers; cave uses jsonwebtoken crate only)
- Asymmetric key generation (cave validates only, signing deferred to IdP except PQC stub)


### Keycloak Admin REST API + Event/Audit Listeners

*upstream ≈ 16,930 LOC · cave ≈ 2,699 LOC*

**Present (verified in cave):**

- Admin CRUD flows (list/get/create/update/delete authentication flows)
- Admin CRUD executions (add/list/delete authentication executions)
- Admin CRUD required-actions (list/get/update/delete required action providers)
- Admin CRUD identity provider instances (list/get/create/update/delete)
- Admin CRUD identity provider mappers (list/get/create/update/delete)
- Email event listener (port of Keycloak EmailEventListenerProvider)
- SMTP outbox with retry and deadletter handling
- Audit logging via tracing (auth success/failure, authz decisions)
- RBAC engine with role bindings and resource policies
- JWT and PAT authentication middleware
- Email templates for events (new device, password change, MFA enrollment)

**Stubbed / partial:**

- Audit.rs is event-only (auth/authz), not admin-operation aware — needs extension to emit AdminEvent-like structures on CRUD operations
- No explicit guard_permission logic on admin flow/idp routes (they accept any authenticated request)
- Email listener only fires on auth events, not admin events (new user created, user updated, etc.)

**Missing vs Keycloak 26.6.2:**

- Admin event auditing (AdminEventBuilder pattern absent — no OperationType.CREATE/UPDATE/DELETE emission on admin mutations)
- AdminEventQuery interface and event store for querying admin operations
- Event listener SPI (EventListenerProvider interface missing — only email listener hardcoded)
- Fine-grained admin RBAC enforcement on admin endpoints (no require_permission! guards in flows/idp routes)
- Resource type auditing for 30+ resource types (USER, GROUP, CLIENT, REALM, etc.)
- Admin events endpoint (GET /admin/realms/{realm}/admin-events for querying)
- Representation capture in admin events (JSON diff on CREATE/UPDATE)
- Error event recording in admin audit trail
- RealmEventConfig persistence (event listeners enablement per realm)
- Event store provider SPI for persistence layer
- ClientsResource admin CRUD endpoints
- UsersResource admin CRUD endpoints
- RolesResource admin CRUD endpoints
- GroupsResource admin CRUD endpoints
- Partial-export endpoint (/partial-export)
- Partial-import endpoint (/partialImport)
- Event pruning/cleanup endpoints

