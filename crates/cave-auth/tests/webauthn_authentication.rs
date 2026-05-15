// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j/webauthn4j@82345b8
//   webauthn4j-core/src/test/java/com/webauthn4j/WebAuthnAuthenticationManagerTest.java
//
// RED — Authentication ceremony (W3C §7.2 "Verifying an authentication
// assertion").  Uses a real ES256 keypair so the assertion-signature
// path is exercised end-to-end.

use base64::Engine;
use cave_auth::webauthn::authentication::{
    finish_authentication, start_authentication, AuthenticationError, AuthenticationOptions,
    AuthenticationRequest,
};
use cave_auth::webauthn::cose::{CoseAlgorithm, CoseKey};
use cave_auth::webauthn::model::{Credential, Transport};
use p256::ecdsa::{signature::Signer, Signature, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

fn make_es256_credential() -> (SigningKey, Credential) {
    let sk = SigningKey::random(&mut rand::thread_rng());
    let vk: VerifyingKey = VerifyingKey::from(&sk);
    let ep = vk.to_encoded_point(false);
    let x = ep.x().unwrap().to_vec();
    let y = ep.y().unwrap().to_vec();
    let public_key = CoseKey::Ec2 {
        alg: CoseAlgorithm::Es256,
        crv: 1,
        x,
        y,
    };
    let cred = Credential {
        credential_id: vec![0x42; 16],
        public_key,
        sign_counter: 7,
        transports: vec![Transport::Internal],
        aaguid: [0; 16],
        attestation_format: "none".into(),
        user_handle: None,
        backup_eligible: false,
        backup_state: false,
        uv_initialized: true,
    };
    (sk, cred)
}

fn build_signed_assertion(
    sk: &SigningKey,
    rp_id: &str,
    origin: &str,
    challenge: &[u8],
    flags: u8,
    sign_count: u32,
) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let challenge_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(challenge);
    let client_data = serde_json::json!({
        "type": "webauthn.get",
        "challenge": challenge_b64,
        "origin": origin,
        "crossOrigin": false,
    });
    let client_data_json = serde_json::to_vec(&client_data).unwrap();
    let rp_id_hash = Sha256::digest(rp_id.as_bytes());
    let mut authenticator_data = Vec::new();
    authenticator_data.extend_from_slice(&rp_id_hash);
    authenticator_data.push(flags);
    authenticator_data.extend_from_slice(&sign_count.to_be_bytes());

    let client_data_hash = Sha256::digest(&client_data_json);
    let mut signed = Vec::with_capacity(authenticator_data.len() + 32);
    signed.extend_from_slice(&authenticator_data);
    signed.extend_from_slice(&client_data_hash);
    let signature: Signature = sk.sign(&signed);
    let signature_der = signature.to_der().to_bytes().to_vec();
    (authenticator_data, client_data_json, signature_der)
}

#[test]
fn start_authentication_emits_challenge() {
    let opts = AuthenticationOptions {
        rp_id: "cave.example".into(),
        allow_credentials: vec![vec![0x42; 16]],
        user_verification: "preferred".into(),
        timeout_ms: 60_000,
    };
    let req = start_authentication(&opts);
    assert!(req.challenge.len() >= 16);
    assert_eq!(req.allow_credentials.len(), 1);
    assert_eq!(req.rp_id, "cave.example");
}

#[test]
fn finish_authentication_accepts_valid_es256_assertion() {
    let (sk, cred) = make_es256_credential();
    let challenge = vec![0xAA; 32];
    let (ad, cdj, sig) = build_signed_assertion(
        &sk,
        "cave.example",
        "https://cave.example",
        &challenge,
        0b0000_0101, // UP|UV
        cred.sign_counter + 1,
    );
    let req = AuthenticationRequest {
        challenge: challenge.clone(),
        expected_origins: vec!["https://cave.example".into()],
        rp_id: "cave.example".into(),
        require_user_verification: false,
        credential: cred,
        authenticator_data: ad,
        client_data_json: cdj,
        signature: sig,
        user_handle: None,
    };
    let res = finish_authentication(req).expect("must accept");
    assert!(res.new_sign_counter > 7);
}

#[test]
fn finish_authentication_rejects_counter_regression() {
    let (sk, cred) = make_es256_credential();
    let challenge = vec![0xAA; 32];
    let (ad, cdj, sig) = build_signed_assertion(
        &sk,
        "cave.example",
        "https://cave.example",
        &challenge,
        0b0000_0101,
        cred.sign_counter, // = previous, must reject
    );
    let req = AuthenticationRequest {
        challenge,
        expected_origins: vec!["https://cave.example".into()],
        rp_id: "cave.example".into(),
        require_user_verification: false,
        credential: cred,
        authenticator_data: ad,
        client_data_json: cdj,
        signature: sig,
        user_handle: None,
    };
    let err = finish_authentication(req).unwrap_err();
    assert!(matches!(err, AuthenticationError::CounterRegression { .. }));
}

#[test]
fn finish_authentication_rejects_bad_signature() {
    let (sk, cred) = make_es256_credential();
    let challenge = vec![0xAA; 32];
    let (ad, cdj, mut sig) = build_signed_assertion(
        &sk,
        "cave.example",
        "https://cave.example",
        &challenge,
        0b0000_0101,
        cred.sign_counter + 1,
    );
    // Flip the last byte of the DER signature.
    let last = sig.len() - 1;
    sig[last] ^= 0x01;
    let req = AuthenticationRequest {
        challenge,
        expected_origins: vec!["https://cave.example".into()],
        rp_id: "cave.example".into(),
        require_user_verification: false,
        credential: cred,
        authenticator_data: ad,
        client_data_json: cdj,
        signature: sig,
        user_handle: None,
    };
    let err = finish_authentication(req).unwrap_err();
    assert!(matches!(err, AuthenticationError::BadSignature));
}

#[test]
fn finish_authentication_rejects_user_verification_required_but_zero() {
    let (sk, cred) = make_es256_credential();
    let challenge = vec![0xAA; 32];
    let (ad, cdj, sig) = build_signed_assertion(
        &sk,
        "cave.example",
        "https://cave.example",
        &challenge,
        0b0000_0001, // UP only, no UV
        cred.sign_counter + 1,
    );
    let req = AuthenticationRequest {
        challenge,
        expected_origins: vec!["https://cave.example".into()],
        rp_id: "cave.example".into(),
        require_user_verification: true,
        credential: cred,
        authenticator_data: ad,
        client_data_json: cdj,
        signature: sig,
        user_handle: None,
    };
    let err = finish_authentication(req).unwrap_err();
    assert!(matches!(err, AuthenticationError::UserVerificationRequired));
}

#[test]
fn finish_authentication_rejects_rp_id_hash_mismatch() {
    let (sk, cred) = make_es256_credential();
    let challenge = vec![0xAA; 32];
    let (ad, cdj, sig) = build_signed_assertion(
        &sk,
        "evil.example", // hash of evil.example, but RP expects cave.example
        "https://cave.example",
        &challenge,
        0b0000_0101,
        cred.sign_counter + 1,
    );
    let req = AuthenticationRequest {
        challenge,
        expected_origins: vec!["https://cave.example".into()],
        rp_id: "cave.example".into(),
        require_user_verification: false,
        credential: cred,
        authenticator_data: ad,
        client_data_json: cdj,
        signature: sig,
        user_handle: None,
    };
    let err = finish_authentication(req).unwrap_err();
    assert!(matches!(err, AuthenticationError::RpIdHashMismatch));
}

#[test]
fn finish_authentication_rejects_passkey_user_handle_mismatch() {
    let (sk, mut cred) = make_es256_credential();
    cred.user_handle = Some(b"alice-id".to_vec());
    let challenge = vec![0xAA; 32];
    let (ad, cdj, sig) = build_signed_assertion(
        &sk,
        "cave.example",
        "https://cave.example",
        &challenge,
        0b0000_0101,
        cred.sign_counter + 1,
    );
    let req = AuthenticationRequest {
        challenge,
        expected_origins: vec!["https://cave.example".into()],
        rp_id: "cave.example".into(),
        require_user_verification: false,
        credential: cred,
        authenticator_data: ad,
        client_data_json: cdj,
        signature: sig,
        user_handle: Some(b"mallory-id".to_vec()),
    };
    let err = finish_authentication(req).unwrap_err();
    assert!(matches!(err, AuthenticationError::UserHandleMismatch));
}
