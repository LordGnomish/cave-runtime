// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Charter v2 self-audit — cave-keycloak must carry an honest, measured
//! `fill_ratio` against upstream keycloak/keycloak v26.6.2, a pinned
//! `source_sha` for reproducibility, the 2026-05-23 close-out audit date,
//! `parity_ratio_source = "manifest"`, 100% AGPL SPDX header coverage,
//! no stub macros in `src/`, mapped+partial+skipped+unmapped summing
//! to total, and the full realm/user/role/client/credential/oauth2
//! surface reachable through `cave_keycloak`.
//!
//! 9 assertions — one per gate of the close-out checklist.

use std::fs;
use std::path::PathBuf;

const TODAY: &str = "2026-05-23";
const FLOOR_FILL_RATIO: f64 = 0.65;
const KEYCLOAK_VERSION: &str = "v26.6.2";
const KEYCLOAK_SHA: &str = "0a402f777f8985eccbb07556e96d9b386275e048";

fn manifest_text() -> String {
    let p: PathBuf = [env!("CARGO_MANIFEST_DIR"), "parity.manifest.toml"]
        .iter()
        .collect();
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {:?}: {}", p, e))
}

fn extract_after(text: &str, needle: &str) -> Option<String> {
    let i = text.find(needle)?;
    let rest = &text[i + needle.len()..];
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let line = &rest[..line_end];
    let stripped = line.trim().trim_start_matches('=').trim();
    let comment_split = stripped.split('#').next().unwrap_or(stripped).trim();
    let unquoted = comment_split.trim_matches('"');
    Some(unquoted.to_string())
}

// ─── Assertion 1: keycloak upstream pinned to v26.6.2 ───────────────────────

#[test]
fn assertion_1_keycloak_version_pinned() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some(KEYCLOAK_VERSION),
        "[upstream] version must pin Keycloak {} — Charter v2 always-latest gate (got {:?})",
        KEYCLOAK_VERSION,
        v
    );
}

// ─── Assertion 2: source_sha matches keycloak v26.6.2 tag commit ────────────

#[test]
fn assertion_2_source_sha_matches_version() {
    let m = manifest_text();
    assert!(
        m.contains(KEYCLOAK_SHA),
        "[upstream] keycloak source_sha must contain {} (manifest text scan)",
        KEYCLOAK_SHA
    );
}

// ─── Assertion 3: fill_ratio >= 0.65 ────────────────────────────────────────

#[test]
fn assertion_3_fill_ratio_meets_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="))
        .expect("[parity] fill_ratio must be present");
    let ratio: f64 = raw.parse().expect("fill_ratio must parse as float");
    assert!(
        ratio >= FLOOR_FILL_RATIO,
        "cave-keycloak MVP floor: fill_ratio must be >= {} (got {})",
        FLOOR_FILL_RATIO,
        ratio
    );
    assert!(
        ratio <= 1.0,
        "fill_ratio must be a fraction (got {})",
        ratio
    );
}

// ─── Assertion 4: parity_ratio_source = "manifest" ──────────────────────────

#[test]
fn assertion_4_parity_ratio_source_is_manifest() {
    let m = manifest_text();
    let v = extract_after(&m, "\nparity_ratio_source ")
        .or_else(|| extract_after(&m, "\nparity_ratio_source="));
    assert_eq!(
        v.as_deref(),
        Some("manifest"),
        "parity_ratio_source must be \"manifest\" (got {:?})",
        v
    );
}

// ─── Assertion 5: last_audit == 2026-05-23 ──────────────────────────────────

#[test]
fn assertion_5_last_audit_is_today() {
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    assert_eq!(
        when.as_deref(),
        Some(TODAY),
        "[parity] last_audit must reflect the {} Charter v2 close-out (got {:?})",
        TODAY,
        when
    );
}

// ─── Assertion 6: counts sum to total + >= 15 mapped ────────────────────────

#[test]
fn assertion_6_counts_sum_to_total() {
    let m = manifest_text();
    let read = |k: &str| -> Option<u64> {
        let s = extract_after(&m, &format!("\n{} ", k))
            .or_else(|| extract_after(&m, &format!("\n{}=", k)))?;
        s.parse().ok()
    };
    let mapped = read("mapped_count").expect("mapped_count");
    let partial = read("partial_count").expect("partial_count");
    let skipped = read("skipped_count").expect("skipped_count");
    let unmapped = read("unmapped_count").expect("unmapped_count");
    let total = read("total").expect("total");
    assert_eq!(
        mapped + partial + skipped + unmapped,
        total,
        "mapped+partial+skipped+unmapped must equal total"
    );
    assert!(
        mapped >= 15,
        "cave-keycloak MVP floor: >= 15 mapped Keycloak subsystems (got {})",
        mapped
    );
}

// ─── Assertion 7: AGPL SPDX header coverage 100% ────────────────────────────

#[test]
fn assertion_7_agpl_spdx_header_coverage() {
    let root: PathBuf = [env!("CARGO_MANIFEST_DIR")].iter().collect();
    let mut missing = Vec::new();
    let mut total = 0usize;
    walk(&root, &mut |p| {
        if p.extension().map(|e| e == "rs").unwrap_or(false) {
            total += 1;
            let head = fs::read_to_string(p)
                .ok()
                .and_then(|s| s.lines().next().map(|l| l.to_string()))
                .unwrap_or_default();
            if !head.contains("SPDX-License-Identifier: AGPL-3.0-or-later") {
                missing.push(p.display().to_string());
            }
        }
    });
    assert!(
        missing.is_empty(),
        "{} of {} .rs files missing AGPL SPDX header: {:?}",
        missing.len(),
        total,
        missing
    );
    assert!(
        total >= 15,
        "expected >= 15 .rs files in cave-keycloak; got {}",
        total
    );
}

// ─── Assertion 8: no stub macros in src/ ────────────────────────────────────

#[test]
fn assertion_8_no_stub_macros_in_src() {
    let src: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src"].iter().collect();
    let mut offenders: Vec<String> = Vec::new();
    walk(&src, &mut |p| {
        if !p.extension().map(|e| e == "rs").unwrap_or(false) {
            return;
        }
        let Ok(text) = fs::read_to_string(p) else {
            return;
        };
        for (lineno, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if trimmed.contains("todo!(")
                || trimmed.contains("unimplemented!(")
                || trimmed.contains("panic!(\"stub")
                || trimmed.contains("panic!(\"todo")
            {
                offenders.push(format!("{}:{}: {}", p.display(), lineno + 1, line.trim()));
            }
        }
    });
    assert!(
        offenders.is_empty(),
        "Charter v2 no-stub gate failed in src/:\n{}",
        offenders.join("\n")
    );
}

// ─── Assertion 9: full IAM surface reachable through cave_keycloak ──────────

#[test]
fn assertion_9_keycloak_surface_intact() {
    use cave_keycloak::auth_flow::{
        pending_required_actions, AuthContext, AuthStatus, AuthStep, AuthenticatorId,
        FlowExecutor, Requirement,
    };
    use cave_keycloak::brokering::{map_to_user, BrokerFamily, BrokeredIdentity, ExternalIdp};
    use cave_keycloak::client_registry::ClientController;
    use cave_keycloak::credentials::{
        fingerprint_hex, webauthn_verify_assertion, MagicLink, PasswordCredential,
        TotpAlg, TotpCredential, WebauthnAlg, WebauthnCredential,
    };
    use cave_keycloak::discovery::discovery_for;
    use cave_keycloak::events::{AuditEvent, EventKind, EventSink};
    use cave_keycloak::jwks::jwks_for;
    use cave_keycloak::ldap::{authenticate as ldap_authenticate, InMemoryLdap, LdapBackend, LdapConfig, LdapEntry};
    use cave_keycloak::metrics::{standard_alerts, standard_panels};
    use cave_keycloak::models::{
        Client, GrantType, HashAlgorithm, PasswordPolicy, Protocol, Realm, Role, User,
    };
    use cave_keycloak::oauth2::{
        authorize, jitter, pkce_verify, AuthCodeStore, AuthorizeRequest, DeviceCodeStore,
        IntrospectionResponse, PkceMethod, RefreshTokenStore,
    };
    use cave_keycloak::policies::{
        check_password_policy, evaluate, AccessDecision, BruteForceTracker, ConditionalContext,
        ConditionalRule,
    };
    use cave_keycloak::realm::RealmController;
    use cave_keycloak::role::RoleController;
    use cave_keycloak::saml::{
        build_response as build_saml_response, sp_metadata_xml, verify_response as verify_saml_response,
        SamlAttribute, SamlAuthnRequest,
    };
    use cave_keycloak::session::{issue_tokens, random_token, SessionStore, TokenClaims};
    use cave_keycloak::signer::{jwk_thumbprint, JwsAlg, SignerRegistry, SigningKeyEntry};
    use cave_keycloak::store::{check_tenant, KeycloakStore};
    use cave_keycloak::user::{CredentialStore, UserController};
    use cave_keycloak::{router, State, MODULE_NAME};
    use std::collections::BTreeMap;
    use std::sync::Arc;

    // 1. Module identity + router can be built ─────────────────────────────────
    assert_eq!(MODULE_NAME, "keycloak");
    let _r = router(Arc::new(State::default()));

    // 2. Store + tenant guard ─────────────────────────────────────────────────
    let store = KeycloakStore::new();
    let realm = Realm::new("r1", "t1", "R1");
    store.put_realm(realm.clone()).unwrap();
    assert!(check_tenant("t1", "t1").is_ok());
    assert!(check_tenant("t1", "t2").is_err());

    // 3. User + credentials + policy ──────────────────────────────────────────
    let events = EventSink::default();
    let creds = CredentialStore::default();
    let bf = BruteForceTracker::default();
    let mut policy = PasswordPolicy::default();
    policy.hash_iterations = 1000;
    let realm_ctl = RealmController { store: &store, events: &events };
    let user_ctl = UserController {
        store: &store,
        credentials: &creds,
        events: &events,
        brute_force: &bf,
    };
    let u = User {
        id: "u1".into(),
        realm_id: "r1".into(),
        username: "alice".into(),
        enabled: true,
        email: Some("alice@example.com".into()),
        email_verified: true,
        first_name: Some("Alice".into()),
        last_name: Some("X".into()),
        federated_link: None,
        group_ids: vec![],
        realm_role_ids: vec![],
        client_role_ids: vec![],
        attributes: BTreeMap::new(),
        created_at: chrono::Utc::now(),
    };
    user_ctl.create("t1", u.clone(), Some("hunter2-cave"), &policy).unwrap();
    assert!(check_password_policy("hunter2-cave", &policy).is_ok());
    assert!(check_password_policy("short", &policy).is_err());
    let _back = user_ctl.authenticate_password("t1", "r1", "alice", "hunter2-cave").unwrap();

    // 4. TOTP + WebAuthn + magic link surface present ─────────────────────────
    let _totp = TotpCredential {
        secret_b32: "JBSWY3DPEHPK3PXP".into(),
        digits: 6,
        period_seconds: 30,
        algorithm: TotpAlg::Sha1,
    };
    let _ml = MagicLink::new("u1", "r1", "verify-email", chrono::Duration::seconds(300));
    let _w = WebauthnCredential {
        credential_id: "id".into(),
        public_key_bytes: vec![0u8; 32],
        algorithm: WebauthnAlg::Ed25519,
        sign_count: 0,
    };
    let _ = PasswordCredential::hash("p", HashAlgorithm::Pbkdf2Sha256, 100).unwrap();
    // exercise webauthn_verify_assertion API existence (only the symbol, not a happy-path verify)
    let _ = webauthn_verify_assertion as fn(_, _, _, _) -> _;
    let _ = fingerprint_hex(b"x");

    // 5. Role controller + composite expansion ─────────────────────────────────
    let role_ctl = RoleController { store: &store, events: &events };
    role_ctl.create("t1", Role { id: "admin".into(), realm_id: "r1".into(), client_id: None, name: "admin".into(), description: None, composite_ids: vec!["editor".into()] }).unwrap();
    role_ctl.create("t1", Role { id: "editor".into(), realm_id: "r1".into(), client_id: None, name: "editor".into(), description: None, composite_ids: vec![] }).unwrap();
    role_ctl.assign_to_user("t1", "u1", "admin").unwrap();
    let u_after = store.get_user("t1", "u1").unwrap();
    let eff = role_ctl.effective_role_ids("t1", &u_after).unwrap();
    assert!(eff.contains("editor"));

    // 6. Client controller + OAuth2 authorize/refresh ──────────────────────────
    let client_ctl = ClientController { store: &store, events: &events };
    let c = Client {
        id: "c1".into(),
        realm_id: "r1".into(),
        client_id: "spa".into(),
        name: "SPA".into(),
        enabled: true,
        protocol: Protocol::OpenIdConnect,
        public_client: true,
        client_secret_hash: None,
        redirect_uris: vec!["https://app/cb".into()],
        web_origins: vec![],
        default_scopes: vec!["openid".into(), "profile".into()],
        optional_scopes: vec![],
        allowed_grant_types: vec![GrantType::AuthorizationCode, GrantType::RefreshToken, GrantType::DeviceCode],
        require_pkce: true,
        access_token_lifespan_seconds: None,
        attributes: BTreeMap::new(),
    };
    client_ctl.register_public("t1", c.clone()).unwrap();
    let verifier = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopq";
    use sha2::{Digest, Sha256};
    use base64::Engine;
    let mut h = Sha256::new();
    h.update(verifier.as_bytes());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(h.finalize());
    let code_store = AuthCodeStore::default();
    let ac = authorize(&c, &AuthorizeRequest {
        realm_id: "r1", client_id: "spa", user_id: "u1",
        redirect_uri: "https://app/cb", scope: "openid profile",
        state: None, nonce: Some("n-1"),
        pkce: Some((PkceMethod::S256, &challenge)),
    }).unwrap();
    let code_str = ac.code.clone();
    code_store.issue(ac);
    let _redeemed = code_store.redeem(&code_str, "spa", "https://app/cb", Some(verifier)).unwrap();
    pkce_verify(PkceMethod::S256, &challenge, verifier).unwrap();
    let rt = RefreshTokenStore::default();
    let t0 = rt.issue("spa", "r1", "u1", "openid", 1800, 36_000);
    let _t1 = rt.rotate(&t0.token).unwrap();
    assert!(rt.rotate(&t0.token).is_err()); // replay → chain revoked
    let dc = DeviceCodeStore::default();
    let dev = dc.issue(&c, "r1", "openid profile").unwrap();
    assert!(!dev.user_code.is_empty());
    let _inactive = IntrospectionResponse::inactive();
    let _ = jitter(chrono::Duration::seconds(60));

    // 7. Signer + JWKS + discovery + token assembly ───────────────────────────
    let signer = SignerRegistry::default();
    signer.install("r1", SigningKeyEntry::es256_from_seed("k-r1", &[7u8; 32]).unwrap(), true);
    signer.install("r1", SigningKeyEntry::eddsa_from_seed("k-ed", &[8u8; 32]), false);
    signer.install("r1", SigningKeyEntry::mldsa65_placeholder("k-pqc"), false);
    let jwks = jwks_for("r1", &signer);
    assert_eq!(jwks.keys.len(), 3);
    let _ = jwk_thumbprint(&SigningKeyEntry::es256_from_seed("x", &[1u8; 32]).unwrap());
    assert_eq!(JwsAlg::MlDsa65.jose_str(), "ML-DSA-65");
    let disc = discovery_for("r1", "https://iam.cave.svc");
    assert!(disc.id_token_signing_alg_values_supported.contains(&"ES256".to_string()));
    let sessions = SessionStore::default();
    let sess = sessions.create(&realm, &u, "password", false, false);
    let tokens = issue_tokens(
        TokenClaims {
            realm: &realm, user: &u, client_id: "spa", session_id: &sess.id,
            scope: "openid profile",
            effective_roles: &["admin".to_string()],
            nonce: Some("n-1"),
            issuer_url: "https://iam.cave.svc/realms/r1",
        },
        &signer, "ES256", "k-r1",
    ).unwrap();
    assert!(tokens.access_token.contains('.'));
    let _rt = random_token(16);

    // 8. SAML + LDAP + brokering + conditional access ──────────────────────────
    let req = SamlAuthnRequest {
        id: "rq-1".into(),
        issue_instant: chrono::Utc::now(),
        destination: "https://idp.cave/realms/r1/saml".into(),
        issuer: "https://app.cave/sp".into(),
        assertion_consumer_service_url: "https://app.cave/sp/acs".into(),
        name_id_format: "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress".into(),
        force_authn: false,
        is_passive: false,
        relay_state: None,
    };
    let _xml = req.to_xml();
    let resp = build_saml_response(
        "rq-1",
        "https://app.cave/sp/acs",
        "https://idp.cave/realms/r1",
        "https://app.cave/sp",
        "alice@example.com",
        "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress",
        vec![SamlAttribute { name: "email".into(), values: vec!["alice@example.com".into()] }],
        chrono::Duration::seconds(300),
    );
    verify_saml_response(&resp, "https://idp.cave/realms/r1", "https://app.cave/sp", "https://app.cave/sp/acs", "rq-1", chrono::Utc::now(), chrono::Duration::seconds(30)).unwrap();
    let _md = sp_metadata_xml("eid", "acs", "sls");

    let ldap = InMemoryLdap::default();
    let mut attrs = BTreeMap::new();
    attrs.insert("uid".to_string(), vec!["alice".to_string()]);
    attrs.insert("objectClass".to_string(), vec!["inetOrgPerson".to_string()]);
    attrs.insert("entryUUID".to_string(), vec!["u-1".to_string()]);
    ldap.insert("uid=alice,ou=People,dc=cave", "secret", LdapEntry { dn: "uid=alice,ou=People,dc=cave".into(), uuid: "u-1".into(), attributes: attrs });
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
    let _entry = ldap_authenticate(&ldap, &cfg, "alice", "secret").unwrap();
    assert!(ldap.search("ou=People,dc=cave", "(uid=alice)").unwrap().len() == 1);

    let idp = ExternalIdp {
        alias: "google".into(), realm_id: "r1".into(), family: BrokerFamily::Google,
        client_id: "cid".into(),
        client_secret_keychain_handle: "keychain:cave-keycloak/idp/google".into(),
        authorization_url: String::new(), token_url: String::new(),
        userinfo_url: String::new(), jwks_url: None, default_scopes: vec![],
        trust_email: false,
    }.with_family_defaults();
    let url = idp.build_authorize_url("https://cave/cb", "s", "n");
    assert!(url.contains("accounts.google.com"));
    let _u2 = map_to_user("r1", &BrokeredIdentity {
        provider_alias: "google".into(), provider_user_id: "123".into(),
        provider_username: "alice".into(), email: Some("alice@x".into()),
        email_verified: true, first_name: None, last_name: None,
    });

    let dec = evaluate(
        &[ConditionalRule::DenyClient { client_id: "evil".into() }],
        &ConditionalContext { client_id: "evil".into(), ip_address: "10.0.0.1".into(), last_auth_age_seconds: 0, mfa_present: true },
    );
    assert!(matches!(dec, AccessDecision::Deny(_)));

    // 9. Auth flow executor + observability + events ──────────────────────────
    let mut flow_exec = FlowExecutor::new();
    fn pass(_: &mut AuthContext) -> AuthStatus { AuthStatus::Success }
    flow_exec.register("ok", pass);
    let steps = vec![AuthStep { authenticator: AuthenticatorId("ok".into()), requirement: Requirement::Required }];
    let mut ctx = AuthContext::default();
    assert_eq!(flow_exec.execute(&steps, &mut ctx), AuthStatus::Success);
    let _ = pending_required_actions(&BTreeMap::new());

    assert_eq!(standard_panels().len(), 10);
    assert_eq!(standard_alerts().len(), 6);
    let _ = AuditEvent::new("t", "s", EventKind::Login).with_client("c").with_ip("1.2.3.4").with_detail("d");

    let _ = realm_ctl.list("t1");

    // sanity: at least one event has happened by now
    assert!(events.snapshot().len() >= 1);
}

// ─── helpers ────────────────────────────────────────────────────────────────

fn walk(dir: &PathBuf, cb: &mut dyn FnMut(&PathBuf)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name()
                .map(|n| n.to_string_lossy().starts_with('.'))
                .unwrap_or(false)
            {
                continue;
            }
            if p.file_name().map(|n| n == "target").unwrap_or(false) {
                continue;
            }
            walk(&p, cb);
        } else {
            cb(&p);
        }
    }
}
