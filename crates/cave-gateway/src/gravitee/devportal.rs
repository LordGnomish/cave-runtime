// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Developer portal (Gravitee APIM Portal).
//!
//! Pages, subscriptions, plans, API keys management.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageKind {
    Home,
    ApiDocs,
    Article,
    FAQ,
    Guide,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevPortalPage {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub content_md: String,
    pub kind: PageKind,
    pub published: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl DevPortalPage {
    /// Create a new page.
    pub fn new(slug: String, title: String, kind: PageKind) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            slug,
            title,
            content_md: String::new(),
            kind,
            published: false,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubStatus {
    Pending,
    Active,
    Revoked,
    Expired,
}

/// Subscription to an API plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: Uuid,
    pub consumer_id: String,
    pub api_id: Uuid,
    pub version: String,
    pub plan: String,
    pub status: SubStatus,
    pub created_at: DateTime<Utc>,
    pub approved_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanTier {
    Public,
    Trial,
    Professional,
    Enterprise,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthType {
    KeyAuth,
    OAuth2,
    Jwt,
    None,
}

/// API plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: Uuid,
    pub name: String,
    pub api_id: Uuid,
    pub tier: PlanTier,
    pub rate_limit_per_minute: u32,
    pub quota_per_month: Option<u64>,
    pub auth_type: AuthType,
}

impl Plan {
    /// Create a new plan.
    pub fn new(
        name: String,
        api_id: Uuid,
        tier: PlanTier,
        auth_type: AuthType,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            api_id,
            tier,
            rate_limit_per_minute: 100,
            quota_per_month: None,
            auth_type,
        }
    }
}

/// Developer portal store.
pub struct DevPortalStore {
    pages: DashMap<Uuid, DevPortalPage>,
    subscriptions: DashMap<Uuid, Subscription>,
    plans: DashMap<Uuid, Plan>,
    api_keys: DashMap<String, Uuid>, // key → subscription_id
}

impl DevPortalStore {
    /// Create a new dev portal store.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            pages: DashMap::new(),
            subscriptions: DashMap::new(),
            plans: DashMap::new(),
            api_keys: DashMap::new(),
        })
    }

    // ── Pages ────────────────────────────────────────────────────────────

    /// Create or update a page.
    pub fn upsert_page(&self, page: DevPortalPage) {
        self.pages.insert(page.id, page);
    }

    /// Get a page by ID.
    pub fn get_page(&self, id: Uuid) -> Option<DevPortalPage> {
        self.pages.get(&id).map(|p| p.value().clone())
    }

    /// List all pages.
    pub fn list_pages(&self) -> Vec<DevPortalPage> {
        self.pages.iter().map(|p| p.value().clone()).collect()
    }

    /// Delete a page.
    pub fn delete_page(&self, id: Uuid) -> bool {
        self.pages.remove(&id).is_some()
    }

    // ── Plans ────────────────────────────────────────────────────────────

    /// Create or update a plan.
    pub fn upsert_plan(&self, plan: Plan) {
        self.plans.insert(plan.id, plan);
    }

    /// Get a plan by ID.
    pub fn get_plan(&self, id: Uuid) -> Option<Plan> {
        self.plans.get(&id).map(|p| p.value().clone())
    }

    /// List all plans.
    pub fn list_plans(&self) -> Vec<Plan> {
        self.plans.iter().map(|p| p.value().clone()).collect()
    }

    /// List plans for an API.
    pub fn list_plans_for_api(&self, api_id: Uuid) -> Vec<Plan> {
        self.plans
            .iter()
            .filter(|p| p.value().api_id == api_id)
            .map(|p| p.value().clone())
            .collect()
    }

    /// Delete a plan.
    pub fn delete_plan(&self, id: Uuid) -> bool {
        self.plans.remove(&id).is_some()
    }

    // ── Subscriptions ────────────────────────────────────────────────────

    /// Subscribe a consumer to an API plan.
    pub fn subscribe(
        &self,
        consumer_id: String,
        api_id: Uuid,
        version: String,
        plan: String,
    ) -> Subscription {
        let now = Utc::now();
        let sub = Subscription {
            id: Uuid::new_v4(),
            consumer_id,
            api_id,
            version,
            plan,
            status: SubStatus::Pending,
            created_at: now,
            approved_at: None,
            revoked_at: None,
        };
        self.subscriptions.insert(sub.id, sub.clone());
        sub
    }

    /// Get a subscription by ID.
    pub fn get_subscription(&self, id: Uuid) -> Option<Subscription> {
        self.subscriptions.get(&id).map(|s| s.value().clone())
    }

    /// List subscriptions for a consumer.
    pub fn list_subscriptions(&self, consumer_id: &str) -> Vec<Subscription> {
        self.subscriptions
            .iter()
            .filter(|s| s.value().consumer_id == consumer_id)
            .map(|s| s.value().clone())
            .collect()
    }

    /// Approve a subscription.
    pub fn approve(&self, id: Uuid) -> bool {
        if let Some(mut sub) = self.subscriptions.get_mut(&id) {
            sub.status = SubStatus::Active;
            sub.approved_at = Some(Utc::now());
            return true;
        }
        false
    }

    /// Revoke a subscription.
    pub fn revoke(&self, id: Uuid) -> bool {
        if let Some(mut sub) = self.subscriptions.get_mut(&id) {
            sub.status = SubStatus::Revoked;
            sub.revoked_at = Some(Utc::now());
            return true;
        }
        false
    }

    // ── API Keys ─────────────────────────────────────────────────────────

    /// Issue a new API key for a subscription.
    pub fn issue_key(&self, subscription_id: Uuid) -> Option<String> {
        // Check subscription exists
        self.subscriptions.get(&subscription_id)?;

        // Generate random base64 32 bytes
        use base64::Engine as _;
        let key = format!("sk_{}", base64::engine::general_purpose::STANDARD.encode(rand::random::<[u8; 32]>()));
        self.api_keys.insert(key.clone(), subscription_id);
        Some(key)
    }

    /// Rotate an API key (revoke old, issue new).
    pub fn rotate_key(&self, subscription_id: Uuid) -> Option<String> {
        // Remove old key
        self.api_keys.retain(|_, &mut v| v != subscription_id);
        // Issue new key
        self.issue_key(subscription_id)
    }

    /// Get subscription from API key.
    pub fn get_subscription_from_key(&self, key: &str) -> Option<Subscription> {
        let sub_id = self.api_keys.get(key)?;
        self.subscriptions.get(sub_id.value()).map(|s| s.value().clone())
    }
}

impl Default for DevPortalStore {
    fn default() -> Self {
        DevPortalStore {
            pages: DashMap::new(),
            subscriptions: DashMap::new(),
            plans: DashMap::new(),
            api_keys: DashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_portal_create_page() {
        let store = DevPortalStore::new();
        let page = DevPortalPage::new("getting-started".to_string(), "Getting Started".to_string(), PageKind::Guide);
        let id = page.id;

        store.upsert_page(page);
        let retrieved = store.get_page(id);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().title, "Getting Started");
    }

    #[test]
    fn test_portal_subscribe_and_approve() {
        let store = DevPortalStore::new();
        let api_id = Uuid::new_v4();

        let sub = store.subscribe(
            "consumer1".to_string(),
            api_id,
            "1.0.0".to_string(),
            "pro".to_string(),
        );
        let sub_id = sub.id;

        assert_eq!(sub.status, SubStatus::Pending);

        // Approve
        assert!(store.approve(sub_id));
        let approved = store.get_subscription(sub_id).unwrap();
        assert_eq!(approved.status, SubStatus::Active);
    }

    #[test]
    fn test_portal_api_key_issue_and_rotate() {
        let store = DevPortalStore::new();
        let api_id = Uuid::new_v4();

        let sub = store.subscribe(
            "consumer1".to_string(),
            api_id,
            "1.0.0".to_string(),
            "pro".to_string(),
        );
        let sub_id = sub.id;

        // Issue key
        let key1 = store.issue_key(sub_id).unwrap();
        assert!(key1.starts_with("sk_"));

        // Verify lookup
        let found = store.get_subscription_from_key(&key1);
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, sub_id);

        // Rotate
        let key2 = store.rotate_key(sub_id).unwrap();
        assert_ne!(key1, key2);
        assert!(store.get_subscription_from_key(&key2).is_some());
        // Old key should be revoked
        assert!(store.get_subscription_from_key(&key1).is_none());
    }
}
