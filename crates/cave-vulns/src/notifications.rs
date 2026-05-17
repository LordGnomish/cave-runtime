// SPDX-License-Identifier: AGPL-3.0-or-later
//! Notification sinks — fan-out finding events to Slack/Teams/Jira/email.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/notifications/helper.py
//!         (`send_notification`, scope-based recipient resolution).
//!
//! Pluggable trait + 3 implementations:
//!   - `InMemorySink`  — captures events, used in tests
//!   - `WebhookSink`   — POSTs JSON to a configured URL (Slack/Teams compat)
//!   - `LogSink`       — tracing::info!() the event
//!
//! Email/Jira are documented `status="missing"` (Phase 2): both
//! require external creds wiring, which we'd rather thread through
//! cave-vault than embed here.

use crate::finding::{Finding, FindingSeverity};
use async_trait::async_trait;
use serde::Serialize;
use std::sync::{Arc, Mutex};

/// One event worth notifying about.
#[derive(Debug, Clone, Serialize)]
pub struct NotificationEvent {
    pub kind: EventKind,
    pub finding_id: uuid::Uuid,
    pub title: String,
    pub severity: FindingSeverity,
    pub product: Option<String>,
    pub url: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    FindingCreated,
    SeverityIncreased,
    SlaBreachImminent,
    SlaBreached,
    RiskAcceptanceExpired,
    ScanFailed,
    ScanCompleted,
}

#[derive(Debug, thiserror::Error)]
pub enum NotifyError {
    #[error("transport failure: {0}")]
    Transport(String),
    #[error("sink mutex poisoned")]
    Poisoned,
}

#[async_trait]
pub trait NotificationSink: Send + Sync {
    async fn send(&self, event: &NotificationEvent) -> Result<(), NotifyError>;
}

/// In-memory sink — buffers events for inspection. Used by tests
/// and by the Portal "recent notifications" feed.
#[derive(Default, Clone)]
pub struct InMemorySink {
    pub events: Arc<Mutex<Vec<NotificationEvent>>>,
}

impl InMemorySink {
    pub fn new() -> Self { Self::default() }
    pub fn drain(&self) -> Vec<NotificationEvent> {
        self.events.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default()
    }
    pub fn snapshot(&self) -> Vec<NotificationEvent> {
        self.events.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

#[async_trait]
impl NotificationSink for InMemorySink {
    async fn send(&self, event: &NotificationEvent) -> Result<(), NotifyError> {
        self.events.lock().map_err(|_| NotifyError::Poisoned)?.push(event.clone());
        Ok(())
    }
}

/// Webhook sink — POSTs JSON to a configured URL. Compatible with
/// Slack incoming webhooks (`{"text": "…"}`) and MS Teams
/// (`{"text": "…"}` for connector cards).
pub struct WebhookSink {
    pub url: String,
    pub style: WebhookStyle,
    /// Pluggable sender — by default uses `reqwest`. Tests inject a
    /// closure that captures the payload without doing network I/O.
    sender: Arc<dyn Fn(String, String) -> Result<(), NotifyError> + Send + Sync>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookStyle {
    /// `{"text": "…"}` — Slack/Teams.
    SlackCompatible,
    /// `{"event":..., "severity":..., ...}` — opaque JSON.
    Raw,
}

impl WebhookSink {
    /// Construct with a custom `sender` for testing (no network I/O).
    pub fn with_sender(
        url: impl Into<String>,
        style: WebhookStyle,
        sender: Arc<dyn Fn(String, String) -> Result<(), NotifyError> + Send + Sync>,
    ) -> Self {
        Self { url: url.into(), style, sender }
    }

    fn payload(&self, event: &NotificationEvent) -> String {
        match self.style {
            WebhookStyle::SlackCompatible => {
                let text = format!(
                    "*[{sev:?}]* {title}\n_{kind:?}_ on {prod}",
                    sev = event.severity,
                    title = event.title,
                    kind = event.kind,
                    prod = event.product.as_deref().unwrap_or("unknown"),
                );
                serde_json::json!({"text": text}).to_string()
            }
            WebhookStyle::Raw => serde_json::to_string(event).unwrap_or_else(|_| "{}".into()),
        }
    }
}

#[async_trait]
impl NotificationSink for WebhookSink {
    async fn send(&self, event: &NotificationEvent) -> Result<(), NotifyError> {
        let payload = self.payload(event);
        (self.sender)(self.url.clone(), payload)
    }
}

/// `tracing::info!()` sink — drops events into structured logs.
#[derive(Default)]
pub struct LogSink;

#[async_trait]
impl NotificationSink for LogSink {
    async fn send(&self, event: &NotificationEvent) -> Result<(), NotifyError> {
        tracing::info!(
            target: "cave_vulns::notifications",
            severity = ?event.severity,
            kind = ?event.kind,
            finding = ?event.finding_id,
            "{}", event.message
        );
        Ok(())
    }
}

/// Build a default `created` event from a finding.
pub fn event_for(kind: EventKind, finding: &Finding, product: Option<&str>) -> NotificationEvent {
    NotificationEvent {
        kind,
        finding_id: finding.id,
        title: finding.title.clone(),
        severity: finding.severity,
        product: product.map(String::from),
        url: None,
        message: match kind {
            EventKind::FindingCreated => format!("New {:?} finding: {}", finding.severity, finding.title),
            EventKind::SeverityIncreased => format!("Severity promoted to {:?}: {}", finding.severity, finding.title),
            EventKind::SlaBreached => format!("SLA breached: {}", finding.title),
            EventKind::SlaBreachImminent => format!("SLA breach imminent: {}", finding.title),
            EventKind::RiskAcceptanceExpired => format!("Risk acceptance expired: {}", finding.title),
            EventKind::ScanFailed => format!("Scan failed for: {}", finding.title),
            EventKind::ScanCompleted => format!("Scan completed for: {}", finding.title),
        },
    }
}

/// Fan out a single event to many sinks. Returns the first error,
/// but every sink is still invoked.
pub async fn fanout(sinks: &[Arc<dyn NotificationSink>], event: &NotificationEvent) -> Result<(), NotifyError> {
    let mut first_err = None;
    for s in sinks {
        if let Err(e) = s.send(event).await {
            if first_err.is_none() { first_err = Some(e); }
        }
    }
    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn fin(sev: FindingSeverity) -> Finding {
        Finding::new("SQLi in /login", sev)
    }

    #[tokio::test]
    async fn in_memory_sink_captures_events() {
        let sink = InMemorySink::new();
        let event = event_for(EventKind::FindingCreated, &fin(FindingSeverity::Critical), Some("acme"));
        sink.send(&event).await.unwrap();
        let snapshot = sink.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].kind, EventKind::FindingCreated);
        assert_eq!(snapshot[0].severity, FindingSeverity::Critical);
    }

    #[tokio::test]
    async fn webhook_slack_payload_contains_severity_and_title() {
        let captured = Arc::new(Mutex::new(String::new()));
        let cap = captured.clone();
        let sink = WebhookSink::with_sender(
            "https://hooks.slack.com/x",
            WebhookStyle::SlackCompatible,
            Arc::new(move |_url, body| {
                *cap.lock().unwrap() = body;
                Ok(())
            }),
        );
        let f = fin(FindingSeverity::High);
        sink.send(&event_for(EventKind::FindingCreated, &f, Some("p"))).await.unwrap();
        let body = captured.lock().unwrap().clone();
        assert!(body.contains("High"));
        assert!(body.contains("SQLi"));
        assert!(body.contains("\"text\""));
    }

    #[tokio::test]
    async fn webhook_raw_payload_is_full_event_json() {
        let captured = Arc::new(Mutex::new(String::new()));
        let cap = captured.clone();
        let sink = WebhookSink::with_sender(
            "https://example.com/hook",
            WebhookStyle::Raw,
            Arc::new(move |_, body| { *cap.lock().unwrap() = body; Ok(()) }),
        );
        sink.send(&event_for(EventKind::SlaBreached, &fin(FindingSeverity::Medium), None)).await.unwrap();
        let body = captured.lock().unwrap().clone();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["kind"], "sla_breached");
    }

    #[tokio::test]
    async fn log_sink_returns_ok() {
        LogSink::default().send(&event_for(EventKind::ScanCompleted, &fin(FindingSeverity::Info), None)).await.unwrap();
    }

    #[tokio::test]
    async fn fanout_to_multiple_sinks() {
        let s1 = InMemorySink::new();
        let s2 = InMemorySink::new();
        let sinks: Vec<Arc<dyn NotificationSink>> = vec![Arc::new(s1.clone()), Arc::new(s2.clone())];
        let event = event_for(EventKind::FindingCreated, &fin(FindingSeverity::Low), None);
        fanout(&sinks, &event).await.unwrap();
        assert_eq!(s1.snapshot().len(), 1);
        assert_eq!(s2.snapshot().len(), 1);
    }

    #[tokio::test]
    async fn fanout_continues_after_sink_failure() {
        let s_ok = InMemorySink::new();
        struct Failing;
        #[async_trait]
        impl NotificationSink for Failing {
            async fn send(&self, _: &NotificationEvent) -> Result<(), NotifyError> {
                Err(NotifyError::Transport("nope".into()))
            }
        }
        let sinks: Vec<Arc<dyn NotificationSink>> = vec![Arc::new(Failing), Arc::new(s_ok.clone())];
        let event = event_for(EventKind::FindingCreated, &fin(FindingSeverity::Low), None);
        let res = fanout(&sinks, &event).await;
        assert!(res.is_err());
        // Second sink still received it.
        assert_eq!(s_ok.snapshot().len(), 1);
    }

    #[test]
    fn event_for_sla_breached_has_correct_message() {
        let f = fin(FindingSeverity::Critical);
        let e = event_for(EventKind::SlaBreached, &f, None);
        assert_eq!(e.kind, EventKind::SlaBreached);
        assert!(e.message.contains("SLA breached"));
        assert_eq!(e.finding_id, f.id);
    }

    #[tokio::test]
    async fn in_memory_sink_drain_empties_buffer() {
        let sink = InMemorySink::new();
        sink.send(&event_for(EventKind::FindingCreated, &fin(FindingSeverity::Low), None)).await.unwrap();
        assert_eq!(sink.drain().len(), 1);
        assert_eq!(sink.snapshot().len(), 0);
        let _ = Uuid::new_v4();
    }
}
