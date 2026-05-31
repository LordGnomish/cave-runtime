// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Sigstore protobuf bundle v0.3 — the modern, self-describing envelope.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::CosignBundle;
    use crate::models::SigKind;

    fn keyless_bundle() -> CosignBundle {
        CosignBundle {
            kind: SigKind::Keyless,
            signed_payload_b64: "c2lnbmF0dXJl".into(), // "signature"
            // PEM body decodes (base64) to DER bytes — rawBytes must equal the body.
            cert_pem: "-----BEGIN CERTIFICATE-----\nQUJDREVG\nR0hJSktM\n-----END CERTIFICATE-----"
                .into(),
            chain_pem: None,
            rekor_log_index: Some(42),
            rekor_uuid: Some("deadbeefcafe".into()),
            rekor_integrated_time: Some(1_700_000_042),
            artifact_digest:
                "sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890".into(),
        }
    }

    fn keypair_bundle() -> CosignBundle {
        CosignBundle {
            kind: SigKind::Keypair,
            signed_payload_b64: "c2ln".into(),
            cert_pem: "-----BEGIN PUBLIC KEY-----\nQQ==\n-----END PUBLIC KEY-----".into(),
            chain_pem: None,
            rekor_log_index: None,
            rekor_uuid: None,
            rekor_integrated_time: None,
            artifact_digest:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000".into(),
        }
    }

    #[test]
    fn media_type_is_v03() {
        let b = SigstoreBundle::from_cosign_bundle(&keyless_bundle()).unwrap();
        assert_eq!(b.media_type, BUNDLE_MEDIA_TYPE_V03);
    }

    #[test]
    fn keyless_carries_certificate_raw_bytes() {
        let b = SigstoreBundle::from_cosign_bundle(&keyless_bundle()).unwrap();
        let cert = b.verification_material.certificate.expect("cert present");
        // PEM body (newlines stripped) is the base64 DER == rawBytes.
        assert_eq!(cert.raw_bytes, "QUJDREVGR0hJSktM");
        assert!(b.verification_material.public_key.is_none());
    }

    #[test]
    fn keypair_carries_public_key_not_certificate() {
        let b = SigstoreBundle::from_cosign_bundle(&keypair_bundle()).unwrap();
        assert!(b.verification_material.certificate.is_none());
        assert!(b.verification_material.public_key.is_some());
    }

    #[test]
    fn message_signature_digest_is_sha2_256_base64() {
        let b = SigstoreBundle::from_cosign_bundle(&keyless_bundle()).unwrap();
        let ms = b.message_signature.expect("message signature present");
        assert_eq!(ms.message_digest.algorithm, "SHA2_256");
        // sha256:abcdef...90 (hex) -> raw 32 bytes -> base64.
        assert_eq!(
            ms.message_digest.digest,
            "q83vEjRWeJCrze8SNFZ4kKvN7xI0VniQq83vEjRWeJA="
        );
        assert_eq!(ms.signature, "c2lnbmF0dXJl");
    }

    #[test]
    fn tlog_entry_encodes_int64_as_string() {
        let b = SigstoreBundle::from_cosign_bundle(&keyless_bundle()).unwrap();
        assert_eq!(b.verification_material.tlog_entries.len(), 1);
        let e = &b.verification_material.tlog_entries[0];
        // protojson encodes int64 fields as strings.
        assert_eq!(e.log_index, "42");
        assert_eq!(e.integrated_time, "1700000042");
        assert_eq!(e.kind_version.kind, "hashedrekord");
        assert_eq!(e.kind_version.version, "0.0.1");
    }

    #[test]
    fn no_rekor_means_no_tlog_entries() {
        let b = SigstoreBundle::from_cosign_bundle(&keypair_bundle()).unwrap();
        assert!(b.verification_material.tlog_entries.is_empty());
    }

    #[test]
    fn json_uses_protobuf_camelcase_field_names() {
        let b = SigstoreBundle::from_cosign_bundle(&keyless_bundle()).unwrap();
        let j = b.encode_json().unwrap();
        assert!(j.contains("\"mediaType\""));
        assert!(j.contains("\"verificationMaterial\""));
        assert!(j.contains("\"messageSignature\""));
        assert!(j.contains("\"messageDigest\""));
        assert!(j.contains("\"tlogEntries\""));
        assert!(j.contains("\"logIndex\""));
        // snake_case must NOT leak.
        assert!(!j.contains("message_signature"));
        assert!(!j.contains("log_index"));
    }

    #[test]
    fn json_roundtrip() {
        let b = SigstoreBundle::from_cosign_bundle(&keyless_bundle()).unwrap();
        let j = b.encode_json().unwrap();
        let back = SigstoreBundle::decode_json(&j).unwrap();
        assert_eq!(b, back);
    }

    #[test]
    fn dsse_bundle_has_envelope_not_message_signature() {
        let env = serde_json::json!({
            "payload": "eyJfdHlwZSI6Imh0dHBzOi8vaW4tdG90by5pby9TdGF0ZW1lbnQvdjEifQ==",
            "payloadType": "application/vnd.in-toto+json",
            "signatures": [{"sig": "YWJj"}]
        });
        let b = SigstoreBundle::from_dsse(&keyless_bundle(), env.clone()).unwrap();
        assert!(b.message_signature.is_none());
        assert_eq!(b.dsse_envelope, Some(env));
        let j = b.encode_json().unwrap();
        assert!(j.contains("\"dsseEnvelope\""));
        assert!(!j.contains("\"messageSignature\""));
    }
}
