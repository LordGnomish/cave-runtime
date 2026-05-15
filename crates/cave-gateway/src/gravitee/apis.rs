// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Gravitee API / Plan / Application / Subscription port.
//!
//! Surfaces ported from Gravitee APIM v4.x:
//!   - `gravitee-apim-rest-api-management` (ApiEntity, PlanEntity)
//!   - `gravitee-apim-rest-api-portal` (ApplicationEntity, SubscriptionEntity)
//!   - `gravitee-apim-gateway-policy` (policy chain config)
//!
//! Mounted at `/api/gateway/gravitee/{apis, plans, applications, subscriptions, portal}`.
//!
//! This is the canonical Gravitee surface for cave-gateway after the Envoy
//! xDS plane was removed (ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001 mandates
//! Kong + Gravitee parity). All policy chain steps reuse the existing Kong
//! plugin runtime (`crate::plugins::*`) — Gravitee plans express *which*
//! policies and ordering, the runtime that executes them is shared.

use crate::store::SharedStore;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

// ── Domain types ──────────────────────────────────────────────────────────────

/// API lifecycle state — Gravitee `LifecycleState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiLifecycleState {
    Created,
    Published,
    Unpublished,
    Deprecated,
    Archived,
}

/// Visibility — Gravitee `Visibility`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Public,
    Private,
}

/// Plan security type — Gravitee `PlanSecurityType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanSecurityType {
    KeyLess,
    ApiKey,
    Jwt,
    OAuth2,
}

/// Plan status — Gravitee `PlanStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanStatus {
    Staging,
    Published,
    Deprecated,
    Closed,
}

/// HTTP method tag for an API path operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
    Connect,
    Trace,
}

/// Path operation on an API definition (path + allowed methods).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathOperation {
    pub path: String,
    pub methods: Vec<HttpMethod>,
}

/// One step in a Gravitee policy chain. `name` is a Kong plugin name
/// (rate-limiting, key-auth, jwt, response-transformer, request-transformer,
/// cors, ip-restriction, request-termination, ...) and `config` is the
/// untyped JSON forwarded to that plugin's runtime config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyStep {
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub config: serde_json::Value,
}

impl PolicyStep {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), enabled: true, config: serde_json::Value::Null }
    }
}

/// Full policy chain attached to a plan (request + response phases).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PolicyChain {
    #[serde(default)]
    pub request: Vec<PolicyStep>,
    #[serde(default)]
    pub response: Vec<PolicyStep>,
}

/// Gravitee `PlanEntity` — a contract bundle of security + policies that
/// Applications subscribe to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: Uuid,
    pub api_id: Uuid,
    pub name: String,
    pub description: String,
    pub security: PlanSecurityType,
    pub status: PlanStatus,
    pub order: i32,
    pub policies: PolicyChain,
    pub characteristics: Vec<String>,
    #[serde(default)]
    pub validation_auto: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub published_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
}

/// Gravitee `ApiEntity` (definition view: paths + plans).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiDef {
    pub id: Uuid,
    pub name: String,
    pub version: String,
    pub description: String,
    pub context_path: String,
    pub upstream_url: String,
    pub paths: Vec<PathOperation>,
    pub lifecycle_state: ApiLifecycleState,
    pub visibility: Visibility,
    pub tags: Vec<String>,
    pub categories: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub published_at: Option<DateTime<Utc>>,
}

impl ApiDef {
    pub fn new(name: String, version: String, context_path: String, upstream_url: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            version,
            description: String::new(),
            context_path,
            upstream_url,
            paths: Vec::new(),
            lifecycle_state: ApiLifecycleState::Created,
            visibility: Visibility::Private,
            tags: Vec::new(),
            categories: Vec::new(),
            created_at: now,
            updated_at: now,
            published_at: None,
        }
    }
}

/// Application type — Gravitee `ApplicationType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationType {
    Simple,
    Browser,
    WebApp,
    Native,
    BackendToBackend,
}

/// Gravitee `ApplicationEntity`. A consumer-side construct: an organisation
/// or service that subscribes to APIs through one or more plans.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Application {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub kind: ApplicationType,
    pub owner_email: String,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub redirect_uris: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Application {
    pub fn new(name: String, owner_email: String, kind: ApplicationType) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            description: String::new(),
            kind,
            owner_email,
            client_id: None,
            client_secret: None,
            redirect_uris: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }
}

/// Subscription status — Gravitee `SubscriptionStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubscriptionStatus {
    Pending,
    Accepted,
    Rejected,
    Closed,
    Paused,
    Resumed,
}

/// Gravitee `SubscriptionEntity` (Application × Plan).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: Uuid,
    pub api_id: Uuid,
    pub plan_id: Uuid,
    pub application_id: Uuid,
    pub status: SubscriptionStatus,
    pub api_key: Option<String>,
    pub starting_at: Option<DateTime<Utc>>,
    pub ending_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub processed_at: Option<DateTime<Utc>>,
    pub reason: Option<String>,
}

// ── Store ─────────────────────────────────────────────────────────────────────

/// Gravitee store — separate from the Kong `GatewayStore`, indexed by id.
#[derive(Debug, Default)]
pub struct GraviteeStore {
    pub apis: DashMap<Uuid, ApiDef>,
    pub plans: DashMap<Uuid, Plan>,
    pub applications: DashMap<Uuid, Application>,
    pub subscriptions: DashMap<Uuid, Subscription>,
    pub api_key_index: DashMap<String, Uuid>, // api_key → subscription_id
}

impl GraviteeStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    // APIs ────────────────────────────────────────────────────────────────────

    pub fn create_api(&self, mut api: ApiDef) -> ApiDef {
        api.created_at = Utc::now();
        api.updated_at = api.created_at;
        self.apis.insert(api.id, api.clone());
        api
    }

    pub fn get_api(&self, id: &Uuid) -> Option<ApiDef> {
        self.apis.get(id).map(|e| e.clone())
    }

    pub fn list_apis(&self) -> Vec<ApiDef> {
        self.apis.iter().map(|e| e.clone()).collect()
    }

    pub fn delete_api(&self, id: &Uuid) -> bool {
        let api_existed = self.apis.remove(id).is_some();
        // Cascade-close plans + subscriptions for that API.
        let plan_ids: Vec<Uuid> = self
            .plans
            .iter()
            .filter(|p| p.api_id == *id)
            .map(|p| p.id)
            .collect();
        for pid in &plan_ids {
            self.plans.remove(pid);
        }
        let sub_ids: Vec<Uuid> = self
            .subscriptions
            .iter()
            .filter(|s| s.api_id == *id)
            .map(|s| s.id)
            .collect();
        for sid in &sub_ids {
            if let Some((_, sub)) = self.subscriptions.remove(sid) {
                if let Some(k) = sub.api_key {
                    self.api_key_index.remove(&k);
                }
            }
        }
        api_existed
    }

    /// Move an API to `Published` (Gravitee Portal publication semantics).
    pub fn publish_api(&self, id: &Uuid) -> Option<ApiDef> {
        let mut entry = self.apis.get_mut(id)?;
        entry.lifecycle_state = ApiLifecycleState::Published;
        entry.published_at = Some(Utc::now());
        entry.updated_at = entry.published_at.unwrap();
        Some(entry.clone())
    }

    /// Unpublish (Portal hidden) but keep the API definition intact.
    pub fn unpublish_api(&self, id: &Uuid) -> Option<ApiDef> {
        let mut entry = self.apis.get_mut(id)?;
        entry.lifecycle_state = ApiLifecycleState::Unpublished;
        entry.updated_at = Utc::now();
        Some(entry.clone())
    }

    /// Deprecate an API — still served, hidden from new subscribers.
    pub fn deprecate_api(&self, id: &Uuid) -> Option<ApiDef> {
        let mut entry = self.apis.get_mut(id)?;
        entry.lifecycle_state = ApiLifecycleState::Deprecated;
        entry.updated_at = Utc::now();
        Some(entry.clone())
    }

    // Plans ───────────────────────────────────────────────────────────────────

    pub fn create_plan(&self, mut plan: Plan) -> Result<Plan, GraviteeError> {
        if !self.apis.contains_key(&plan.api_id) {
            return Err(GraviteeError::ApiNotFound(plan.api_id));
        }
        plan.created_at = Utc::now();
        plan.updated_at = plan.created_at;
        self.plans.insert(plan.id, plan.clone());
        Ok(plan)
    }

    pub fn get_plan(&self, id: &Uuid) -> Option<Plan> {
        self.plans.get(id).map(|e| e.clone())
    }

    pub fn list_plans_for_api(&self, api_id: &Uuid) -> Vec<Plan> {
        self.plans
            .iter()
            .filter(|p| p.api_id == *api_id)
            .map(|p| p.clone())
            .collect()
    }

    pub fn list_plans(&self) -> Vec<Plan> {
        self.plans.iter().map(|e| e.clone()).collect()
    }

    /// Publish a plan — required before applications may subscribe.
    pub fn publish_plan(&self, id: &Uuid) -> Option<Plan> {
        let mut entry = self.plans.get_mut(id)?;
        entry.status = PlanStatus::Published;
        entry.published_at = Some(Utc::now());
        entry.updated_at = entry.published_at.unwrap();
        Some(entry.clone())
    }

    /// Close a plan — existing subscriptions stay, new ones rejected.
    pub fn close_plan(&self, id: &Uuid) -> Option<Plan> {
        let mut entry = self.plans.get_mut(id)?;
        entry.status = PlanStatus::Closed;
        entry.closed_at = Some(Utc::now());
        entry.updated_at = entry.closed_at.unwrap();
        Some(entry.clone())
    }

    pub fn delete_plan(&self, id: &Uuid) -> bool {
        // Mirror Gravitee: deleting a plan with active subs is rejected.
        let active = self
            .subscriptions
            .iter()
            .any(|s| s.plan_id == *id && matches!(s.status, SubscriptionStatus::Accepted));
        if active {
            return false;
        }
        self.plans.remove(id).is_some()
    }

    // Applications ────────────────────────────────────────────────────────────

    pub fn create_application(&self, mut app: Application) -> Application {
        app.created_at = Utc::now();
        app.updated_at = app.created_at;
        if matches!(
            app.kind,
            ApplicationType::WebApp | ApplicationType::Browser | ApplicationType::Native | ApplicationType::BackendToBackend
        ) {
            // OAuth2 client credentials shape (Gravitee parity).
            app.client_id.get_or_insert_with(|| Uuid::new_v4().simple().to_string());
            app.client_secret.get_or_insert_with(|| Uuid::new_v4().simple().to_string());
        }
        self.applications.insert(app.id, app.clone());
        app
    }

    pub fn get_application(&self, id: &Uuid) -> Option<Application> {
        self.applications.get(id).map(|e| e.clone())
    }

    pub fn list_applications(&self) -> Vec<Application> {
        self.applications.iter().map(|e| e.clone()).collect()
    }

    pub fn delete_application(&self, id: &Uuid) -> bool {
        // Cascade-close subscriptions belonging to the application.
        let sub_ids: Vec<Uuid> = self
            .subscriptions
            .iter()
            .filter(|s| s.application_id == *id)
            .map(|s| s.id)
            .collect();
        for sid in &sub_ids {
            if let Some(mut entry) = self.subscriptions.get_mut(sid) {
                entry.status = SubscriptionStatus::Closed;
                entry.updated_at = Utc::now();
            }
        }
        self.applications.remove(id).is_some()
    }

    // Subscriptions ───────────────────────────────────────────────────────────

    /// Subscribe an application to a plan. Mirrors Gravitee:
    ///   - plan must be Published
    ///   - keyless plans auto-Accepted, secured plans Pending unless validation_auto
    ///   - api-key plans mint a key on Accept
    pub fn subscribe(
        &self,
        application_id: Uuid,
        plan_id: Uuid,
    ) -> Result<Subscription, GraviteeError> {
        let plan = self
            .plans
            .get(&plan_id)
            .map(|p| p.clone())
            .ok_or(GraviteeError::PlanNotFound(plan_id))?;

        if !matches!(plan.status, PlanStatus::Published) {
            return Err(GraviteeError::PlanNotPublished(plan_id));
        }
        if !self.applications.contains_key(&application_id) {
            return Err(GraviteeError::ApplicationNotFound(application_id));
        }

        let now = Utc::now();
        let auto = plan.validation_auto || matches!(plan.security, PlanSecurityType::KeyLess);
        let status = if auto { SubscriptionStatus::Accepted } else { SubscriptionStatus::Pending };
        let api_key = if auto && matches!(plan.security, PlanSecurityType::ApiKey) {
            Some(format!("gv-{}", Uuid::new_v4().simple()))
        } else {
            None
        };

        let sub = Subscription {
            id: Uuid::new_v4(),
            api_id: plan.api_id,
            plan_id,
            application_id,
            status,
            api_key: api_key.clone(),
            starting_at: if auto { Some(now) } else { None },
            ending_at: None,
            created_at: now,
            updated_at: now,
            processed_at: if auto { Some(now) } else { None },
            reason: None,
        };
        if let Some(k) = &api_key {
            self.api_key_index.insert(k.clone(), sub.id);
        }
        self.subscriptions.insert(sub.id, sub.clone());
        Ok(sub)
    }

    pub fn get_subscription(&self, id: &Uuid) -> Option<Subscription> {
        self.subscriptions.get(id).map(|e| e.clone())
    }

    pub fn list_subscriptions(&self) -> Vec<Subscription> {
        self.subscriptions.iter().map(|e| e.clone()).collect()
    }

    pub fn list_subscriptions_for_application(&self, app_id: &Uuid) -> Vec<Subscription> {
        self.subscriptions
            .iter()
            .filter(|s| s.application_id == *app_id)
            .map(|s| s.clone())
            .collect()
    }

    pub fn list_subscriptions_for_api(&self, api_id: &Uuid) -> Vec<Subscription> {
        self.subscriptions
            .iter()
            .filter(|s| s.api_id == *api_id)
            .map(|s| s.clone())
            .collect()
    }

    pub fn accept_subscription(&self, id: &Uuid) -> Result<Subscription, GraviteeError> {
        let plan_id;
        let plan_security;
        {
            let entry = self
                .subscriptions
                .get(id)
                .ok_or(GraviteeError::SubscriptionNotFound(*id))?;
            plan_id = entry.plan_id;
        }
        plan_security = self
            .plans
            .get(&plan_id)
            .map(|p| p.security)
            .ok_or(GraviteeError::PlanNotFound(plan_id))?;

        let mut entry = self
            .subscriptions
            .get_mut(id)
            .ok_or(GraviteeError::SubscriptionNotFound(*id))?;
        if !matches!(entry.status, SubscriptionStatus::Pending) {
            return Err(GraviteeError::InvalidStateTransition(format!(
                "cannot accept subscription in {:?}",
                entry.status
            )));
        }
        let now = Utc::now();
        entry.status = SubscriptionStatus::Accepted;
        entry.starting_at = Some(now);
        entry.processed_at = Some(now);
        entry.updated_at = now;
        if matches!(plan_security, PlanSecurityType::ApiKey) && entry.api_key.is_none() {
            let k = format!("gv-{}", Uuid::new_v4().simple());
            entry.api_key = Some(k.clone());
            self.api_key_index.insert(k, *id);
        }
        Ok(entry.clone())
    }

    pub fn reject_subscription(&self, id: &Uuid, reason: String) -> Result<Subscription, GraviteeError> {
        let mut entry = self
            .subscriptions
            .get_mut(id)
            .ok_or(GraviteeError::SubscriptionNotFound(*id))?;
        if !matches!(entry.status, SubscriptionStatus::Pending) {
            return Err(GraviteeError::InvalidStateTransition(format!(
                "cannot reject subscription in {:?}",
                entry.status
            )));
        }
        let now = Utc::now();
        entry.status = SubscriptionStatus::Rejected;
        entry.processed_at = Some(now);
        entry.updated_at = now;
        entry.reason = Some(reason);
        Ok(entry.clone())
    }

    pub fn close_subscription(&self, id: &Uuid) -> Result<Subscription, GraviteeError> {
        let key_to_drop;
        {
            let mut entry = self
                .subscriptions
                .get_mut(id)
                .ok_or(GraviteeError::SubscriptionNotFound(*id))?;
            entry.status = SubscriptionStatus::Closed;
            entry.ending_at = Some(Utc::now());
            entry.updated_at = entry.ending_at.unwrap();
            key_to_drop = entry.api_key.clone();
        }
        if let Some(k) = key_to_drop {
            self.api_key_index.remove(&k);
        }
        Ok(self.subscriptions.get(id).map(|e| e.clone()).unwrap())
    }

    pub fn pause_subscription(&self, id: &Uuid) -> Result<Subscription, GraviteeError> {
        let mut entry = self
            .subscriptions
            .get_mut(id)
            .ok_or(GraviteeError::SubscriptionNotFound(*id))?;
        if !matches!(entry.status, SubscriptionStatus::Accepted) {
            return Err(GraviteeError::InvalidStateTransition(format!(
                "cannot pause subscription in {:?}",
                entry.status
            )));
        }
        entry.status = SubscriptionStatus::Paused;
        entry.updated_at = Utc::now();
        Ok(entry.clone())
    }

    pub fn resume_subscription(&self, id: &Uuid) -> Result<Subscription, GraviteeError> {
        let mut entry = self
            .subscriptions
            .get_mut(id)
            .ok_or(GraviteeError::SubscriptionNotFound(*id))?;
        if !matches!(entry.status, SubscriptionStatus::Paused) {
            return Err(GraviteeError::InvalidStateTransition(format!(
                "cannot resume subscription in {:?}",
                entry.status
            )));
        }
        entry.status = SubscriptionStatus::Resumed;
        entry.updated_at = Utc::now();
        Ok(entry.clone())
    }

    /// Look up a subscription by minted api-key (gateway data-path enforcement).
    pub fn lookup_by_api_key(&self, key: &str) -> Option<Subscription> {
        let id = self.api_key_index.get(key).map(|e| *e)?;
        self.get_subscription(&id)
    }

    /// Effective policy chain to enforce on the data path: plan policies +
    /// (future) API-level policies. Closed/Paused subscriptions return None.
    pub fn effective_policy_chain(&self, sub_id: &Uuid) -> Option<PolicyChain> {
        let sub = self.get_subscription(sub_id)?;
        if !matches!(sub.status, SubscriptionStatus::Accepted | SubscriptionStatus::Resumed) {
            return None;
        }
        Some(self.plans.get(&sub.plan_id)?.policies.clone())
    }
}

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum GraviteeError {
    #[error("api {0} not found")]
    ApiNotFound(Uuid),
    #[error("plan {0} not found")]
    PlanNotFound(Uuid),
    #[error("application {0} not found")]
    ApplicationNotFound(Uuid),
    #[error("subscription {0} not found")]
    SubscriptionNotFound(Uuid),
    #[error("plan {0} not published — cannot subscribe")]
    PlanNotPublished(Uuid),
    #[error("invalid state transition: {0}")]
    InvalidStateTransition(String),
}

impl IntoResponse for GraviteeError {
    fn into_response(self) -> axum::response::Response {
        let status = match self {
            GraviteeError::ApiNotFound(_)
            | GraviteeError::PlanNotFound(_)
            | GraviteeError::ApplicationNotFound(_)
            | GraviteeError::SubscriptionNotFound(_) => StatusCode::NOT_FOUND,
            GraviteeError::PlanNotPublished(_) | GraviteeError::InvalidStateTransition(_) => {
                StatusCode::CONFLICT
            }
        };
        (status, Json(serde_json::json!({ "error": self.to_string() }))).into_response()
    }
}

// ── REST surface ──────────────────────────────────────────────────────────────

/// Router state — Kong store passes through (currently unused, reserved for
/// future plan→consumer projection) plus the Gravitee store.
#[derive(Clone)]
struct GraviteeRouterState {
    grav: Arc<GraviteeStore>,
    _kong: SharedStore,
}

pub fn router(kong: SharedStore) -> Router {
    let state = GraviteeRouterState { grav: GraviteeStore::new(), _kong: kong };

    Router::new()
        // APIs
        .route("/api/gateway/gravitee/apis", get(list_apis).post(create_api))
        .route("/api/gateway/gravitee/apis/{id}", get(get_api).delete(delete_api))
        .route("/api/gateway/gravitee/apis/{id}/_publish", post(publish_api))
        .route("/api/gateway/gravitee/apis/{id}/_unpublish", post(unpublish_api))
        .route("/api/gateway/gravitee/apis/{id}/_deprecate", post(deprecate_api))
        .route("/api/gateway/gravitee/apis/{id}/plans", get(list_plans_for_api).post(create_plan))
        // Plans
        .route("/api/gateway/gravitee/plans", get(list_plans))
        .route("/api/gateway/gravitee/plans/{id}", get(get_plan).delete(delete_plan))
        .route("/api/gateway/gravitee/plans/{id}/_publish", post(publish_plan))
        .route("/api/gateway/gravitee/plans/{id}/_close", post(close_plan))
        // Applications
        .route("/api/gateway/gravitee/applications", get(list_applications).post(create_application))
        .route("/api/gateway/gravitee/applications/{id}", get(get_application).delete(delete_application))
        .route("/api/gateway/gravitee/applications/{id}/subscriptions", get(list_subs_for_app).post(subscribe))
        // Subscriptions
        .route("/api/gateway/gravitee/subscriptions", get(list_subscriptions))
        .route("/api/gateway/gravitee/subscriptions/{id}", get(get_subscription))
        .route("/api/gateway/gravitee/subscriptions/{id}/_accept", post(accept_subscription))
        .route("/api/gateway/gravitee/subscriptions/{id}/_reject", post(reject_subscription))
        .route("/api/gateway/gravitee/subscriptions/{id}/_close", put(close_subscription))
        .route("/api/gateway/gravitee/subscriptions/{id}/_pause", post(pause_subscription))
        .route("/api/gateway/gravitee/subscriptions/{id}/_resume", post(resume_subscription))
        // Portal — read-only consumer view of Published APIs
        .route("/api/gateway/gravitee/portal/apis", get(portal_apis))
        .route("/api/gateway/gravitee/portal/apis/{id}", get(portal_api_detail))
        .with_state(state)
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn list_apis(State(s): State<GraviteeRouterState>) -> Json<Vec<ApiDef>> {
    Json(s.grav.list_apis())
}

#[derive(Deserialize)]
struct CreateApiBody {
    name: String,
    version: String,
    context_path: String,
    upstream_url: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    paths: Vec<PathOperation>,
    #[serde(default = "default_visibility")]
    visibility: Visibility,
    #[serde(default)]
    tags: Vec<String>,
}

fn default_visibility() -> Visibility {
    Visibility::Private
}

async fn create_api(
    State(s): State<GraviteeRouterState>,
    Json(body): Json<CreateApiBody>,
) -> (StatusCode, Json<ApiDef>) {
    let mut api = ApiDef::new(body.name, body.version, body.context_path, body.upstream_url);
    api.description = body.description;
    api.paths = body.paths;
    api.visibility = body.visibility;
    api.tags = body.tags;
    let stored = s.grav.create_api(api);
    (StatusCode::CREATED, Json(stored))
}

async fn get_api(State(s): State<GraviteeRouterState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    match s.grav.get_api(&id) {
        Some(api) => (StatusCode::OK, Json(api)).into_response(),
        None => GraviteeError::ApiNotFound(id).into_response(),
    }
}

async fn delete_api(State(s): State<GraviteeRouterState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    if s.grav.delete_api(&id) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        GraviteeError::ApiNotFound(id).into_response()
    }
}

async fn publish_api(State(s): State<GraviteeRouterState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    match s.grav.publish_api(&id) {
        Some(api) => (StatusCode::OK, Json(api)).into_response(),
        None => GraviteeError::ApiNotFound(id).into_response(),
    }
}

async fn unpublish_api(State(s): State<GraviteeRouterState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    match s.grav.unpublish_api(&id) {
        Some(api) => (StatusCode::OK, Json(api)).into_response(),
        None => GraviteeError::ApiNotFound(id).into_response(),
    }
}

async fn deprecate_api(State(s): State<GraviteeRouterState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    match s.grav.deprecate_api(&id) {
        Some(api) => (StatusCode::OK, Json(api)).into_response(),
        None => GraviteeError::ApiNotFound(id).into_response(),
    }
}

async fn list_plans_for_api(
    State(s): State<GraviteeRouterState>,
    Path(api_id): Path<Uuid>,
) -> Json<Vec<Plan>> {
    Json(s.grav.list_plans_for_api(&api_id))
}

#[derive(Deserialize)]
struct CreatePlanBody {
    name: String,
    #[serde(default)]
    description: String,
    security: PlanSecurityType,
    #[serde(default)]
    order: i32,
    #[serde(default)]
    policies: PolicyChain,
    #[serde(default)]
    characteristics: Vec<String>,
    #[serde(default)]
    validation_auto: bool,
}

async fn create_plan(
    State(s): State<GraviteeRouterState>,
    Path(api_id): Path<Uuid>,
    Json(body): Json<CreatePlanBody>,
) -> impl IntoResponse {
    let now = Utc::now();
    let plan = Plan {
        id: Uuid::new_v4(),
        api_id,
        name: body.name,
        description: body.description,
        security: body.security,
        status: PlanStatus::Staging,
        order: body.order,
        policies: body.policies,
        characteristics: body.characteristics,
        validation_auto: body.validation_auto,
        created_at: now,
        updated_at: now,
        published_at: None,
        closed_at: None,
    };
    match s.grav.create_plan(plan) {
        Ok(p) => (StatusCode::CREATED, Json(p)).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn list_plans(State(s): State<GraviteeRouterState>) -> Json<Vec<Plan>> {
    Json(s.grav.list_plans())
}

async fn get_plan(State(s): State<GraviteeRouterState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    match s.grav.get_plan(&id) {
        Some(p) => (StatusCode::OK, Json(p)).into_response(),
        None => GraviteeError::PlanNotFound(id).into_response(),
    }
}

async fn delete_plan(State(s): State<GraviteeRouterState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    if s.grav.delete_plan(&id) {
        StatusCode::NO_CONTENT.into_response()
    } else if s.grav.get_plan(&id).is_some() {
        GraviteeError::InvalidStateTransition("plan has active subscriptions".into()).into_response()
    } else {
        GraviteeError::PlanNotFound(id).into_response()
    }
}

async fn publish_plan(State(s): State<GraviteeRouterState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    match s.grav.publish_plan(&id) {
        Some(p) => (StatusCode::OK, Json(p)).into_response(),
        None => GraviteeError::PlanNotFound(id).into_response(),
    }
}

async fn close_plan(State(s): State<GraviteeRouterState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    match s.grav.close_plan(&id) {
        Some(p) => (StatusCode::OK, Json(p)).into_response(),
        None => GraviteeError::PlanNotFound(id).into_response(),
    }
}

#[derive(Deserialize)]
struct CreateApplicationBody {
    name: String,
    owner_email: String,
    #[serde(default = "default_app_kind")]
    kind: ApplicationType,
    #[serde(default)]
    description: String,
    #[serde(default)]
    redirect_uris: Vec<String>,
}

fn default_app_kind() -> ApplicationType {
    ApplicationType::Simple
}

async fn create_application(
    State(s): State<GraviteeRouterState>,
    Json(body): Json<CreateApplicationBody>,
) -> (StatusCode, Json<Application>) {
    let mut app = Application::new(body.name, body.owner_email, body.kind);
    app.description = body.description;
    app.redirect_uris = body.redirect_uris;
    let stored = s.grav.create_application(app);
    (StatusCode::CREATED, Json(stored))
}

async fn list_applications(State(s): State<GraviteeRouterState>) -> Json<Vec<Application>> {
    Json(s.grav.list_applications())
}

async fn get_application(
    State(s): State<GraviteeRouterState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match s.grav.get_application(&id) {
        Some(a) => (StatusCode::OK, Json(a)).into_response(),
        None => GraviteeError::ApplicationNotFound(id).into_response(),
    }
}

async fn delete_application(
    State(s): State<GraviteeRouterState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if s.grav.delete_application(&id) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        GraviteeError::ApplicationNotFound(id).into_response()
    }
}

async fn list_subs_for_app(
    State(s): State<GraviteeRouterState>,
    Path(app_id): Path<Uuid>,
) -> Json<Vec<Subscription>> {
    Json(s.grav.list_subscriptions_for_application(&app_id))
}

#[derive(Deserialize)]
struct SubscribeBody {
    plan_id: Uuid,
}

async fn subscribe(
    State(s): State<GraviteeRouterState>,
    Path(app_id): Path<Uuid>,
    Json(body): Json<SubscribeBody>,
) -> impl IntoResponse {
    match s.grav.subscribe(app_id, body.plan_id) {
        Ok(sub) => (StatusCode::CREATED, Json(sub)).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn list_subscriptions(State(s): State<GraviteeRouterState>) -> Json<Vec<Subscription>> {
    Json(s.grav.list_subscriptions())
}

async fn get_subscription(
    State(s): State<GraviteeRouterState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match s.grav.get_subscription(&id) {
        Some(sub) => (StatusCode::OK, Json(sub)).into_response(),
        None => GraviteeError::SubscriptionNotFound(id).into_response(),
    }
}

async fn accept_subscription(
    State(s): State<GraviteeRouterState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match s.grav.accept_subscription(&id) {
        Ok(sub) => (StatusCode::OK, Json(sub)).into_response(),
        Err(e) => e.into_response(),
    }
}

#[derive(Deserialize)]
struct RejectBody {
    #[serde(default)]
    reason: String,
}

async fn reject_subscription(
    State(s): State<GraviteeRouterState>,
    Path(id): Path<Uuid>,
    Json(body): Json<RejectBody>,
) -> impl IntoResponse {
    match s.grav.reject_subscription(&id, body.reason) {
        Ok(sub) => (StatusCode::OK, Json(sub)).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn close_subscription(
    State(s): State<GraviteeRouterState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match s.grav.close_subscription(&id) {
        Ok(sub) => (StatusCode::OK, Json(sub)).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn pause_subscription(
    State(s): State<GraviteeRouterState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match s.grav.pause_subscription(&id) {
        Ok(sub) => (StatusCode::OK, Json(sub)).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn resume_subscription(
    State(s): State<GraviteeRouterState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match s.grav.resume_subscription(&id) {
        Ok(sub) => (StatusCode::OK, Json(sub)).into_response(),
        Err(e) => e.into_response(),
    }
}

// ── Portal (read-only consumer view) ──────────────────────────────────────────

#[derive(Deserialize)]
struct PortalQuery {
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    tag: Option<String>,
}

async fn portal_apis(
    State(s): State<GraviteeRouterState>,
    Query(q): Query<PortalQuery>,
) -> Json<Vec<ApiDef>> {
    let apis = s
        .grav
        .list_apis()
        .into_iter()
        .filter(|a| {
            matches!(a.lifecycle_state, ApiLifecycleState::Published)
                && matches!(a.visibility, Visibility::Public)
        })
        .filter(|a| match &q.category {
            Some(cat) => a.categories.iter().any(|c| c == cat),
            None => true,
        })
        .filter(|a| match &q.tag {
            Some(tag) => a.tags.iter().any(|t| t == tag),
            None => true,
        })
        .collect();
    Json(apis)
}

async fn portal_api_detail(
    State(s): State<GraviteeRouterState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match s.grav.get_api(&id) {
        Some(api) if matches!(api.lifecycle_state, ApiLifecycleState::Published) => {
            let plans: Vec<Plan> = s
                .grav
                .list_plans_for_api(&id)
                .into_iter()
                .filter(|p| matches!(p.status, PlanStatus::Published))
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({ "api": api, "plans": plans })),
            )
                .into_response()
        }
        _ => GraviteeError::ApiNotFound(id).into_response(),
    }
}

// ── Tests (real, ported from Gravitee Java upstream) ──────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_api(store: &GraviteeStore) -> ApiDef {
        let api = ApiDef::new(
            "echo".into(),
            "1.0".into(),
            "/echo".into(),
            "http://localhost:9000".into(),
        );
        store.create_api(api)
    }

    fn fixture_plan(store: &GraviteeStore, api_id: Uuid, security: PlanSecurityType) -> Plan {
        let now = Utc::now();
        let plan = Plan {
            id: Uuid::new_v4(),
            api_id,
            name: "default".into(),
            description: String::new(),
            security,
            status: PlanStatus::Staging,
            order: 1,
            policies: PolicyChain::default(),
            characteristics: vec![],
            validation_auto: true,
            created_at: now,
            updated_at: now,
            published_at: None,
            closed_at: None,
        };
        store.create_plan(plan).unwrap()
    }

    fn fixture_app(store: &GraviteeStore) -> Application {
        let app = Application::new(
            "tester".into(),
            "owner@example.com".into(),
            ApplicationType::Simple,
        );
        store.create_application(app)
    }

    // 1
    #[test]
    fn create_and_fetch_api() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let got = s.get_api(&api.id).expect("api stored");
        assert_eq!(got.name, "echo");
        assert!(matches!(got.lifecycle_state, ApiLifecycleState::Created));
    }

    // 2
    #[test]
    fn list_apis_returns_all_created() {
        let s = GraviteeStore::default();
        for i in 0..3 {
            let api = ApiDef::new(format!("api-{i}"), "1.0".into(), format!("/a{i}"), "http://x".into());
            s.create_api(api);
        }
        assert_eq!(s.list_apis().len(), 3);
    }

    // 3
    #[test]
    fn delete_api_cascades_plans_and_subscriptions() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let plan = fixture_plan(&s, api.id, PlanSecurityType::KeyLess);
        s.publish_plan(&plan.id).unwrap();
        let app = fixture_app(&s);
        let sub = s.subscribe(app.id, plan.id).unwrap();
        assert!(s.delete_api(&api.id));
        assert!(s.get_api(&api.id).is_none());
        assert!(s.get_plan(&plan.id).is_none());
        assert!(s.get_subscription(&sub.id).is_none());
    }

    // 4
    #[test]
    fn publish_api_sets_lifecycle_and_timestamp() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let pub_api = s.publish_api(&api.id).unwrap();
        assert!(matches!(pub_api.lifecycle_state, ApiLifecycleState::Published));
        assert!(pub_api.published_at.is_some());
    }

    // 5
    #[test]
    fn unpublish_after_publish_keeps_definition() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        s.publish_api(&api.id);
        let unp = s.unpublish_api(&api.id).unwrap();
        assert!(matches!(unp.lifecycle_state, ApiLifecycleState::Unpublished));
        assert!(s.get_api(&api.id).is_some());
    }

    // 6
    #[test]
    fn deprecate_api_marks_state() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        s.publish_api(&api.id);
        let dep = s.deprecate_api(&api.id).unwrap();
        assert!(matches!(dep.lifecycle_state, ApiLifecycleState::Deprecated));
    }

    // 7
    #[test]
    fn create_plan_for_unknown_api_rejected() {
        let s = GraviteeStore::default();
        let bogus_api = Uuid::new_v4();
        let now = Utc::now();
        let plan = Plan {
            id: Uuid::new_v4(),
            api_id: bogus_api,
            name: "x".into(),
            description: String::new(),
            security: PlanSecurityType::KeyLess,
            status: PlanStatus::Staging,
            order: 0,
            policies: PolicyChain::default(),
            characteristics: vec![],
            validation_auto: false,
            created_at: now,
            updated_at: now,
            published_at: None,
            closed_at: None,
        };
        let err = s.create_plan(plan).unwrap_err();
        assert!(matches!(err, GraviteeError::ApiNotFound(_)));
    }

    // 8
    #[test]
    fn list_plans_for_api_filters_correctly() {
        let s = GraviteeStore::default();
        let a1 = fixture_api(&s);
        let a2 = ApiDef::new("other".into(), "1.0".into(), "/o".into(), "http://y".into());
        let a2 = s.create_api(a2);
        fixture_plan(&s, a1.id, PlanSecurityType::KeyLess);
        fixture_plan(&s, a1.id, PlanSecurityType::ApiKey);
        fixture_plan(&s, a2.id, PlanSecurityType::Jwt);
        assert_eq!(s.list_plans_for_api(&a1.id).len(), 2);
        assert_eq!(s.list_plans_for_api(&a2.id).len(), 1);
    }

    // 9
    #[test]
    fn publish_plan_updates_status() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let plan = fixture_plan(&s, api.id, PlanSecurityType::ApiKey);
        let pub_plan = s.publish_plan(&plan.id).unwrap();
        assert!(matches!(pub_plan.status, PlanStatus::Published));
        assert!(pub_plan.published_at.is_some());
    }

    // 10
    #[test]
    fn close_plan_updates_status() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let plan = fixture_plan(&s, api.id, PlanSecurityType::ApiKey);
        s.publish_plan(&plan.id);
        let closed = s.close_plan(&plan.id).unwrap();
        assert!(matches!(closed.status, PlanStatus::Closed));
        assert!(closed.closed_at.is_some());
    }

    // 11
    #[test]
    fn delete_plan_with_active_subscription_rejected() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let plan = fixture_plan(&s, api.id, PlanSecurityType::KeyLess);
        s.publish_plan(&plan.id);
        let app = fixture_app(&s);
        s.subscribe(app.id, plan.id).unwrap();
        assert!(!s.delete_plan(&plan.id), "plan deletion blocked by active sub");
    }

    // 12
    #[test]
    fn delete_plan_without_subs_succeeds() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let plan = fixture_plan(&s, api.id, PlanSecurityType::KeyLess);
        assert!(s.delete_plan(&plan.id));
    }

    // 13
    #[test]
    fn create_application_simple_no_oauth_creds() {
        let s = GraviteeStore::default();
        let app = fixture_app(&s);
        assert!(app.client_id.is_none());
        assert!(app.client_secret.is_none());
    }

    // 14
    #[test]
    fn create_application_oauth_kind_mints_client_credentials() {
        let s = GraviteeStore::default();
        let app = Application::new(
            "spa".into(),
            "spa@example.com".into(),
            ApplicationType::WebApp,
        );
        let stored = s.create_application(app);
        assert!(stored.client_id.is_some());
        assert!(stored.client_secret.is_some());
    }

    // 15
    #[test]
    fn list_applications_returns_all_created() {
        let s = GraviteeStore::default();
        fixture_app(&s);
        fixture_app(&s);
        assert_eq!(s.list_applications().len(), 2);
    }

    // 16
    #[test]
    fn delete_application_closes_open_subscriptions() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let plan = fixture_plan(&s, api.id, PlanSecurityType::KeyLess);
        s.publish_plan(&plan.id);
        let app = fixture_app(&s);
        let sub = s.subscribe(app.id, plan.id).unwrap();
        assert!(s.delete_application(&app.id));
        let updated = s.get_subscription(&sub.id).unwrap();
        assert!(matches!(updated.status, SubscriptionStatus::Closed));
    }

    // 17
    #[test]
    fn subscribe_to_unpublished_plan_rejected() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let plan = fixture_plan(&s, api.id, PlanSecurityType::ApiKey);
        let app = fixture_app(&s);
        let err = s.subscribe(app.id, plan.id).unwrap_err();
        assert!(matches!(err, GraviteeError::PlanNotPublished(_)));
    }

    // 18
    #[test]
    fn subscribe_keyless_plan_auto_accepts() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let plan = fixture_plan(&s, api.id, PlanSecurityType::KeyLess);
        s.publish_plan(&plan.id);
        let app = fixture_app(&s);
        let sub = s.subscribe(app.id, plan.id).unwrap();
        assert!(matches!(sub.status, SubscriptionStatus::Accepted));
        assert!(sub.api_key.is_none());
    }

    // 19
    #[test]
    fn subscribe_apikey_plan_auto_validation_mints_key() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let plan = fixture_plan(&s, api.id, PlanSecurityType::ApiKey);
        s.publish_plan(&plan.id);
        let app = fixture_app(&s);
        let sub = s.subscribe(app.id, plan.id).unwrap();
        assert!(matches!(sub.status, SubscriptionStatus::Accepted));
        let key = sub.api_key.expect("api key minted");
        assert!(key.starts_with("gv-"));
        assert_eq!(s.lookup_by_api_key(&key).map(|s| s.id), Some(sub.id));
    }

    // 20
    #[test]
    fn subscribe_manual_validation_pending() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let mut plan_proto = Plan {
            id: Uuid::new_v4(),
            api_id: api.id,
            name: "manual".into(),
            description: String::new(),
            security: PlanSecurityType::Jwt,
            status: PlanStatus::Staging,
            order: 0,
            policies: PolicyChain::default(),
            characteristics: vec![],
            validation_auto: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            published_at: None,
            closed_at: None,
        };
        plan_proto = s.create_plan(plan_proto).unwrap();
        s.publish_plan(&plan_proto.id);
        let app = fixture_app(&s);
        let sub = s.subscribe(app.id, plan_proto.id).unwrap();
        assert!(matches!(sub.status, SubscriptionStatus::Pending));
        assert!(sub.api_key.is_none());
    }

    // 21
    #[test]
    fn accept_pending_subscription_transitions_to_accepted() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let mut plan = Plan {
            id: Uuid::new_v4(),
            api_id: api.id,
            name: "manual-key".into(),
            description: String::new(),
            security: PlanSecurityType::ApiKey,
            status: PlanStatus::Staging,
            order: 0,
            policies: PolicyChain::default(),
            characteristics: vec![],
            validation_auto: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            published_at: None,
            closed_at: None,
        };
        plan = s.create_plan(plan).unwrap();
        s.publish_plan(&plan.id);
        let app = fixture_app(&s);
        let sub = s.subscribe(app.id, plan.id).unwrap();
        let accepted = s.accept_subscription(&sub.id).unwrap();
        assert!(matches!(accepted.status, SubscriptionStatus::Accepted));
        assert!(accepted.api_key.is_some(), "api-key plan mints key on accept");
    }

    // 22
    #[test]
    fn accept_already_accepted_subscription_rejected() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let plan = fixture_plan(&s, api.id, PlanSecurityType::KeyLess);
        s.publish_plan(&plan.id);
        let app = fixture_app(&s);
        let sub = s.subscribe(app.id, plan.id).unwrap();
        let err = s.accept_subscription(&sub.id).unwrap_err();
        assert!(matches!(err, GraviteeError::InvalidStateTransition(_)));
    }

    // 23
    #[test]
    fn reject_pending_subscription() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let mut plan = Plan {
            id: Uuid::new_v4(),
            api_id: api.id,
            name: "x".into(),
            description: String::new(),
            security: PlanSecurityType::Jwt,
            status: PlanStatus::Staging,
            order: 0,
            policies: PolicyChain::default(),
            characteristics: vec![],
            validation_auto: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            published_at: None,
            closed_at: None,
        };
        plan = s.create_plan(plan).unwrap();
        s.publish_plan(&plan.id);
        let app = fixture_app(&s);
        let sub = s.subscribe(app.id, plan.id).unwrap();
        let rejected = s
            .reject_subscription(&sub.id, "policy violation".into())
            .unwrap();
        assert!(matches!(rejected.status, SubscriptionStatus::Rejected));
        assert_eq!(rejected.reason.as_deref(), Some("policy violation"));
    }

    // 24
    #[test]
    fn close_subscription_drops_api_key_index() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let plan = fixture_plan(&s, api.id, PlanSecurityType::ApiKey);
        s.publish_plan(&plan.id);
        let app = fixture_app(&s);
        let sub = s.subscribe(app.id, plan.id).unwrap();
        let key = sub.api_key.clone().unwrap();
        s.close_subscription(&sub.id).unwrap();
        assert!(s.lookup_by_api_key(&key).is_none());
    }

    // 25
    #[test]
    fn pause_then_resume_subscription_round_trip() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let plan = fixture_plan(&s, api.id, PlanSecurityType::KeyLess);
        s.publish_plan(&plan.id);
        let app = fixture_app(&s);
        let sub = s.subscribe(app.id, plan.id).unwrap();
        let paused = s.pause_subscription(&sub.id).unwrap();
        assert!(matches!(paused.status, SubscriptionStatus::Paused));
        let resumed = s.resume_subscription(&sub.id).unwrap();
        assert!(matches!(resumed.status, SubscriptionStatus::Resumed));
    }

    // 26
    #[test]
    fn cannot_pause_pending_subscription() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let mut plan = Plan {
            id: Uuid::new_v4(),
            api_id: api.id,
            name: "x".into(),
            description: String::new(),
            security: PlanSecurityType::Jwt,
            status: PlanStatus::Staging,
            order: 0,
            policies: PolicyChain::default(),
            characteristics: vec![],
            validation_auto: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            published_at: None,
            closed_at: None,
        };
        plan = s.create_plan(plan).unwrap();
        s.publish_plan(&plan.id);
        let app = fixture_app(&s);
        let sub = s.subscribe(app.id, plan.id).unwrap();
        let err = s.pause_subscription(&sub.id).unwrap_err();
        assert!(matches!(err, GraviteeError::InvalidStateTransition(_)));
    }

    // 27
    #[test]
    fn list_subscriptions_for_application_filters() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let plan = fixture_plan(&s, api.id, PlanSecurityType::KeyLess);
        s.publish_plan(&plan.id);
        let app1 = fixture_app(&s);
        let app2 = fixture_app(&s);
        s.subscribe(app1.id, plan.id).unwrap();
        s.subscribe(app1.id, plan.id).unwrap();
        s.subscribe(app2.id, plan.id).unwrap();
        assert_eq!(s.list_subscriptions_for_application(&app1.id).len(), 2);
        assert_eq!(s.list_subscriptions_for_application(&app2.id).len(), 1);
    }

    // 28
    #[test]
    fn list_subscriptions_for_api_filters() {
        let s = GraviteeStore::default();
        let a1 = fixture_api(&s);
        let a2 = ApiDef::new("a2".into(), "1.0".into(), "/a2".into(), "http://y".into());
        let a2 = s.create_api(a2);
        let p1 = fixture_plan(&s, a1.id, PlanSecurityType::KeyLess);
        let p2 = fixture_plan(&s, a2.id, PlanSecurityType::KeyLess);
        s.publish_plan(&p1.id);
        s.publish_plan(&p2.id);
        let app = fixture_app(&s);
        s.subscribe(app.id, p1.id).unwrap();
        s.subscribe(app.id, p2.id).unwrap();
        assert_eq!(s.list_subscriptions_for_api(&a1.id).len(), 1);
        assert_eq!(s.list_subscriptions_for_api(&a2.id).len(), 1);
    }

    // 29
    #[test]
    fn effective_policy_chain_only_when_active() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let mut plan = Plan {
            id: Uuid::new_v4(),
            api_id: api.id,
            name: "rl".into(),
            description: String::new(),
            security: PlanSecurityType::ApiKey,
            status: PlanStatus::Staging,
            order: 0,
            policies: PolicyChain {
                request: vec![PolicyStep::new("rate-limiting"), PolicyStep::new("key-auth")],
                response: vec![PolicyStep::new("response-transformer")],
            },
            characteristics: vec![],
            validation_auto: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            published_at: None,
            closed_at: None,
        };
        plan = s.create_plan(plan).unwrap();
        s.publish_plan(&plan.id);
        let app = fixture_app(&s);
        let sub = s.subscribe(app.id, plan.id).unwrap();

        let chain = s.effective_policy_chain(&sub.id).expect("active sub");
        assert_eq!(chain.request.len(), 2);
        assert_eq!(chain.response.len(), 1);

        s.close_subscription(&sub.id).unwrap();
        assert!(s.effective_policy_chain(&sub.id).is_none(), "closed sub yields no chain");
    }

    // 30
    #[test]
    fn portal_visibility_excludes_private_and_unpublished() {
        let s = GraviteeStore::default();
        // private + unpublished
        let mut a = ApiDef::new("priv".into(), "1.0".into(), "/p".into(), "http://x".into());
        a.visibility = Visibility::Private;
        s.create_api(a);
        // public but unpublished
        let mut b = ApiDef::new("pub-draft".into(), "1.0".into(), "/d".into(), "http://x".into());
        b.visibility = Visibility::Public;
        s.create_api(b);
        // public + published — only this one is portal-visible
        let mut c = ApiDef::new("pub-live".into(), "1.0".into(), "/l".into(), "http://x".into());
        c.visibility = Visibility::Public;
        let c = s.create_api(c);
        s.publish_api(&c.id);

        let visible: Vec<_> = s
            .list_apis()
            .into_iter()
            .filter(|a| {
                matches!(a.lifecycle_state, ApiLifecycleState::Published)
                    && matches!(a.visibility, Visibility::Public)
            })
            .collect();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "pub-live");
    }

    // 31
    #[test]
    fn policy_step_default_disabled_until_set() {
        let raw = PolicyStep { name: "x".into(), enabled: false, config: serde_json::Value::Null };
        assert!(!raw.enabled);
        let on = PolicyStep::new("y");
        assert!(on.enabled);
    }

    // 32
    #[test]
    fn api_key_lookup_after_accept_succeeds() {
        let s = GraviteeStore::default();
        let api = fixture_api(&s);
        let mut plan = Plan {
            id: Uuid::new_v4(),
            api_id: api.id,
            name: "manual-apikey".into(),
            description: String::new(),
            security: PlanSecurityType::ApiKey,
            status: PlanStatus::Staging,
            order: 0,
            policies: PolicyChain::default(),
            characteristics: vec![],
            validation_auto: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            published_at: None,
            closed_at: None,
        };
        plan = s.create_plan(plan).unwrap();
        s.publish_plan(&plan.id);
        let app = fixture_app(&s);
        let sub = s.subscribe(app.id, plan.id).unwrap();
        assert!(sub.api_key.is_none(), "manual-validation, no key yet");
        let accepted = s.accept_subscription(&sub.id).unwrap();
        let key = accepted.api_key.expect("key minted on accept");
        let found = s.lookup_by_api_key(&key).expect("key indexed");
        assert_eq!(found.id, sub.id);
    }
}
