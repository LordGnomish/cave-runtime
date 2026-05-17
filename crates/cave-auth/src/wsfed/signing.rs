// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 saml-core/src/main/java/org/keycloak/saml/processing/api/saml/v1/sig/

//! SAML 1.1 signing — RED phase: tests authored, implementation lands in GREEN.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

use super::WsFedError;

pub struct Saml11SignedAssertion {
    pub assertion_xml: String,
    pub assertion_id: String,
}

impl Saml11SignedAssertion {
    pub fn new(assertion_xml: impl Into<String>, assertion_id: impl Into<String>) -> Self {
        Self {
            assertion_xml: assertion_xml.into(),
            assertion_id: assertion_id.into(),
        }
    }
    pub fn sign(&self, _pkcs8_der: &[u8]) -> Result<String, WsFedError> {
        Err(WsFedError::Signature("RED-phase stub".into()))
    }
    pub fn verify(_signed_xml: &str, _rsa_pub_der: &[u8]) -> Result<(), WsFedError> {
        Err(WsFedError::Signature("RED-phase stub".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::saml11_assertion::Saml11Assertion;

    fn test_keypair_pkcs8_der() -> Vec<u8> {
        const KEY_B64: &str = "MIIEvwIBADANBgkqhkiG9w0BAQEFAASCBKkwggSlAgEAAoIBAQCPegzMZl+1jHVMT0PW68K/qcIYqbBkkO6ooVUmxuDLFq0NIQmuteQ30RM06txbzpJdtBO/vAxOfcUBQ+jmKwixHC0JUcW6jixFfTOwFKIdeByzIRNoi1i/ZbrhhknLKZ3U2IQz4VwroyKbL2mFg5dPDA1oj1cJG4QODWLqbcjngRmExdM8remq+c6HGiI2TS0aldg3/wGBI5C+IyOeniVjzaFN/Z3GCqq9uC7Ij8spDoGBZPpskH8ehFLb6RsoxvVWJKJmB7LSNkabWXVLD+a+oqVO9ozMlV1R6qZZ4IUV7+lNS4BQp4Vla3RIKajjj2YKzGIl9UUEyH/A3SOlkqrrAgMBAAECggEANzZ8nlv3EOJQcWE/dgGcHC2zp9IFM24iqXoMTrPR5dWAGsFP/I+6l1A51+9ZhWrlIHIf93TiN4Jmwankgk6lNaLmIeP592Sm3MblkSkfib+jK7vawCx/pof7drY6x5foSPRZS625zoEk3BtOvDZ7j8vPjSE8GSEhnFbCbfx5h7yu4RqjBVEAz7feOMade++Qjn/IyfoNJ2Wq7oq/w7lXVYUNVIS7Ulj9cdTXIF6QFf+84B46d+YTsYiZRGMb/eZYk5IyXdv0vDg+qCD2mV+JYs1PD2qZOKemCxLjYs0OMYy1fKxbYVra4g0gOtTcnUYTJFixuyFifnfOyKKNrpbpgQKBgQDKRtc94O2t+Bah/bU4+90RNB4/lVmCRB0ExkMzJ/djT9TLYYFtnjX6DympmQ6ACzO8cqsArB2nEgbsXCV2lcwCldVY5/I9SuplyqtGKfPRdXlU3GopFNjZ/bdi1GF7MgRpD59yWWRHNN55HV94Eef/LumDOvTBtVu28jRWGJXYmwKBgQC1lUsoBAyBCnXkddsVIm5bqoi84CvcNC+nVRTcn8+x+GYb8o35RSSOymMQzlNd/b1YHzhi2b0R3vikSU/r3LtMdrWgdoIV6ElKgAwcbaoqIb/Zovh3qUXimIZvB8krR6a60QqJVw/1lTRnSuU82zV3ZncCSOJo64TZXmEdm47T8QKBgQCO/smG6w3bYHjPh8WnRRYg5VFE7dXbKz/AclBrR6Oxx2vNY17WGXRbFIEFbjg7+K9YV0/gJ8zGoQ3X5cRuMrOIWFf8g+xRvDY8Q6wU6+97caqWfUNnS1+Jq70K1s0bBF7tzqePdPZZCF0GDefBwBbb5VQa+4Cvt//gMxUgkDzOZQKBgQCRrjQ853qssJrC7vcUrqoBawEHH4awxUGSK0Vwd9qm+xXYyDG1Ug6xbJgsLIxf9SnKoEmZrPzucIflLlgrb8zo3Lh9A3b8Yn8igTa2PBlwceE8l25memzyDdKVE5cG3RZb/UhJxYqtScZgNItT1r6/i3phX94dtQ7BYeHiYiIl0QKBgQCCGN21FfQalMH2duGu7UQnZ03To0uDyn3zoaxxVK7M+9xB8bQ5rFq23ZOuGy1qYE7CitzGkCLf9goiJaNCowwUIKVsj+Joufxg1K9usyThr/OpWwQYNu1TOXpzBmKY1AnK+JVpUsRppc0BzpaPiDcnfi1Ch0ds0gVgPLUfflmX/A==";
        B64.decode(KEY_B64).unwrap()
    }

    fn test_public_key_der() -> Vec<u8> {
        const PUB_B64: &str = "MIIBCgKCAQEAj3oMzGZftYx1TE9D1uvCv6nCGKmwZJDuqKFVJsbgyxatDSEJrrXkN9ETNOrcW86SXbQTv7wMTn3FAUPo5isIsRwtCVHFuo4sRX0zsBSiHXgcsyETaItYv2W64YZJyymd1NiEM+FcK6Mimy9phYOXTwwNaI9XCRuEDg1i6m3I54EZhMXTPK3pqvnOhxoiNk0tGpXYN/8BgSOQviMjnp4lY82hTf2dxgqqvbguyI/LKQ6BgWT6bJB/HoRS2+kbKMb1ViSiZgey0jZGm1l1Sw/mvqKlTvaMzJVdUeqmWeCFFe/pTUuAUKeFZWt0SCmo449mCsxiJfVFBMh/wN0jpZKq6wIDAQAB";
        B64.decode(PUB_B64).unwrap()
    }

    #[test]
    fn signature_block_inserted_into_assertion() {
        let a = Saml11Assertion::new("https://idp.example", "alice@example.com");
        let xml = a.to_xml().unwrap();
        let signer = Saml11SignedAssertion::new(xml.clone(), a.assertion_id.clone());
        let signed = signer.sign(&test_keypair_pkcs8_der()).unwrap();
        assert!(signed.contains("<ds:Signature"));
        assert!(signed.contains("<ds:SignatureValue>"));
        assert!(signed.contains("<ds:DigestValue>"));
    }

    #[test]
    fn signature_reference_points_at_assertion_id() {
        let a = Saml11Assertion::new("https://idp.example", "alice@example.com");
        let xml = a.to_xml().unwrap();
        let signer = Saml11SignedAssertion::new(xml.clone(), a.assertion_id.clone());
        let signed = signer.sign(&test_keypair_pkcs8_der()).unwrap();
        let expect = format!("URI=\"#{}\"", a.assertion_id);
        assert!(signed.contains(&expect), "signed: {signed}");
    }

    #[test]
    fn signed_assertion_round_trips_through_verify() {
        let a = Saml11Assertion::new("https://idp.example", "alice@example.com");
        let xml = a.to_xml().unwrap();
        let signer = Saml11SignedAssertion::new(xml.clone(), a.assertion_id.clone());
        let signed = signer.sign(&test_keypair_pkcs8_der()).unwrap();
        Saml11SignedAssertion::verify(&signed, &test_public_key_der()).unwrap();
    }

    #[test]
    fn tampered_assertion_fails_verify() {
        let a = Saml11Assertion::new("https://idp.example", "alice@example.com");
        let xml = a.to_xml().unwrap();
        let signer = Saml11SignedAssertion::new(xml.clone(), a.assertion_id.clone());
        let signed = signer.sign(&test_keypair_pkcs8_der()).unwrap();
        let tampered = signed.replace("alice@example.com", "mallory@example.com");
        assert!(Saml11SignedAssertion::verify(&tampered, &test_public_key_der()).is_err());
    }

    #[test]
    fn verify_rejects_unsigned_input() {
        let a = Saml11Assertion::new("https://idp.example", "alice@example.com");
        let xml = a.to_xml().unwrap();
        let err = Saml11SignedAssertion::verify(&xml, &test_public_key_der()).unwrap_err();
        assert!(matches!(err, WsFedError::MissingField(_)));
    }

    #[test]
    fn signature_uses_rsa_sha256_alg_uri() {
        let a = Saml11Assertion::new("https://idp.example", "alice@example.com");
        let xml = a.to_xml().unwrap();
        let signer = Saml11SignedAssertion::new(xml.clone(), a.assertion_id.clone());
        let signed = signer.sign(&test_keypair_pkcs8_der()).unwrap();
        assert!(signed.contains("http://www.w3.org/2001/04/xmldsig-more#rsa-sha256"));
    }
}
