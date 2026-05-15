// SPDX-License-Identifier: AGPL-3.0-or-later
//! XML Encryption (XML-ENC 1.1) for `<saml:EncryptedAssertion>`.
//!
//! Source: keycloak/keycloak@b825ba97
//!         saml-core/src/main/java/org/keycloak/saml/processing/core/util/XMLEncryptionUtil.java
//!         saml-core/src/main/java/org/keycloak/saml/processing/api/util/KeyInfoTools.java
//!
//! ## Wire layout
//!
//! cave-auth emits the standard XML-ENC 1.1 envelope: an outer
//! `<saml:EncryptedAssertion>` wraps an `<xenc:EncryptedData>` with the
//! data-cipher algorithm, plus a `<xenc:EncryptedKey>` (inside the
//! `<ds:KeyInfo>`) that key-transports the symmetric content-encryption
//! key (CEK) with RSA-OAEP-MGF1-SHA256.
//!
//! ```xml
//! <saml:EncryptedAssertion xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion">
//!   <xenc:EncryptedData xmlns:xenc="http://www.w3.org/2001/04/xmlenc#"
//!                       Type="http://www.w3.org/2001/04/xmlenc#Element">
//!     <xenc:EncryptionMethod Algorithm="http://www.w3.org/2009/xmlenc11#aes256-gcm"/>
//!     <ds:KeyInfo xmlns:ds="http://www.w3.org/2000/09/xmldsig#">
//!       <xenc:EncryptedKey>
//!         <xenc:EncryptionMethod Algorithm="http://www.w3.org/2009/xmlenc11#rsa-oaep">
//!           <ds:DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/>
//!           <xenc11:MGF Algorithm="http://www.w3.org/2009/xmlenc11#mgf1sha256"/>
//!         </xenc:EncryptionMethod>
//!         <xenc:CipherData>
//!           <xenc:CipherValue>base64(wrapped CEK)</xenc:CipherValue>
//!         </xenc:CipherData>
//!       </xenc:EncryptedKey>
//!     </ds:KeyInfo>
//!     <xenc:CipherData>
//!       <xenc:CipherValue>base64(nonce || ciphertext || gcm_tag)</xenc:CipherValue>
//!     </xenc:CipherData>
//!   </xenc:EncryptedData>
//! </saml:EncryptedAssertion>
//! ```
//!
//! ## AES-GCM nonce layout
//!
//! Per the XML-ENC 1.1 spec, the `CipherValue` for an AES-GCM-encrypted
//! payload is `nonce(12) || ciphertext || tag(16)`. cave-auth uses the
//! same packing the W3C test vectors and `xmlsec1` use.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use rand::RngCore;
use rsa::{RsaPrivateKey, RsaPublicKey};
use sha2::Sha256;

use super::SamlError;

pub const ALG_AES128_GCM: &str = "http://www.w3.org/2009/xmlenc11#aes128-gcm";
pub const ALG_AES256_GCM: &str = "http://www.w3.org/2009/xmlenc11#aes256-gcm";
pub const ALG_RSA_OAEP_MGF1P_SHA256: &str = "http://www.w3.org/2009/xmlenc11#rsa-oaep";
pub const ALG_MGF1_SHA256: &str = "http://www.w3.org/2009/xmlenc11#mgf1sha256";
pub const ALG_SHA256: &str = "http://www.w3.org/2001/04/xmlenc#sha256";

/// Which AES-GCM variant the data cipher uses for the CEK.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XmlEncKey {
    Aes128Gcm,
    Aes256Gcm,
}

impl XmlEncKey {
    pub fn alg_urn(self) -> &'static str {
        match self {
            XmlEncKey::Aes128Gcm => ALG_AES128_GCM,
            XmlEncKey::Aes256Gcm => ALG_AES256_GCM,
        }
    }

    pub fn key_bytes(self) -> usize {
        match self {
            XmlEncKey::Aes128Gcm => 16,
            XmlEncKey::Aes256Gcm => 32,
        }
    }
}

// ─── AES-GCM data cipher ────────────────────────────────────────────────────

/// AES-128-GCM encrypt. Returns `(ciphertext_with_tag, nonce)`. The
/// 16-byte authentication tag is appended to the ciphertext exactly
/// as `aes_gcm::Aes128Gcm::encrypt` returns it.
pub fn aes128_gcm_encrypt(key: &[u8; 16], pt: &[u8]) -> Result<(Vec<u8>, [u8; 12]), SamlError> {
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes128Gcm, KeyInit, Nonce};
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let cipher = Aes128Gcm::new(key.into());
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), pt)
        .map_err(|e| SamlError::Other(format!("aes-128-gcm encrypt: {e}")))?;
    Ok((ct, nonce_bytes))
}

/// AES-128-GCM decrypt. Verifies the appended tag; returns
/// `InvalidSignature` on tag mismatch.
pub fn aes128_gcm_decrypt(
    key: &[u8; 16],
    nonce: &[u8; 12],
    ct: &[u8],
) -> Result<Vec<u8>, SamlError> {
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes128Gcm, KeyInit, Nonce};
    let cipher = Aes128Gcm::new(key.into());
    cipher
        .decrypt(Nonce::from_slice(nonce), ct)
        .map_err(|_| SamlError::InvalidSignature("aes-128-gcm tag invalid".into()))
}

/// AES-256-GCM encrypt.
pub fn aes256_gcm_encrypt(key: &[u8; 32], pt: &[u8]) -> Result<(Vec<u8>, [u8; 12]), SamlError> {
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let cipher = Aes256Gcm::new(key.into());
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), pt)
        .map_err(|e| SamlError::Other(format!("aes-256-gcm encrypt: {e}")))?;
    Ok((ct, nonce_bytes))
}

/// AES-256-GCM decrypt.
pub fn aes256_gcm_decrypt(
    key: &[u8; 32],
    nonce: &[u8; 12],
    ct: &[u8],
) -> Result<Vec<u8>, SamlError> {
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
    let cipher = Aes256Gcm::new(key.into());
    cipher
        .decrypt(Nonce::from_slice(nonce), ct)
        .map_err(|_| SamlError::InvalidSignature("aes-256-gcm tag invalid".into()))
}

// ─── RSA-OAEP-MGF1-SHA256 key transport ─────────────────────────────────────

/// Wrap (encrypt) a CEK to the recipient's RSA SPKI DER public key
/// using RSA-OAEP with MGF1-SHA256.
pub fn rsa_oaep_sha256_wrap(spki_der: &[u8], cek: &[u8]) -> Result<Vec<u8>, SamlError> {
    use rsa::pkcs8::DecodePublicKey;
    let pk = RsaPublicKey::from_public_key_der(spki_der)
        .map_err(|e| SamlError::Other(format!("rsa pub spki: {e}")))?;
    let padding = rsa::Oaep::new::<Sha256>();
    pk.encrypt(&mut rand::thread_rng(), padding, cek)
        .map_err(|e| SamlError::Other(format!("rsa-oaep wrap: {e}")))
}

/// Unwrap (decrypt) a wrapped CEK using the local RSA private key.
pub fn rsa_oaep_sha256_unwrap(
    sk: &RsaPrivateKey,
    wrapped: &[u8],
) -> Result<Vec<u8>, SamlError> {
    let padding = rsa::Oaep::new::<Sha256>();
    sk.decrypt(padding, wrapped)
        .map_err(|e| SamlError::InvalidSignature(format!("rsa-oaep unwrap: {e}")))
}

// ─── Composed: <saml:EncryptedAssertion> ────────────────────────────────────

/// Encrypt a plaintext SAML assertion blob using a fresh symmetric CEK
/// (AES-128 or AES-256 GCM), then key-transport that CEK to the
/// recipient's RSA public key (SPKI DER). Returns the full
/// `<saml:EncryptedAssertion>` XML wire form.
pub fn encrypt_assertion(
    plaintext: &[u8],
    recipient_rsa_spki_der: &[u8],
    cek_alg: XmlEncKey,
) -> Result<Vec<u8>, SamlError> {
    let mut cek = vec![0u8; cek_alg.key_bytes()];
    rand::thread_rng().fill_bytes(&mut cek);

    let (ciphertext, nonce) = match cek_alg {
        XmlEncKey::Aes128Gcm => {
            let mut k = [0u8; 16];
            k.copy_from_slice(&cek);
            aes128_gcm_encrypt(&k, plaintext)?
        }
        XmlEncKey::Aes256Gcm => {
            let mut k = [0u8; 32];
            k.copy_from_slice(&cek);
            aes256_gcm_encrypt(&k, plaintext)?
        }
    };

    let wrapped_cek = rsa_oaep_sha256_wrap(recipient_rsa_spki_der, &cek)?;

    // XML-ENC packing: cipher value = nonce || ciphertext (which already
    // has the 16-byte tag appended by the aead crate).
    let mut cipher_value = Vec::with_capacity(12 + ciphertext.len());
    cipher_value.extend_from_slice(&nonce);
    cipher_value.extend_from_slice(&ciphertext);
    let cipher_value_b64 = B64.encode(&cipher_value);
    let wrapped_b64 = B64.encode(&wrapped_cek);

    let xml = format!(
        r#"<saml:EncryptedAssertion xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion">\
<xenc:EncryptedData xmlns:xenc="http://www.w3.org/2001/04/xmlenc#" Type="http://www.w3.org/2001/04/xmlenc#Element">\
<xenc:EncryptionMethod Algorithm="{data_alg}"/>\
<ds:KeyInfo xmlns:ds="http://www.w3.org/2000/09/xmldsig#">\
<xenc:EncryptedKey xmlns:xenc11="http://www.w3.org/2009/xmlenc11#">\
<xenc:EncryptionMethod Algorithm="{key_alg}">\
<ds:DigestMethod Algorithm="{digest_alg}"/>\
<xenc11:MGF Algorithm="{mgf_alg}"/>\
</xenc:EncryptionMethod>\
<xenc:CipherData><xenc:CipherValue>{wrapped}</xenc:CipherValue></xenc:CipherData>\
</xenc:EncryptedKey>\
</ds:KeyInfo>\
<xenc:CipherData><xenc:CipherValue>{cv}</xenc:CipherValue></xenc:CipherData>\
</xenc:EncryptedData>\
</saml:EncryptedAssertion>"#,
        data_alg = cek_alg.alg_urn(),
        key_alg = ALG_RSA_OAEP_MGF1P_SHA256,
        digest_alg = ALG_SHA256,
        mgf_alg = ALG_MGF1_SHA256,
        wrapped = wrapped_b64,
        cv = cipher_value_b64,
    );
    // The format! string includes literal `\` line continuations purely
    // for source readability — strip them out before emitting.
    let cleaned: String = xml.split('\\').collect();
    Ok(cleaned.into_bytes())
}

/// Decrypt a `<saml:EncryptedAssertion>` blob, returning the inner
/// assertion plaintext. Pulls the data-cipher algorithm from the
/// outer `<xenc:EncryptionMethod>`, the wrapped CEK from the
/// `<xenc:EncryptedKey>`, then performs RSA-OAEP unwrap → AES-GCM
/// decrypt.
pub fn decrypt_assertion(encrypted_xml: &[u8], sk: &RsaPrivateKey) -> Result<Vec<u8>, SamlError> {
    let xml = std::str::from_utf8(encrypted_xml)
        .map_err(|e| SamlError::Parse(format!("utf-8: {e}")))?;

    let alg = find_first_attr(xml, "EncryptionMethod", "Algorithm")
        .ok_or_else(|| SamlError::MissingField("EncryptionMethod/Algorithm".into()))?;
    let cek_alg = match alg.as_str() {
        ALG_AES128_GCM => XmlEncKey::Aes128Gcm,
        ALG_AES256_GCM => XmlEncKey::Aes256Gcm,
        other => {
            return Err(SamlError::Other(format!(
                "unsupported xmlenc data cipher: {other}"
            )))
        }
    };

    // The first <xenc:CipherValue> belongs to <xenc:EncryptedKey>
    // (per the XML-ENC 1.1 schema), the second is the data cipher.
    let cipher_values = collect_cipher_values(xml);
    if cipher_values.len() < 2 {
        return Err(SamlError::MissingField(
            "two <xenc:CipherValue> elements (wrapped CEK + data)".into(),
        ));
    }
    let wrapped_b64 = &cipher_values[0];
    let data_b64 = &cipher_values[1];

    let wrapped = B64
        .decode(wrapped_b64)
        .map_err(|e| SamlError::Parse(format!("wrapped base64: {e}")))?;
    let data = B64
        .decode(data_b64)
        .map_err(|e| SamlError::Parse(format!("data base64: {e}")))?;
    if data.len() < 12 + 16 {
        return Err(SamlError::Parse(format!(
            "cipher value too short ({} bytes)",
            data.len()
        )));
    }
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&data[..12]);
    let ciphertext = &data[12..];

    let cek = rsa_oaep_sha256_unwrap(sk, &wrapped)?;

    match cek_alg {
        XmlEncKey::Aes128Gcm => {
            if cek.len() != 16 {
                return Err(SamlError::Parse(format!(
                    "unwrapped CEK len {} != 16 for AES-128",
                    cek.len()
                )));
            }
            let mut k = [0u8; 16];
            k.copy_from_slice(&cek);
            aes128_gcm_decrypt(&k, &nonce, ciphertext)
        }
        XmlEncKey::Aes256Gcm => {
            if cek.len() != 32 {
                return Err(SamlError::Parse(format!(
                    "unwrapped CEK len {} != 32 for AES-256",
                    cek.len()
                )));
            }
            let mut k = [0u8; 32];
            k.copy_from_slice(&cek);
            aes256_gcm_decrypt(&k, &nonce, ciphertext)
        }
    }
}

// ─── tiny XML pinch-point helpers (single-axis text extraction) ─────────────

fn find_first_attr(xml: &str, tag: &str, attr: &str) -> Option<String> {
    // Match either `<xenc:EncryptionMethod` or `<EncryptionMethod`.
    for prefix in [format!("<xenc:{}", tag), format!("<{}", tag)] {
        if let Some(start) = xml.find(&prefix) {
            let after = &xml[start..];
            let needle = format!("{}=\"", attr);
            if let Some(i) = after.find(&needle) {
                let after_eq = &after[i + needle.len()..];
                if let Some(end) = after_eq.find('"') {
                    return Some(after_eq[..end].to_string());
                }
            }
        }
    }
    None
}

fn collect_cipher_values(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(start) = xml[pos..].find("<xenc:CipherValue") {
        let abs = pos + start;
        let after_open = &xml[abs..];
        if let Some(gt) = after_open.find('>') {
            let body_start = abs + gt + 1;
            if let Some(end_rel) = xml[body_start..].find("</xenc:CipherValue>") {
                let body = xml[body_start..body_start + end_rel]
                    .trim()
                    .to_string();
                out.push(body);
                pos = body_start + end_rel + "</xenc:CipherValue>".len();
                continue;
            }
        }
        break;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xmlenc_key_alg_urns() {
        assert_eq!(XmlEncKey::Aes128Gcm.alg_urn(), ALG_AES128_GCM);
        assert_eq!(XmlEncKey::Aes256Gcm.alg_urn(), ALG_AES256_GCM);
        assert_eq!(XmlEncKey::Aes128Gcm.key_bytes(), 16);
        assert_eq!(XmlEncKey::Aes256Gcm.key_bytes(), 32);
    }

    #[test]
    fn collect_cipher_values_pulls_two() {
        let xml = "<xenc:CipherValue>A</xenc:CipherValue><xenc:CipherValue>B</xenc:CipherValue>";
        let v = collect_cipher_values(xml);
        assert_eq!(v, vec!["A".to_string(), "B".to_string()]);
    }

    #[test]
    fn find_first_attr_handles_prefix() {
        let xml = r#"<xenc:EncryptionMethod Algorithm="urn:x"/>"#;
        assert_eq!(find_first_attr(xml, "EncryptionMethod", "Algorithm"), Some("urn:x".into()));
    }
}
