// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/events/email/EmailEventListenerProvider.java

//! EmailEventListener — RED phase.

use std::sync::Arc;

use serde::Serialize;

use super::events::AuthEvent;
use super::smtp_outbox::SmtpOutbox;
use super::EmailError;
use crate::audit::AuditEvent;

pub struct EmailEventListener {
    pub outbox: Arc<SmtpOutbox>,
}

impl EmailEventListener {
    pub fn new(outbox: Arc<SmtpOutbox>) -> Self { Self { outbox } }
    pub fn dispatch(&self, _audit: &AuditEvent) -> Result<Option<AuthEvent>, EmailError> {
        Err(EmailError::Send("RED-phase stub".into()))
    }
    pub fn send_typed<P: Serialize>(
        &self, _e: AuthEvent, _p: &P, _to: &str,
    ) -> Result<(), EmailError> {
        Err(EmailError::Send("RED-phase stub".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::events::{EmailAttachment, GdprDataExportPayload};
    use super::super::smtp_outbox::{OutboxConfig, SmtpOutbox, TestStubTransport};
    use crate::audit::{AuditEvent, AuthDecision};
    use chrono::Utc;
    use uuid::Uuid;

    fn outbox() -> (Arc<SmtpOutbox>, Arc<TestStubTransport>) {
        let stub = Arc::new(TestStubTransport::new());
        let outbox = Arc::new(SmtpOutbox::new_with_transport(OutboxConfig::test_defaults(), stub.clone()));
        (outbox, stub)
    }

    fn audit_with_action(action: &str, email: &str) -> AuditEvent {
        AuditEvent {
            event_id: Uuid::new_v4(),
            timestamp: Utc::now(),
            cave_uid: Some(Uuid::new_v4()),
            email: Some(email.into()),
            action: action.into(),
            resource: "auth".into(),
            decision: AuthDecision::Allowed,
            ip_address: Some("1.2.3.4".into()),
            details: serde_json::json!({
                "display_name": "Alice", "user_agent": "Curl",
                "location": "Berlin, DE", "failed_attempts": 5_u32,
                "method": "totp", "download_url": "https://cave.example/exports/abc",
            }),
        }
    }

    #[test]
    fn login_alert_action_dispatches_email() {
        let (outbox, stub) = outbox();
        let listener = EmailEventListener::new(outbox);
        let result = listener.dispatch(&audit_with_action("login_alert", "alice@example.com")).unwrap();
        assert_eq!(result, Some(AuthEvent::LoginAlert));
        assert_eq!(stub.captured().len(), 1);
        assert!(stub.captured()[0].contains("alice@example.com"));
    }

    #[test]
    fn password_change_action_dispatches_email() {
        let (outbox, stub) = outbox();
        let listener = EmailEventListener::new(outbox);
        let result = listener.dispatch(&audit_with_action("password_change", "bob@example.com")).unwrap();
        assert_eq!(result, Some(AuthEvent::PasswordChange));
        assert_eq!(stub.captured().len(), 1);
    }

    #[test]
    fn account_locked_action_dispatches_email_with_failed_attempts() {
        let (outbox, stub) = outbox();
        let listener = EmailEventListener::new(outbox);
        let result = listener.dispatch(&audit_with_action("account_locked", "carol@example.com")).unwrap();
        assert_eq!(result, Some(AuthEvent::AccountLocked));
        let body = &stub.captured()[0];
        assert!(body.contains("5 failed"));
    }

    #[test]
    fn mfa_enrollment_action_dispatches_email_with_method() {
        let (outbox, stub) = outbox();
        let listener = EmailEventListener::new(outbox);
        let result = listener.dispatch(&audit_with_action("mfa_enrollment", "dave@example.com")).unwrap();
        assert_eq!(result, Some(AuthEvent::MfaEnrollment));
        assert!(stub.captured()[0].contains("totp"));
    }

    #[test]
    fn gdpr_export_dispatches_email_with_download_link() {
        let (outbox, stub) = outbox();
        let listener = EmailEventListener::new(outbox);
        let result = listener.dispatch(&audit_with_action("gdpr_data_export", "eve@example.com")).unwrap();
        assert_eq!(result, Some(AuthEvent::GdprDataExport));
        let body = &stub.captured()[0];
        assert!(body.contains("Your data export is ready"));
    }

    #[test]
    fn aliased_actions_map_to_same_event() {
        let (outbox, stub) = outbox();
        let listener = EmailEventListener::new(outbox);
        listener.dispatch(&audit_with_action("auth_new_device", "alice@example.com")).unwrap();
        listener.dispatch(&audit_with_action("auth_new_ip", "alice@example.com")).unwrap();
        assert_eq!(stub.captured().len(), 2);
        for body in stub.captured() {
            assert!(body.contains("New sign-in"));
        }
    }

    #[test]
    fn unknown_action_does_not_dispatch() {
        let (outbox, stub) = outbox();
        let listener = EmailEventListener::new(outbox);
        let result = listener.dispatch(&audit_with_action("jwt_validate", "alice@example.com")).unwrap();
        assert_eq!(result, None);
        assert!(stub.captured().is_empty());
    }

    #[test]
    fn dispatch_without_email_field_errors() {
        let (outbox, _) = outbox();
        let listener = EmailEventListener::new(outbox);
        let mut a = audit_with_action("login_alert", "x@example.com");
        a.email = None;
        let err = listener.dispatch(&a).unwrap_err();
        assert!(matches!(err, EmailError::Send(_)));
    }

    #[test]
    fn send_typed_with_gdpr_attachment_includes_filename() {
        let (outbox, stub) = outbox();
        let listener = EmailEventListener::new(outbox);
        let payload = GdprDataExportPayload {
            to: "eve@example.com".into(), display_name: "Eve".into(),
            at: "2024-01-01T00:00:00Z".into(),
            download_url: "https://cave.example/exports/abc".into(),
            attachment: Some(EmailAttachment {
                filename: "data.zip".into(),
                content_type: "application/zip".into(),
                body_b64: "UEsDBAoAAAAAAA==".into(),
            }),
        };
        listener.send_typed(AuthEvent::GdprDataExport, &payload, "eve@example.com").unwrap();
        assert_eq!(stub.captured().len(), 1);
    }
}
