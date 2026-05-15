// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j@v0.24.0 + keycloak@v22.0.0 services/.../webauthn + W3C WebAuthn L2
//
// W3C WebAuthn L2 spec test vectors + 5 register-then-authenticate round
// trips covering ES256, EdDSA, signCount monotonicity, passkey discoverable
// flow, and registration→assertion replay rejection. These ports drive the
// public API end-to-end; webauthn4j keeps the same in
// `webauthn4j-test/src/test/java/com/webauthn4j/test/E2EIntegrationTest.java`.

use ciborium::value::Value;
use sha2::{Digest, Sha256};

use crate::webauthn::attestation;
use crate::webauthn::authentication::{
    AssertionOptions, AssertionResponse, AuthenticationManager,
};
use crate::webauthn::authenticator_data::{self, AuthFlags};
use crate::webauthn::cbor;
use crate::webauthn::client_data;
use crate::webauthn::cose::{self, CoseKey};
use crate::webauthn::credential_store::InMemoryCredentialStore;
use crate::webauthn::registration::{
    self, AttestationResponse, RegistrationManager, RegistrationOptions,
    ResidentKeyRequirement, UserVerification,
};

fn b64u(s: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s)
}

fn client_data(typ: &str, challenge: &[u8], origin: &str) -> Vec<u8> {
    format!(
        r#"{{"type":"{typ}","challenge":"{ch}","origin":"{origin}","crossOrigin":false}}"#,
        typ = typ,
        ch = b64u(challenge),
        origin = origin
    )
    .into_bytes()
}

fn auth_data_with_cred(
    rp_id: &str,
    flags: AuthFlags,
    sign_count: u32,
    cred_id: &[u8],
    aaguid: [u8; 16],
    cose_bytes: &[u8],
) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&authenticator_data::rp_id_hash(rp_id));
    v.push(flags.bits());
    v.extend_from_slice(&sign_count.to_be_bytes());
    v.extend_from_slice(&aaguid);
    v.extend_from_slice(&(cred_id.len() as u16).to_be_bytes());
    v.extend_from_slice(cred_id);
    v.extend_from_slice(cose_bytes);
    v
}

fn auth_data_assertion(rp_id: &str, flags: AuthFlags, sign_count: u32) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&authenticator_data::rp_id_hash(rp_id));
    v.push(flags.bits());
    v.extend_from_slice(&sign_count.to_be_bytes());
    v
}

/// W3C §C.3 example — clientDataJSON canonical form parses cleanly.
#[test]
fn w3c_appendix_c3_client_data_parses() {
    // Modeled on the W3C example block; we substitute deterministic values so
    // we can verify the parse + the helper roundtrip.
    let raw = client_data(
        "webauthn.create",
        b"random-challenge",
        "https://login.cave.dev",
    );
    let cd = client_data::parse(&raw).unwrap();
    assert_eq!(cd.typ, "webauthn.create");
    assert!(cd.challenge.contains("cmFuZG9t"));
    assert_eq!(cd.origin, "https://login.cave.dev");
}

/// Integration ceremony #1 — EdDSA registration + EdDSA assertion roundtrip.
#[test]
fn ceremony_1_eddsa_register_then_authenticate() {
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    let sk = SigningKey::generate(&mut OsRng);
    let vk = sk.verifying_key();
    let cose_key = CoseKey::EdDsa { x: vk.to_bytes() };
    let cose_bytes = cose::encode(&cose_key).unwrap();
    let cred_id = b"e2e-cred-1".to_vec();
    let challenge_reg = b"reg-chal".to_vec();

    // Register with BE+BS set — this is the typical passkey registration
    // shape and lets the assertion path also surface BE without tripping the
    // "BE flag changed false->true" guard.
    let auth_data = auth_data_with_cred(
        "login.cave.dev",
        AuthFlags::UP | AuthFlags::UV | AuthFlags::AT | AuthFlags::BE | AuthFlags::BS,
        0,
        &cred_id,
        [0xee; 16],
        &cose_bytes,
    );
    let att_obj = registration::build_attestation_object_none(&auth_data);
    let cd_reg = client_data("webauthn.create", &challenge_reg, "https://login.cave.dev");
    let store = InMemoryCredentialStore::new();
    let reg_mgr = RegistrationManager {
        rp_id: "login.cave.dev".into(),
        origin: "https://login.cave.dev".into(),
        store,
    };
    let _ = reg_mgr
        .verify(
            &RegistrationOptions {
                challenge: challenge_reg,
                rp_id: "login.cave.dev".into(),
                rp_name: "Cave".into(),
                user_id: b"user-1".to_vec(),
                user_name: "alice".into(),
                user_display_name: "Alice".into(),
                user_verification: UserVerification::Required,
                exclude_credentials: vec![],
                resident_key: ResidentKeyRequirement::Preferred,
            },
            &AttestationResponse {
                client_data_json: cd_reg,
                attestation_object: att_obj,
                transports: vec!["internal".into()],
            },
        )
        .expect("registration must succeed");

    // Now do an assertion ceremony.
    let challenge_auth = b"auth-chal-eddsa".to_vec();
    let auth_data_a = auth_data_assertion(
        "login.cave.dev",
        AuthFlags::UP | AuthFlags::UV | AuthFlags::BE | AuthFlags::BS,
        1,
    );
    let cd_auth = client_data("webauthn.get", &challenge_auth, "https://login.cave.dev");
    let mut hasher = Sha256::new();
    hasher.update(&cd_auth);
    let cdh: [u8; 32] = hasher.finalize().into();
    let mut signed = auth_data_a.clone();
    signed.extend_from_slice(&cdh);
    let sig = sk.sign(&signed);

    // Note: we re-use the same store from reg_mgr by re-borrowing.
    let auth_mgr = AuthenticationManager::new(
        reg_mgr.store,
        "login.cave.dev",
        "https://login.cave.dev",
    );
    let stored = auth_mgr
        .verify(
            &AssertionOptions {
                challenge: challenge_auth,
                rp_id: "login.cave.dev".into(),
                user_verification: UserVerification::Required,
                allow_credentials: vec![cred_id.clone()],
            },
            &AssertionResponse {
                credential_id: cred_id.clone(),
                client_data_json: cd_auth,
                authenticator_data: auth_data_a,
                signature: sig.to_bytes().to_vec(),
                user_handle: Some(b"user-1".to_vec()),
            },
        )
        .expect("assertion must succeed");
    assert_eq!(stored.credential_id, cred_id);
    assert_eq!(stored.sign_count, 1);
}

/// Integration ceremony #2 — ES256 registration + ES256 assertion roundtrip.
#[test]
fn ceremony_2_es256_register_then_authenticate() {
    use p256::ecdsa::{signature::Signer as _, SigningKey};
    use rand::rngs::OsRng;

    let sk = SigningKey::random(&mut OsRng);
    let vk = sk.verifying_key();
    let pt = vk.to_encoded_point(false);
    let x: [u8; 32] = (*pt.x().unwrap()).into();
    let y: [u8; 32] = (*pt.y().unwrap()).into();
    let cose_key = CoseKey::Es256 { x, y };
    let cose_bytes = cose::encode(&cose_key).unwrap();

    let cred_id = b"e2e-cred-2".to_vec();
    let challenge_reg = b"es256-reg".to_vec();
    let auth_data = auth_data_with_cred(
        "login.cave.dev",
        AuthFlags::UP | AuthFlags::UV | AuthFlags::AT,
        0,
        &cred_id,
        [0xee; 16],
        &cose_bytes,
    );
    let att_obj = registration::build_attestation_object_none(&auth_data);
    let cd_reg = client_data("webauthn.create", &challenge_reg, "https://login.cave.dev");

    let reg_mgr = RegistrationManager::new(
        InMemoryCredentialStore::new(),
        "login.cave.dev",
        "https://login.cave.dev",
    );
    reg_mgr
        .verify(
            &RegistrationOptions {
                challenge: challenge_reg,
                rp_id: "login.cave.dev".into(),
                rp_name: "Cave".into(),
                user_id: b"u".to_vec(),
                user_name: "u".into(),
                user_display_name: "u".into(),
                user_verification: UserVerification::Required,
                exclude_credentials: vec![],
                resident_key: ResidentKeyRequirement::Preferred,
            },
            &AttestationResponse {
                client_data_json: cd_reg,
                attestation_object: att_obj,
                transports: vec!["usb".into()],
            },
        )
        .unwrap();

    let challenge_auth = b"es256-auth".to_vec();
    let auth_data_a = auth_data_assertion("login.cave.dev", AuthFlags::UP | AuthFlags::UV, 2);
    let cd_auth = client_data("webauthn.get", &challenge_auth, "https://login.cave.dev");
    let mut hasher = Sha256::new();
    hasher.update(&cd_auth);
    let cdh: [u8; 32] = hasher.finalize().into();
    let mut signed = auth_data_a.clone();
    signed.extend_from_slice(&cdh);
    let sig: p256::ecdsa::Signature = sk.sign(&signed);
    let der = sig.to_der();
    let auth_mgr = AuthenticationManager::new(
        reg_mgr.store,
        "login.cave.dev",
        "https://login.cave.dev",
    );
    let stored = auth_mgr
        .verify(
            &AssertionOptions {
                challenge: challenge_auth,
                rp_id: "login.cave.dev".into(),
                user_verification: UserVerification::Required,
                allow_credentials: vec![cred_id.clone()],
            },
            &AssertionResponse {
                credential_id: cred_id.clone(),
                client_data_json: cd_auth,
                authenticator_data: auth_data_a,
                signature: der.as_bytes().to_vec(),
                user_handle: None,
            },
        )
        .unwrap();
    assert_eq!(stored.sign_count, 2);
}

/// Integration ceremony #3 — sign-count monotonicity replay rejection.
#[test]
fn ceremony_3_replay_rejected_by_sign_count() {
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    let sk = SigningKey::generate(&mut OsRng);
    let vk = sk.verifying_key();
    let cose_bytes = cose::encode(&CoseKey::EdDsa { x: vk.to_bytes() }).unwrap();
    let cred_id = b"replay-cred".to_vec();
    let auth_data = auth_data_with_cred(
        "login.cave.dev",
        AuthFlags::UP | AuthFlags::AT,
        0,
        &cred_id,
        [0; 16],
        &cose_bytes,
    );
    let att = registration::build_attestation_object_none(&auth_data);
    let cd_r = client_data("webauthn.create", b"c", "https://login.cave.dev");
    let reg_mgr = RegistrationManager::new(
        InMemoryCredentialStore::new(),
        "login.cave.dev",
        "https://login.cave.dev",
    );
    reg_mgr
        .verify(
            &RegistrationOptions {
                challenge: b"c".to_vec(),
                rp_id: "login.cave.dev".into(),
                rp_name: "".into(),
                user_id: b"u".to_vec(),
                user_name: "u".into(),
                user_display_name: "u".into(),
                user_verification: UserVerification::Discouraged,
                exclude_credentials: vec![],
                resident_key: ResidentKeyRequirement::Preferred,
            },
            &AttestationResponse {
                client_data_json: cd_r,
                attestation_object: att,
                transports: vec![],
            },
        )
        .unwrap();
    let auth_mgr = AuthenticationManager::new(
        reg_mgr.store,
        "login.cave.dev",
        "https://login.cave.dev",
    );
    // First assertion succeeds at sign_count=5.
    let mut do_assert = |sign_count: u32| -> Result<_, _> {
        let auth_data = auth_data_assertion("login.cave.dev", AuthFlags::UP, sign_count);
        let cd = client_data("webauthn.get", b"x", "https://login.cave.dev");
        let mut hasher = Sha256::new();
        hasher.update(&cd);
        let cdh: [u8; 32] = hasher.finalize().into();
        let mut signed = auth_data.clone();
        signed.extend_from_slice(&cdh);
        let sig = sk.sign(&signed);
        auth_mgr.verify(
            &AssertionOptions {
                challenge: b"x".to_vec(),
                rp_id: "login.cave.dev".into(),
                user_verification: UserVerification::Discouraged,
                allow_credentials: vec![],
            },
            &AssertionResponse {
                credential_id: cred_id.clone(),
                client_data_json: cd,
                authenticator_data: auth_data,
                signature: sig.to_bytes().to_vec(),
                user_handle: None,
            },
        )
    };
    do_assert(5).unwrap();
    // Replay — same sign_count must fail.
    assert!(do_assert(5).is_err());
    // Lower sign_count must fail.
    assert!(do_assert(4).is_err());
    // Higher sign_count succeeds.
    do_assert(6).unwrap();
}

/// Integration ceremony #4 — passkey discoverable flow with user_handle.
#[test]
fn ceremony_4_passkey_discoverable() {
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    let sk = SigningKey::generate(&mut OsRng);
    let vk = sk.verifying_key();
    let cose_bytes = cose::encode(&CoseKey::EdDsa { x: vk.to_bytes() }).unwrap();
    let cred_id = b"passkey-cred".to_vec();
    let auth_data = auth_data_with_cred(
        "login.cave.dev",
        AuthFlags::UP | AuthFlags::UV | AuthFlags::AT | AuthFlags::BE | AuthFlags::BS,
        0,
        &cred_id,
        [0x11; 16],
        &cose_bytes,
    );
    let att = registration::build_attestation_object_none(&auth_data);
    let cd_r = client_data("webauthn.create", b"c", "https://login.cave.dev");
    let reg_mgr = RegistrationManager::new(
        InMemoryCredentialStore::new(),
        "login.cave.dev",
        "https://login.cave.dev",
    );
    reg_mgr
        .verify(
            &RegistrationOptions {
                challenge: b"c".to_vec(),
                rp_id: "login.cave.dev".into(),
                rp_name: "".into(),
                user_id: b"user-007".to_vec(),
                user_name: "u".into(),
                user_display_name: "u".into(),
                user_verification: UserVerification::Required,
                exclude_credentials: vec![],
                resident_key: ResidentKeyRequirement::Required,
            },
            &AttestationResponse {
                client_data_json: cd_r,
                attestation_object: att,
                transports: vec!["hybrid".into()],
            },
        )
        .unwrap();
    let auth_mgr = AuthenticationManager::new(
        reg_mgr.store,
        "login.cave.dev",
        "https://login.cave.dev",
    );
    let challenge = b"passkey-chal".to_vec();
    let auth_data_a = auth_data_assertion(
        "login.cave.dev",
        AuthFlags::UP | AuthFlags::UV | AuthFlags::BE | AuthFlags::BS,
        1,
    );
    let cd = client_data("webauthn.get", &challenge, "https://login.cave.dev");
    let mut hasher = Sha256::new();
    hasher.update(&cd);
    let cdh: [u8; 32] = hasher.finalize().into();
    let mut signed = auth_data_a.clone();
    signed.extend_from_slice(&cdh);
    let sig = sk.sign(&signed);
    let out = auth_mgr
        .verify_discoverable(
            &AssertionOptions {
                challenge,
                rp_id: "login.cave.dev".into(),
                user_verification: UserVerification::Required,
                allow_credentials: vec![],
            },
            &AssertionResponse {
                credential_id: cred_id,
                client_data_json: cd,
                authenticator_data: auth_data_a,
                signature: sig.to_bytes().to_vec(),
                user_handle: Some(b"user-007".to_vec()),
            },
        )
        .unwrap();
    assert_eq!(out.user_handle, b"user-007");
}

/// Integration ceremony #5 — packed self-attestation registration succeeds.
#[test]
fn ceremony_5_packed_self_attestation_register() {
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    let sk = SigningKey::generate(&mut OsRng);
    let vk = sk.verifying_key();
    let cose_bytes = cose::encode(&CoseKey::EdDsa { x: vk.to_bytes() }).unwrap();
    let cred_id = b"packed-self".to_vec();
    let auth_data = auth_data_with_cred(
        "login.cave.dev",
        AuthFlags::UP | AuthFlags::UV | AuthFlags::AT,
        0,
        &cred_id,
        [0x33; 16],
        &cose_bytes,
    );
    let challenge = b"packed-self-chal".to_vec();
    let cd = client_data("webauthn.create", &challenge, "https://login.cave.dev");
    let mut hasher = Sha256::new();
    hasher.update(&cd);
    let cdh: [u8; 32] = hasher.finalize().into();
    // Build the packed-self signature: sign(authData || cdh) under the
    // credential key (alg=-8, EdDSA).
    let mut signed = auth_data.clone();
    signed.extend_from_slice(&cdh);
    let sig = sk.sign(&signed);
    // attStmt = {alg: -8, sig: <bytes>} (no x5c = self-attestation).
    let stmt = Value::Map(vec![
        (Value::Text("alg".into()), Value::Integer((-8i64).into())),
        (
            Value::Text("sig".into()),
            Value::Bytes(sig.to_bytes().to_vec()),
        ),
    ]);
    let att_obj_v = Value::Map(vec![
        (Value::Text("fmt".into()), Value::Text("packed".into())),
        (Value::Text("authData".into()), Value::Bytes(auth_data)),
        (Value::Text("attStmt".into()), stmt),
    ]);
    let att_obj = cbor::encode(&att_obj_v).unwrap();
    // Sanity — top-level attestation parse picks the packed verifier.
    let parsed = attestation::parse(&att_obj).unwrap();
    assert_eq!(parsed.fmt, "packed");
    let reg_mgr = RegistrationManager::new(
        InMemoryCredentialStore::new(),
        "login.cave.dev",
        "https://login.cave.dev",
    );
    reg_mgr
        .verify(
            &RegistrationOptions {
                challenge,
                rp_id: "login.cave.dev".into(),
                rp_name: "".into(),
                user_id: b"u".to_vec(),
                user_name: "u".into(),
                user_display_name: "u".into(),
                user_verification: UserVerification::Required,
                exclude_credentials: vec![],
                resident_key: ResidentKeyRequirement::Preferred,
            },
            &AttestationResponse {
                client_data_json: cd,
                attestation_object: att_obj,
                transports: vec![],
            },
        )
        .expect("packed self-attestation must succeed");
}

/// Spec test vector — round-trip through CBOR + attestation parser for the
/// canonical `fmt=none`, empty `attStmt`, dummy authData.
#[test]
fn spec_vector_none_attestation_roundtrip() {
    let auth = vec![0u8; 37];
    let raw = registration::build_attestation_object_none(&auth);
    let parsed = attestation::parse(&raw).unwrap();
    assert_eq!(parsed.fmt, "none");
    assert_eq!(parsed.auth_data_raw, auth);
}
