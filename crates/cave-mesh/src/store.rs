// SPDX-License-Identifier: AGPL-3.0-or-later
//! Mesh resource store — in-memory with optional cave-db persistence.
//!
//! Persists: VS, DR, Gateway, ServiceEntry, PeerAuthentication,
//! RequestAuthentication, AuthorizationPolicy, RateLimitPolicy,
//! Sidecar, EnvoyFilter, WorkloadGroup, WorkloadEntry, Telemetry.

use crate::{
    auth::AuthEngine,
    error::{MeshError, MeshResult},
    models::{
        AuthorizationPolicy, DestinationRule, EnvoyFilter, Gateway, PeerAuthentication,
        RateLimitPolicy, RequestAuthentication, ServiceEntry, Sidecar, Telemetry, VirtualService,
        WorkloadEntry, WorkloadGroup,
    },
    mtls::MtlsManager,
    rate_limit::RateLimiter,
    sidecar::{EnvoyFilterManager, SidecarManager, WorkloadGroupManager},
    telemetry::TelemetryManager,
    traffic::TrafficManager,
    MeshState,
};
use cave_db::{migrate::run_migrations, CavePool};
use std::sync::Arc;
use tracing::{error, info};

// ─────────────────────────────────────────────────────────────
// MeshStorage trait
// ─────────────────────────────────────────────────────────────

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

    fn save_sidecar(
        &self,
        sc: &Sidecar,
    ) -> impl std::future::Future<Output = MeshResult<()>> + Send;
    fn load_sidecars(
        &self,
    ) -> impl std::future::Future<Output = MeshResult<Vec<Sidecar>>> + Send;

    fn save_envoy_filter(
        &self,
        ef: &EnvoyFilter,
    ) -> impl std::future::Future<Output = MeshResult<()>> + Send;
    fn load_envoy_filters(
        &self,
    ) -> impl std::future::Future<Output = MeshResult<Vec<EnvoyFilter>>> + Send;

    fn save_workload_group(
        &self,
        wg: &WorkloadGroup,
    ) -> impl std::future::Future<Output = MeshResult<()>> + Send;
    fn load_workload_groups(
        &self,
    ) -> impl std::future::Future<Output = MeshResult<Vec<WorkloadGroup>>> + Send;

    fn save_workload_entry(
        &self,
        we: &WorkloadEntry,
    ) -> impl std::future::Future<Output = MeshResult<()>> + Send;
    fn load_workload_entries(
        &self,
    ) -> impl std::future::Future<Output = MeshResult<Vec<WorkloadEntry>>> + Send;

    fn save_telemetry(
        &self,
        t: &Telemetry,
    ) -> impl std::future::Future<Output = MeshResult<()>> + Send;
    fn load_telemetries(
        &self,
    ) -> impl std::future::Future<Output = MeshResult<Vec<Telemetry>>> + Send;
}

// ─────────────────────────────────────────────────────────────
// DbMeshStorage — cave-db backed implementation
// ─────────────────────────────────────────────────────────────

const MODULE: &str = "mesh";

const MIGRATIONS: &[(i32, &str)] = &[(
    1,
    r#"
    CREATE TABLE IF NOT EXISTS virtual_services (
        key TEXT PRIMARY KEY, data JSONB NOT NULL,
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE IF NOT EXISTS destination_rules (
        key TEXT PRIMARY KEY, data JSONB NOT NULL,
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE IF NOT EXISTS gateways (
        key TEXT PRIMARY KEY, data JSONB NOT NULL,
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE IF NOT EXISTS service_entries (
        key TEXT PRIMARY KEY, data JSONB NOT NULL,
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE IF NOT EXISTS peer_authentications (
        key TEXT PRIMARY KEY, data JSONB NOT NULL,
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE IF NOT EXISTS request_authentications (
        key TEXT PRIMARY KEY, data JSONB NOT NULL,
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE IF NOT EXISTS authz_policies (
        key TEXT PRIMARY KEY, data JSONB NOT NULL,
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE IF NOT EXISTS rate_limit_policies (
        key TEXT PRIMARY KEY, data JSONB NOT NULL,
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE IF NOT EXISTS sidecars (
        key TEXT PRIMARY KEY, data JSONB NOT NULL,
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE IF NOT EXISTS envoy_filters (
        key TEXT PRIMARY KEY, data JSONB NOT NULL,
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE IF NOT EXISTS workload_groups (
        key TEXT PRIMARY KEY, data JSONB NOT NULL,
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE IF NOT EXISTS workload_entries (
        key TEXT PRIMARY KEY, data JSONB NOT NULL,
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE IF NOT EXISTS telemetries (
        key TEXT PRIMARY KEY, data JSONB NOT NULL,
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    "#,
)];

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

    async fn delete_by_key(&self, table: &str, key: &str) -> MeshResult<()> {
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
        self.upsert("virtual_services", &format!("{}/{}", vs.namespace, vs.name), vs).await
    }
    async fn load_virtual_services(&self) -> MeshResult<Vec<VirtualService>> {
        self.load_all("virtual_services").await
    }
    async fn delete_virtual_service(&self, host: &str) -> MeshResult<()> {
        self.delete_by_key("virtual_services", host).await
    }

    async fn save_destination_rule(&self, dr: &DestinationRule) -> MeshResult<()> {
        self.upsert("destination_rules", &format!("{}/{}", dr.namespace, dr.name), dr).await
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
        self.upsert("peer_authentications", &format!("{}/{}", pa.namespace, pa.name), pa).await
    }
    async fn load_peer_authentications(&self) -> MeshResult<Vec<PeerAuthentication>> {
        self.load_all("peer_authentications").await
    }

    async fn save_request_authentication(&self, ra: &RequestAuthentication) -> MeshResult<()> {
        self.upsert("request_authentications", &format!("{}/{}", ra.namespace, ra.name), ra).await
    }
    async fn load_request_authentications(&self) -> MeshResult<Vec<RequestAuthentication>> {
        self.load_all("request_authentications").await
    }

    async fn save_authz_policy(&self, policy: &AuthorizationPolicy) -> MeshResult<()> {
        self.upsert("authz_policies", &format!("{}/{}", policy.namespace, policy.name), policy)
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

    async fn save_sidecar(&self, sc: &Sidecar) -> MeshResult<()> {
        self.upsert("sidecars", &format!("{}/{}", sc.namespace, sc.name), sc).await
    }
    async fn load_sidecars(&self) -> MeshResult<Vec<Sidecar>> {
        self.load_all("sidecars").await
    }

    async fn save_envoy_filter(&self, ef: &EnvoyFilter) -> MeshResult<()> {
        self.upsert("envoy_filters", &format!("{}/{}", ef.namespace, ef.name), ef).await
    }
    async fn load_envoy_filters(&self) -> MeshResult<Vec<EnvoyFilter>> {
        self.load_all("envoy_filters").await
    }

    async fn save_workload_group(&self, wg: &WorkloadGroup) -> MeshResult<()> {
        self.upsert("workload_groups", &format!("{}/{}", wg.namespace, wg.name), wg).await
    }
    async fn load_workload_groups(&self) -> MeshResult<Vec<WorkloadGroup>> {
        self.load_all("workload_groups").await
    }

    async fn save_workload_entry(&self, we: &WorkloadEntry) -> MeshResult<()> {
        let key = format!(
            "{}/{}",
            we.namespace.as_deref().unwrap_or("default"),
            we.name.as_deref().unwrap_or(&we.address)
        );
        self.upsert("workload_entries", &key, we).await
    }
    async fn load_workload_entries(&self) -> MeshResult<Vec<WorkloadEntry>> {
        self.load_all("workload_entries").await
    }

    async fn save_telemetry(&self, t: &Telemetry) -> MeshResult<()> {
        self.upsert("telemetries", &format!("{}/{}", t.namespace, t.name), t).await
    }
    async fn load_telemetries(&self) -> MeshResult<Vec<Telemetry>> {
        self.load_all("telemetries").await
    }
}

// ─────────────────────────────────────────────────────────────
// Boot loader
// ─────────────────────────────────────────────────────────────

pub async fn load_persisted_state<S: MeshStorage>(
    storage: &S,
    state: &MeshState,
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
        state.traffic.upsert_virtual_service(vs);
    }
    for dr in load_or_warn!(storage.load_destination_rules(), "DestinationRules") {
        state.traffic.upsert_destination_rule(dr);
    }
    for pa in load_or_warn!(storage.load_peer_authentications(), "PeerAuthentications") {
        state.mtls.upsert_policy(pa);
    }
    for ra in load_or_warn!(storage.load_request_authentications(), "RequestAuthentications") {
        state.auth.upsert_request_auth(ra);
    }
    for ap in load_or_warn!(storage.load_authz_policies(), "AuthzPolicies") {
        state.auth.upsert_authz_policy(ap);
    }
    for rl in load_or_warn!(storage.load_rate_limit_policies(), "RateLimitPolicies") {
        state.rate_limiter.upsert_policy(rl);
    }
    for sc in load_or_warn!(storage.load_sidecars(), "Sidecars") {
        state.sidecar_mgr.upsert(sc);
    }
    for ef in load_or_warn!(storage.load_envoy_filters(), "EnvoyFilters") {
        state.envoy_filter_mgr.upsert(ef);
    }
    for wg in load_or_warn!(storage.load_workload_groups(), "WorkloadGroups") {
        state.workload_group_mgr.upsert_group(wg);
    }
    for we in load_or_warn!(storage.load_workload_entries(), "WorkloadEntries") {
        state.workload_group_mgr.upsert_entry(we);
    }
    for t in load_or_warn!(storage.load_telemetries(), "Telemetries") {
        state.telemetry_mgr.upsert(t);
    }
    for gw in load_or_warn!(storage.load_gateways(), "Gateways") {
        let key = format!("{}/{}", gw.namespace, gw.name);
        state.gateways.write().unwrap().insert(key, gw);
    }
    for se in load_or_warn!(storage.load_service_entries(), "ServiceEntries") {
        let key = format!("{}/{}", se.namespace, se.name);
        state.service_entries.write().unwrap().insert(key, se);
    }

    info!("Mesh state reloaded from persistent storage");
}
