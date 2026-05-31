// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kem_classification() {
        for a in [
            PqcAlgorithm::MlKem512,
            PqcAlgorithm::MlKem768,
            PqcAlgorithm::MlKem1024,
        ] {
            assert!(a.is_kem(), "{} should be a KEM", a.name());
            assert!(!a.is_signature(), "{} should not be a signature", a.name());
        }
    }

    #[test]
    fn signature_classification() {
        for a in [
            PqcAlgorithm::MlDsa44,
            PqcAlgorithm::MlDsa65,
            PqcAlgorithm::MlDsa87,
            PqcAlgorithm::SlhDsaSha2_128s,
            PqcAlgorithm::SlhDsaSha2_192s,
            PqcAlgorithm::SlhDsaSha2_256s,
        ] {
            assert!(a.is_signature(), "{} should be a signature", a.name());
            assert!(!a.is_kem(), "{} should not be a KEM", a.name());
        }
    }

    #[test]
    fn ml_kem_768_sizes() {
        let s = PqcAlgorithm::MlKem768.sizes();
        assert_eq!(s.public_key_len, 1184);
        assert_eq!(s.secret_key_len, 2400);
        assert_eq!(s.ciphertext_len, Some(1088));
        assert_eq!(s.shared_secret_len, Some(32));
        assert_eq!(s.signature_len, None);
    }

    #[test]
    fn ml_dsa_65_sizes() {
        let s = PqcAlgorithm::MlDsa65.sizes();
        assert_eq!(s.public_key_len, 1952);
        assert_eq!(s.secret_key_len, 4032);
        assert_eq!(s.signature_len, Some(3309));
        assert_eq!(s.ciphertext_len, None);
        assert_eq!(s.shared_secret_len, None);
    }

    #[test]
    fn slh_dsa_128s_sizes() {
        let s = PqcAlgorithm::SlhDsaSha2_128s.sizes();
        assert_eq!(s.public_key_len, 32);
        assert_eq!(s.secret_key_len, 64);
        assert_eq!(s.signature_len, Some(7856));
        assert_eq!(s.ciphertext_len, None);
        assert_eq!(s.shared_secret_len, None);
    }

    #[test]
    fn name_roundtrip_all() {
        for a in PqcAlgorithm::ALL {
            let n = a.name();
            assert_eq!(
                PqcAlgorithm::from_name(n),
                Some(a),
                "name roundtrip failed for {n}"
            );
        }
        // Spot-check exact canonical spellings.
        assert_eq!(PqcAlgorithm::MlKem768.name(), "ML-KEM-768");
        assert_eq!(PqcAlgorithm::MlDsa65.name(), "ML-DSA-65");
        assert_eq!(PqcAlgorithm::SlhDsaSha2_128s.name(), "SLH-DSA-SHA2-128s");
    }

    #[test]
    fn name_roundtrip_unknown() {
        assert_eq!(PqcAlgorithm::from_name("RSA-2048"), None);
        assert_eq!(PqcAlgorithm::from_name("ml-kem-768"), None); // case sensitive
        assert_eq!(PqcAlgorithm::from_name(""), None);
    }

    #[test]
    fn oid_distinct_per_algorithm() {
        let mut seen = std::collections::HashSet::new();
        for a in PqcAlgorithm::ALL {
            let oid = a.oid();
            assert!(oid.starts_with("2.16.840.1.101.3.4."), "{oid}");
            assert!(seen.insert(oid), "duplicate OID {oid}");
        }
        assert_eq!(seen.len(), PqcAlgorithm::ALL.len());
    }

    #[test]
    fn kem_sizes_none_for_signature_algos() {
        // Every signature algorithm has no KEM fields.
        for a in PqcAlgorithm::ALL.into_iter().filter(|a| a.is_signature()) {
            let s = a.sizes();
            assert_eq!(s.ciphertext_len, None, "{}", a.name());
            assert_eq!(s.shared_secret_len, None, "{}", a.name());
            assert!(s.signature_len.is_some(), "{}", a.name());
        }
    }

    #[test]
    fn sig_sizes_none_for_kem_algos() {
        // Every KEM algorithm has no signature field but has KEM fields.
        for a in PqcAlgorithm::ALL.into_iter().filter(|a| a.is_kem()) {
            let s = a.sizes();
            assert_eq!(s.signature_len, None, "{}", a.name());
            assert!(s.ciphertext_len.is_some(), "{}", a.name());
            assert_eq!(s.shared_secret_len, Some(32), "{}", a.name());
        }
    }

    #[test]
    fn composite_version_constant_is_one() {
        assert_eq!(COMPOSITE_VERSION, 0x01);
    }

    #[test]
    fn composite_assemble_parse_roundtrip() {
        // Typical hybrid layout: [pqc_sig, classical_sig].
        let pqc = vec![0xABu8; 3309];
        let classical = vec![0xCDu8; 64];
        let original = CompositeSignature::new(vec![pqc.clone(), classical.clone()]);

        let blob = original.assemble().unwrap();
        // Layout sanity: version byte + per-component (4-byte len + payload).
        assert_eq!(blob[0], COMPOSITE_VERSION);
        assert_eq!(blob.len(), 1 + (4 + 3309) + (4 + 64));

        let parsed = CompositeSignature::parse(&blob).unwrap();
        assert_eq!(parsed, original);
        assert_eq!(parsed.components[0], pqc);
        assert_eq!(parsed.components[1], classical);
    }

    #[test]
    fn composite_empty_components_roundtrip() {
        let original = CompositeSignature::default();
        let blob = original.assemble().unwrap();
        assert_eq!(blob, vec![COMPOSITE_VERSION]);
        let parsed = CompositeSignature::parse(&blob).unwrap();
        assert_eq!(parsed.components.len(), 0);
        assert_eq!(parsed, original);
    }

    #[test]
    fn composite_three_components_roundtrip() {
        let original = CompositeSignature::new(vec![
            b"first".to_vec(),
            Vec::new(), // zero-length component must survive
            b"third-component-bytes".to_vec(),
        ]);
        let blob = original.assemble().unwrap();
        let parsed = CompositeSignature::parse(&blob).unwrap();
        assert_eq!(parsed, original);
        assert_eq!(parsed.components[1].len(), 0);
    }

    #[test]
    fn composite_rejects_empty_blob() {
        let err = CompositeSignature::parse(&[]).unwrap_err();
        assert_eq!(
            err,
            PqcError::Truncated {
                expected: 1,
                actual: 0
            }
        );
    }

    #[test]
    fn composite_rejects_bad_version() {
        let blob = [0x02u8, 0, 0, 0, 0];
        let err = CompositeSignature::parse(&blob).unwrap_err();
        assert_eq!(
            err,
            PqcError::VersionMismatch {
                expected: 0x01,
                actual: 0x02
            }
        );
    }

    #[test]
    fn composite_rejects_truncated_length_prefix() {
        // Version byte present, but the 4-byte length prefix is incomplete.
        let blob = [COMPOSITE_VERSION, 0x00, 0x00]; // only 2 of 4 length bytes
        let err = CompositeSignature::parse(&blob).unwrap_err();
        assert_eq!(
            err,
            PqcError::Truncated {
                expected: 5,
                actual: 3
            }
        );
    }

    #[test]
    fn composite_rejects_truncated_payload() {
        // Declares a 10-byte component but only supplies 3 payload bytes.
        let mut blob = vec![COMPOSITE_VERSION];
        blob.extend_from_slice(&10u32.to_be_bytes());
        blob.extend_from_slice(&[1, 2, 3]);
        let err = CompositeSignature::parse(&blob).unwrap_err();
        assert_eq!(
            err,
            PqcError::Truncated {
                expected: 5 + 10,
                actual: 5 + 3
            }
        );
    }
}
