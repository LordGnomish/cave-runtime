// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/email/DefaultEmailSenderProvider.java

//! SMTP outbox — RED phase.

use std::sync::Arc;
use std::time::Duration;

use lettre::message::Mailbox;
use lettre::transport::stub::StubTransport;
use lettre::Message;
use parking_lot::Mutex;
use serde::Serialize;

use super::events::AuthEvent;
use super::templates::Templates;
use super::EmailError;

pub struct OutgoingEmail {
    pub to: String,
    pub subject: String,
    pub html: String,
    pub text: String,
}

pub trait OutboxTransport: Send + Sync {
    fn try_send(&self, msg: &Message) -> Result<bool, EmailError>;
}

#[derive(Clone)]
pub struct TestStubTransport {
    pub inner: Arc<Mutex<StubTransport>>,
    pub transient_remaining: Arc<Mutex<u32>>,
    pub fail_permanently: Arc<Mutex<bool>>,
    pub sent: Arc<Mutex<Vec<String>>>,
}

impl Default for TestStubTransport {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(StubTransport::new_ok())),
            transient_remaining: Arc::new(Mutex::new(0)),
            fail_permanently: Arc::new(Mutex::new(false)),
            sent: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl TestStubTransport {
    pub fn new() -> Self { Self::default() }
    pub fn with_transient_failures(self, n: u32) -> Self { *self.transient_remaining.lock() = n; self }
    pub fn with_permanent_failure(self) -> Self { *self.fail_permanently.lock() = true; self }
    pub fn captured(&self) -> Vec<String> { self.sent.lock().clone() }
}

impl OutboxTransport for TestStubTransport {
    fn try_send(&self, _msg: &Message) -> Result<bool, EmailError> {
        // RED stub: pretend nothing is captured.
        Ok(false)
    }
}

pub struct OutboxConfig {
    pub from: Mailbox,
    pub max_retries: u32,
    pub initial_backoff: Duration,
}

impl OutboxConfig {
    pub fn test_defaults() -> Self {
        Self {
            from: "no-reply@cave.example".parse().unwrap(),
            max_retries: 3,
            initial_backoff: Duration::from_millis(1),
        }
    }
}

pub struct SmtpOutbox {
    pub cfg: OutboxConfig,
    pub transport: Arc<dyn OutboxTransport>,
    pub templates: Arc<Templates>,
    pub deadletter: Arc<Mutex<Vec<DeadletterEntry>>>,
}

#[derive(Clone, Debug)]
pub struct DeadletterEntry {
    pub to: String,
    pub subject: String,
    pub reason: String,
}

impl SmtpOutbox {
    pub fn new_with_transport(cfg: OutboxConfig, transport: Arc<dyn OutboxTransport>) -> Self {
        Self {
            cfg, transport,
            templates: Arc::new(Templates::default()),
            deadletter: Arc::new(Mutex::new(Vec::new())),
        }
    }
    pub fn send_event<P: Serialize>(
        &self,
        _event: AuthEvent,
        _payload: &P,
        _to: &str,
    ) -> Result<(), EmailError> {
        Err(EmailError::Send("RED-phase stub".into()))
    }
    pub fn send_outgoing(&self, _out: &OutgoingEmail) -> Result<(), EmailError> {
        Err(EmailError::Send("RED-phase stub".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::events::*;
    use std::sync::Arc;

    fn login_payload() -> LoginAlertPayload {
        LoginAlertPayload {
            to: "alice@example.com".into(),
            display_name: "Alice".into(),
            ip_address: "1.2.3.4".into(),
            user_agent: "Curl".into(),
            location: "Berlin, DE".into(),
            at: "2024-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn smtp_send_via_stub_succeeds_first_try() {
        let stub = Arc::new(TestStubTransport::new());
        let outbox = SmtpOutbox::new_with_transport(OutboxConfig::test_defaults(), stub.clone());
        outbox.send_event(AuthEvent::LoginAlert, &login_payload(), "alice@example.com").unwrap();
        let captured = stub.captured();
        assert_eq!(captured.len(), 1);
        assert!(captured[0].contains("alice@example.com"));
        assert!(captured[0].contains("Alice"));
        assert!(captured[0].contains("text/plain"));
        assert!(captured[0].contains("text/html"));
    }

    #[test]
    fn smtp_send_retries_on_transient_failure() {
        let stub = Arc::new(TestStubTransport::new().with_transient_failures(2));
        let outbox = SmtpOutbox::new_with_transport(OutboxConfig::test_defaults(), stub.clone());
        outbox.send_event(AuthEvent::LoginAlert, &login_payload(), "alice@example.com").unwrap();
        let captured = stub.captured();
        assert_eq!(captured.len(), 1);
    }

    #[test]
    fn smtp_send_deadletters_on_permanent_failure() {
        let stub = Arc::new(TestStubTransport::new().with_permanent_failure());
        let outbox = SmtpOutbox::new_with_transport(OutboxConfig::test_defaults(), stub.clone());
        let err = outbox.send_event(AuthEvent::LoginAlert, &login_payload(), "alice@example.com").unwrap_err();
        assert!(matches!(err, EmailError::Deadletter(_)));
        let dl = outbox.deadletter.lock();
        assert_eq!(dl.len(), 1);
        assert_eq!(dl[0].to, "alice@example.com");
    }

    #[test]
    fn smtp_send_deadletters_after_exhausting_retries() {
        let stub = Arc::new(TestStubTransport::new().with_transient_failures(10));
        let cfg = OutboxConfig {
            from: "no-reply@cave.example".parse().unwrap(),
            max_retries: 2,
            initial_backoff: Duration::from_millis(1),
        };
        let outbox = SmtpOutbox::new_with_transport(cfg, stub.clone());
        let err = outbox.send_event(AuthEvent::LoginAlert, &login_payload(), "alice@example.com").unwrap_err();
        assert!(matches!(err, EmailError::Deadletter(_)));
        let dl = outbox.deadletter.lock();
        assert_eq!(dl.len(), 1);
    }

    #[test]
    fn smtp_send_includes_subject_line() {
        let stub = Arc::new(TestStubTransport::new());
        let outbox = SmtpOutbox::new_with_transport(OutboxConfig::test_defaults(), stub.clone());
        outbox.send_event(AuthEvent::PasswordChange, &PasswordChangePayload {
            to: "bob@example.com".into(), display_name: "Bob".into(),
            at: "2024-01-01T00:00:00Z".into(), ip_address: "5.6.7.8".into(),
        }, "bob@example.com").unwrap();
        let captured = stub.captured();
        assert!(captured[0].contains("Your password was changed"));
    }

    #[test]
    fn smtp_send_rejects_invalid_to_address() {
        let stub = Arc::new(TestStubTransport::new());
        let outbox = SmtpOutbox::new_with_transport(OutboxConfig::test_defaults(), stub.clone());
        let err = outbox.send_event(AuthEvent::LoginAlert, &login_payload(), "not-an-email").unwrap_err();
        assert!(matches!(err, EmailError::Send(_) | EmailError::Deadletter(_)));
    }
}
