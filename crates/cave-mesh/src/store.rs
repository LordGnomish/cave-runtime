//! Mesh resource store — in-memory with optional cave-db persistence.
//!
//! All mesh resources (VS, DR, Gateway, ServiceEntry, PeerAuthentication,
//! RequestAuthentication, AuthorizationPolicy, RateLimitPolicy) are stored
//! in-memory for low-latency access and optionally persisted to PostgreSQL
//! via cave-db so they survive restarts.
//!
//! The `MeshStorage` trait defines the persistence interface; `DbMeshStorage`
//! implements it using cave-db's `CavePool`.

use crate::{
    auth::AuthEngine,
    error::{MeshError, MeshResult},
    models::{
        AuthorizationPolicy, DestinationRule, Gateway, PeerAuthentication, RateLimitPolicy,
        RequestAuthentication, ServiceEntry, VirtualService,
    },
    mtls::MtlsManager,
    rate_limit::RateLimiter,
    traffic::TrafficManager,
};
use cave_db::{migrate::run_migrations, CavePool};
use std::sync::Arc;
use tracing::{error, info};

// ─────────────────────────────────────────────────────────────
// MeshStorage trait
// ─────────────────────────────────────────────────────────────

/// Persistence interface for mesh resources.
///
/// Implemented by `DbMeshStorage` (backed by cave-db / PostgreSQL) and
/// optionally by a no-op in-memory variant for testing.
pub trait MeshStorage: Send + Sync {
    fn save_virtual_service(
        &self,
        vs: &VirtualService,
    ) -> impl std::future::Future<Output = MeshResult<()>> + Send;
    fn load_virtual_services(
        &self,
    ) -> impl std::future::Future<Output = MeshResult<Vec<VirtualService>>> + Send;
    fn delete_virtual_service(
        &self,
        host: &str,
    ) -> impl std::future::Future<Output = MeshResult<()>> + Send;

    fn save_destination_rule(
        &self,
        dr: &DestinationRule,
    ) -> impl std::future::Future<Output = MeshResult<()>> + Send;
    fn load_destination_rules(
        &self,
    ) -> impl std::future::Future<Output = MeshResult<Vec<DestinationRule>>> + Send;

    fn save_gateway(
        &self,
        gw: &Gateway,
    ) -> impl std::future::Future<Output = MeshResult<()>> + Send;
    fn load_gateways(
        &self,
    ) -> impl std::future::Future<Output = MeshResult<Vec<Gateway>>> + Send;

    fn save_service_entry(
        &self,
        se: &ServiceEntry,
    ) -> impl std::future::Future<Output = MeshResult<()>> + Send;
    fn load_service_entries(
        &self,
    ) -> impl std::future::Future<Output = MeshResult<Vec<ServiceEntry>>> + Send;

    fn save_peer_authentication(
        &self,
        pa: &PeerAuthentication,
    ) -> impl std::future::Future<Output = MeshResult<()>> + Send;
    fn load_peer_authentications(
        &self,
    ) -> impl std::future::Future<Output = MeshResult<Vec<PeerAuthentication>>> + Send;

    fn save_request_authentication(
        &self,
        ra: &RequestAuthentication,
    ) -> impl std::future::Future<Output = MeshResult<()>> + Send;
    fn load_request_authentications(
        &self,
    ) -> impl std::future::Future<Output = MeshResult<Vec<RequestAuthentication>>> + Send;

    fn save_authz_policy(
        &self,
        policy: &AuthorizationPolicy,
    ) -> impl std::future::Future<Output = MeshResult<()>> + Send;
    fn load_authz_policies(
        &self,
    ) -> impl std::future::Future<Output = MeshResult<Vec<AuthorizationPolicy>>> + Send;

    fn save_rate_limit_policy(
        &self,
        policy: &RateLimitPolicy,
    ) -> impl std::future::Future<Output = MeshResult<()>> + Send;
    fn load_rate_limit_policies(
        &self,
    ) -> impl std::future::Future<Output = MeshResult<Vec<RateLimitPolicy>>> + Send;
}

// ─────────────────────────────────────────────────────────────
// DbMeshStorage — cave-db backed implementation
// ─────────────────────────────────────────────────────────────

const MODULE: &str = "mesh";

/// SQL migrations for the cave_mesh schema.
const MIGRATIONS: &[(i32, &str)] = &[(
    1,
    r#"
    CREATE TABLE IF NOT EXISTS virtual_services (
        key         TEXT PRIMARY KEY,
        data        JSONB NOT NULL,
        updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE IF NOT EXISTS destination_rules (
        key         TEXT PRIMARY KEY,
        data        JSONB NOT NULL,
        updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE IF NOT EXISTS gateways (
        key         TEXT PRIMARY KEY,
        data        JSONB NOT NULL,
        updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE IF NOT EXISTS service_entries (
        key         TEXT PRIMARY KEY,
        data        JSONB NOT NULL,
        updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE IF NOT EXISTS peer_authentications (
        key         TEXT PRIMARY KEY,
        data        JSONB NOT NULL,
        updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE IF NOT EXISTS request_authentications (
        key         TEXT PRIMARY KEY,
        data        JSONB NOT NULL,
        updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE IF NOT EXISTS authz_policies (
        key         TEXT PRIMARY KEY,
        data        JSONB NOT NULL,
        updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE IF NOT EXISTS rate_limit_policies (
        key         TEXT PRIMARY KEY,
        data        JSONB NOT NULL,
        updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    "#,
)];

/// cave-db backed implementation of `MeshStorage`.
pub struct DbMeshStorage {
    pool: Arc<CavePool>,
}

impl DbMeshStorage {
    pub async fn new(pool: Arc<CavePool>) -> MeshResult<Self> {
        run_migrations(&pool, MODULE, MIGRATIONS)
            .await
            .map_err(|e| MeshError::Storage(e))?;
        info!("cave-mesh DB migrations applied");
        Ok(Self { pool })
    }

    async fn upsert<T: serde::Serialize>(&self, table: &str, key: &str, value: &T) -> MeshResult<()> {
        let data = serde_json::to_value(value)?;
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| MeshError::Storage(e.to_string()))?;
        let sql = format!(
            r#"INSERT INTO cave_mesh.{table} (key, data, updated_at)
               VALUES ($1, $2, NOW())
               ON CONFLICT (key) DO UPDATE
               SET data = EXCLUDED.data, updated_at = NOW()"#
        );
        client
            .execute(&sql as &str, &[&key, &data])
            .await
            .map_err(|e| MeshError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn load_all<T: serde::de::DeserializeOwned>(&self, table: &str) -> MeshResult<Vec<T>> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| MeshError::Storage(e.to_string()))?;
        let sql = format!("SELECT data FROM cave_mesh.{table}");
        let rows = client
            .query(&sql as &str, &[])
            .await
            .map_err(|e| MeshError::Storage(e.to_string()))?;
        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let val: serde_json::Value = row.get(0);
            let item: T = serde_json::from_value(val)?;
            results.push(item);
        }
        Ok(results)
    }

    async fn delete(&self, table: &str, key: &str) -> MeshResult<()> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| MeshError::Storage(e.to_string()))?;
        let sql = format!("DELETE FROM cave_mesh.{table} WHERE key = $1");
        client
            .execute(&sql as &str, &[&key])
            .await
            .map_err(|e| MeshError::Storage(e.to_string()))?;
        Ok(())
    }
}

impl MeshStorage for DbMeshStorage {
    async fn save_virtual_service(&self, vs: &VirtualService) -> MeshResult<()> {
        self.upsert("virtual_services", &vs.hosts[0], vs).await
    }
    async fn load_virtual_services(&self) -> MeshResult<Vec<VirtualService>> {
        self.load_all("virtual_services").await
    }
    async fn delete_virtual_service(&self, host: &str) -> MeshResult<()> {
        self.delete("virtual_services", host).await
    }

    async fn save_destination_rule(&self, dr: &DestinationRule) -> MeshResult<()> {
        self.upsert("destination_rules", &dr.host, dr).await
    }
    async fn load_destination_rules(&self) -> MeshResult<Vec<DestinationRule>> {
        self.load_all("destination_rules").await
    }

    async fn save_gateway(&self, gw: &Gateway) -> MeshResult<()> {
        self.upsert("gateways", &format!("{}/{}", gw.namespace, gw.name), gw).await
    }
    async fn load_gateways(&self) -> MeshResult<Vec<Gateway>> {
        self.load_all("gateways").await
    }

    async fn save_service_entry(&self, se: &ServiceEntry) -> MeshResult<()> {
        self.upsert("service_entries", &format!("{}/{}", se.namespace, se.name), se).await
    }
    async fn load_service_entries(&self) -> MeshResult<Vec<ServiceEntry>> {
        self.load_all("service_entries").await
    }

    async fn save_peer_authentication(&self, pa: &PeerAuthentication) -> MeshResult<()> {
        self.upsert(
            "peer_authentications",
            &format!("{}/{}", pa.namespace, pa.name),
            pa,
        )
        .await
    }
    async fn load_peer_authentications(&self) -> MeshResult<Vec<PeerAuthentication>> {
        self.load_all("peer_authentications").await
    }

    async fn save_request_authentication(&self, ra: &RequestAuthentication) -> MeshResult<()> {
        self.upsert(
            "request_authentications",
            &format!("{}/{}", ra.namespace, ra.name),
            ra,
        )
        .await
    }
    async fn load_request_authentications(&self) -> MeshResult<Vec<RequestAuthentication>> {
        self.load_all("request_authentications").await
    }

    async fn save_authz_policy(&self, policy: &AuthorizationPolicy) -> MeshResult<()> {
        self.upsert(
            "authz_policies",
            &format!("{}/{}", policy.namespace, policy.name),
            policy,
        )
        .await
    }
    async fn load_authz_policies(&self) -> MeshResult<Vec<AuthorizationPolicy>> {
        self.load_all("authz_policies").await
    }

    async fn save_rate_limit_policy(&self, policy: &RateLimitPolicy) -> MeshResult<()> {
        self.upsert(
            "rate_limit_policies",
            &format!("{}/{}", policy.namespace, policy.name),
            policy,
        )
        .await
    }
    async fn load_rate_limit_policies(&self) -> MeshResult<Vec<RateLimitPolicy>> {
        self.load_all("rate_limit_policies").await
    }
}

// ─────────────────────────────────────────────────────────────
// Boot loader — reload persisted state into in-memory engines
// ─────────────────────────────────────────────────────────────

/// Load all persisted mesh resources into the in-memory engines at startup.
pub async fn load_persisted_state<S: MeshStorage>(
    storage: &S,
    traffic: &TrafficManager,
    mtls: &MtlsManager,
    auth: &AuthEngine,
    rate_limiter: &RateLimiter,
) {
    macro_rules! load_or_warn {
        ($fut:expr, $label:expr) => {
            match $fut.await {
                Ok(items) => items,
                Err(e) => {
                    error!(error = %e, "Failed to load {} from DB", $label);
                    vec![]
                }
            }
        };
    }

    for vs in load_or_warn!(storage.load_virtual_services(), "VirtualServices") {
        traffic.upsert_virtual_service(vs);
    }
    for dr in load_or_warn!(storage.load_destination_rules(), "DestinationRules") {
        traffic.upsert_destination_rule(dr);
    }
    for pa in load_or_warn!(storage.load_peer_authentications(), "PeerAuthentications") {
        mtls.upsert_policy(pa);
    }
    for ra in load_or_warn!(storage.load_request_authentications(), "RequestAuthentications") {
        auth.upsert_request_auth(ra);
    }
    for ap in load_or_warn!(storage.load_authz_policies(), "AuthzPolicies") {
        auth.upsert_authz_policy(ap);
    }
    for rl in load_or_warn!(storage.load_rate_limit_policies(), "RateLimitPolicies") {
        rate_limiter.upsert_policy(rl);
    }

    info!("Mesh state reloaded from persistent storage");
}
