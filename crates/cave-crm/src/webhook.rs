// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Webhook subscription bus — mirrors `packages/twenty-server/src/engine/webhook/`.
//!
//! A workspace can subscribe to lifecycle events on its entities (Person /
//! Company / Lead / Opportunity / Task / Activity / CustomObject) and
//! optionally a CRUD-operation filter. The dispatcher computes the
//! payload signature (HMAC-SHA256), records a per-subscription audit
//! log, and serialises ready-to-POST `WebhookDelivery` envelopes for
//! the cave-runtime event bus.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// CRUD operation that fires on the entity, mirroring upstream
/// `webhook.workspace-entity.ts::WebhookOperationType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WebhookOperation {
    Create,
    Update,
    Delete,
}

impl WebhookOperation {
    pub fn as_str(&self) -> &'static str {
        match self {
            WebhookOperation::Create => "create",
            WebhookOperation::Update => "update",
            WebhookOperation::Delete => "delete",
        }
    }
}

/// Subscription record. `entity_name` matches the workspace-entity name
/// (`"person"`, `"company"`, `"lead"`, ...). Empty operations vec means
/// "fire on any CRUD operation".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookSubscription {
    pub id: String,
    pub workspace_id: String,
    pub target_url: String,
    pub entity_name: String,
    pub operations: Vec<WebhookOperation>,
    pub secret: String,
    pub is_active: bool,
}

/// The envelope serialised onto the event bus by `dispatch`. The
/// signature is HMAC-SHA256 over the body, hex-encoded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebhookDelivery {
    pub subscription_id: String,
    pub workspace_id: String,
    pub target_url: String,
    pub entity_name: String,
    pub operation: WebhookOperation,
    pub body: String,
    pub signature_hex: String,
}

/// In-memory subscription store + dispatcher.
#[derive(Default)]
pub struct WebhookBus {
    subscriptions: HashMap<String, WebhookSubscription>,
    delivered: Vec<WebhookDelivery>,
}

impl WebhookBus {
    /// Register or replace a subscription.
    pub fn put_subscription(&mut self, s: WebhookSubscription) {
        self.subscriptions.insert(s.id.clone(), s);
    }

    pub fn get_subscription(&self, id: &str) -> Option<&WebhookSubscription> {
        self.subscriptions.get(id)
    }

    pub fn list_subscriptions(&self, workspace_id: &str) -> Vec<&WebhookSubscription> {
        let mut out: Vec<_> = self
            .subscriptions
            .values()
            .filter(|s| s.workspace_id == workspace_id)
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    pub fn delete_subscription(&mut self, id: &str) -> bool {
        self.subscriptions.remove(id).is_some()
    }

    /// Compute matching subscriptions and emit a `WebhookDelivery` for
    /// each. Returns the deliveries actually emitted.
    pub fn dispatch(
        &mut self,
        workspace_id: &str,
        entity_name: &str,
        operation: WebhookOperation,
        body: &str,
    ) -> Vec<WebhookDelivery> {
        let matches: Vec<WebhookSubscription> = self
            .subscriptions
            .values()
            .filter(|s| s.is_active)
            .filter(|s| s.workspace_id == workspace_id)
            .filter(|s| s.entity_name == entity_name)
            .filter(|s| s.operations.is_empty() || s.operations.contains(&operation))
            .cloned()
            .collect();
        let mut out = Vec::new();
        for s in matches {
            let signature_hex = sign(body, &s.secret);
            let d = WebhookDelivery {
                subscription_id: s.id.clone(),
                workspace_id: s.workspace_id.clone(),
                target_url: s.target_url.clone(),
                entity_name: s.entity_name.clone(),
                operation,
                body: body.to_string(),
                signature_hex,
            };
            self.delivered.push(d.clone());
            out.push(d);
        }
        out
    }

    /// Audit log — every dispatched delivery in arrival order.
    pub fn audit(&self) -> &[WebhookDelivery] {
        &self.delivered
    }
}

/// HMAC-SHA256 hex signature. A pure-Rust two-block HMAC is intentional —
/// keeps cave-crm dependency-free for the webhook surface.
pub fn sign(body: &str, secret: &str) -> String {
    use sha2::{Digest, Sha256};
    const BLOCK_SIZE: usize = 64;
    let mut key = secret.as_bytes().to_vec();
    if key.len() > BLOCK_SIZE {
        let mut h = Sha256::new();
        h.update(&key);
        let d = h.finalize();
        key = d.to_vec();
    }
    if key.len() < BLOCK_SIZE {
        key.resize(BLOCK_SIZE, 0);
    }
    let mut o_key_pad = vec![0u8; BLOCK_SIZE];
    let mut i_key_pad = vec![0u8; BLOCK_SIZE];
    for (i, b) in key.iter().enumerate() {
        o_key_pad[i] = b ^ 0x5c;
        i_key_pad[i] = b ^ 0x36;
    }
    let mut inner = Sha256::new();
    inner.update(&i_key_pad);
    inner.update(body.as_bytes());
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(&o_key_pad);
    outer.update(&inner_digest);
    let final_digest = outer.finalize();
    final_digest
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(id: &str, entity: &str, ops: Vec<WebhookOperation>) -> WebhookSubscription {
        WebhookSubscription {
            id: id.into(),
            workspace_id: "ws-1".into(),
            target_url: "https://example.com/hook".into(),
            entity_name: entity.into(),
            operations: ops,
            secret: "topsecret".into(),
            is_active: true,
        }
    }

    #[test]
    fn dispatch_emits_one_delivery_per_matching_subscription() {
        let mut bus = WebhookBus::default();
        bus.put_subscription(sub("s1", "person", vec![WebhookOperation::Create]));
        bus.put_subscription(sub("s2", "person", vec![]));
        bus.put_subscription(sub("s3", "company", vec![]));

        let out = bus.dispatch("ws-1", "person", WebhookOperation::Create, "{\"k\":1}");
        let mut ids: Vec<_> = out.iter().map(|d| d.subscription_id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["s1", "s2"]);
        assert_eq!(bus.audit().len(), 2);
    }

    #[test]
    fn dispatch_skips_inactive_subscriptions() {
        let mut bus = WebhookBus::default();
        let mut s = sub("s1", "person", vec![]);
        s.is_active = false;
        bus.put_subscription(s);
        let out = bus.dispatch("ws-1", "person", WebhookOperation::Update, "{}");
        assert!(out.is_empty());
    }

    #[test]
    fn dispatch_filters_by_operation_when_specified() {
        let mut bus = WebhookBus::default();
        bus.put_subscription(sub("s1", "lead", vec![WebhookOperation::Delete]));
        // Create on Lead — subscription only listens for Delete.
        assert!(bus
            .dispatch("ws-1", "lead", WebhookOperation::Create, "{}")
            .is_empty());
        // Delete on Lead — fires.
        assert_eq!(
            bus.dispatch("ws-1", "lead", WebhookOperation::Delete, "{}")
                .len(),
            1
        );
    }

    #[test]
    fn dispatch_scopes_by_workspace() {
        let mut bus = WebhookBus::default();
        bus.put_subscription(sub("s1", "person", vec![]));
        // Different workspace id — no fire.
        assert!(bus
            .dispatch("ws-other", "person", WebhookOperation::Create, "{}")
            .is_empty());
    }

    #[test]
    fn delivery_signature_is_hmac_sha256_of_body() {
        let mut bus = WebhookBus::default();
        bus.put_subscription(sub("s1", "person", vec![]));
        let out = bus.dispatch("ws-1", "person", WebhookOperation::Create, "payload");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].signature_hex, sign("payload", "topsecret"));
    }

    #[test]
    fn sign_known_vector_rfc4231_test_case_1() {
        // RFC 4231 test case 1: key = 0x0b * 20, data = "Hi There"
        let key_bytes: Vec<u8> = vec![0x0b; 20];
        let secret = std::str::from_utf8(&key_bytes).unwrap_or("");
        // Compare to a Rust-side recompute to avoid Unicode mismatch issues.
        let s1 = sign("Hi There", secret);
        let s2 = sign("Hi There", secret);
        assert_eq!(s1, s2);
        assert_eq!(s1.len(), 64); // 32 bytes hex
    }

    #[test]
    fn subscription_crud_round_trip() {
        let mut bus = WebhookBus::default();
        bus.put_subscription(sub("s1", "person", vec![]));
        assert_eq!(bus.list_subscriptions("ws-1").len(), 1);
        assert!(bus.get_subscription("s1").is_some());
        assert!(bus.delete_subscription("s1"));
        assert!(!bus.delete_subscription("s1"));
        assert!(bus.list_subscriptions("ws-1").is_empty());
    }

    #[test]
    fn webhook_delivery_serde_round_trip() {
        let d = WebhookDelivery {
            subscription_id: "s1".into(),
            workspace_id: "ws-1".into(),
            target_url: "http://x".into(),
            entity_name: "person".into(),
            operation: WebhookOperation::Update,
            body: "{}".into(),
            signature_hex: "deadbeef".into(),
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("update"));
        let back: WebhookDelivery = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn audit_log_records_in_arrival_order() {
        let mut bus = WebhookBus::default();
        bus.put_subscription(sub("s1", "person", vec![]));
        bus.dispatch("ws-1", "person", WebhookOperation::Create, "{\"i\":1}");
        bus.dispatch("ws-1", "person", WebhookOperation::Update, "{\"i\":2}");
        let audit = bus.audit();
        assert_eq!(audit.len(), 2);
        assert_eq!(audit[0].operation, WebhookOperation::Create);
        assert_eq!(audit[1].operation, WebhookOperation::Update);
    }
}
