// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD line-port of Keycloak HmacOTP + TimeBasedOTP.
// Upstream (Apache-2.0):
//   server-spi/src/main/java/org/keycloak/models/utils/HmacOTP.java
//   server-spi-private/src/main/java/org/keycloak/models/utils/TimeBasedOTP.java
//
// Test vectors are the canonical published ones:
//   - RFC 4226 Appendix D (HOTP, HMAC-SHA1, secret "12345678901234567890", 6 digits)
//   - RFC 6238 Appendix B (TOTP, secret "12345678901234567890", 30s interval)

use cave_auth::otp::{HmacOtp, OtpAlg, TimeBasedOtp};

const SECRET: &[u8] = b"12345678901234567890";

#[test]
fn hotp_rfc4226_appendix_d_vectors() {
    // RFC 4226 §Appendix D — HMAC-SHA1, 6 digits, counters 0..=9.
    let expected = [
        "755224", "287082", "359152", "969429", "338314", "254676", "287922", "162583", "399871",
        "520489",
    ];
    let otp = HmacOtp::new(6, OtpAlg::Sha1, 0);
    for (counter, want) in expected.iter().enumerate() {
        let got = otp.generate_hotp(SECRET, counter as u64);
        assert_eq!(&got, want, "HOTP counter {counter} mismatch");
    }
}

#[test]
fn hotp_validate_returns_next_counter_or_minus_one() {
    let otp = HmacOtp::new(6, OtpAlg::Sha1, 0);
    // "287082" is the token at counter 1 — validating against counter 1 succeeds and
    // returns the next counter value (2), exactly like Keycloak's validateHOTP.
    assert_eq!(otp.validate_hotp("287082", SECRET, 1), 2);
    // A wrong token returns -1.
    assert_eq!(otp.validate_hotp("000000", SECRET, 1), -1);
    // With a look-around window, an out-of-sync counter is recovered.
    let windowed = HmacOtp::new(6, OtpAlg::Sha1, 3);
    // token for counter 4 validated starting from counter 1 -> returns 5.
    assert_eq!(windowed.validate_hotp("338314", SECRET, 1), 5);
}

#[test]
fn totp_rfc6238_appendix_b_sha1_8digit_vectors() {
    // RFC 6238 §Appendix B — SHA-1 column, secret "12345678901234567890", 30s, 8 digits.
    let cases: [(u64, &str); 6] = [
        (59, "94287082"),
        (1111111109, "07081804"),
        (1111111111, "14050471"),
        (1234567890, "89005924"),
        (2000000000, "69279037"),
        (20000000000, "65353130"),
    ];
    let totp = TimeBasedOtp::new(OtpAlg::Sha1, 8, 30, 1);
    for (time, want) in cases {
        assert_eq!(totp.generate_totp_at(SECRET, time), want, "TOTP at t={time}");
    }
}

#[test]
fn totp_validate_honours_look_around_window() {
    // Default Keycloak TOTP: SHA-1, 6 digits, 30s, look-around window 1.
    let totp = TimeBasedOtp::default_totp();
    // At t=59 the interval is T=1 and the 6-digit code is 287082.
    assert_eq!(totp.generate_totp_at(SECRET, 59), "287082");
    // Validates within the same interval.
    assert!(totp.validate_totp_at("287082", SECRET, 59));
    // Still valid one interval later (t=89, T=2) because window 1 reaches back to T=1.
    assert!(totp.validate_totp_at("287082", SECRET, 89));
    // No longer valid three intervals later (t=149, T=4): window only reaches T=3..5.
    assert!(!totp.validate_totp_at("287082", SECRET, 149));
}
