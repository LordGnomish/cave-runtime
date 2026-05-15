// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 federation/ldap/src/main/java/org/keycloak/storage/ldap/LDAPUtils.java
// Source: keycloak/keycloak@b825ba97 federation/ldap/src/main/java/org/keycloak/storage/ldap/mappers/PasswordUpdateCallback.java
//
// OpenLDAP-flavored quirks: ppolicy (`pwdAccountLockedTime`,
// `pwdReset`), `shadowExpire` (NIS), GeneralizedTime parser.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Parse RFC 4517 GeneralizedTime — the OpenLDAP-canonical
/// `YYYYMMDDHHMMSSZ` plus the looser `±hhmm` offset form
/// (`YYYYMMDDHHMMSS+0500`) and decimal-fractions
/// (`YYYYMMDDHHMMSS.123Z`).  Returns Unix seconds.
pub fn parse_generalized_time(s: &str) -> Option<SystemTime> {
    if s.len() < 15 {
        return None;
    }
    let year: i32 = s.get(0..4)?.parse().ok()?;
    let month: u32 = s.get(4..6)?.parse().ok()?;
    let day: u32 = s.get(6..8)?.parse().ok()?;
    let hour: u32 = s.get(8..10)?.parse().ok()?;
    let minute: u32 = s.get(10..12)?.parse().ok()?;
    let second: u32 = s.get(12..14)?.parse().ok()?;
    let mut tail = &s[14..];
    // Skip fractional seconds.
    if tail.starts_with('.') {
        let end = tail
            .char_indices()
            .skip(1)
            .find(|(_, c)| !c.is_ascii_digit())
            .map(|(i, _)| i)
            .unwrap_or(tail.len());
        tail = &tail[end..];
    }
    let offset_secs: i64 = if tail == "Z" {
        0
    } else if tail.len() == 5 {
        let sign: i64 = if tail.starts_with('+') { 1 } else if tail.starts_with('-') { -1 } else { return None };
        let h: i64 = tail.get(1..3)?.parse().ok()?;
        let m: i64 = tail.get(3..5)?.parse().ok()?;
        sign * (h * 3600 + m * 60)
    } else {
        return None;
    };
    let unix = civil_to_epoch(year, month, day) + (hour as i64) * 3600 + (minute as i64) * 60 + second as i64 - offset_secs;
    if unix < 0 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_secs(unix as u64))
}

fn civil_to_epoch(year: i32, month: u32, day: u32) -> i64 {
    // Inverse of `epoch_to_ymdhms` (Hinnant 2013).
    let y = if month <= 2 { year - 1 } else { year } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let m = month as i64;
    let d = day as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = (yoe as i64) * 365 + (yoe as i64) / 4 - (yoe as i64) / 100 + doy;
    (era * 146_097 + doe - 719_468) * 86_400
}

/// ppolicy account-state summary used by the portal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PpolicyState {
    pub locked: bool,
    pub must_change_password: bool,
    pub expired: bool,
}

impl PpolicyState {
    pub fn from_attrs(pwd_account_locked_time: Option<&str>, pwd_reset: Option<&str>, shadow_expire: Option<&str>, now_unix: i64) -> Self {
        let mut s = PpolicyState::default();
        if let Some(v) = pwd_account_locked_time {
            // OpenLDAP marks permanent lock with literal
            // "000001010000Z".  Otherwise the value is a regular
            // GeneralizedTime — presence alone signals lock.
            if v == "000001010000Z" || parse_generalized_time(v).is_some() {
                s.locked = true;
            }
        }
        if matches!(pwd_reset, Some(v) if v.eq_ignore_ascii_case("TRUE")) {
            s.must_change_password = true;
        }
        if let Some(v) = shadow_expire {
            if let Ok(days) = v.parse::<i64>() {
                // shadowExpire is days since epoch; negative = no expiry.
                if days >= 0 && days * 86_400 < now_unix {
                    s.expired = true;
                }
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_generalized_time_zulu() {
        let t = parse_generalized_time("20231114221320Z").unwrap();
        let secs = t.duration_since(UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(secs, 1_700_000_000);
    }

    #[test]
    fn parse_generalized_time_with_fraction() {
        let t = parse_generalized_time("20231114221320.500Z").unwrap();
        let secs = t.duration_since(UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(secs, 1_700_000_000);
    }

    #[test]
    fn parse_generalized_time_with_offset() {
        let t = parse_generalized_time("20231114231320+0100").unwrap();
        let secs = t.duration_since(UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(secs, 1_700_000_000);
    }

    #[test]
    fn parse_generalized_time_rejects_garbage() {
        assert!(parse_generalized_time("nope").is_none());
        assert!(parse_generalized_time("20231114221320?00").is_none());
    }

    #[test]
    fn ppolicy_locked_when_locked_time_present() {
        let s = PpolicyState::from_attrs(Some("20231114221320Z"), None, None, 1_700_000_100);
        assert!(s.locked);
        assert!(!s.expired);
    }

    #[test]
    fn ppolicy_must_change_when_pwd_reset_true() {
        let s = PpolicyState::from_attrs(None, Some("TRUE"), None, 0);
        assert!(s.must_change_password);
    }

    #[test]
    fn ppolicy_shadow_expire_flips_expired() {
        // shadowExpire 19500 days = 1683504000 seconds (2023-05-08).
        let s = PpolicyState::from_attrs(None, None, Some("19500"), 1_700_000_000);
        assert!(s.expired);
        // Negative = no expiry.
        let s = PpolicyState::from_attrs(None, None, Some("-1"), 1_700_000_000);
        assert!(!s.expired);
    }
}
