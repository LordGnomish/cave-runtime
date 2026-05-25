// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/events/EventType.java

//! Event types the email listener cares about, with structured per-event
//! payloads.

use serde::{Deserialize, Serialize};

/// Authentication event kinds that trigger a transactional email.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthEvent {
    /// New device or new IP signed in.
    LoginAlert,
    /// User changed their password.
    PasswordChange,
    /// Account hit lock-out threshold.
    AccountLocked,
    /// MFA enrolment completed.
    MfaEnrollment,
    /// GDPR data export ready for download.
    GdprDataExport,
}

impl AuthEvent {
    /// Stable string identifier (used as the handlebars template key).
    pub fn as_key(self) -> &'static str {
        match self {
            AuthEvent::LoginAlert => "login_alert",
            AuthEvent::PasswordChange => "password_change",
            AuthEvent::AccountLocked => "account_locked",
            AuthEvent::MfaEnrollment => "mfa_enrollment",
            AuthEvent::GdprDataExport => "gdpr_data_export",
        }
    }

    /// All variants — used by the template registry to install bodies.
    pub fn all() -> &'static [AuthEvent] {
        &[
            AuthEvent::LoginAlert,
            AuthEvent::PasswordChange,
            AuthEvent::AccountLocked,
            AuthEvent::MfaEnrollment,
            AuthEvent::GdprDataExport,
        ]
    }

    /// Reverse lookup.
    pub fn from_key(s: &str) -> Option<Self> {
        match s {
            "login_alert" => Some(AuthEvent::LoginAlert),
            "password_change" => Some(AuthEvent::PasswordChange),
            "account_locked" => Some(AuthEvent::AccountLocked),
            "mfa_enrollment" => Some(AuthEvent::MfaEnrollment),
            "gdpr_data_export" => Some(AuthEvent::GdprDataExport),
            _ => None,
        }
    }
}

/// Payload accompanying a [`AuthEvent::LoginAlert`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginAlertPayload {
    /// Email of the recipient.
    pub to: String,
    /// Friendly display name.
    pub display_name: String,
    /// IP that triggered the alert.
    pub ip_address: String,
    /// Best-effort user agent string.
    pub user_agent: String,
    /// Best-effort geo location label (e.g. "Berlin, DE").
    pub location: String,
    /// Login timestamp (RFC3339).
    pub at: String,
}

/// Payload for [`AuthEvent::PasswordChange`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasswordChangePayload {
    pub to: String,
    pub display_name: String,
    pub at: String,
    /// IP that performed the change.
    pub ip_address: String,
}

/// Payload for [`AuthEvent::AccountLocked`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountLockedPayload {
    pub to: String,
    pub display_name: String,
    pub at: String,
    /// Auto-unlock instant (RFC3339), or `None` if locked indefinitely.
    pub unlock_at: Option<String>,
    /// Number of failed attempts that triggered the lock.
    pub failed_attempts: u32,
}

/// Payload for [`AuthEvent::MfaEnrollment`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MfaEnrollmentPayload {
    pub to: String,
    pub display_name: String,
    pub at: String,
    /// `totp` | `webauthn` | `sms` | etc.
    pub method: String,
}

/// Payload for [`AuthEvent::GdprDataExport`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GdprDataExportPayload {
    pub to: String,
    pub display_name: String,
    pub at: String,
    /// Pre-signed download URL.
    pub download_url: String,
    /// Optional in-line attachment (filename + bytes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<EmailAttachment>,
}

/// Email attachment (filename + content-type + bytes).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailAttachment {
    pub filename: String,
    pub content_type: String,
    /// Base64-encoded body.
    pub body_b64: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_event_keys_roundtrip() {
        for ev in AuthEvent::all() {
            assert_eq!(AuthEvent::from_key(ev.as_key()), Some(*ev));
        }
    }

    #[test]
    fn auth_event_keys_are_stable_strings() {
        assert_eq!(AuthEvent::LoginAlert.as_key(), "login_alert");
        assert_eq!(AuthEvent::PasswordChange.as_key(), "password_change");
        assert_eq!(AuthEvent::AccountLocked.as_key(), "account_locked");
        assert_eq!(AuthEvent::MfaEnrollment.as_key(), "mfa_enrollment");
        assert_eq!(AuthEvent::GdprDataExport.as_key(), "gdpr_data_export");
    }

    #[test]
    fn unknown_key_returns_none() {
        assert!(AuthEvent::from_key("nothing").is_none());
        assert!(AuthEvent::from_key("").is_none());
    }

    #[test]
    fn payload_roundtrip_login_alert() {
        let p = LoginAlertPayload {
            to: "alice@example.com".into(),
            display_name: "Alice".into(),
            ip_address: "203.0.113.5".into(),
            user_agent: "Mozilla/5.0".into(),
            location: "Berlin, DE".into(),
            at: "2024-01-01T00:00:00Z".into(),
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: LoginAlertPayload = serde_json::from_str(&s).unwrap();
        assert_eq!(back.to, p.to);
    }
}
