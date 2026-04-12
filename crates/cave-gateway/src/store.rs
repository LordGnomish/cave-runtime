//! In-memory GatewayStore with optional cave-db persistence.
//!
//! All entities are stored in HashMaps protected by RwLock for concurrent
//! read access. cave-db (PostgreSQL) integration is optional — the gateway
//! runs fully in-memory when no pool is provided (useful for tests).

use crate::models::*;
use std::collections::HashMap;
use uuid::Uuid;

/// Primary in-memory store for all gateway entities.
#[derive(Debug, Default)]
pub struct GatewayStore {
    // Core Kong entities
    pub services: HashMap<Uuid, Service>,
    pub routes: HashMap<Uuid, Route>,
    pub upstreams: HashMap<Uuid, Upstream>,
    /// Keyed by target ID; look up by upstream with `targets_for_upstream`
    pub targets: HashMap<Uuid, Target>,
    pub consumers: HashMap<Uuid, Consumer>,
    pub plugins: HashMap<Uuid, Plugin>,

    // Credentials
    pub key_auth_creds: HashMap<Uuid, KeyAuthCredential>,
    pub jwt_creds: HashMap<Uuid, JwtCredential>,
    pub basic_auth_creds: HashMap<Uuid, BasicAuthCredential>,
    pub hmac_auth_creds: HashMap<Uuid, HmacAuthCredential>,
    pub oauth2_creds: HashMap<Uuid, OAuth2Credential>,

    // Lifecycle / versioning
    pub api_versions: HashMap<Uuid, ApiVersion>,

    // Developer portal
    pub subscriptions: HashMap<Uuid, PortalSubscription>,
    pub docs: HashMap<Uuid, ApiDoc>,

    // Monetization
    pub usage_records: Vec<UsageRecord>,
}

impl GatewayStore {
    // ── Service helpers ────────────────────────────────────────────────────

    pub fn add_service(&mut self, s: Service) {
        self.services.insert(s.id, s);
    }

    pub fn get_service(&self, id: Uuid) -> Option<&Service> {
        self.services.get(&id)
    }

    pub fn get_service_by_name(&self, name: &str) -> Option<&Service> {
        self.services.values().find(|s| s.name == name)
    }

    pub fn list_services(&self) -> Vec<&Service> {
        let mut v: Vec<&Service> = self.services.values().collect();
        v.sort_by_key(|s| s.created_at);
        v
    }

    pub fn remove_service(&mut self, id: Uuid) -> Option<Service> {
        self.services.remove(&id)
    }

    // ── Route helpers ──────────────────────────────────────────────────────

    pub fn add_route(&mut self, r: Route) {
        self.routes.insert(r.id, r);
    }

    pub fn get_route(&self, id: Uuid) -> Option<&Route> {
        self.routes.get(&id)
    }

    pub fn list_routes(&self) -> Vec<&Route> {
        let mut v: Vec<&Route> = self.routes.values().collect();
        v.sort_by_key(|r| r.created_at);
        v
    }

    pub fn remove_route(&mut self, id: Uuid) -> Option<Route> {
        self.routes.remove(&id)
    }

    pub fn routes_for_service(&self, service_id: Uuid) -> Vec<&Route> {
        self.routes
            .values()
            .filter(|r| r.service_id == service_id)
            .collect()
    }

    // ── Upstream helpers ───────────────────────────────────────────────────

    pub fn add_upstream(&mut self, u: Upstream) {
        self.upstreams.insert(u.id, u);
    }

    pub fn get_upstream(&self, id: Uuid) -> Option<&Upstream> {
        self.upstreams.get(&id)
    }

    pub fn get_upstream_by_name(&self, name: &str) -> Option<&Upstream> {
        self.upstreams.values().find(|u| u.name == name)
    }

    pub fn list_upstreams(&self) -> Vec<&Upstream> {
        let mut v: Vec<&Upstream> = self.upstreams.values().collect();
        v.sort_by_key(|u| u.created_at);
        v
    }

    pub fn remove_upstream(&mut self, id: Uuid) -> Option<Upstream> {
        self.upstreams.remove(&id)
    }

    // ── Target helpers ─────────────────────────────────────────────────────

    pub fn add_target(&mut self, t: Target) {
        self.targets.insert(t.id, t);
    }

    pub fn get_target(&self, id: Uuid) -> Option<&Target> {
        self.targets.get(&id)
    }

    pub fn targets_for_upstream(&self, upstream_id: Uuid) -> Vec<&Target> {
        let mut v: Vec<&Target> = self
            .targets
            .values()
            .filter(|t| t.upstream_id == upstream_id)
            .collect();
        v.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        v
    }

    pub fn healthy_targets(&self, upstream_id: Uuid) -> Vec<&Target> {
        self.targets_for_upstream(upstream_id)
            .into_iter()
            .filter(|t| t.health == TargetHealth::Healthy)
            .collect()
    }

    pub fn remove_target(&mut self, id: Uuid) -> Option<Target> {
        self.targets.remove(&id)
    }

    pub fn update_target_health(&mut self, id: Uuid, health: TargetHealth) {
        if let Some(t) = self.targets.get_mut(&id) {
            t.health = health;
            t.updated_at = chrono::Utc::now();
        }
    }

    // ── Consumer helpers ───────────────────────────────────────────────────

    pub fn add_consumer(&mut self, c: Consumer) {
        self.consumers.insert(c.id, c);
    }

    pub fn get_consumer(&self, id: Uuid) -> Option<&Consumer> {
        self.consumers.get(&id)
    }

    pub fn list_consumers(&self) -> Vec<&Consumer> {
        let mut v: Vec<&Consumer> = self.consumers.values().collect();
        v.sort_by_key(|c| c.created_at);
        v
    }

    pub fn remove_consumer(&mut self, id: Uuid) -> Option<Consumer> {
        self.consumers.remove(&id)
    }

    // ── Plugin helpers ─────────────────────────────────────────────────────

    pub fn add_plugin(&mut self, p: Plugin) {
        self.plugins.insert(p.id, p);
    }

    pub fn get_plugin(&self, id: Uuid) -> Option<&Plugin> {
        self.plugins.get(&id)
    }

    pub fn list_plugins(&self) -> Vec<&Plugin> {
        let mut v: Vec<&Plugin> = self.plugins.values().collect();
        v.sort_by_key(|p| p.created_at);
        v
    }

    /// Return enabled plugins that apply to the given route/service, in order.
    pub fn plugins_for_route(&self, route_id: Uuid, service_id: Uuid) -> Vec<&Plugin> {
        let mut v: Vec<&Plugin> = self
            .plugins
            .values()
            .filter(|p| {
                p.enabled
                    && (p.route_id == Some(route_id)
                        || p.service_id == Some(service_id)
                        || (p.route_id.is_none() && p.service_id.is_none()))
            })
            .collect();
        v.sort_by_key(|p| p.created_at);
        v
    }

    pub fn remove_plugin(&mut self, id: Uuid) -> Option<Plugin> {
        self.plugins.remove(&id)
    }

    // ── Credential helpers ─────────────────────────────────────────────────

    pub fn find_key_auth(&self, key: &str) -> Option<&KeyAuthCredential> {
        self.key_auth_creds.values().find(|c| c.key == key)
    }

    pub fn key_auth_for_consumer(&self, consumer_id: Uuid) -> Vec<&KeyAuthCredential> {
        self.key_auth_creds
            .values()
            .filter(|c| c.consumer_id == consumer_id)
            .collect()
    }

    pub fn find_jwt_by_key(&self, key: &str) -> Option<&JwtCredential> {
        self.jwt_creds.values().find(|c| c.key == key)
    }

    pub fn find_basic_auth(&self, username: &str) -> Option<&BasicAuthCredential> {
        self.basic_auth_creds
            .values()
            .find(|c| c.username == username)
    }

    pub fn find_hmac_auth(&self, username: &str) -> Option<&HmacAuthCredential> {
        self.hmac_auth_creds
            .values()
            .find(|c| c.username == username)
    }

    // ── API Version helpers ────────────────────────────────────────────────

    pub fn add_version(&mut self, v: ApiVersion) {
        self.api_versions.insert(v.id, v);
    }

    pub fn versions_for_service(&self, service_id: Uuid) -> Vec<&ApiVersion> {
        let mut v: Vec<&ApiVersion> = self
            .api_versions
            .values()
            .filter(|av| av.service_id == service_id)
            .collect();
        v.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        v
    }

    // ── Subscription helpers ───────────────────────────────────────────────

    pub fn add_subscription(&mut self, s: PortalSubscription) {
        self.subscriptions.insert(s.id, s);
    }

    pub fn subscriptions_for_consumer(&self, consumer_id: Uuid) -> Vec<&PortalSubscription> {
        self.subscriptions
            .values()
            .filter(|s| s.consumer_id == consumer_id)
            .collect()
    }

    pub fn subscriptions_for_service(&self, service_id: Uuid) -> Vec<&PortalSubscription> {
        self.subscriptions
            .values()
            .filter(|s| s.service_id == service_id)
            .collect()
    }

    // ── Doc helpers ────────────────────────────────────────────────────────

    pub fn add_doc(&mut self, d: ApiDoc) {
        self.docs.insert(d.id, d);
    }

    pub fn docs_for_service(&self, service_id: Uuid) -> Vec<&ApiDoc> {
        self.docs
            .values()
            .filter(|d| d.service_id == service_id)
            .collect()
    }

    // ── Usage / Monetization ───────────────────────────────────────────────

    pub fn record_usage(&mut self, r: UsageRecord) {
        // Keep last 100k records in memory; older ones would flush to DB
        if self.usage_records.len() >= 100_000 {
            self.usage_records.remove(0);
        }
        self.usage_records.push(r);
    }

    pub fn usage_for_consumer(&self, consumer_id: Uuid) -> Vec<&UsageRecord> {
        self.usage_records
            .iter()
            .filter(|r| r.consumer_id == consumer_id)
            .collect()
    }

    pub fn usage_summary(
        &self,
        consumer_id: Uuid,
        service_id: Uuid,
    ) -> UsageSummary {
        let records: Vec<&UsageRecord> = self
            .usage_records
            .iter()
            .filter(|r| r.consumer_id == consumer_id && r.service_id == service_id)
            .collect();

        let total_requests: u64 = records.iter().map(|r| r.request_count).sum();
        let total_bytes: u64 = records.iter().map(|r| r.response_bytes).sum();
        let avg_latency = if records.is_empty() {
            0.0
        } else {
            records.iter().map(|r| r.latency_ms as f64).sum::<f64>() / records.len() as f64
        };

        let period_start = records
            .iter()
            .map(|r| r.timestamp)
            .min()
            .unwrap_or_else(chrono::Utc::now);
        let period_end = records
            .iter()
            .map(|r| r.timestamp)
            .max()
            .unwrap_or_else(chrono::Utc::now);

        UsageSummary {
            consumer_id,
            service_id,
            total_requests,
            total_bytes,
            avg_latency_ms: avg_latency,
            period_start,
            period_end,
        }
    }
}
