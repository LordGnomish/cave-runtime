// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/events/email/EmailEventListenerProvider.java

//! Email event listener — port of Keycloak's `EmailEventListenerProvider`.
//!
//! Hooks the cave-auth audit pipeline ([`crate::audit::AuditEvent`]) up
//! to an SMTP outbox so that selected events (new-device login, password
//! change, account lock-out, MFA enrolment, GDPR export) fire a
//! transactional email to the affected user.
//!
//! Layers:
//!
//! * [`smtp_outbox`] — async `SmtpOutbox` (lettre + retry-with-backoff +
//!   deadletter on permanent failure)
//! * [`templates`]   — handlebars-rendered HTML + plain-text bodies
//! * [`events`]      — `AuthEvent` enum + per-event payload structs
//! * [`dispatcher`]  — observes audit events, decides which fire emails

pub mod dispatcher;
pub mod events;
pub mod smtp_outbox;
pub mod templates;

use thiserror::Error;

/// Errors emitted from the listener surface.
#[derive(Debug, Error)]
pub enum EmailError {
    /// Template render failed.
    #[error("email render: {0}")]
    Render(String),
    /// SMTP send failed (transient or permanent).
    #[error("email send: {0}")]
    Send(String),
    /// Deadlettered after retries.
    #[error("email deadlettered: {0}")]
    Deadletter(String),
    /// Unknown event kind (no template registered).
    #[error("email unknown event: {0}")]
    UnknownEvent(String),
}
