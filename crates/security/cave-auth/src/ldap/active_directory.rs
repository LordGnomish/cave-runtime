// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 federation/ldap/src/main/java/org/keycloak/storage/ldap/mappers/msad/MSADUserAccountControlStorageMapper.java + microsoft AD schema docs

//! Active Directory-specific attribute helpers. Three pieces
//! Keycloak's MSAD mapper covers:
//!
//! * `userAccountControl` — bitfield (Microsoft "UAC") with the
//!   disabled / locked / password-expired / no-expire flags.
//! * `objectSid` — binary security identifier. AD returns a
//!   compact 28-byte structure; we parse it back to the
//!   canonical `S-1-5-21-...` string form.
//! * `pwdLastSet` — Windows FILETIME (100-ns ticks since
//!   1601-01-01). Convert to `chrono::DateTime<Utc>` so the
//!   admin page can render "password set 7 days ago".

/// `userAccountControl` flags. Names from `MS-ADTS` 2.2.16.
/// Only the flags Keycloak's `MSADUserAccountControlStorageMapper`
/// branches on are enumerated — the full bitfield is `u32`.
#[repr(u32)]
pub enum UacFlag {
    Script = 0x0001,
    AccountDisable = 0x0002,
    HomedirRequired = 0x0008,
    Lockout = 0x0010,
    PasswordNotRequired = 0x0020,
    PasswordCantChange = 0x0040,
    NormalAccount = 0x0200,
    DontExpirePassword = 0x10000,
    PasswordExpired = 0x800000,
}

/// Decoded UAC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserAccountControl(pub u32);

impl UserAccountControl {
    /// Parse from the LDAP string form (`"514"` etc).
    pub fn parse(s: &str) -> Option<Self> {
        s.trim().parse::<u32>().ok().map(UserAccountControl)
    }
    pub fn is_set(self, f: UacFlag) -> bool {
        self.0 & f as u32 != 0
    }
    /// Account is disabled (`AccountDisable` flag).
    pub fn is_disabled(self) -> bool {
        self.is_set(UacFlag::AccountDisable)
    }
    /// Account is locked-out (`Lockout` flag).
    pub fn is_locked(self) -> bool {
        self.is_set(UacFlag::Lockout)
    }
    /// Password is flagged expired by the AD server.
    pub fn password_expired(self) -> bool {
        self.is_set(UacFlag::PasswordExpired)
    }
    /// Password is set to never expire.
    pub fn never_expires(self) -> bool {
        self.is_set(UacFlag::DontExpirePassword)
    }
}

/// Parse a binary AD `objectSid` blob into its canonical
/// string form `S-<revision>-<authority>-<subauth>-...`. Follows
/// the `MS-DTYP` 2.4.2 layout:
/// ```text
/// SID := Revision(1) SubAuthorityCount(1) IdentifierAuthority(6) SubAuthority(4)*N
/// ```
/// Returns `None` if the buffer is too short.
pub fn parse_object_sid(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 8 {
        return None;
    }
    let revision = bytes[0];
    let sub_count = bytes[1] as usize;
    let expected_len = 8 + 4 * sub_count;
    if bytes.len() < expected_len {
        return None;
    }
    // Authority is a 6-byte big-endian integer.
    let mut authority: u64 = 0;
    for b in &bytes[2..8] {
        authority = (authority << 8) | *b as u64;
    }
    let mut out = format!("S-{}-{}", revision, authority);
    for i in 0..sub_count {
        let off = 8 + i * 4;
        let sub = u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        out.push('-');
        out.push_str(&sub.to_string());
    }
    Some(out)
}

/// Parse a Windows FILETIME string (e.g. `"132514689000000000"`)
/// to a unix epoch (seconds). FILETIME is 100-ns ticks since
/// 1601-01-01 UTC; unix epoch is 1970-01-01 UTC, so the offset
/// in seconds is `11644473600`.
pub fn pwd_last_set_to_unix_epoch(filetime_str: &str) -> Option<i64> {
    let ticks: i64 = filetime_str.trim().parse().ok()?;
    if ticks == 0 {
        // 0 in AD means "user must change password at next
        // logon" — we report Unix epoch 0 (1970) to make the
        // semantic obvious.
        return Some(0);
    }
    let unix_offset_seconds: i64 = 11_644_473_600;
    Some(ticks / 10_000_000 - unix_offset_seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uac_normal_account_disabled_flag() {
        // 514 = NORMAL_ACCOUNT | ACCOUNTDISABLE
        let uac = UserAccountControl::parse("514").unwrap();
        assert!(uac.is_set(UacFlag::NormalAccount));
        assert!(uac.is_disabled());
    }

    #[test]
    fn uac_normal_account_enabled() {
        let uac = UserAccountControl::parse("512").unwrap();
        assert!(uac.is_set(UacFlag::NormalAccount));
        assert!(!uac.is_disabled());
    }

    #[test]
    fn uac_dont_expire_flag() {
        // 66048 = NORMAL_ACCOUNT | DONT_EXPIRE_PASSWORD
        let uac = UserAccountControl::parse("66048").unwrap();
        assert!(uac.never_expires());
        assert!(!uac.password_expired());
    }

    #[test]
    fn uac_password_expired_flag() {
        let uac = UserAccountControl(0x800000 | 0x200);
        assert!(uac.password_expired());
    }

    #[test]
    fn uac_parse_rejects_garbage() {
        assert!(UserAccountControl::parse("abc").is_none());
    }

    #[test]
    fn parse_object_sid_handles_well_known() {
        // S-1-5-32-544 (Builtin\Administrators) on the wire:
        // rev=01, count=02, auth=00 00 00 00 00 05 (NT auth.),
        // sub1=32 (LE: 20 00 00 00), sub2=544 (LE: 20 02 00 00).
        let bytes = [
            0x01, 0x02, // revision, sub-count
            0x00, 0x00, 0x00, 0x00, 0x00, 0x05, // authority = 5
            0x20, 0x00, 0x00, 0x00, // sub-auth 32
            0x20, 0x02, 0x00, 0x00, // sub-auth 544
        ];
        let sid = parse_object_sid(&bytes).unwrap();
        assert_eq!(sid, "S-1-5-32-544");
    }

    #[test]
    fn parse_object_sid_handles_domain_sid() {
        // S-1-5-21-A-B-C-RID — typical AD user SID
        let bytes = [
            0x01, 0x05, // revision, sub-count (5)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x05, // authority = 5
            0x15, 0x00, 0x00, 0x00, // 21
            0x01, 0x00, 0x00, 0x00, // 1
            0x02, 0x00, 0x00, 0x00, // 2
            0x03, 0x00, 0x00, 0x00, // 3
            0xF4, 0x01, 0x00, 0x00, // 500
        ];
        let sid = parse_object_sid(&bytes).unwrap();
        assert_eq!(sid, "S-1-5-21-1-2-3-500");
    }

    #[test]
    fn parse_object_sid_returns_none_for_short_buffer() {
        assert!(parse_object_sid(&[0x01, 0x02, 0x00]).is_none());
    }

    #[test]
    fn pwd_last_set_zero_means_unix_epoch() {
        assert_eq!(pwd_last_set_to_unix_epoch("0"), Some(0));
    }

    #[test]
    fn pwd_last_set_translates_filetime_to_unix() {
        // FILETIME ticks since 1601-01-01 / 10^7 - 11644473600
        // = unix epoch seconds. 132540480000000000 ticks
        // resolves to 2021-01-02 00:00:00 UTC.
        let ts = pwd_last_set_to_unix_epoch("132540480000000000").unwrap();
        assert_eq!(ts, 1_609_574_400);
    }

    #[test]
    fn pwd_last_set_rejects_garbage() {
        assert!(pwd_last_set_to_unix_epoch("not-a-number").is_none());
    }
}
