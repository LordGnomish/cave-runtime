// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 federation/ldap/src/main/java/org/keycloak/storage/ldap/mappers/msad/MSADUserAccountControlStorageMapper.java
// Source: keycloak/keycloak@b825ba97 federation/ldap/src/main/java/org/keycloak/storage/ldap/idm/model/LDAPDn.java (objectGUID handling)
//
// Active-Directory-specific quirks.
//
//   * `objectGUID`         — 16-byte binary GUID with a Microsoft
//                            byte-swap on the first three groups
//                            (Data1/Data2/Data3).  We expose two
//                            functions: the "endian-aware" canonical
//                            form (matches MS-DTYP §2.3.4.2) and the
//                            "raw" form (just hex-dump of the bytes).
//   * `userAccountControl` — bitmask documented in MS-ADTS §2.2.16.
//   * `pwdLastSet`         — FILETIME (100-ns ticks since 1601).
//   * `accountExpires`     — FILETIME, 0 or i64::MAX = never.

/// `userAccountControl` bit flags.  Verbatim from MS-ADTS §2.2.16
/// and Keycloak's `UserAccountControl` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum UacFlag {
    Script = 0x0001,
    AccountDisable = 0x0002,
    HomeDirRequired = 0x0008,
    Lockout = 0x0010,
    PasswordNotRequired = 0x0020,
    PasswordCantChange = 0x0040,
    EncryptedTextPwdAllowed = 0x0080,
    TempDuplicateAccount = 0x0100,
    NormalAccount = 0x0200,
    InterDomainTrustAccount = 0x0800,
    WorkstationTrustAccount = 0x1000,
    ServerTrustAccount = 0x2000,
    DontExpirePassword = 0x10000,
    MnsLogonAccount = 0x20000,
    SmartcardRequired = 0x40000,
    TrustedForDelegation = 0x80000,
    NotDelegated = 0x100000,
    UseDesKeyOnly = 0x200000,
    DontReqPreauth = 0x400000,
    PasswordExpired = 0x800000,
    TrustedToAuthForDelegation = 0x1000000,
    PartialSecretsAccount = 0x4000000,
}

impl UacFlag {
    pub fn is_set(self, bits: u32) -> bool {
        bits & self as u32 != 0
    }
}

/// Pretty-print an `objectGUID`.  Input is the 16 raw bytes as stored
/// in AD; output is the canonical hyphenated form (MS-DTYP §2.3.4.2)
/// with the first three groups byte-swapped.
pub fn object_guid_to_string(bytes: &[u8]) -> String {
    if bytes.len() != 16 {
        // Fallback: hex-dump whatever we got.  Caller should already
        // have filtered length, but this keeps the API total.
        return bytes.iter().map(|b| format!("{b:02x}")).collect();
    }
    let d1 = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let d2 = u16::from_le_bytes([bytes[4], bytes[5]]);
    let d3 = u16::from_le_bytes([bytes[6], bytes[7]]);
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        d1, d2, d3, bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

/// FILETIME → Unix seconds.  FILETIME counts 100-ns ticks since
/// 1601-01-01.  `0` means "never set"; `i64::MAX` means "never
/// expires".  Returns `None` for those sentinels.
pub fn filetime_to_unix_seconds(filetime: i64) -> Option<i64> {
    if filetime == 0 || filetime == i64::MAX {
        return None;
    }
    // 11644473600 = seconds between 1601-01-01 and 1970-01-01.
    let unix = (filetime / 10_000_000) - 11_644_473_600;
    Some(unix)
}

/// MS-ADTS UAC summary used by the portal "Account state" badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountState {
    pub disabled: bool,
    pub locked: bool,
    pub password_expired: bool,
    pub password_never_expires: bool,
    pub smartcard_required: bool,
}

impl AccountState {
    pub fn from_uac(bits: u32) -> Self {
        Self {
            disabled: UacFlag::AccountDisable.is_set(bits),
            locked: UacFlag::Lockout.is_set(bits),
            password_expired: UacFlag::PasswordExpired.is_set(bits),
            password_never_expires: UacFlag::DontExpirePassword.is_set(bits),
            smartcard_required: UacFlag::SmartcardRequired.is_set(bits),
        }
    }

    /// Account-locked-out check.  AD uses both UAC and a separate
    /// `lockoutTime` attribute; Keycloak's logic is: account is
    /// locked if `lockoutTime` is non-zero AND not in the future
    /// (i.e. lockout duration has not yet elapsed).
    pub fn locked_out(uac: u32, lockout_time_filetime: i64, now_unix: i64, lockout_duration_secs: i64) -> bool {
        if UacFlag::Lockout.is_set(uac) {
            return true;
        }
        match filetime_to_unix_seconds(lockout_time_filetime) {
            None => false,
            Some(t) => (now_unix - t) < lockout_duration_secs,
        }
    }
}

/// `msDS-User-Account-Disabled` (different from UAC's bit).
/// Optional attribute introduced in Server 2008 for forest
/// trusts — when present and `TRUE` the account is disabled.
pub fn is_msds_disabled(value: Option<&str>) -> bool {
    matches!(value, Some(v) if v.eq_ignore_ascii_case("TRUE"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_guid_canonical_byte_swap() {
        // From MS-DTYP §2.3.4.2 worked example.  Raw bytes
        // 78 56 34 12 34 12 78 56 ab cd ef 01 23 45 67 89 →
        // canonical 12345678-1234-5678-abcd-ef0123456789.
        let bytes: Vec<u8> = vec![
            0x78, 0x56, 0x34, 0x12, 0x34, 0x12, 0x78, 0x56, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
        ];
        assert_eq!(object_guid_to_string(&bytes), "12345678-1234-5678-abcd-ef0123456789");
    }

    #[test]
    fn object_guid_short_input_does_not_panic() {
        assert_eq!(object_guid_to_string(&[]), "");
        assert_eq!(object_guid_to_string(&[0xff]), "ff");
    }

    #[test]
    fn uac_disabled_bit_detected() {
        let bits = 0x0202; // NORMAL_ACCOUNT | ACCOUNT_DISABLE
        let s = AccountState::from_uac(bits);
        assert!(s.disabled);
        assert!(!s.password_expired);
    }

    #[test]
    fn uac_password_never_expires_detected() {
        let bits = 0x10200; // NORMAL_ACCOUNT | DONT_EXPIRE_PASSWORD
        let s = AccountState::from_uac(bits);
        assert!(s.password_never_expires);
    }

    #[test]
    fn filetime_zero_and_max_are_none() {
        assert!(filetime_to_unix_seconds(0).is_none());
        assert!(filetime_to_unix_seconds(i64::MAX).is_none());
    }

    #[test]
    fn filetime_converts_known_value() {
        // 2025-01-01 00:00:00 UTC = unix 1735689600 = filetime
        // 133800768000000000 (1735689600 + 11644473600 = 13380163200,
        // then * 10_000_000).
        let unix = 1_735_689_600_i64;
        let filetime = (unix + 11_644_473_600) * 10_000_000;
        assert_eq!(filetime_to_unix_seconds(filetime), Some(unix));
    }

    #[test]
    fn locked_out_when_uac_lockout_bit_set() {
        assert!(AccountState::locked_out(0x0210, 0, 0, 600));
    }

    #[test]
    fn locked_out_when_recent_lockout_time_and_duration_not_elapsed() {
        let now = 1_735_689_700_i64; // 100s after lock
        let lockout_unix = 1_735_689_600_i64;
        let filetime = (lockout_unix + 11_644_473_600) * 10_000_000;
        assert!(AccountState::locked_out(0x0200, filetime, now, 600));
        // After duration elapsed:
        assert!(!AccountState::locked_out(0x0200, filetime, now + 600, 600));
    }

    #[test]
    fn msds_disabled_string_handling() {
        assert!(is_msds_disabled(Some("TRUE")));
        assert!(is_msds_disabled(Some("true")));
        assert!(!is_msds_disabled(Some("FALSE")));
        assert!(!is_msds_disabled(None));
    }
}
