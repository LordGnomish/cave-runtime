// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j/webauthn4j@82345b8
//   webauthn4j-core/src/test/java/com/webauthn4j/WebAuthnRegistrationManagerTest.java
//
// RED — Registration ceremony (W3C §7.1 "Registering a new credential").

use cave_auth::webauthn::registration::{
    finish_registration, start_registration, RegistrationError, RegistrationOptions,
    RegistrationRequest, UserVerificationRequirement,
};

#[test]
fn start_registration_emits_challenge_with_16_bytes_min() {
    let opts = RegistrationOptions {
        rp_id: "cave.example".into(),
        rp_name: "Cave".into(),
        user_id: b"alice-id".to_vec(),
        user_name: "alice".into(),
        user_display_name: "Alice".into(),
        user_verification: UserVerificationRequirement::Preferred,
        timeout_ms: 60_000,
        attestation: "none".into(),
    };
    let cc = start_registration(&opts);
    assert!(cc.challenge.len() >= 16);
    assert_eq!(cc.rp.id, "cave.example");
    assert_eq!(cc.user.id, b"alice-id");
}

#[test]
fn start_registration_offers_es256_es384_rs256_eddsa() {
    let opts = RegistrationOptions::test_default();
    let cc = start_registration(&opts);
    let algs: Vec<i64> = cc.pub_key_cred_params.iter().map(|p| p.alg).collect();
    assert!(algs.contains(&-7));
    assert!(algs.contains(&-35));
    assert!(algs.contains(&-257));
    assert!(algs.contains(&-8));
}

#[test]
fn finish_registration_rejects_origin_mismatch() {
    let opts = RegistrationOptions::test_default();
    let cc = start_registration(&opts);
    // craft clientDataJSON whose origin doesn't match expected
    let body = synth_attestation_none(
        &cc.challenge,
        "https://attacker.example",
        "webauthn.create",
        &opts.rp_id,
    );
    let req = RegistrationRequest {
        challenge: cc.challenge.clone(),
        expected_origins: vec!["https://cave.example".into()],
        rp_id: opts.rp_id.clone(),
        require_user_verification: false,
        client_data_json: body.client_data_json,
        attestation_object: body.attestation_object,
        client_extension_results: serde_json::json!({}),
    };
    let err = finish_registration(req).unwrap_err();
    assert!(matches!(err, RegistrationError::OriginMismatch { .. }));
}

#[test]
fn finish_registration_rejects_wrong_type() {
    let opts = RegistrationOptions::test_default();
    let cc = start_registration(&opts);
    let body = synth_attestation_none(
        &cc.challenge,
        "https://cave.example",
        "webauthn.get", // wrong
        &opts.rp_id,
    );
    let req = RegistrationRequest {
        challenge: cc.challenge.clone(),
        expected_origins: vec!["https://cave.example".into()],
        rp_id: opts.rp_id.clone(),
        require_user_verification: false,
        client_data_json: body.client_data_json,
        attestation_object: body.attestation_object,
        client_extension_results: serde_json::json!({}),
    };
    let err = finish_registration(req).unwrap_err();
    assert!(matches!(err, RegistrationError::WrongType { .. }));
}

#[test]
fn finish_registration_rejects_challenge_mismatch() {
    let opts = RegistrationOptions::test_default();
    let _ = start_registration(&opts);
    let body = synth_attestation_none(
        b"different-challenge-12345678901234",
        "https://cave.example",
        "webauthn.create",
        &opts.rp_id,
    );
    let req = RegistrationRequest {
        challenge: vec![0xAA; 32],
        expected_origins: vec!["https://cave.example".into()],
        rp_id: opts.rp_id.clone(),
        require_user_verification: false,
        client_data_json: body.client_data_json,
        attestation_object: body.attestation_object,
        client_extension_results: serde_json::json!({}),
    };
    let err = finish_registration(req).unwrap_err();
    assert!(matches!(err, RegistrationError::ChallengeMismatch));
}

#[test]
fn finish_registration_accepts_none_format_with_matching_origin_and_challenge() {
    let opts = RegistrationOptions::test_default();
    let cc = start_registration(&opts);
    let body = synth_attestation_none(
        &cc.challenge,
        "https://cave.example",
        "webauthn.create",
        &opts.rp_id,
    );
    let req = RegistrationRequest {
        challenge: cc.challenge.clone(),
        expected_origins: vec!["https://cave.example".into()],
        rp_id: opts.rp_id.clone(),
        require_user_verification: false,
        client_data_json: body.client_data_json,
        attestation_object: body.attestation_object,
        client_extension_results: serde_json::json!({}),
    };
    let result = finish_registration(req).expect("should accept");
    assert_eq!(result.attestation_format, "none");
    assert_eq!(result.credential.attestation_format, "none");
    assert!(!result.credential.credential_id.is_empty());
}

#[test]
fn finish_registration_enforces_user_verification_when_required() {
    let opts = RegistrationOptions::test_default();
    let cc = start_registration(&opts);
    // UP=1, UV=0
    let body = synth_attestation_none_with_flags(
        &cc.challenge,
        "https://cave.example",
        "webauthn.create",
        &opts.rp_id,
        0b0100_0001,
    );
    let req = RegistrationRequest {
        challenge: cc.challenge.clone(),
        expected_origins: vec!["https://cave.example".into()],
        rp_id: opts.rp_id.clone(),
        require_user_verification: true,
        client_data_json: body.client_data_json,
        attestation_object: body.attestation_object,
        client_extension_results: serde_json::json!({}),
    };
    let err = finish_registration(req).unwrap_err();
    assert!(matches!(err, RegistrationError::UserVerificationRequired));
}

#[test]
fn finish_registration_rejects_user_presence_zero() {
    let opts = RegistrationOptions::test_default();
    let cc = start_registration(&opts);
    let body = synth_attestation_none_with_flags(
        &cc.challenge,
        "https://cave.example",
        "webauthn.create",
        &opts.rp_id,
        0b0100_0000,
    );
    let req = RegistrationRequest {
        challenge: cc.challenge.clone(),
        expected_origins: vec!["https://cave.example".into()],
        rp_id: opts.rp_id.clone(),
        require_user_verification: false,
        client_data_json: body.client_data_json,
        attestation_object: body.attestation_object,
        client_extension_results: serde_json::json!({}),
    };
    let err = finish_registration(req).unwrap_err();
    assert!(matches!(err, RegistrationError::UserNotPresent));
}

// ─── test helpers ────────────────────────────────────────────────────────────

struct Crafted {
    client_data_json: Vec<u8>,
    attestation_object: Vec<u8>,
}

fn synth_attestation_none(challenge: &[u8], origin: &str, type_: &str, rp_id: &str) -> Crafted {
    synth_attestation_none_with_flags(challenge, origin, type_, rp_id, 0b0100_0101) // AT|UV|UP
}

fn synth_attestation_none_with_flags(
    challenge: &[u8],
    origin: &str,
    type_: &str,
    rp_id: &str,
    flags: u8,
) -> Crafted {
    use base64::Engine;
    let challenge_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(challenge);
    let client_data = serde_json::json!({
        "type": type_,
        "challenge": challenge_b64,
        "origin": origin,
        "crossOrigin": false,
    });
    let client_data_json = serde_json::to_vec(&client_data).unwrap();

    // authData
    use sha2::Digest;
    let rp_id_hash = sha2::Sha256::digest(rp_id.as_bytes());
    let mut auth_data = Vec::new();
    auth_data.extend_from_slice(&rp_id_hash);
    auth_data.push(flags);
    auth_data.extend_from_slice(&0u32.to_be_bytes());
    // AAGUID
    auth_data.extend_from_slice(&[0; 16]);
    // credentialId
    let cred_id = vec![0x42; 16];
    auth_data.extend_from_slice(&(cred_id.len() as u16).to_be_bytes());
    auth_data.extend_from_slice(&cred_id);
    // COSE_Key ES256
    auth_data.extend_from_slice(&cose_es256_stub());

    // attestationObject CBOR map {fmt: "none", authData: <bytes>, attStmt: {}}
    use ciborium::Value;
    let att_obj = Value::Map(vec![
        (Value::Text("fmt".into()), Value::Text("none".into())),
        (Value::Text("authData".into()), Value::Bytes(auth_data)),
        (Value::Text("attStmt".into()), Value::Map(Vec::new())),
    ]);
    let mut attestation_object = Vec::new();
    ciborium::ser::into_writer(&att_obj, &mut attestation_object).unwrap();

    Crafted {
        client_data_json,
        attestation_object,
    }
}

fn cose_es256_stub() -> Vec<u8> {
    use ciborium::Value;
    let pairs = vec![
        (Value::Integer(1.into()), Value::Integer(2.into())),
        (Value::Integer(3.into()), Value::Integer((-7i64).into())),
        (Value::Integer((-1i64).into()), Value::Integer(1.into())),
        (Value::Integer((-2i64).into()), Value::Bytes(vec![0x11; 32])),
        (Value::Integer((-3i64).into()), Value::Bytes(vec![0x22; 32])),
    ];
    let mut out = Vec::new();
    ciborium::ser::into_writer(&Value::Map(pairs), &mut out).unwrap();
    out
}

impl RegistrationOptions {
    fn test_default() -> Self {
        Self {
            rp_id: "cave.example".into(),
            rp_name: "Cave".into(),
            user_id: b"alice-id".to_vec(),
            user_name: "alice".into(),
            user_display_name: "Alice".into(),
            user_verification: UserVerificationRequirement::Preferred,
            timeout_ms: 60_000,
            attestation: "none".into(),
        }
    }
}
