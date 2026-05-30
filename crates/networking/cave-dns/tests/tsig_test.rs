// SPDX-License-Identifier: AGPL-3.0-or-later
//! TSIG signing tests — closes the `tsig-hmac-zone-transfer` partial.
//!
//! Ground truth:
//!   * HMAC primitive — RFC 4231 (SHA-256) + RFC 2202 (SHA-1) published vectors.
//!   * TSIG variables wire layout — RFC 8945 §4.3.3 (hand-encoded reference).
//!   * Request/response MAC binding + fudge window — RFC 8945 §5.2/§5.3.
//! Port of coredns vendor/miekg/dns/tsig.go onto the hickory-proto name model.
use cave_dns::tsig::{compute_mac, fudge_valid, TsigAlgorithm, TsigVariables};
use hickory_proto::rr::Name;
use std::str::FromStr;

fn name(s: &str) -> Name {
    Name::from_str(s).unwrap()
}

// ─── HMAC primitive — published test vectors ───────────────────────────────

#[test]
fn rfc4231_tc1_hmac_sha256() {
    // key = 20 × 0x0b, data = "Hi There"
    let mac = TsigAlgorithm::HmacSha256.raw_hmac(&[0x0b; 20], b"Hi There");
    assert_eq!(
        hex::encode(mac),
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
    );
}

#[test]
fn rfc4231_tc2_hmac_sha256() {
    // key = "Jefe", data = "what do ya want for nothing?"
    let mac = TsigAlgorithm::HmacSha256.raw_hmac(b"Jefe", b"what do ya want for nothing?");
    assert_eq!(
        hex::encode(mac),
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
    );
}

#[test]
fn rfc2202_tc2_hmac_sha1() {
    let mac = TsigAlgorithm::HmacSha1.raw_hmac(b"Jefe", b"what do ya want for nothing?");
    assert_eq!(hex::encode(mac), "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79");
}

// ─── Algorithm <-> canonical DNS name ──────────────────────────────────────

#[test]
fn algorithm_canonical_names() {
    assert_eq!(TsigAlgorithm::HmacSha256.dns_name(), "hmac-sha256.");
    assert_eq!(TsigAlgorithm::HmacSha1.dns_name(), "hmac-sha1.");
    assert_eq!(TsigAlgorithm::HmacSha512.dns_name(), "hmac-sha512.");
    assert_eq!(
        TsigAlgorithm::from_name("hmac-sha256."),
        Some(TsigAlgorithm::HmacSha256)
    );
    assert_eq!(
        TsigAlgorithm::from_name("HMAC-SHA1."),
        Some(TsigAlgorithm::HmacSha1)
    );
    assert_eq!(TsigAlgorithm::from_name("bogus."), None);
}

// ─── TSIG variables wire layout — RFC 8945 §4.3.3 ──────────────────────────

#[test]
fn tsig_variables_wire_layout() {
    let vars = TsigVariables {
        key_name: name("k."),
        algorithm: TsigAlgorithm::HmacSha1,
        time_signed: 0,
        fudge: 300,
        error: 0,
        other: vec![],
    };
    // name "k." canonical = 01 6b 00
    // CLASS ANY = 00 FF ; TTL 0 = 00 00 00 00
    // alg "hmac-sha1." = 09 'hmac-sha1' 00
    // time(6)=0 ; fudge=012C ; error=0000 ; otherlen=0000
    let expected: Vec<u8> = vec![
        0x01, 0x6b, 0x00, // k.
        0x00, 0xFF, // CLASS ANY
        0x00, 0x00, 0x00, 0x00, // TTL 0
        0x09, b'h', b'm', b'a', b'c', b'-', b's', b'h', b'a', b'1', 0x00, // hmac-sha1.
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // time signed (48-bit) = 0
        0x01, 0x2C, // fudge = 300
        0x00, 0x00, // error = 0
        0x00, 0x00, // other len = 0
    ];
    assert_eq!(vars.encode(), expected);
}

#[test]
fn tsig_variables_lowercases_name() {
    let upper = TsigVariables {
        key_name: name("KEY.EXAMPLE."),
        algorithm: TsigAlgorithm::HmacSha256,
        time_signed: 1,
        fudge: 300,
        error: 0,
        other: vec![],
    };
    let lower = TsigVariables {
        key_name: name("key.example."),
        ..upper.clone()
    };
    assert_eq!(upper.encode(), lower.encode());
}

#[test]
fn tsig_variables_time_signed_is_48_bit_be() {
    let vars = TsigVariables {
        key_name: name("."),
        algorithm: TsigAlgorithm::HmacSha256,
        time_signed: 0x0102_0304_0506,
        fudge: 0,
        error: 0,
        other: vec![],
    };
    let enc = vars.encode();
    // after root name (1) + class(2) + ttl(4) + alg("hmac-sha256." = 13) = 20 bytes
    assert_eq!(&enc[20..26], &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
}

// ─── MAC computation, request/response binding, verification ───────────────

fn sample_vars() -> TsigVariables {
    TsigVariables {
        key_name: name("transfer.key."),
        algorithm: TsigAlgorithm::HmacSha256,
        time_signed: 1_700_000_000,
        fudge: 300,
        error: 0,
        other: vec![],
    }
}

#[test]
fn compute_mac_is_deterministic() {
    let key = b"sekret-bytes";
    let msg = b"\x00\x01\x84\x00 a fake dns message";
    let v = sample_vars();
    let a = compute_mac(TsigAlgorithm::HmacSha256, key, None, msg, &v);
    let b = compute_mac(TsigAlgorithm::HmacSha256, key, None, msg, &v);
    assert_eq!(a, b);
    assert_eq!(a.len(), 32); // SHA-256 tag
}

#[test]
fn response_mac_binds_request_mac() {
    // A response MAC prepends the request MAC (RFC 8945 §5.3.1) and so must
    // differ from the same message signed as a request (no prior MAC).
    let key = b"sekret-bytes";
    let msg = b"response-message-body";
    let v = sample_vars();
    let request_mac = vec![0xAA; 32];
    let as_request = compute_mac(TsigAlgorithm::HmacSha256, key, None, msg, &v);
    let as_response = compute_mac(TsigAlgorithm::HmacSha256, key, Some(&request_mac), msg, &v);
    assert_ne!(as_request, as_response);
}

#[test]
fn verify_accepts_matching_and_rejects_tampered() {
    let key = b"sekret-bytes";
    let msg = b"axfr-soa-answer";
    let v = sample_vars();
    let mac = compute_mac(TsigAlgorithm::HmacSha256, key, None, msg, &v);
    assert!(TsigAlgorithm::HmacSha256.verify(key, None, msg, &v, &mac));

    let mut tampered = mac.clone();
    tampered[0] ^= 0xFF;
    assert!(!TsigAlgorithm::HmacSha256.verify(key, None, msg, &v, &tampered));

    // wrong key fails
    assert!(!TsigAlgorithm::HmacSha256.verify(b"other-key", None, msg, &v, &mac));
}

// ─── Fudge time window — RFC 8945 §5.2.3 ───────────────────────────────────

#[test]
fn fudge_window_accepts_within_and_rejects_outside() {
    let signed = 1_700_000_000u64;
    assert!(fudge_valid(signed, 300, signed)); // exact
    assert!(fudge_valid(signed, 300, signed + 300)); // edge late
    assert!(fudge_valid(signed, 300, signed - 300)); // edge early
    assert!(!fudge_valid(signed, 300, signed + 301)); // too late
    assert!(!fudge_valid(signed, 300, signed - 301)); // too early
}
