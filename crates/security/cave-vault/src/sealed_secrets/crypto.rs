// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sealed Secrets hybrid encryption — `pkg/crypto/crypto.go` port.
//!
//! Upstream uses RSA-OAEP-SHA256 to wrap an ephemeral AES-256-GCM session key,
//! with the SealedSecret's binding label (see `super::binding_label`) mixed in
//! via the OAEP `label` argument.
//!
//! ## Scope cut
//! The ring/rsa-oaep dependency story isn't worth pulling in for the in-memory
//! deep-port — Vault's existing `transit` engine already provides RSA-OAEP via
//! `ring`. Here we expose the wire format + key-derivation primitives without
//! the live OAEP wrap (delegated to `crate::engines::transit`). The wire
//! format itself round-trips losslessly.

use sha2::{Digest, Sha256};

/// SealedSecret v0 wire format:
///
/// ```text
/// [u16 BE wrapped-key-len][wrapped-key bytes][nonce: 12 bytes][ciphertext...]
/// ```
///
/// Returns `(wrapped_key, nonce, ciphertext)` or an error.
pub fn split_envelope(envelope: &[u8]) -> Result<(Vec<u8>, [u8; 12], Vec<u8>), &'static str> {
    if envelope.len() < 2 {
        return Err("envelope too short");
    }
    let wrapped_len = u16::from_be_bytes([envelope[0], envelope[1]]) as usize;
    let header_len = 2 + wrapped_len;
    if envelope.len() < header_len + 12 {
        return Err("envelope missing nonce");
    }
    let wrapped = envelope[2..header_len].to_vec();
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&envelope[header_len..header_len + 12]);
    let ct = envelope[header_len + 12..].to_vec();
    Ok((wrapped, nonce, ct))
}

/// Assemble a SealedSecret envelope from its parts.
pub fn assemble_envelope(wrapped_key: &[u8], nonce: &[u8; 12], ciphertext: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + wrapped_key.len() + 12 + ciphertext.len());
    let len = u16::try_from(wrapped_key.len()).expect("wrapped-key fits in u16");
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(wrapped_key);
    out.extend_from_slice(nonce);
    out.extend_from_slice(ciphertext);
    out
}

/// HKDF-SHA256 binding-label derivation step.
///
/// label_hash = SHA256(scope_label || "|" || secret_name)
///
/// The hash is then used as the OAEP `label` when wrapping the AES key.
pub fn label_hash(scope_label: &[u8], secret_name: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(scope_label);
    h.update(b"|");
    h.update(secret_name);
    let out = h.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    arr
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_round_trip() {
        let wrapped = b"wrapped-aes-key-bytes-here";
        let nonce: [u8; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let ct = b"ciphertext-of-the-secret-payload";
        let env = assemble_envelope(wrapped, &nonce, ct);
        let (w2, n2, c2) = split_envelope(&env).unwrap();
        assert_eq!(w2, wrapped);
        assert_eq!(n2, nonce);
        assert_eq!(c2, ct);
    }

    #[test]
    fn envelope_too_short_errors() {
        let bad = [0u8; 1];
        assert!(split_envelope(&bad).is_err());
    }

    #[test]
    fn envelope_missing_nonce_errors() {
        // wrapped-len = 4, but only 2 bytes after header.
        let bad = vec![0, 4, 1, 2, 3, 4, 9, 9];
        assert!(split_envelope(&bad).is_err());
    }

    #[test]
    fn label_hash_strict_uses_namespace_and_name() {
        let a = label_hash(b"default", b"my-secret");
        let b = label_hash(b"default", b"other-secret");
        assert_ne!(a, b);
    }

    #[test]
    fn label_hash_cluster_vs_namespace() {
        let cluster = label_hash(b"", b"foo");
        let ns = label_hash(b"default", b"foo");
        assert_ne!(cluster, ns);
    }
}
