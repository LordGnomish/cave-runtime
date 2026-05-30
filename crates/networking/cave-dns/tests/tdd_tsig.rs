// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: TSIG transaction signatures for zone transfer (RFC 8945),
//! HMAC-SHA256 MAC computation verified against RFC 4231 test vectors.
//!
//! This closes the `tsig-hmac-zone-transfer` partial: previously cave-dns
//! parsed the TSIG key name but never computed the HMAC. Upstream reference:
//! miekg/dns `tsig.go` (vendored by CoreDNS v1.14.3).

use cave_dns::zone::tsig::TsigKey;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn rfc4231_test_case_1_hmac_sha256() {
    // RFC 4231 §4.2: key = 0x0b x20, data = "Hi There".
    let key = TsigKey::new("test.example.", vec![0x0b; 20]);
    let mac = key.sign(b"Hi There");
    assert_eq!(
        hex(&mac),
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
    );
}

#[test]
fn verify_accepts_matching_mac() {
    let key = TsigKey::new("k.example.", b"super-secret-zone-key".to_vec());
    let mac = key.sign(b"axfr payload bytes");
    assert!(key.verify(b"axfr payload bytes", &mac));
}

#[test]
fn verify_rejects_tampered_payload() {
    let key = TsigKey::new("k.example.", b"super-secret-zone-key".to_vec());
    let mac = key.sign(b"axfr payload bytes");
    assert!(!key.verify(b"tampered payload bytes", &mac));
}

#[test]
fn verify_rejects_wrong_key() {
    let signer = TsigKey::new("k.example.", b"key-one".to_vec());
    let attacker = TsigKey::new("k.example.", b"key-two".to_vec());
    let mac = signer.sign(b"axfr payload");
    assert!(!attacker.verify(b"axfr payload", &mac));
}
