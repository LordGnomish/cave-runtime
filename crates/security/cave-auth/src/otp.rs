// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! HOTP / TOTP one-time-password primitives.
//!
//! Line-ported from Keycloak (Apache-2.0):
//!   - `server-spi/.../models/utils/HmacOTP.java`          (RFC 4226 HOTP)
//!   - `server-spi-private/.../models/utils/TimeBasedOTP.java` (RFC 6238 TOTP)
//!
//! Semantics are preserved exactly, including the dynamic-truncation offset,
//! the `validateHOTP` "return next counter or -1" contract, and the TOTP
//! look-around window expansion order (`0, -1, +1, -2, +2, ...`).

use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Sha256, Sha512};

/// HMAC hash function backing the OTP generator.
///
/// Mirrors Keycloak's `HmacOTP.HMAC_SHA1 / HMAC_SHA256 / HMAC_SHA512`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtpAlg {
    Sha1,
    Sha256,
    Sha512,
}

// 10^0 .. 10^8 — Keycloak's `DIGITS_POWER` table.
const DIGITS_POWER: [u32; 9] = [1, 10, 100, 1000, 10000, 100000, 1000000, 10000000, 100000000];

/// Counter-based one-time password (RFC 4226), the shared base for HOTP and TOTP.
///
/// Port of `org.keycloak.models.utils.HmacOTP`.
#[derive(Debug, Clone)]
pub struct HmacOtp {
    number_digits: u8,
    algorithm: OtpAlg,
    look_around_window: u32,
}

impl HmacOtp {
    /// `new(numberDigits, algorithm, delayWindow)` — same parameter order as the Java ctor.
    pub fn new(number_digits: u8, algorithm: OtpAlg, look_around_window: u32) -> Self {
        Self { number_digits, algorithm, look_around_window }
    }

    fn mac(&self, key: &[u8], msg: &[u8]) -> Vec<u8> {
        match self.algorithm {
            OtpAlg::Sha1 => {
                let mut m = <Hmac<Sha1>>::new_from_slice(key).expect("HMAC accepts any key length");
                m.update(msg);
                m.finalize().into_bytes().to_vec()
            }
            OtpAlg::Sha256 => {
                let mut m = <Hmac<Sha256>>::new_from_slice(key).expect("HMAC accepts any key length");
                m.update(msg);
                m.finalize().into_bytes().to_vec()
            }
            OtpAlg::Sha512 => {
                let mut m = <Hmac<Sha512>>::new_from_slice(key).expect("HMAC accepts any key length");
                m.update(msg);
                m.finalize().into_bytes().to_vec()
            }
        }
    }

    /// Port of `generateOTP(byte[] key, String counter, int returnDigits, String crypto)`.
    ///
    /// Keycloak hex-pads the counter to 16 chars then decodes to an 8-byte big-endian
    /// moving factor; that is identical to `counter.to_be_bytes()` for a `u64`.
    fn generate_otp(&self, key: &[u8], counter: u64) -> String {
        let msg = counter.to_be_bytes();
        let hash = self.mac(key, &msg);

        // Dynamic truncation (RFC 4226 §5.3).
        let offset = (hash[hash.len() - 1] & 0xf) as usize;
        let binary = (((hash[offset] & 0x7f) as u32) << 24)
            | ((hash[offset + 1] as u32) << 16)
            | ((hash[offset + 2] as u32) << 8)
            | (hash[offset + 3] as u32);

        let otp = binary % DIGITS_POWER[self.number_digits as usize];
        format!("{:0width$}", otp, width = self.number_digits as usize)
    }

    /// Port of `generateHOTP(byte[] key, int counter)`.
    pub fn generate_hotp(&self, key: &[u8], counter: u64) -> String {
        self.generate_otp(key, counter)
    }

    /// Port of `validateHOTP(String token, byte[] key, int counter)`.
    ///
    /// Returns `-1` on no match, otherwise the new counter value
    /// (`matched_counter + 1`), searching `counter ..= counter + lookAroundWindow`.
    pub fn validate_hotp(&self, token: &str, key: &[u8], counter: u64) -> i64 {
        for new_counter in counter..=counter + self.look_around_window as u64 {
            if self.generate_hotp(key, new_counter) == token {
                return new_counter as i64 + 1;
            }
        }
        -1
    }
}

/// Time-based one-time password (RFC 6238).
///
/// Port of `org.keycloak.models.utils.TimeBasedOTP`. Time is supplied explicitly
/// (`*_at` methods take unix seconds) so the algorithm is deterministic and
/// testable — the analogue of Keycloak's injectable `Clock`/`setCalendar`.
#[derive(Debug, Clone)]
pub struct TimeBasedOtp {
    inner: HmacOtp,
    interval_seconds: u64,
}

impl TimeBasedOtp {
    pub const DEFAULT_INTERVAL_SECONDS: u64 = 30;
    pub const DEFAULT_DELAY_WINDOW: u32 = 1;

    pub fn new(
        algorithm: OtpAlg,
        number_digits: u8,
        interval_seconds: u64,
        look_around_window: u32,
    ) -> Self {
        Self { inner: HmacOtp::new(number_digits, algorithm, look_around_window), interval_seconds }
    }

    /// Keycloak's no-arg default: HMAC-SHA1, 6 digits, 30s interval, look-around window 1.
    pub fn default_totp() -> Self {
        Self::new(OtpAlg::Sha1, 6, Self::DEFAULT_INTERVAL_SECONDS, Self::DEFAULT_DELAY_WINDOW)
    }

    fn current_interval(&self, unix_seconds: u64) -> u64 {
        unix_seconds / self.interval_seconds
    }

    /// Port of `generateTOTP(byte[] secretKey)`, with the clock supplied explicitly.
    pub fn generate_totp_at(&self, secret: &[u8], unix_seconds: u64) -> String {
        self.inner.generate_otp(secret, self.current_interval(unix_seconds))
    }

    /// Maps `0, 1, 2, 3, 4, ...` to `0, -1, 1, -2, 2, ...` — Keycloak's
    /// `clockSkewIndexToDelta`, the look-around expansion order.
    fn clock_skew_index_to_delta(idx: u32) -> i64 {
        ((idx as i64 + 1) / 2) * (1 - (idx as i64 % 2) * 2)
    }

    /// Port of `validateTOTP(String token, byte[] secret)`.
    pub fn validate_totp_at(&self, token: &str, secret: &[u8], unix_seconds: u64) -> bool {
        let current = self.current_interval(unix_seconds) as i64;
        for i in 0..=(self.inner.look_around_window * 2) {
            let adjusted = current + Self::clock_skew_index_to_delta(i);
            if adjusted < 0 {
                continue;
            }
            if self.inner.generate_otp(secret, adjusted as u64) == token {
                return true;
            }
        }
        false
    }
}
