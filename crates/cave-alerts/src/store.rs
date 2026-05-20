// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory store for the Alertmanager-style HTTP surface.

use crate::models::{Alert, InhibitRule, Receiver, Route, Silence};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Default)]
struct Inner {
    alerts: HashMap<Uuid, Alert>,
    silences: HashMap<Uuid, Silence>,
    inhibit_rules: HashMap<Uuid, InhibitRule>,
    receivers: HashMap<String, Receiver>,
    routes: Option<Route>,
}

#[derive(Clone, Default)]
pub struct AlertStore {
    inner: Arc<RwLock<Inner>>,
}

impl AlertStore {
    pub fn new() -> Self {
        Self::default()
    }

    // ─── Alerts ────────────────────────────────────────────────────────

    pub fn upsert_alert(&self, alert: Alert) -> Alert {
        self.inner.write().alerts.insert(alert.id, alert.clone());
        alert
    }

    pub fn list_alerts(&self, tenant: Option<&str>) -> Vec<Alert> {
        let inner = self.inner.read();
        inner
            .alerts
            .values()
            .filter(|a| tenant.map_or(true, |t| a.tenant_id == t))
            .cloned()
            .collect()
    }

    pub fn get_alert(&self, id: Uuid) -> Option<Alert> {
        self.inner.read().alerts.get(&id).cloned()
    }

    pub fn delete_alert(&self, id: Uuid) -> bool {
        self.inner.write().alerts.remove(&id).is_some()
    }

    // ─── Silences ──────────────────────────────────────────────────────

    pub fn create_silence(&self, silence: Silence) -> Silence {
        self.inner
            .write()
            .silences
            .insert(silence.id, silence.clone());
        silence
    }

    pub fn list_silences(&self, tenant: Option<&str>) -> Vec<Silence> {
        let inner = self.inner.read();
        inner
            .silences
            .values()
            .filter(|s| tenant.map_or(true, |t| s.tenant_id == t))
            .cloned()
            .collect()
    }

    pub fn delete_silence(&self, id: Uuid) -> bool {
        self.inner.write().silences.remove(&id).is_some()
    }

    pub fn get_silence(&self, id: Uuid) -> Option<Silence> {
        self.inner.read().silences.get(&id).cloned()
    }

    // ─── Inhibit rules ────────────────────────────────────────────────

    pub fn create_inhibit_rule(&self, rule: InhibitRule) -> InhibitRule {
        self.inner
            .write()
            .inhibit_rules
            .insert(rule.id, rule.clone());
        rule
    }

    pub fn list_inhibit_rules(&self, tenant: Option<&str>) -> Vec<InhibitRule> {
        let inner = self.inner.read();
        inner
            .inhibit_rules
            .values()
            .filter(|r| tenant.map_or(true, |t| r.tenant_id == t))
            .cloned()
            .collect()
    }

    pub fn delete_inhibit_rule(&self, id: Uuid) -> bool {
        self.inner.write().inhibit_rules.remove(&id).is_some()
    }

    // ─── Receivers ─────────────────────────────────────────────────────

    pub fn upsert_receiver(&self, receiver: Receiver) {
        self.inner
            .write()
            .receivers
            .insert(receiver.name.clone(), receiver);
    }

    pub fn list_receivers(&self) -> Vec<Receiver> {
        self.inner.read().receivers.values().cloned().collect()
    }

    pub fn receiver_map(&self) -> HashMap<String, Receiver> {
        self.inner.read().receivers.clone()
    }

    pub fn delete_receiver(&self, name: &str) -> bool {
        self.inner.write().receivers.remove(name).is_some()
    }

    // ─── Routes ────────────────────────────────────────────────────────

    pub fn set_root_route(&self, route: Route) {
        self.inner.write().routes = Some(route);
    }

    pub fn get_root_route(&self) -> Option<Route> {
        self.inner.read().routes.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Alert, AlertSeverity, AlertState, Matcher, ReceiverConfig, WebhookConfig};
    use chrono::{Duration, Utc};
    use std::collections::HashMap;

    fn alert_with_tenant(tenant: &str) -> Alert {
        Alert {
            id: Uuid::new_v4(),
            name: "X".into(),
            labels: HashMap::new(),
            annotations: HashMap::new(),
            severity: AlertSeverity::Warning,
            state: AlertState::Firing,
            starts_at: Utc::now(),
            ends_at: None,
            fingerprint: "fp".into(),
            tenant_id: tenant.into(),
            generator_url: None,
        }
    }

    #[test]
    fn test_upsert_and_list_alerts() {
        let store = AlertStore::new();
        store.upsert_alert(alert_with_tenant("a"));
        store.upsert_alert(alert_with_tenant("b"));
        assert_eq!(store.list_alerts(None).len(), 2);
        assert_eq!(store.list_alerts(Some("a")).len(), 1);
    }

    #[test]
    fn test_delete_alert() {
        let store = AlertStore::new();
        let a = store.upsert_alert(alert_with_tenant("a"));
        assert!(store.delete_alert(a.id));
        assert!(!store.delete_alert(a.id));
    }

    #[test]
    fn test_create_and_list_silences_with_tenant_filter() {
        let store = AlertStore::new();
        let now = Utc::now();
        let mut s = Silence::new(vec![], now, now + Duration::hours(1), "alice", "x");
        s.tenant_id = "acme".into();
        store.create_silence(s);
        assert_eq!(store.list_silences(Some("acme")).len(), 1);
        assert_eq!(store.list_silences(Some("globex")).len(), 0);
    }

    #[test]
    fn test_create_and_delete_inhibit_rule() {
        let store = AlertStore::new();
        let rule = InhibitRule::new(
            "r",
            vec![Matcher::equal("a", "b")],
            vec![Matcher::equal("c", "d")],
            vec![],
        );
        let r = store.create_inhibit_rule(rule);
        assert!(store.delete_inhibit_rule(r.id));
    }

    #[test]
    fn test_upsert_receiver_and_map() {
        let store = AlertStore::new();
        let r = Receiver::new("rcv").with_config(ReceiverConfig::Webhook(WebhookConfig {
            url: "http://x".into(),
            send_resolved: true,
        }));
        store.upsert_receiver(r);
        let m = store.receiver_map();
        assert!(m.contains_key("rcv"));
    }

    #[test]
    fn test_set_root_route_round_trip() {
        let store = AlertStore::new();
        let root = Route::root("default");
        store.set_root_route(root);
        assert!(store.get_root_route().is_some());
    }
}
