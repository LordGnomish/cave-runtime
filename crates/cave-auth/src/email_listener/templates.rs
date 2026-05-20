// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 themes/base/email/

//! Handlebars-rendered email bodies for the five auth events. Plain-text
//! and HTML are registered per event under stable keys
//! `{event}_html` / `{event}_text`.

use handlebars::Handlebars;
use serde::Serialize;

use super::EmailError;
use super::events::AuthEvent;

/// Registry of compiled templates for the five auth events. One renderer
/// per cave-auth instance — cheap to clone.
pub struct Templates {
    hb: Handlebars<'static>,
}

impl Default for Templates {
    fn default() -> Self {
        Self::new()
    }
}

impl Templates {
    /// Install bodies for every event type.
    pub fn new() -> Self {
        let mut hb = Handlebars::new();
        hb.set_strict_mode(true);
        for ev in AuthEvent::all() {
            let (html, text) = default_bodies(*ev);
            hb.register_template_string(&format!("{}_html", ev.as_key()), html)
                .unwrap();
            hb.register_template_string(&format!("{}_text", ev.as_key()), text)
                .unwrap();
        }
        Self { hb }
    }

    /// Render the HTML body for `event`.
    pub fn render_html<P: Serialize>(
        &self,
        event: AuthEvent,
        payload: &P,
    ) -> Result<String, EmailError> {
        self.hb
            .render(&format!("{}_html", event.as_key()), payload)
            .map_err(|e| EmailError::Render(format!("{e}")))
    }

    /// Render the plain-text body for `event`.
    pub fn render_text<P: Serialize>(
        &self,
        event: AuthEvent,
        payload: &P,
    ) -> Result<String, EmailError> {
        self.hb
            .render(&format!("{}_text", event.as_key()), payload)
            .map_err(|e| EmailError::Render(format!("{e}")))
    }

    /// Render the subject line for `event`. Subjects are static so we
    /// don't need handlebars — but expose the same `render_*` shape.
    pub fn subject(&self, event: AuthEvent) -> &'static str {
        match event {
            AuthEvent::LoginAlert => "New sign-in to your account",
            AuthEvent::PasswordChange => "Your password was changed",
            AuthEvent::AccountLocked => "Your account is locked",
            AuthEvent::MfaEnrollment => "Two-factor enabled on your account",
            AuthEvent::GdprDataExport => "Your data export is ready",
        }
    }
}

/// Default in-source bodies. Kept in code so the build doesn't require a
/// templates/ directory at runtime.
fn default_bodies(ev: AuthEvent) -> (&'static str, &'static str) {
    match ev {
        AuthEvent::LoginAlert => (
            r#"<html><body><p>Hi {{display_name}},</p>
<p>We saw a new sign-in to your account from <strong>{{location}}</strong>
(IP {{ip_address}}, {{user_agent}}) at {{at}}.</p>
<p>If this was you, no action needed. If not, please change your password.</p>
</body></html>"#,
            r#"Hi {{display_name}},

We saw a new sign-in to your account from {{location}}
(IP {{ip_address}}, {{user_agent}}) at {{at}}.

If this was you, no action needed. If not, please change your password.
"#,
        ),
        AuthEvent::PasswordChange => (
            r#"<html><body><p>Hi {{display_name}},</p>
<p>Your account password was changed at {{at}} from IP {{ip_address}}.</p>
<p>If this wasn't you, contact support immediately.</p>
</body></html>"#,
            r#"Hi {{display_name}},

Your account password was changed at {{at}} from IP {{ip_address}}.

If this wasn't you, contact support immediately.
"#,
        ),
        AuthEvent::AccountLocked => (
            r#"<html><body><p>Hi {{display_name}},</p>
<p>Your account has been temporarily locked after {{failed_attempts}} failed sign-in attempts.</p>
<p>{{#if unlock_at}}It will unlock automatically at {{unlock_at}}.{{else}}Please contact support to unlock it.{{/if}}</p>
</body></html>"#,
            r#"Hi {{display_name}},

Your account has been temporarily locked after {{failed_attempts}} failed sign-in attempts.

{{#if unlock_at}}It will unlock automatically at {{unlock_at}}.{{else}}Please contact support to unlock it.{{/if}}
"#,
        ),
        AuthEvent::MfaEnrollment => (
            r#"<html><body><p>Hi {{display_name}},</p>
<p>Two-factor authentication ({{method}}) was enabled on your account at {{at}}.</p>
<p>If this wasn't you, contact support immediately.</p>
</body></html>"#,
            r#"Hi {{display_name}},

Two-factor authentication ({{method}}) was enabled on your account at {{at}}.

If this wasn't you, contact support immediately.
"#,
        ),
        AuthEvent::GdprDataExport => (
            r#"<html><body><p>Hi {{display_name}},</p>
<p>Your data export is ready as of {{at}}.</p>
<p><a href="{{download_url}}">Download it here</a> — the link is valid for 7 days.</p>
</body></html>"#,
            r#"Hi {{display_name}},

Your data export is ready as of {{at}}.

Download it here: {{download_url}}
The link is valid for 7 days.
"#,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::super::events::*;
    use super::*;

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
        let body = t
            .render_html(AuthEvent::LoginAlert, &login_payload())
            .unwrap();
        assert!(body.contains("Alice"));
        assert!(body.contains("Berlin, DE"));
        assert!(body.contains("1.2.3.4"));
        assert!(body.contains("<html>"));
    }

    #[test]
    fn login_alert_text_renders_payload_vars() {
        let t = Templates::default();
        let body = t
            .render_text(AuthEvent::LoginAlert, &login_payload())
            .unwrap();
        assert!(body.contains("Alice"));
        assert!(body.contains("Berlin, DE"));
        assert!(!body.contains("<html>"));
    }

    #[test]
    fn password_change_renders() {
        let t = Templates::default();
        let p = PasswordChangePayload {
            to: "bob@example.com".into(),
            display_name: "Bob".into(),
            at: "2024-02-01T12:00:00Z".into(),
            ip_address: "5.6.7.8".into(),
        };
        let html = t.render_html(AuthEvent::PasswordChange, &p).unwrap();
        let text = t.render_text(AuthEvent::PasswordChange, &p).unwrap();
        assert!(html.contains("Bob"));
        assert!(text.contains("5.6.7.8"));
    }

    #[test]
    fn account_locked_handles_unlock_at_some() {
        let t = Templates::default();
        let p = AccountLockedPayload {
            to: "c@example.com".into(),
            display_name: "Carol".into(),
            at: "2024-03-01T10:00:00Z".into(),
            unlock_at: Some("2024-03-01T11:00:00Z".into()),
            failed_attempts: 5,
        };
        let body = t.render_html(AuthEvent::AccountLocked, &p).unwrap();
        assert!(body.contains("It will unlock automatically"));
        assert!(body.contains("2024-03-01T11:00:00Z"));
        assert!(body.contains("5 failed"));
    }

    #[test]
    fn account_locked_handles_unlock_at_none() {
        let t = Templates::default();
        let p = AccountLockedPayload {
            to: "c@example.com".into(),
            display_name: "Carol".into(),
            at: "2024-03-01T10:00:00Z".into(),
            unlock_at: None,
            failed_attempts: 7,
        };
        let body = t.render_html(AuthEvent::AccountLocked, &p).unwrap();
        assert!(body.contains("contact support to unlock"));
        assert!(!body.contains("It will unlock automatically"));
    }

    #[test]
    fn mfa_enrollment_renders_method() {
        let t = Templates::default();
        let p = MfaEnrollmentPayload {
            to: "d@example.com".into(),
            display_name: "Dave".into(),
            at: "2024-04-01T10:00:00Z".into(),
            method: "webauthn".into(),
        };
        let body = t.render_html(AuthEvent::MfaEnrollment, &p).unwrap();
        assert!(body.contains("webauthn"));
    }

    #[test]
    fn gdpr_export_renders_download_url() {
        let t = Templates::default();
        let p = GdprDataExportPayload {
            to: "e@example.com".into(),
            display_name: "Eve".into(),
            at: "2024-05-01T10:00:00Z".into(),
            download_url: "https://cave.example/exports/abc123".into(),
            attachment: None,
        };
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
        // Distinct
        let mut subjects: Vec<&'static str> =
            AuthEvent::all().iter().map(|ev| t.subject(*ev)).collect();
        subjects.sort();
        let dedup_len = {
            let mut s = subjects.clone();
            s.dedup();
            s.len()
        };
        assert_eq!(dedup_len, subjects.len());
    }

    #[test]
    fn strict_mode_errors_on_missing_variable() {
        let t = Templates::default();
        // Missing required fields => render_html should fail.
        #[derive(Serialize)]
        struct Empty {}
        let err = t.render_html(AuthEvent::LoginAlert, &Empty {}).unwrap_err();
        assert!(matches!(err, EmailError::Render(_)));
    }
}
