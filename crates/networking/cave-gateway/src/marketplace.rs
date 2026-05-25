// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Gravitee Developer Portal & API Marketplace — self-service API catalog,
//! consumer management, subscription plans, and API key self-provisioning.

use crate::models::*;
use crate::GatewayState;
use axum::{
    extract::{Path, State},
    routing::{delete, get, post},
    Json, Router,
};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

// ── Store ─────────────────────────────────────────────────────────────────────

pub struct MarketplaceStore {
    pub consumers: HashMap<Uuid, ApiConsumer>,
    pub plans: HashMap<Uuid, SubscriptionPlan>,
    /// (consumer_id, api_id) -> request count this period.
    pub usage_counters: HashMap<(Uuid, Uuid), u64>,
}

impl MarketplaceStore {
    pub fn new() -> Self {
        let mut store = Self {
            consumers: HashMap::new(),
            plans: HashMap::new(),
            usage_counters: HashMap::new(),
        };
        store.seed_default_plans();
        store
    }

    fn seed_default_plans(&mut self) {
        let now = chrono::Utc::now();
        for (name, tier, per_month, per_1k, included) in [
            ("Free", PlanTier::Free, 0.0, 0.0, 10_000u64),
            ("Basic", PlanTier::Basic, 29.0, 0.50, 100_000),
            ("Pro", PlanTier::Pro, 99.0, 0.20, 1_000_000),
            ("Enterprise", PlanTier::Enterprise, 499.0, 0.05, u64::MAX),
        ] {
            let id = Uuid::new_v4();
            self.plans.insert(id, SubscriptionPlan {
                id,
                name: name.to_string(),
                tier,
                rate_limit: None,
                max_api_keys: 5,
                price_per_month: per_month,
                price_per_1k_requests: per_1k,
                included_requests: included,
                created_at: now,
            });
        }
    }

    pub fn create_consumer(&mut self, req: CreateConsumerRequest) -> ApiConsumer {
        let consumer = ApiConsumer {
            id: Uuid::new_v4(),
            name: req.name,
            email: req.email,
            organization: req.organization,
            api_keys: Vec::new(),
            subscriptions: Vec::new(),
            created_at: chrono::Utc::now(),
        };
        self.consumers.insert(consumer.id, consumer.clone());
        consumer
    }

    pub fn provision_api_key(&mut self, consumer_id: Uuid, label: String) -> Option<String> {
        let consumer = self.consumers.get_mut(&consumer_id)?;
        let key = format!("cave_{}", Uuid::new_v4().as_simple());
        consumer.api_keys.push(ApiKeyEntry {
            key: key.clone(),
            label,
            active: true,
            created_at: chrono::Utc::now(),
        });
        Some(key)
    }

    pub fn revoke_api_key(&mut self, consumer_id: Uuid, key: &str) -> bool {
        let consumer = match self.consumers.get_mut(&consumer_id) {
            Some(c) => c,
            None => return false,
        };
        if let Some(entry) = consumer.api_keys.iter_mut().find(|k| k.key == key) {
            entry.active = false;
            return true;
        }
        false
    }

    pub fn subscribe(&mut self, consumer_id: Uuid, req: SubscribeRequest) -> Option<ConsumerSubscription> {
        // Validate plan exists.
        self.plans.get(&req.plan_id)?;
        let consumer = self.consumers.get_mut(&consumer_id)?;
        let sub = ConsumerSubscription {
            id: Uuid::new_v4(),
            plan_id: req.plan_id,
            api_id: req.api_id,
            active: true,
            subscribed_at: chrono::Utc::now(),
        };
        consumer.subscriptions.push(sub.clone());
        Some(sub)
    }

    pub fn create_plan(&mut self, req: CreatePlanRequest) -> SubscriptionPlan {
        let plan = SubscriptionPlan {
            id: Uuid::new_v4(),
            name: req.name,
            tier: req.tier,
            rate_limit: None,
            max_api_keys: 5,
            price_per_month: req.price_per_month.unwrap_or(0.0),
            price_per_1k_requests: req.price_per_1k_requests.unwrap_or(0.0),
            included_requests: req.included_requests.unwrap_or(10_000),
            created_at: chrono::Utc::now(),
        };
        self.plans.insert(plan.id, plan.clone());
        plan
    }

    pub fn get_dashboard(&self, consumer_id: Uuid) -> Option<ConsumerDashboard> {
        let consumer = self.consumers.get(&consumer_id)?;
        let total_requests: u64 = self.usage_counters.iter()
            .filter(|((cid, _), _)| *cid == consumer_id)
            .map(|(_, &v)| v)
            .sum();
        let mut top_apis: Vec<(String, u64)> = self.usage_counters.iter()
            .filter(|((cid, _), _)| *cid == consumer_id)
            .map(|((_, api_id), &count)| (api_id.to_string(), count))
            .collect();
        top_apis.sort_by(|a, b| b.1.cmp(&a.1));
        top_apis.truncate(5);

        Some(ConsumerDashboard {
            consumer_id,
            consumer_name: consumer.name.clone(),
            total_requests_this_month: total_requests,
            active_keys: consumer.api_keys.iter().filter(|k| k.active).count(),
            active_subscriptions: consumer.subscriptions.iter().filter(|s| s.active).count(),
            top_apis,
        })
    }

    pub fn increment_usage(&mut self, consumer_id: Uuid, api_id: Uuid) {
        *self.usage_counters.entry((consumer_id, api_id)).or_insert(0) += 1;
    }
}

impl Default for MarketplaceStore {
    fn default() -> Self {
        Self::new()
    }
}

// ── Routes ────────────────────────────────────────────────────────────────────

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new()
        // Catalog
        .route("/api/v1/gateway/marketplace/catalog", get(list_catalog))
        // Subscription plans
        .route("/api/v1/gateway/marketplace/plans", get(list_plans).post(create_plan))
        // Consumer management
        .route("/api/v1/gateway/marketplace/consumers", get(list_consumers).post(create_consumer))
        .route("/api/v1/gateway/marketplace/consumers/{id}", get(get_consumer))
        .route("/api/v1/gateway/marketplace/consumers/{id}/dashboard", get(get_dashboard))
        .route("/api/v1/gateway/marketplace/consumers/{id}/keys", post(provision_key))
        .route("/api/v1/gateway/marketplace/consumers/{id}/keys/{key}", delete(revoke_key))
        .route("/api/v1/gateway/marketplace/consumers/{id}/subscribe", post(subscribe))
}

async fn list_catalog(State(state): State<Arc<GatewayState>>) -> Json<serde_json::Value> {
    let engine = state.engine.lock().unwrap();
    let plans = state.marketplace.lock().unwrap();
    Json(serde_json::json!({
        "apis": engine.routes.len(),
        "plans": plans.plans.values().cloned().collect::<Vec<_>>(),
        "total_consumers": plans.consumers.len(),
    }))
}

async fn list_plans(State(state): State<Arc<GatewayState>>) -> Json<Vec<SubscriptionPlan>> {
    let store = state.marketplace.lock().unwrap();
    Json(store.plans.values().cloned().collect())
}

async fn create_plan(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<CreatePlanRequest>,
) -> Json<SubscriptionPlan> {
    let mut store = state.marketplace.lock().unwrap();
    Json(store.create_plan(req))
}

async fn list_consumers(State(state): State<Arc<GatewayState>>) -> Json<Vec<ApiConsumer>> {
    let store = state.marketplace.lock().unwrap();
    Json(store.consumers.values().cloned().collect())
}

async fn create_consumer(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<CreateConsumerRequest>,
) -> Json<ApiConsumer> {
    let mut store = state.marketplace.lock().unwrap();
    Json(store.create_consumer(req))
}

async fn get_consumer(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let store = state.marketplace.lock().unwrap();
    match store.consumers.get(&id) {
        Some(c) => Json(serde_json::to_value(c).unwrap()),
        None => Json(serde_json::json!({ "error": "consumer not found" })),
    }
}

async fn get_dashboard(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let store = state.marketplace.lock().unwrap();
    match store.get_dashboard(id) {
        Some(d) => Json(serde_json::to_value(d).unwrap()),
        None => Json(serde_json::json!({ "error": "consumer not found" })),
    }
}

async fn provision_key(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<ProvisionKeyRequest>,
) -> Json<serde_json::Value> {
    let mut store = state.marketplace.lock().unwrap();
    match store.provision_api_key(id, req.label) {
        Some(key) => Json(serde_json::json!({ "api_key": key, "consumer_id": id })),
        None => Json(serde_json::json!({ "error": "consumer not found" })),
    }
}

async fn revoke_key(
    State(state): State<Arc<GatewayState>>,
    Path((id, key)): Path<(Uuid, String)>,
) -> Json<serde_json::Value> {
    let mut store = state.marketplace.lock().unwrap();
    Json(serde_json::json!({ "revoked": store.revoke_api_key(id, &key) }))
}

async fn subscribe(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<SubscribeRequest>,
) -> Json<serde_json::Value> {
    let mut store = state.marketplace.lock().unwrap();
    match store.subscribe(id, req) {
        Some(sub) => Json(serde_json::to_value(sub).unwrap()),
        None => Json(serde_json::json!({ "error": "consumer or plan not found" })),
    }
}
