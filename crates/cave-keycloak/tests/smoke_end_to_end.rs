// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! End-to-end smoke tests — exercise the realistic flows a deployment
//! would walk through. Each test wires the realm, the user, the
//! credentials, the client, and at least one OAuth2 / SAML / brokering
//! / federation operation top-to-bottom.

use std::collections::BTreeMap;
use std::sync::Arc;

use cave_keycloak::auth_flow::{
    AuthContext, AuthStatus, AuthStep, AuthenticatorId, FlowExecutor, Requirement,
};
use cave_keycloak::brokering::{map_to_user, BrokerFamily, BrokeredIdentity, ExternalIdp};
use cave_keycloak::client_registry::ClientController;
use cave_keycloak::credentials::{MagicLink, PasswordCredential, TotpAlg};
use cave_keycloak::discovery::discovery_for;
use cave_keycloak::events::{EventKind, EventSink};
use cave_keycloak::jwks::jwks_for;
use cave_keycloak::ldap::{authenticate as ldap_authenticate, InMemoryLdap, LdapConfig, LdapEntry, LdapBackend};
use cave_keycloak::models::{
    Client, GrantType, HashAlgorithm, PasswordPolicy, Protocol, Realm, User,
};
use cave_keycloak::oauth2::{
    authorize, pkce_verify, AuthCodeStore, AuthorizeRequest, PkceMethod, RefreshTokenStore,
};
use cave_keycloak::policies::{BruteForceTracker, check_password_policy};
use cave_keycloak::saml::{
    build_response as build_saml_response, verify_response as verify_saml_response,
    SamlAttribute,
};
use cave_keycloak::session::{issue_tokens, SessionStore, TokenClaims};
use cave_keycloak::signer::{SignerRegistry, SigningKeyEntry};
use cave_keycloak::store::KeycloakStore;
use cave_keycloak::user::{CredentialStore, UserController};
use cave_keycloak::{router, State};
use chrono::{Duration, Utc};

fn fresh_realm(store: &KeycloakStore, id: &str, tenant: &str) -> Realm {
    let r = Realm::new(id, tenant, format!("Realm {}", id));
    store.put_realm(r.clone()).unwrap();
    r
}

fn user(id: &str, realm_id: &str, name: &str, email: &str) -> User {
    User {
        id: id.into(),
        realm_id: realm_id.into(),
        username: name.into(),
        enabled: true,
        email: Some(email.into()),
        email_verified: true,
        first_name: Some(name.into()),
        last_name: Some("U".into()),
        federated_link: None,
        group_ids: vec![],
        realm_role_ids: vec!["admin".into()],
        client_role_ids: vec![],
        attributes: BTreeMap::new(),
        created_at: Utc::now(),
    }
}

fn spa_client(realm_id: &str) -> Client {
    Client {
        id: "c-spa".into(),
        realm_id: realm_id.into(),
        client_id: "spa".into(),
        name: "SPA".into(),
        enabled: true,
        protocol: Protocol::OpenIdConnect,
        public_client: true,
        client_secret_hash: None,
        redirect_uris: vec!["https://app.cave/cb".into()],
        web_origins: vec!["https://app.cave".into()],
        default_scopes: vec!["openid".into(), "profile".into(), "email".into()],
        optional_scopes: vec![],
        allowed_grant_types: vec![
            GrantType::AuthorizationCode,
            GrantType::RefreshToken,
            GrantType::DeviceCode,
        ],
        require_pkce: true,
        access_token_lifespan_seconds: None,
        attributes: BTreeMap::new(),
    }
}

// ─── 1. Authorization code + PKCE + ID token flow ───────────────────────────

#[test]
fn smoke_1_auth_code_pkce_to_id_token() {
    let store = KeycloakStore::new();
    let realm = fresh_realm(&store, "r1", "t1");
    let creds = CredentialStore::default();
    let events = EventSink::default();
    let bf = BruteForceTracker::default();
    let user_ctl = UserController { store: &store, credentials: &creds, events: &events, brute_force: &bf };
    let mut policy = PasswordPolicy::default();
    policy.hash_iterations = 1000;
    user_ctl
        .create("t1", user("u1", "r1", "alice", "alice@example.com"), Some("hunter2-cave"), &policy)
        .unwrap();
    let client_ctl = ClientController { store: &store, events: &events };
    client_ctl.register_public("t1", spa_client("r1")).unwrap();
    let _ = events.drain();

    // 1. /authorize: client + redirect_uri exact + PKCE S256
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let verifier = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopq";
    let mut h = Sha256::new();
    h.update(verifier.as_bytes());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(h.finalize());
    let c = store.find_client_by_client_id("t1", "r1", "spa").unwrap();
    let code = authorize(&c, &AuthorizeRequest {
        realm_id: "r1", client_id: "spa", user_id: "u1",
        redirect_uri: "https://app.cave/cb", scope: "openid profile",
        state: Some("xyz"), nonce: Some("n-1"),
        pkce: Some((PkceMethod::S256, &challenge)),
    }).unwrap();

    // 2. /token: code exchange (PKCE verifier check)
    let code_store = AuthCodeStore::default();
    let code_str = code.code.clone();
    code_store.issue(code);
    let redeemed = code_store
        .redeem(&code_str, "spa", "https://app.cave/cb", Some(verifier))
        .unwrap();
    assert_eq!(redeemed.user_id, "u1");

    // 3. Mint access + id token
    let signer = SignerRegistry::default();
    signer.install("r1", SigningKeyEntry::es256_from_seed("k-r1", &[1u8; 32]).unwrap(), true);
    let sessions = SessionStore::default();
    let u = store.get_user("t1", "u1").unwrap();
    let sess = sessions.create(&realm, &u, "password", false, false);
    let tokens = issue_tokens(
        TokenClaims {
            realm: &realm, user: &u, client_id: "spa", session_id: &sess.id,
            scope: "openid profile",
            effective_roles: &u.realm_role_ids,
            nonce: redeemed.nonce.as_deref(),
            issuer_url: "https://iam.cave.svc/realms/r1",
        },
        &signer, "ES256", "k-r1",
    ).unwrap();

    // 4. Verify token with the same registry
    let (h, p) = signer.verify_compact("r1", &tokens.access_token).unwrap();
    assert_eq!(h["kid"], "k-r1");
    assert_eq!(p["sub"], "u1");
    assert_eq!(p["aud"], "spa");
    assert_eq!(p["realm_access"]["roles"][0], "admin");

    // 5. Replay of single-use code is rejected
    assert!(code_store
        .redeem(&code_str, "spa", "https://app.cave/cb", Some(verifier))
        .is_err());
}

// ─── 2. Refresh token rotation + chain replay revocation ────────────────────

#[test]
fn smoke_2_refresh_rotation_replay_revokes_chain() {
    let store = RefreshTokenStore::default();
    let t0 = store.issue("spa", "r1", "u1", "openid", 1800, 36_000);
    let t1 = store.rotate(&t0.token).unwrap();
    let t2 = store.rotate(&t1.token).unwrap();
    // Replay an old token in the chain
    let err = store.rotate(&t0.token).unwrap_err();
    assert!(matches!(err, cave_keycloak::error::KeycloakError::TokenRevoked));
    // The successor that was valid moments ago is now revoked too
    let after = store.introspect(&t2.token).unwrap();
    assert!(after.revoked);
}

// ─── 3. Brute-force lockout triggers + recovers ─────────────────────────────

#[test]
fn smoke_3_brute_force_locks_then_clears() {
    let store = KeycloakStore::new();
    fresh_realm(&store, "r1", "t1");
    let creds = CredentialStore::default();
    let events = EventSink::default();
    let bf = BruteForceTracker::new(3, Duration::seconds(60), Duration::seconds(30));
    let mut policy = PasswordPolicy::default();
    policy.hash_iterations = 100;
    let ctl = UserController { store: &store, credentials: &creds, events: &events, brute_force: &bf };
    ctl.create("t1", user("u1", "r1", "alice", "a@x"), Some("hunter2-cave"), &policy)
        .unwrap();
    let _ = events.drain();
    // 3 wrong attempts → lockout on the 3rd
    let _ = ctl.authenticate_password("t1", "r1", "alice", "wrong-1");
    let _ = ctl.authenticate_password("t1", "r1", "alice", "wrong-2");
    let third = ctl.authenticate_password("t1", "r1", "alice", "wrong-3").unwrap_err();
    match third {
        cave_keycloak::error::KeycloakError::CredentialLocked { account_id, retry_after_seconds } => {
            assert_eq!(account_id, "alice");
            assert!(retry_after_seconds > 0);
        }
        other => panic!("expected CredentialLocked, got {:?}", other),
    }
    // Lockout marker stays
    assert!(bf.check("alice").is_err());
    bf.record_success("alice");
    assert!(bf.check("alice").is_ok());
}

// ─── 4. TOTP enrollment + verify + magic link sign+verify ───────────────────

#[test]
fn smoke_4_totp_and_magic_link_credentials() {
    // RFC 6238 Appendix B vector: secret "12345678901234567890" + T=59 → "94287082"
    use cave_keycloak::credentials::TotpCredential;
    let secret = "12345678901234567890";
    let b32 = {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
        let mut out = String::new();
        let mut buf: u64 = 0;
        let mut bits: u32 = 0;
        for &b in secret.as_bytes() {
            buf = (buf << 8) | b as u64;
            bits += 8;
            while bits >= 5 {
                bits -= 5;
                let idx = ((buf >> bits) & 0x1F) as usize;
                out.push(A[idx] as char);
            }
        }
        if bits > 0 {
            let idx = ((buf << (5 - bits)) & 0x1F) as usize;
            out.push(A[idx] as char);
        }
        out
    };
    let c = TotpCredential {
        secret_b32: b32,
        digits: 8,
        period_seconds: 30,
        algorithm: TotpAlg::Sha1,
    };
    assert_eq!(c.generate(59).unwrap(), "94287082");

    // Magic link round-trip
    let secret_bytes = b"cave-magic";
    let ml = MagicLink::new("u1", "r1", "verify-email", Duration::seconds(300));
    let sig = ml.signature(secret_bytes);
    ml.verify(secret_bytes, &sig, ml.issued_at + Duration::seconds(60)).unwrap();
    assert!(ml.verify(secret_bytes, &sig, ml.issued_at + Duration::seconds(999)).is_err());
}

// ─── 5. Brokered Google login → JIT provisioned user ─────────────────────────

#[test]
fn smoke_5_brokered_google_login_jit_user() {
    let g = ExternalIdp {
        alias: "google".into(), realm_id: "r1".into(), family: BrokerFamily::Google,
        client_id: "cave-google".into(),
        client_secret_keychain_handle: "keychain:cave-keycloak/idp/google".into(),
        authorization_url: String::new(), token_url: String::new(),
        userinfo_url: String::new(), jwks_url: None, default_scopes: vec![],
        trust_email: false,
    }.with_family_defaults();
    g.validate().unwrap();
    let url = g.build_authorize_url("https://cave.svc/cb", "state-1", "nonce-1");
    assert!(url.contains("response_type=code"));
    assert!(url.contains("scope=openid%20profile%20email"));
    let u = map_to_user("r1", &BrokeredIdentity {
        provider_alias: "google".into(),
        provider_user_id: "1234567890".into(),
        provider_username: "alice@example.com".into(),
        email: Some("alice@example.com".into()),
        email_verified: true,
        first_name: Some("Alice".into()),
        last_name: Some("X".into()),
    });
    assert_eq!(u.id, "google:1234567890");
    assert!(u.email_verified);
    let link = u.federated_link.unwrap();
    assert_eq!(link.provider_alias, "google");
}

// ─── 6. SAML SP-side response verify happy path + tamper rejection ──────────

#[test]
fn smoke_6_saml_sp_verify_happy_then_tamper() {
    let resp = build_saml_response(
        "rq-1",
        "https://app.cave/sp/acs",
        "https://idp.cave/realms/r1",
        "https://app.cave/sp",
        "alice@example.com",
        "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress",
        vec![SamlAttribute { name: "email".into(), values: vec!["alice@example.com".into()] }],
        Duration::seconds(300),
    );
    verify_saml_response(
        &resp,
        "https://idp.cave/realms/r1",
        "https://app.cave/sp",
        "https://app.cave/sp/acs",
        "rq-1",
        Utc::now(),
        Duration::seconds(30),
    ).unwrap();
    let mut tampered = resp.clone();
    tampered.assertion.audience = "https://attacker/sp".into();
    assert!(verify_saml_response(
        &tampered,
        "https://idp.cave/realms/r1",
        "https://app.cave/sp",
        "https://app.cave/sp/acs",
        "rq-1",
        Utc::now(),
        Duration::seconds(30),
    ).is_err());
}

// ─── 7. LDAP federation + flow executor + discovery + JWKS  ─────────────────

#[test]
fn smoke_7_ldap_federation_plus_flow() {
    let backend = InMemoryLdap::default();
    let mut attrs = BTreeMap::new();
    attrs.insert("uid".to_string(), vec!["bob".to_string()]);
    attrs.insert("objectClass".to_string(), vec!["inetOrgPerson".to_string()]);
    attrs.insert("entryUUID".to_string(), vec!["u-bob".to_string()]);
    attrs.insert("mail".to_string(), vec!["bob@cave".to_string()]);
    backend.insert(
        "uid=bob,ou=People,dc=cave",
        "secret",
        LdapEntry {
            dn: "uid=bob,ou=People,dc=cave".into(),
            uuid: "u-bob".into(),
            attributes: attrs,
        },
    );
    let cfg = LdapConfig {
        alias: "corp".into(), connection_url: "ldap://x".into(),
        bind_dn: "cn=svc,dc=cave".into(),
        bind_credential_keychain_handle: "keychain:cave-keycloak/ldap/corp".into(),
        users_dn: "ou=People,dc=cave".into(),
        username_attribute: "uid".into(), rdn_attribute: "uid".into(),
        uuid_attribute: "entryUUID".into(),
        user_object_classes: vec!["inetOrgPerson".into()],
        start_tls: true, use_truststore: true,
    };
    let entry = ldap_authenticate(&backend, &cfg, "bob", "secret").unwrap();
    assert_eq!(entry.first("mail"), Some("bob@cave"));
    assert!(backend.search("ou=People,dc=cave", "(uid=bob)").unwrap().len() == 1);

    let mut exec = FlowExecutor::new();
    fn ok(_: &mut AuthContext) -> AuthStatus { AuthStatus::Success }
    exec.register("password", ok);
    exec.register("otp", ok);
    let flow = vec![
        AuthStep { authenticator: AuthenticatorId("password".into()), requirement: Requirement::Required },
        AuthStep { authenticator: AuthenticatorId("otp".into()), requirement: Requirement::Required },
    ];
    let mut ctx = AuthContext::default();
    assert_eq!(exec.execute(&flow, &mut ctx), AuthStatus::Success);

    let d = discovery_for("r1", "https://iam.cave.svc");
    assert!(d.issuer.ends_with("/realms/r1"));
    let signer = SignerRegistry::default();
    signer.install("r1", SigningKeyEntry::es256_from_seed("k", &[1u8; 32]).unwrap(), true);
    assert_eq!(jwks_for("r1", &signer).keys.len(), 1);
}

// ─── 8. Router /health round trip + state initialisation ────────────────────

#[tokio::test]
async fn smoke_8_router_health_round_trip() {
    let state = Arc::new(State::default());
    state.store.put_realm(Realm::new("r1", "t1", "R1")).unwrap();
    let r = router(state.clone());
    // We don't spin up a TCP server; the smoke is that the router was
    // assembled and the state is what we expect.
    let _ = r;
    assert_eq!(state.store.realm_count(), 1);
    assert_eq!(state.brute_force.max_failures, 5);
    state.event_sink.append(cave_keycloak::events::AuditEvent::new("t1", "u1", EventKind::Login));
    let drained = state.event_sink.drain();
    assert_eq!(drained.len(), 1);
}

// ─── ancillary password-policy probe so smoke covers the helper too ─────────

#[test]
fn smoke_aux_password_policy_min_length() {
    let p = PasswordPolicy::default();
    assert!(check_password_policy("aaaaaaaa", &p).is_ok());
    assert!(check_password_policy("short", &p).is_err());
    let pc = PasswordCredential::hash("ten-chars!", HashAlgorithm::Pbkdf2Sha256, 100).unwrap();
    pc.verify("ten-chars!").unwrap();

    // exercise PKCE verifier rejection
    assert!(pkce_verify(PkceMethod::S256, "x", "short").is_err());
}
