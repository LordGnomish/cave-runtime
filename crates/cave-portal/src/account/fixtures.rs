// SPDX-License-Identifier: AGPL-3.0-or-later
//! Per-principal seeded fixtures for the account console.
//!
//! Source shape: keycloak/keycloak@b825ba97
//! `js/apps/account-ui/src/api/representations/*`. Each struct is the
//! minimal subset our six views consume. Fixtures are derived
//! deterministically from the principal string so tests can rely on
//! stable counts without holding mutable state.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalInfo {
    pub username: String,
    pub email: String,
    pub email_verified: bool,
    pub first_name: String,
    pub last_name: String,
    /// `attribute_name -> value`. Free-form realm attributes
    /// (Keycloak `UserRepresentation.attributes`).
    pub attributes: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialEntry {
    /// `password` / `otp` / `webauthn` / `webauthn-passwordless`.
    pub kind: String,
    /// User-visible label ("Authenticator app", "YubiKey 5C", …).
    pub label: String,
    /// Truncated id (Keycloak `CredentialRepresentation.id`).
    pub credential_id: String,
    pub created_unix: i64,
    pub removable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceSession {
    pub session_id: String,
    pub ip: String,
    pub browser: String,
    pub os: String,
    pub last_access_unix: i64,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedApplication {
    pub client_id: String,
    pub client_name: String,
    /// Granted OIDC scopes. Matches `consentScopes` in upstream.
    pub scopes: Vec<String>,
    pub last_used_unix: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedAccount {
    /// Identity provider alias (e.g. `github`, `google`, `azure`).
    pub provider_alias: String,
    pub provider_name: String,
    pub linked_username: String,
    pub linked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMembership {
    /// Fully-qualified group path (Keycloak `GroupRepresentation.path`).
    pub path: String,
    pub direct: bool,
}

/// Deterministic fixture set keyed by principal. The point isn't
/// random data — it's that two reads return the same shape, the
/// counts are documented, and the page tests don't have to mock.
pub fn personal_info(principal: &str) -> PersonalInfo {
    let (first, last) = split_name(principal);
    PersonalInfo {
        username: principal.to_string(),
        email: principal.to_string(),
        email_verified: true,
        first_name: first,
        last_name: last,
        attributes: vec![
            ("locale".into(), "en".into()),
            ("phoneNumber".into(), "".into()),
        ],
    }
}

pub fn credentials(principal: &str) -> Vec<CredentialEntry> {
    let _ = principal;
    vec![
        CredentialEntry {
            kind: "password".into(),
            label: "My password".into(),
            credential_id: "cred-pwd-1".into(),
            created_unix: 1_700_000_000,
            removable: false,
        },
        CredentialEntry {
            kind: "otp".into(),
            label: "Authenticator app".into(),
            credential_id: "cred-otp-1".into(),
            created_unix: 1_700_100_000,
            removable: true,
        },
        CredentialEntry {
            kind: "webauthn-passwordless".into(),
            label: "YubiKey 5C".into(),
            credential_id: "cred-wak-1".into(),
            created_unix: 1_700_200_000,
            removable: true,
        },
    ]
}

pub fn devices(principal: &str) -> Vec<DeviceSession> {
    let _ = principal;
    vec![
        DeviceSession {
            session_id: "sess-current".into(),
            ip: "10.0.0.42".into(),
            browser: "Firefox 122".into(),
            os: "macOS 14".into(),
            last_access_unix: 1_700_500_000,
            current: true,
        },
        DeviceSession {
            session_id: "sess-mobile".into(),
            ip: "10.0.0.43".into(),
            browser: "Safari 17".into(),
            os: "iOS 17".into(),
            last_access_unix: 1_700_400_000,
            current: false,
        },
    ]
}

pub fn applications(principal: &str) -> Vec<LinkedApplication> {
    let _ = principal;
    vec![
        LinkedApplication {
            client_id: "cave-portal".into(),
            client_name: "Cave Portal".into(),
            scopes: vec!["openid".into(), "profile".into(), "email".into()],
            last_used_unix: 1_700_500_000,
        },
        LinkedApplication {
            client_id: "cave-cli".into(),
            client_name: "cavectl".into(),
            scopes: vec!["openid".into(), "offline_access".into()],
            last_used_unix: 1_700_490_000,
        },
    ]
}

pub fn linked_accounts(principal: &str) -> Vec<LinkedAccount> {
    let _ = principal;
    vec![
        LinkedAccount {
            provider_alias: "github".into(),
            provider_name: "GitHub".into(),
            linked_username: principal.split('@').next().unwrap_or(principal).into(),
            linked: true,
        },
        LinkedAccount {
            provider_alias: "google".into(),
            provider_name: "Google".into(),
            linked_username: String::new(),
            linked: false,
        },
    ]
}

pub fn group_memberships(principal: &str) -> Vec<GroupMembership> {
    let domain = principal.split('@').nth(1).unwrap_or("cave");
    vec![
        GroupMembership { path: format!("/{}/engineering", domain), direct: true },
        GroupMembership { path: format!("/{}/employees", domain), direct: false },
    ]
}

fn split_name(principal: &str) -> (String, String) {
    let local = principal.split('@').next().unwrap_or(principal);
    let mut parts = local.split(['.', '-', '_']);
    let first = parts.next().unwrap_or("").to_string();
    let last = parts.next().unwrap_or("").to_string();
    (capitalise(&first), capitalise(&last))
}

fn capitalise(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().chain(chars).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn personal_info_splits_dotted_principal() {
        let p = personal_info("alice.smith@acme");
        assert_eq!(p.first_name, "Alice");
        assert_eq!(p.last_name, "Smith");
        assert_eq!(p.email, "alice.smith@acme");
        assert!(p.email_verified);
    }

    #[test]
    fn personal_info_handles_unsplittable_principal() {
        let p = personal_info("bare");
        assert_eq!(p.first_name, "Bare");
        assert_eq!(p.last_name, "");
    }

    #[test]
    fn credentials_returns_password_otp_webauthn_set() {
        let c = credentials("alice");
        assert_eq!(c.len(), 3);
        assert!(c.iter().any(|e| e.kind == "password"));
        assert!(c.iter().any(|e| e.kind == "otp"));
        assert!(c.iter().any(|e| e.kind == "webauthn-passwordless"));
        // Password is never removable from the account console.
        assert!(c.iter().find(|e| e.kind == "password").unwrap().removable == false);
    }

    #[test]
    fn devices_marks_one_session_as_current() {
        let d = devices("alice");
        assert_eq!(d.len(), 2);
        let current_count = d.iter().filter(|x| x.current).count();
        assert_eq!(current_count, 1);
    }

    #[test]
    fn applications_includes_cave_portal_and_cli() {
        let a = applications("alice");
        assert!(a.iter().any(|x| x.client_id == "cave-portal"));
        assert!(a.iter().any(|x| x.client_id == "cave-cli"));
    }

    #[test]
    fn linked_accounts_mixes_linked_and_unlinked() {
        let l = linked_accounts("alice@acme");
        assert!(l.iter().any(|x| x.linked));
        assert!(l.iter().any(|x| !x.linked));
    }

    #[test]
    fn group_memberships_derives_path_from_domain() {
        let g = group_memberships("alice@acme");
        assert!(g.iter().any(|x| x.path.starts_with("/acme/")));
        assert!(g.iter().any(|x| x.direct));
        assert!(g.iter().any(|x| !x.direct));
    }
}
