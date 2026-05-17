// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 themes/base/email/

//! Email templates — RED phase.

use handlebars::Handlebars;
use serde::Serialize;

use super::events::AuthEvent;
use super::EmailError;

pub struct Templates {
    _hb: Handlebars<'static>,
}

impl Default for Templates { fn default() -> Self { Self::new() } }

impl Templates {
    pub fn new() -> Self { Self { _hb: Handlebars::new() } }
    pub fn render_html<P: Serialize>(&self, _e: AuthEvent, _p: &P) -> Result<String, EmailError> {
        Err(EmailError::Render("RED-phase stub".into()))
    }
    pub fn render_text<P: Serialize>(&self, _e: AuthEvent, _p: &P) -> Result<String, EmailError> {
        Err(EmailError::Render("RED-phase stub".into()))
    }
    pub fn subject(&self, _e: AuthEvent) -> &'static str { "" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::events::*;

    fn login_payload() -> LoginAlertPayload {
        LoginAlertPayload {
            to: "alice@example.com".into(),
            display_name: "Alice".into(),
            ip_address: "1.2.3.4".into(),
            user_agent: "Curl/7".into(),
            location: "Berlin, DE".into(),
            at: "2024-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn login_alert_html_renders_payload_vars() {
        let t = Templates::default();
        let body = t.render_html(AuthEvent::LoginAlert, &login_payload()).unwrap();
        assert!(body.contains("Alice"));
        assert!(body.contains("Berlin, DE"));
        assert!(body.contains("1.2.3.4"));
        assert!(body.contains("<html>"));
    }

    #[test]
    fn login_alert_text_renders_payload_vars() {
        let t = Templates::default();
        let body = t.render_text(AuthEvent::LoginAlert, &login_payload()).unwrap();
        assert!(body.contains("Alice"));
        assert!(body.contains("Berlin, DE"));
        assert!(!body.contains("<html>"));
    }

    #[test]
    fn password_change_renders() {
        let t = Templates::default();
        let p = PasswordChangePayload { to: "bob@example.com".into(), display_name: "Bob".into(), at: "2024-02-01T12:00:00Z".into(), ip_address: "5.6.7.8".into() };
        let html = t.render_html(AuthEvent::PasswordChange, &p).unwrap();
        let text = t.render_text(AuthEvent::PasswordChange, &p).unwrap();
        assert!(html.contains("Bob"));
        assert!(text.contains("5.6.7.8"));
    }

    #[test]
    fn account_locked_handles_unlock_at_some() {
        let t = Templates::default();
        let p = AccountLockedPayload { to: "c@example.com".into(), display_name: "Carol".into(), at: "2024-03-01T10:00:00Z".into(), unlock_at: Some("2024-03-01T11:00:00Z".into()), failed_attempts: 5 };
        let body = t.render_html(AuthEvent::AccountLocked, &p).unwrap();
        assert!(body.contains("It will unlock automatically"));
        assert!(body.contains("2024-03-01T11:00:00Z"));
        assert!(body.contains("5 failed"));
    }

    #[test]
    fn account_locked_handles_unlock_at_none() {
        let t = Templates::default();
        let p = AccountLockedPayload { to: "c@example.com".into(), display_name: "Carol".into(), at: "2024-03-01T10:00:00Z".into(), unlock_at: None, failed_attempts: 7 };
        let body = t.render_html(AuthEvent::AccountLocked, &p).unwrap();
        assert!(body.contains("contact support to unlock"));
        assert!(!body.contains("It will unlock automatically"));
    }

    #[test]
    fn mfa_enrollment_renders_method() {
        let t = Templates::default();
        let p = MfaEnrollmentPayload { to: "d@example.com".into(), display_name: "Dave".into(), at: "2024-04-01T10:00:00Z".into(), method: "webauthn".into() };
        let body = t.render_html(AuthEvent::MfaEnrollment, &p).unwrap();
        assert!(body.contains("webauthn"));
    }

    #[test]
    fn gdpr_export_renders_download_url() {
        let t = Templates::default();
        let p = GdprDataExportPayload { to: "e@example.com".into(), display_name: "Eve".into(), at: "2024-05-01T10:00:00Z".into(), download_url: "https://cave.example/exports/abc123".into(), attachment: None };
        let body = t.render_html(AuthEvent::GdprDataExport, &p).unwrap();
        assert!(body.contains("https://cave.example/exports/abc123"));
    }

    #[test]
    fn subjects_are_distinct_and_nonempty() {
        let t = Templates::default();
        for ev in AuthEvent::all() {
            let s = t.subject(*ev);
            assert!(!s.is_empty(), "subject empty for {:?}", ev);
        }
        let mut subjects: Vec<&'static str> = AuthEvent::all().iter().map(|ev| t.subject(*ev)).collect();
        subjects.sort();
        let dedup_len = { let mut s = subjects.clone(); s.dedup(); s.len() };
        assert_eq!(dedup_len, subjects.len());
    }

    #[test]
    fn strict_mode_errors_on_missing_variable() {
        let t = Templates::default();
        #[derive(Serialize)]
        struct Empty {}
        let err = t.render_html(AuthEvent::LoginAlert, &Empty {}).unwrap_err();
        assert!(matches!(err, EmailError::Render(_)));
    }
}
