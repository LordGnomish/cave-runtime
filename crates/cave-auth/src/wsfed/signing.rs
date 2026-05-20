// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 saml-core/src/main/java/org/keycloak/saml/processing/api/saml/v1/sig/

//! Sign a SAML 1.1 assertion using the same RSA-SHA256 path the SAML 2.0
//! side uses. We don't duplicate the RSA implementation — we thin-shim
//! [`crate::saml::signature`] and only touch the parts that differ:
//!
//! * The `<ds:Signature>` block is inserted **inside** the
//!   `<saml:Assertion>` element (before `<saml:Conditions>`), not after.
//! * The `Reference URI` points at the `AssertionID`, not at an ID
//!   attribute named `ID` (SAML 2.0 uses `ID="…"`, SAML 1.1 uses
//!   `AssertionID="…"`).
//!
//! Apart from those two surface differences, the crypto step is
//! identical: PKCS#1 v1.5 over SHA-256, base64 of the result placed in
//! `<ds:SignatureValue>`.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;

use crate::saml::SamlError;
use crate::saml::signature::{
    ALG_EXC_C14N, ALG_RSA_SHA256, ALG_SHA256, SignedDocument, sign_rsa_sha256, verify_signature,
};

use super::WsFedError;

/// XML DSig wrapper inserted into a SAML 1.1 assertion.
///
/// This mirrors `<ds:Signature>` produced by Keycloak's
/// `SAML2Signature.signSAMLDocument` path with the small AssertionID
/// rewrite mentioned above.
pub struct Saml11SignedAssertion {
    /// Original unsigned assertion XML (the [`super::saml11_assertion::Saml11Assertion::to_xml`] output).
    pub assertion_xml: String,
    /// `AssertionID` attribute value (without the `_` prefix already gets
    /// echoed back unchanged in the `Reference URI="#…"`).
    pub assertion_id: String,
}

impl Saml11SignedAssertion {
    pub fn new(assertion_xml: impl Into<String>, assertion_id: impl Into<String>) -> Self {
        Self {
            assertion_xml: assertion_xml.into(),
            assertion_id: assertion_id.into(),
        }
    }

    /// Sign the assertion in-place — returns assertion XML with a
    /// `<ds:Signature>` block inserted right after the opening
    /// `<saml:Assertion …>` tag.
    pub fn sign(&self, pkcs8_der: &[u8]) -> Result<String, WsFedError> {
        // 1. Compute the DSig over the assertion bytes (pre-canonicalised
        //    by our caller — see crate::saml::signature for the c14n
        //    honesty note).
        let doc = SignedDocument::new(self.assertion_xml.as_bytes());
        let sig_b64 = sign_rsa_sha256(&doc, pkcs8_der)
            .map_err(|e: SamlError| WsFedError::Signature(format!("{e}")))?;

        // 2. Build the <ds:Signature> XML.
        //    DigestValue is the SHA-256 of the canonical bytes — for our
        //    purposes the canonical bytes are the assertion bytes.
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(self.assertion_xml.as_bytes());
        let digest_b64 = B64.encode(digest);

        let sig_xml = format!(
            "<ds:Signature xmlns:ds=\"http://www.w3.org/2000/09/xmldsig#\">\
             <ds:SignedInfo>\
             <ds:CanonicalizationMethod Algorithm=\"{c14n}\"/>\
             <ds:SignatureMethod Algorithm=\"{sigalg}\"/>\
             <ds:Reference URI=\"#{id}\">\
             <ds:Transforms>\
             <ds:Transform Algorithm=\"http://www.w3.org/2000/09/xmldsig#enveloped-signature\"/>\
             <ds:Transform Algorithm=\"{c14n}\"/>\
             </ds:Transforms>\
             <ds:DigestMethod Algorithm=\"{digalg}\"/>\
             <ds:DigestValue>{dv}</ds:DigestValue>\
             </ds:Reference>\
             </ds:SignedInfo>\
             <ds:SignatureValue>{sv}</ds:SignatureValue>\
             </ds:Signature>",
            c14n = ALG_EXC_C14N,
            sigalg = ALG_RSA_SHA256,
            digalg = ALG_SHA256,
            id = self.assertion_id,
            dv = digest_b64,
            sv = sig_b64,
        );

        // 3. Insert <ds:Signature> immediately after the opening
        //    <saml:Assertion …> tag. SAML 1.1 differs from 2.0 in
        //    placement: it must come *before* <saml:Conditions>.
        let close = self
            .assertion_xml
            .find('>')
            .ok_or_else(|| WsFedError::Parse("no closing > on Assertion open tag".into()))?;
        let mut out = String::with_capacity(self.assertion_xml.len() + sig_xml.len());
        out.push_str(&self.assertion_xml[..=close]);
        out.push_str(&sig_xml);
        out.push_str(&self.assertion_xml[close + 1..]);
        Ok(out)
    }

    /// Verify that `signed_xml` carries a valid signature against `rsa_pub_der`.
    /// Returns `Ok(())` on a valid signature.
    pub fn verify(signed_xml: &str, rsa_pub_der: &[u8]) -> Result<(), WsFedError> {
        // Extract <ds:SignatureValue> + the original (signature-stripped)
        // assertion bytes.
        let sig_b64 = extract_between(signed_xml, "<ds:SignatureValue>", "</ds:SignatureValue>")
            .ok_or_else(|| WsFedError::MissingField("SignatureValue".into()))?;
        let stripped = strip_ds_signature(signed_xml);
        let doc = SignedDocument::new(stripped.as_bytes());
        verify_signature(&doc, &sig_b64, rsa_pub_der)
            .map_err(|e: SamlError| WsFedError::Signature(format!("{e}")))?;
        Ok(())
    }
}

fn extract_between(s: &str, open: &str, close: &str) -> Option<String> {
    let i = s.find(open)?;
    let after = &s[i + open.len()..];
    let j = after.find(close)?;
    Some(after[..j].to_string())
}

fn strip_ds_signature(s: &str) -> String {
    if let (Some(start), Some(end)) = (s.find("<ds:Signature"), s.find("</ds:Signature>")) {
        let mut out = String::with_capacity(s.len());
        out.push_str(&s[..start]);
        out.push_str(&s[end + "</ds:Signature>".len()..]);
        out
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::super::saml11_assertion::Saml11Assertion;
    use super::*;

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
        // Reference URI must be "#<AssertionID>".
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
