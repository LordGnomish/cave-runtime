// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 saml-core/src/main/java/org/keycloak/saml/processing/core/util/XMLSignatureUtil.java
//
// End-to-end integration tests: sign + verify the wire bytes of
// AuthnRequest / Response / Assertion / Metadata with every one of
// the four supported XMLDSig algorithms (RSA-SHA256 baseline plus
// ECDSA-SHA256/384/512). Exercises the unified dispatch in
// `saml::signature::{sign, verify}` against real serialised SAML
// XML produced by each message type's `to_xml()` builder.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use cave_auth::saml::authn_request::AuthnRequest;
use cave_auth::saml::metadata::EntityDescriptor;
use cave_auth::saml::response::{Assertion, Response};
use cave_auth::saml::signature::{
    Algorithm, SignedDocument, SigningMaterial, VerifyingMaterial, sign, verify,
};
use cave_auth::saml::signing_ecdsa::{EcdsaCurve, generate_keypair};

// Same RSA test material as `saml::signature::tests` — repeated here
// because the test there is `mod tests {}` private; an extra ~3KiB of
// inline base64 keeps the integration crate free of test-helper deps.
const RSA_PKCS8_DER_B64: &str = "MIIEvwIBADANBgkqhkiG9w0BAQEFAASCBKkwggSlAgEAAoIBAQCPegzMZl+1jHVMT0PW68K/qcIYqbBkkO6ooVUmxuDLFq0NIQmuteQ30RM06txbzpJdtBO/vAxOfcUBQ+jmKwixHC0JUcW6jixFfTOwFKIdeByzIRNoi1i/ZbrhhknLKZ3U2IQz4VwroyKbL2mFg5dPDA1oj1cJG4QODWLqbcjngRmExdM8remq+c6HGiI2TS0aldg3/wGBI5C+IyOeniVjzaFN/Z3GCqq9uC7Ij8spDoGBZPpskH8ehFLb6RsoxvVWJKJmB7LSNkabWXVLD+a+oqVO9ozMlV1R6qZZ4IUV7+lNS4BQp4Vla3RIKajjj2YKzGIl9UUEyH/A3SOlkqrrAgMBAAECggEANzZ8nlv3EOJQcWE/dgGcHC2zp9IFM24iqXoMTrPR5dWAGsFP/I+6l1A51+9ZhWrlIHIf93TiN4Jmwankgk6lNaLmIeP592Sm3MblkSkfib+jK7vawCx/pof7drY6x5foSPRZS625zoEk3BtOvDZ7j8vPjSE8GSEhnFbCbfx5h7yu4RqjBVEAz7feOMade++Qjn/IyfoNJ2Wq7oq/w7lXVYUNVIS7Ulj9cdTXIF6QFf+84B46d+YTsYiZRGMb/eZYk5IyXdv0vDg+qCD2mV+JYs1PD2qZOKemCxLjYs0OMYy1fKxbYVra4g0gOtTcnUYTJFixuyFifnfOyKKNrpbpgQKBgQDKRtc94O2t+Bah/bU4+90RNB4/lVmCRB0ExkMzJ/djT9TLYYFtnjX6DympmQ6ACzO8cqsArB2nEgbsXCV2lcwCldVY5/I9SuplyqtGKfPRdXlU3GopFNjZ/bdi1GF7MgRpD59yWWRHNN55HV94Eef/LumDOvTBtVu28jRWGJXYmwKBgQC1lUsoBAyBCnXkddsVIm5bqoi84CvcNC+nVRTcn8+x+GYb8o35RSSOymMQzlNd/b1YHzhi2b0R3vikSU/r3LtMdrWgdoIV6ElKgAwcbaoqIb/Zovh3qUXimIZvB8krR6a60QqJVw/1lTRnSuU82zV3ZncCSOJo64TZXmEdm47T8QKBgQCO/smG6w3bYHjPh8WnRRYg5VFE7dXbKz/AclBrR6Oxx2vNY17WGXRbFIEFbjg7+K9YV0/gJ8zGoQ3X5cRuMrOIWFf8g+xRvDY8Q6wU6+97caqWfUNnS1+Jq70K1s0bBF7tzqePdPZZCF0GDefBwBbb5VQa+4Cvt//gMxUgkDzOZQKBgQCRrjQ853qssJrC7vcUrqoBawEHH4awxUGSK0Vwd9qm+xXYyDG1Ug6xbJgsLIxf9SnKoEmZrPzucIflLlgrb8zo3Lh9A3b8Yn8igTa2PBlwceE8l25memzyDdKVE5cG3RZb/UhJxYqtScZgNItT1r6/i3phX94dtQ7BYeHiYiIl0QKBgQCCGN21FfQalMH2duGu7UQnZ03To0uDyn3zoaxxVK7M+9xB8bQ5rFq23ZOuGy1qYE7CitzGkCLf9goiJaNCowwUIKVsj+Joufxg1K9usyThr/OpWwQYNu1TOXpzBmKY1AnK+JVpUsRppc0BzpaPiDcnfi1Ch0ds0gVgPLUfflmX/A==";
const RSA_SPKI_DER_B64: &str = "MIIBCgKCAQEAj3oMzGZftYx1TE9D1uvCv6nCGKmwZJDuqKFVJsbgyxatDSEJrrXkN9ETNOrcW86SXbQTv7wMTn3FAUPo5isIsRwtCVHFuo4sRX0zsBSiHXgcsyETaItYv2W64YZJyymd1NiEM+FcK6Mimy9phYOXTwwNaI9XCRuEDg1i6m3I54EZhMXTPK3pqvnOhxoiNk0tGpXYN/8BgSOQviMjnp4lY82hTf2dxgqqvbguyI/LKQ6BgWT6bJB/HoRS2+kbKMb1ViSiZgey0jZGm1l1Sw/mvqKlTvaMzJVdUeqmWeCFFe/pTUuAUKeFZWt0SCmo449mCsxiJfVFBMh/wN0jpZKq6wIDAQAB";

fn rsa_pkcs8_der() -> Vec<u8> {
    B64.decode(RSA_PKCS8_DER_B64).unwrap()
}

fn rsa_pub_der() -> Vec<u8> {
    B64.decode(RSA_SPKI_DER_B64).unwrap()
}

/// Sign `xml_bytes` with `alg`, return base64 SignatureValue.
fn sign_with(alg: Algorithm, xml_bytes: &[u8]) -> (String, KeyHolder) {
    let doc = SignedDocument::new(xml_bytes);
    match alg {
        Algorithm::RsaSha256 => {
            let key = rsa_pkcs8_der();
            let sig = sign(&doc, alg, &SigningMaterial::Rsa { pkcs8_der: &key }).unwrap();
            (sig, KeyHolder::Rsa)
        }
        Algorithm::EcdsaSha256 => {
            let kp = generate_keypair(EcdsaCurve::P256);
            let sig = sign(&doc, alg, &SigningMaterial::Ecdsa { key: &kp.signing }).unwrap();
            (sig, KeyHolder::Ecdsa(kp))
        }
        Algorithm::EcdsaSha384 => {
            let kp = generate_keypair(EcdsaCurve::P384);
            let sig = sign(&doc, alg, &SigningMaterial::Ecdsa { key: &kp.signing }).unwrap();
            (sig, KeyHolder::Ecdsa(kp))
        }
        Algorithm::EcdsaSha512 => {
            let kp = generate_keypair(EcdsaCurve::P521);
            let sig = sign(&doc, alg, &SigningMaterial::Ecdsa { key: &kp.signing }).unwrap();
            (sig, KeyHolder::Ecdsa(kp))
        }
    }
}

enum KeyHolder {
    Rsa,
    Ecdsa(cave_auth::saml::signing_ecdsa::EcdsaKeyPair),
}

fn verify_with(alg: Algorithm, xml_bytes: &[u8], sig: &str, holder: &KeyHolder) {
    let doc = SignedDocument::new(xml_bytes);
    let pub_der;
    let mat = match (alg, holder) {
        (Algorithm::RsaSha256, KeyHolder::Rsa) => {
            pub_der = rsa_pub_der();
            VerifyingMaterial::Rsa {
                rsa_pub_der: &pub_der,
            }
        }
        (
            Algorithm::EcdsaSha256 | Algorithm::EcdsaSha384 | Algorithm::EcdsaSha512,
            KeyHolder::Ecdsa(kp),
        ) => VerifyingMaterial::Ecdsa { key: &kp.verifying },
        _ => panic!("test setup mismatch: {alg:?}"),
    };
    verify(&doc, alg, sig, &mat).unwrap();
}

const ALL_ALGS: [Algorithm; 4] = [
    Algorithm::RsaSha256,
    Algorithm::EcdsaSha256,
    Algorithm::EcdsaSha384,
    Algorithm::EcdsaSha512,
];

// ─── AuthnRequest × 4 algorithms ───

#[test]
fn authn_request_signs_and_verifies_all_algorithms() {
    let req = AuthnRequest::new("https://sp.example.com", "https://idp.example.com/sso")
        .with_acs_url("https://sp.example.com/saml/acs");
    let xml = req.to_xml().unwrap();
    assert!(
        xml.windows(b"AuthnRequest".len())
            .any(|w| w == b"AuthnRequest")
    );
    for alg in ALL_ALGS {
        let (sig, holder) = sign_with(alg, &xml);
        verify_with(alg, &xml, &sig, &holder);
    }
}

// ─── Response × 4 algorithms ───

#[test]
fn response_signs_and_verifies_all_algorithms() {
    let assertion = Assertion::new("https://idp.example.com", "alice@example.com")
        .with_audience("https://sp.example.com")
        .with_attribute("email", "alice@example.com");
    let resp = Response::success(
        "https://idp.example.com",
        "https://sp.example.com/saml/acs",
        Some("_in_response_to_id".to_string()),
        assertion,
    );
    let xml = resp.to_xml().unwrap();
    assert!(
        xml.windows(b"samlp:Response".len())
            .any(|w| w == b"samlp:Response")
    );
    for alg in ALL_ALGS {
        let (sig, holder) = sign_with(alg, &xml);
        verify_with(alg, &xml, &sig, &holder);
    }
}

// ─── Assertion (extracted from Response) × 4 algorithms ───
//
// Real SAML deployments sign the inner Assertion in addition to (or
// instead of) the outer Response. This test serialises the full
// Response and then signs the byte range of the inner
// `<saml:Assertion>` element — exactly what an XMLDSig-enclosing
// signer does after canonicalization.

#[test]
fn assertion_inner_signs_and_verifies_all_algorithms() {
    let assertion = Assertion::new("https://idp.example.com", "bob@example.com")
        .with_audience("https://sp.example.com");
    let resp = Response::success(
        "https://idp.example.com",
        "https://sp.example.com/saml/acs",
        None,
        assertion,
    );
    let xml = resp.to_xml().unwrap();
    // Carve out the inner `<saml:Assertion>` element bytes.
    let start_tag = b"<saml:Assertion";
    let end_tag = b"</saml:Assertion>";
    let start = xml
        .windows(start_tag.len())
        .position(|w| w == start_tag)
        .expect("saml:Assertion element present");
    let end = xml
        .windows(end_tag.len())
        .position(|w| w == end_tag)
        .expect("saml:Assertion close tag present")
        + end_tag.len();
    let inner = &xml[start..end];
    for alg in ALL_ALGS {
        let (sig, holder) = sign_with(alg, inner);
        verify_with(alg, inner, &sig, &holder);
    }
}

// ─── Metadata × 4 algorithms (IdP + SP roles) ───

#[test]
fn metadata_idp_signs_and_verifies_all_algorithms() {
    let md = EntityDescriptor::new_idp("https://idp.example.com").add_endpoint(
        "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST",
        "https://idp.example.com/sso",
    );
    let xml = md.to_xml().unwrap();
    assert!(
        xml.windows(b"IDPSSODescriptor".len())
            .any(|w| w == b"IDPSSODescriptor")
    );
    for alg in ALL_ALGS {
        let (sig, holder) = sign_with(alg, &xml);
        verify_with(alg, &xml, &sig, &holder);
    }
}

#[test]
fn metadata_sp_signs_and_verifies_all_algorithms() {
    let md = EntityDescriptor::new_sp("https://sp.example.com").add_endpoint(
        "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST",
        "https://sp.example.com/saml/acs",
    );
    let xml = md.to_xml().unwrap();
    assert!(
        xml.windows(b"SPSSODescriptor".len())
            .any(|w| w == b"SPSSODescriptor")
    );
    for alg in ALL_ALGS {
        let (sig, holder) = sign_with(alg, &xml);
        verify_with(alg, &xml, &sig, &holder);
    }
}

// ─── Tampering rejection across all four message types & all algos ───

#[test]
fn tampered_authn_request_rejected_by_every_algorithm() {
    let req = AuthnRequest::new("https://sp.example.com", "https://idp.example.com/sso");
    let xml = req.to_xml().unwrap();
    for alg in ALL_ALGS {
        let (sig, holder) = sign_with(alg, &xml);
        let mut tampered = xml.clone();
        // Flip a byte deep in the document — any byte will do.
        if let Some(b) = tampered.get_mut(20) {
            *b ^= 0x01;
        }
        let doc = SignedDocument::new(&tampered);
        let mat = match &holder {
            KeyHolder::Rsa => {
                let pub_der = rsa_pub_der();
                let res = verify(
                    &doc,
                    alg,
                    &sig,
                    &VerifyingMaterial::Rsa {
                        rsa_pub_der: &pub_der,
                    },
                );
                assert!(res.is_err(), "{alg:?}: tampered request must be rejected");
                continue;
            }
            KeyHolder::Ecdsa(kp) => VerifyingMaterial::Ecdsa { key: &kp.verifying },
        };
        let res = verify(&doc, alg, &sig, &mat);
        assert!(res.is_err(), "{alg:?}: tampered request must be rejected");
    }
}
