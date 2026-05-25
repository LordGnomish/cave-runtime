// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Impression-data event fanout — parity with
//! `src/lib/services/impression-data-events` (Unleash v5.0.0).
//!
//! Persisted impression events are broadcast to SSE / webhook subscribers
//! so consumers no longer have to poll the `impression_events` table.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImpressionEvent {
    pub feature_name: String,
    pub enabled: bool,
    pub environment: String,
    pub context_user_id: Option<String>,
    pub timestamp: DateTime<Utc>,
}

/// Subscriber descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscriberKind {
    /// In-process SSE subscriber (HTTP /api/admin/impressions/stream).
    Sse(String),
    /// External webhook subscriber — POST to URL.
    Webhook { url: String, signing_secret: Option<String> },
}

#[derive(Debug)]
pub struct Subscriber {
    pub id: String,
    pub kind: SubscriberKind,
    pub feature_filter: Option<String>,
    pub queue: Mutex<Vec<ImpressionEvent>>,
}

/// In-memory broadcaster — durable persistence is handled separately
/// by `store::insert_impression`. This piece is the live fanout.
#[derive(Default)]
pub struct ImpressionBus {
    pub subscribers: Mutex<Vec<Arc<Subscriber>>>,
}

impl ImpressionBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe(&self, id: impl Into<String>, kind: SubscriberKind) -> Arc<Subscriber> {
        self.subscribe_filtered(id, kind, None)
    }

    pub fn subscribe_filtered(
        &self,
        id: impl Into<String>,
        kind: SubscriberKind,
        feature_filter: Option<String>,
    ) -> Arc<Subscriber> {
        let s = Arc::new(Subscriber {
            id: id.into(),
            kind,
            feature_filter,
            queue: Mutex::new(Vec::new()),
        });
        self.subscribers.lock().unwrap().push(Arc::clone(&s));
        s
    }

    pub fn unsubscribe(&self, id: &str) -> bool {
        let mut subs = self.subscribers.lock().unwrap();
        let before = subs.len();
        subs.retain(|s| s.id != id);
        subs.len() != before
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscribers.lock().unwrap().len()
    }

    /// Fanout an event to all matching subscribers. Returns count of deliveries.
    pub fn publish(&self, event: &ImpressionEvent) -> usize {
        let subs = self.subscribers.lock().unwrap();
        let mut delivered = 0;
        for s in subs.iter() {
            if let Some(ref f) = s.feature_filter {
                if f != &event.feature_name {
                    continue;
                }
            }
            s.queue.lock().unwrap().push(event.clone());
            delivered += 1;
        }
        delivered
    }
}

/// SSE wire encoding (text/event-stream).
pub fn sse_encode(event: &ImpressionEvent) -> String {
    let payload = serde_json::to_string(event).unwrap_or_else(|_| "{}".into());
    format!(
        "event: impression\ndata: {}\n\n",
        payload
    )
}

/// HMAC-SHA256 webhook signature (hex-encoded) — used so consumers can verify
/// they got the event from cave-flags and not a forgery.
pub fn webhook_signature(secret: &str, body: &str) -> String {
    use sha2::{Digest, Sha256};
    // HMAC-SHA256 by hand to avoid pulling another crate.
    let block_size = 64;
    let key = if secret.len() > block_size {
        let mut h = Sha256::new();
        h.update(secret.as_bytes());
        h.finalize().to_vec()
    } else {
        secret.as_bytes().to_vec()
    };
    let mut key_padded = key.clone();
    key_padded.resize(block_size, 0);
    let opad: Vec<u8> = key_padded.iter().map(|b| b ^ 0x5c).collect();
    let ipad: Vec<u8> = key_padded.iter().map(|b| b ^ 0x36).collect();
    let mut inner = Sha256::new();
    inner.update(&ipad);
    inner.update(body.as_bytes());
    let inner_hash = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(&inner_hash);
    let out = outer.finalize();
    out.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(name: &str) -> ImpressionEvent {
        ImpressionEvent {
            feature_name: name.into(),
            enabled: true,
            environment: "default".into(),
            context_user_id: Some("u-1".into()),
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn subscribe_and_publish() {
        let bus = ImpressionBus::new();
        let s = bus.subscribe("sse-1", SubscriberKind::Sse("admin".into()));
        let delivered = bus.publish(&ev("a"));
        assert_eq!(delivered, 1);
        assert_eq!(s.queue.lock().unwrap().len(), 1);
    }

    #[test]
    fn feature_filter_respected() {
        let bus = ImpressionBus::new();
        let s = bus.subscribe_filtered(
            "sse-1",
            SubscriberKind::Sse("admin".into()),
            Some("only-this".into()),
        );
        bus.publish(&ev("other"));
        assert_eq!(s.queue.lock().unwrap().len(), 0);
        bus.publish(&ev("only-this"));
        assert_eq!(s.queue.lock().unwrap().len(), 1);
    }

    #[test]
    fn unsubscribe_removes_subscriber() {
        let bus = ImpressionBus::new();
        bus.subscribe("sse-1", SubscriberKind::Sse("admin".into()));
        assert_eq!(bus.subscriber_count(), 1);
        assert!(bus.unsubscribe("sse-1"));
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[test]
    fn unsubscribe_unknown_returns_false() {
        let bus = ImpressionBus::new();
        assert!(!bus.unsubscribe("never-subscribed"));
    }

    #[test]
    fn publish_with_no_subscribers_returns_zero() {
        let bus = ImpressionBus::new();
        assert_eq!(bus.publish(&ev("x")), 0);
    }

    #[test]
    fn multiple_subscribers_all_delivered() {
        let bus = ImpressionBus::new();
        let s1 = bus.subscribe("a", SubscriberKind::Sse("admin".into()));
        let s2 = bus.subscribe(
            "b",
            SubscriberKind::Webhook {
                url: "https://example.com/hook".into(),
                signing_secret: None,
            },
        );
        assert_eq!(bus.publish(&ev("xx")), 2);
        assert_eq!(s1.queue.lock().unwrap().len(), 1);
        assert_eq!(s2.queue.lock().unwrap().len(), 1);
    }

    #[test]
    fn sse_encode_format() {
        let payload = sse_encode(&ev("alpha"));
        assert!(payload.starts_with("event: impression\n"));
        assert!(payload.contains("\"feature_name\":\"alpha\""));
        assert!(payload.ends_with("\n\n"));
    }

    #[test]
    fn webhook_signature_deterministic() {
        let sig1 = webhook_signature("topsecret", r#"{"a":1}"#);
        let sig2 = webhook_signature("topsecret", r#"{"a":1}"#);
        assert_eq!(sig1, sig2);
        assert_eq!(sig1.len(), 64);
    }

    #[test]
    fn webhook_signature_differs_on_body() {
        let a = webhook_signature("k", "body1");
        let b = webhook_signature("k", "body2");
        assert_ne!(a, b);
    }
}
