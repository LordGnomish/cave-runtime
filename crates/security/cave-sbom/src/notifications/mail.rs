// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/notification/publisher/SendMailPublisher.java
//
//! Email publisher — trait + in-memory test/mock sink.

use super::{Notification, PublishError, Publisher};
use async_trait::async_trait;
use std::sync::Mutex;

/// Build the RFC 5322 subject for a Notification. Mirrors
/// `SendMailPublisher.prepareSubject` (configurable prefix + level + title).
pub fn build_subject(prefix: &str, n: &Notification) -> String {
    format!("[{}] {:?}: {}", prefix.trim(), n.level, n.title)
}

/// Convert content to a text-body block (Dependency-Track uses Pebble templates;
/// we ship a plain-text rendering that preserves the original message).
pub fn build_text_body(n: &Notification) -> String {
    format!(
        "{}\n\nGroup: {:?}\nLevel: {:?}\n",
        n.content, n.group, n.level
    )
}

/// In-memory sink — records every email-equivalent send. Useful for tests
/// and for the `--dry-run` reporter mode.
pub struct InMemoryMail {
    pub to: String,
    pub sent: Mutex<Vec<(String, String, String)>>,
}

impl InMemoryMail {
    pub fn new(to: impl Into<String>) -> Self {
        Self {
            to: to.into(),
            sent: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl Publisher for InMemoryMail {
    async fn publish(&self, n: &Notification) -> Result<(), PublishError> {
        let subj = build_subject("cave-sbom", n);
        let body = build_text_body(n);
        self.sent
            .lock()
            .unwrap()
            .push((self.to.clone(), subj, body));
        Ok(())
    }
    fn kind_name(&self) -> &'static str {
        "InMemoryMail"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifications::{Notification, NotificationGroup, NotificationLevel};

    fn n() -> Notification {
        Notification {
            group: NotificationGroup::PolicyViolation,
            level: NotificationLevel::Warning,
            title: "License violation".into(),
            content: "GPL-3.0 in deny-list".into(),
            payload: None,
        }
    }

    #[test]
    fn subject_format_matches_dependencytrack() {
        let s = build_subject("dt", &n());
        assert_eq!(s, "[dt] Warning: License violation");
    }

    #[test]
    fn body_includes_content_and_metadata() {
        let b = build_text_body(&n());
        assert!(b.contains("GPL-3.0"));
        assert!(b.contains("PolicyViolation"));
    }

    #[tokio::test]
    async fn in_memory_records_send() {
        let m = InMemoryMail::new("ops@example.com");
        m.publish(&n()).await.unwrap();
        let sent = m.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "ops@example.com");
        assert!(sent[0].1.contains("License violation"));
    }
}
