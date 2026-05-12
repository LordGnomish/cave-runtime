//! KEDA CRD schema, ported field-by-field from upstream.
//!
//! References:
//! * `kedacore/keda` `apis/keda/v1alpha1/scaledobject_types.go`
//! * `kedacore/keda` `apis/keda/v1alpha1/scaledjob_types.go`
//! * `kedacore/keda` `apis/keda/v1alpha1/triggerauthentication_types.go`
//! * <https://keda.sh/docs/2.14/concepts/scaling-deployments/>
//!
//! Field names follow Rust naming conventions; the docstring on each
//! field records the upstream YAML key for cross-checking against
//! existing manifests.

use crate::admin::types::TenantId;
use serde::{Deserialize, Serialize};

// ── ScaleTargetRef ────────────────────────────────────────────────────────

/// Reference to the workload KEDA should scale. Maps to
/// `spec.scaleTargetRef` in upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KedaScaleTargetRef {
    /// `spec.scaleTargetRef.apiVersion`. Empty string when defaulted to
    /// `apps/v1`.
    pub api_version: String,
    /// `spec.scaleTargetRef.kind` — Deployment, StatefulSet, or a custom
    /// resource with `/scale` subresource.
    pub kind: String,
    /// `spec.scaleTargetRef.name` — target workload name.
    pub name: String,
    /// `spec.scaleTargetRef.envSourceContainerName` — container name to
    /// source env from when evaluating `prefix`-aware triggers.
    pub env_source_container_name: Option<String>,
}

// ── Trigger spec ──────────────────────────────────────────────────────────

/// One trigger inside a ScaledObject's `spec.triggers[]` list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KedaTrigger {
    /// `spec.triggers[].type` — the scaler kind (e.g. `kafka`, `prometheus`).
    /// Must match a registered scaler in the catalog (see `keda::scalers`).
    pub kind: String,
    /// `spec.triggers[].name` — operator-friendly label. Optional.
    pub name: Option<String>,
    /// `spec.triggers[].metadata` — flattened to key/value pairs to keep
    /// the form rendering straightforward.
    pub metadata: Vec<(String, String)>,
    /// `spec.triggers[].authenticationRef`. None when the trigger uses
    /// an env-var / pod-identity authentication mode.
    pub auth_ref: Option<KedaAuthRef>,
    /// `spec.triggers[].metricType` — `AverageValue` (default), `Value`,
    /// or `Utilization`. Kept as a raw string so unknown values flow
    /// through without panicking.
    pub metric_type: String,
    /// `spec.triggers[].useCachedMetrics` — KEDA 2.10+.
    pub use_cached_metrics: bool,
}

/// `spec.triggers[].authenticationRef`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KedaAuthRef {
    pub name: String,
    /// `TriggerAuthentication` (namespaced) or `ClusterTriggerAuthentication`
    /// (cluster-scoped).
    pub kind: String,
}

// ── Fallback ──────────────────────────────────────────────────────────────

/// `spec.fallback` — what KEDA should do when scaler metrics fail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KedaFallback {
    pub failure_threshold: u32,
    pub replicas: u32,
}

// ── Advanced HPA config ──────────────────────────────────────────────────

/// `spec.advanced` — knobs that surface directly on the HPA KEDA generates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KedaAdvanced {
    /// `spec.advanced.restoreToOriginalReplicaCount` — on delete, restore
    /// the workload to the replica count KEDA observed before it started
    /// managing it.
    pub restore_to_original_replica_count: bool,
    /// `spec.advanced.horizontalPodAutoscalerConfig.name` — operator-supplied
    /// HPA name; default is `keda-hpa-<ScaledObject>`.
    pub hpa_name: Option<String>,
    /// `spec.advanced.horizontalPodAutoscalerConfig.behavior` — YAML blob
    /// passed through to the HPA `spec.behavior` field. Kept as text for
    /// the editor view.
    pub hpa_behavior_yaml: Option<String>,
}

// ── Status ────────────────────────────────────────────────────────────────

/// `status` block KEDA writes back onto the ScaledObject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KedaScaledObjectStatus {
    /// `status.lastActiveTime` — Unix seconds of the most recent observation
    /// where at least one trigger reported `active=true`.
    pub last_active_time: Option<i64>,
    /// `status.originalReplicaCount` — replica count observed when KEDA
    /// first attached, used by `restore_to_original_replica_count`.
    pub original_replica_count: u32,
    /// `status.health` per-trigger (KEDA 2.14+).
    pub health: KedaHealth,
    /// `status.external` — the set of trigger names currently reporting
    /// active (the rollup of `status.health[*].active`).
    pub active_triggers: Vec<String>,
    /// Most recent observed reason text, for the status banner.
    pub reason: String,
}

/// Per-trigger health rollup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KedaHealth {
    pub overall: String, // "Healthy" | "Degraded" | "Unhealthy" | "Unknown"
    pub message: String,
}

// ── ScaledObject (rich) ──────────────────────────────────────────────────

/// Rich detail view used by the drill-down. Mirrors the upstream
/// `ScaledObject` CRD field-for-field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KedaScaledObjectDetail {
    pub tenant: TenantId,
    /// `metadata.namespace` — KEDA is namespaced.
    pub namespace: String,
    /// `metadata.name`.
    pub name: String,
    /// `metadata.annotations` — surfaces `autoscaling.keda.sh/paused`,
    /// `paused-replicas`, etc.
    pub annotations: Vec<(String, String)>,
    /// `spec.scaleTargetRef`.
    pub scale_target_ref: KedaScaleTargetRef,
    /// `spec.minReplicaCount` — default 0.
    pub min_replica_count: u32,
    /// `spec.maxReplicaCount` — default 100.
    pub max_replica_count: u32,
    /// `spec.idleReplicaCount` — when set, KEDA scales to this when *no*
    /// trigger is active (vs. scaling to min). Must be `< min_replica_count`.
    pub idle_replica_count: Option<u32>,
    /// `spec.pollingInterval` — seconds between scaler polls.
    pub polling_interval_secs: u32,
    /// `spec.cooldownPeriod` — seconds to wait before scaling back down
    /// after the last active observation.
    pub cooldown_period_secs: u32,
    /// `spec.initialCooldownPeriod` (KEDA 2.13+).
    pub initial_cooldown_period_secs: u32,
    /// `spec.fallback`.
    pub fallback: Option<KedaFallback>,
    /// `spec.triggers[]`.
    pub triggers: Vec<KedaTrigger>,
    /// `spec.advanced`.
    pub advanced: Option<KedaAdvanced>,
    /// Observed status.
    pub status: KedaScaledObjectStatus,
}

// ── ScaledJob ────────────────────────────────────────────────────────────

/// `ScaledJob` — KEDA's per-event Job creator (vs. ScaledObject which
/// scales an existing workload).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KedaScaledJob {
    pub tenant: TenantId,
    pub namespace: String,
    pub name: String,
    /// `spec.jobTargetRef.template` — opaque pod-template YAML.
    pub job_template_yaml: String,
    /// `spec.pollingInterval`.
    pub polling_interval_secs: u32,
    /// `spec.successfulJobsHistoryLimit`.
    pub successful_jobs_history_limit: u32,
    /// `spec.failedJobsHistoryLimit`.
    pub failed_jobs_history_limit: u32,
    /// `spec.maxReplicaCount` — upper bound on concurrent jobs.
    pub max_replica_count: u32,
    /// `spec.scalingStrategy.strategy` — `default`|`custom`|`accurate`.
    pub scaling_strategy: String,
    /// `spec.triggers[]`.
    pub triggers: Vec<KedaTrigger>,
    pub status: KedaScaledJobStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KedaScaledJobStatus {
    pub last_active_time: Option<i64>,
    pub running_jobs: u32,
    pub pending_jobs: u32,
    pub succeeded_jobs_24h: u32,
    pub failed_jobs_24h: u32,
}

// ── TriggerAuthentication ────────────────────────────────────────────────

/// `TriggerAuthentication` — secret/env/pod-identity refs that triggers
/// look up via `authenticationRef`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KedaTriggerAuthentication {
    pub tenant: TenantId,
    pub namespace: String,
    pub name: String,
    /// `cluster` — true for ClusterTriggerAuthentication.
    pub cluster_scoped: bool,
    /// `spec.secretTargetRef` — maps Secret keys to scaler parameters.
    pub secret_refs: Vec<KedaSecretRef>,
    /// `spec.env` — env-var bindings into the trigger's scope.
    pub env_refs: Vec<KedaEnvRef>,
    /// `spec.podIdentity.provider` — none/aws-eks/aws-kiam/aws/azure/
    /// azure-workload/gcp/spiffe.
    pub pod_identity_provider: String,
    /// `spec.hashicorpVault` — populated when the trigger pulls from Vault.
    pub hashicorp_vault: Option<KedaVaultBinding>,
    /// `spec.azureKeyVault` — Azure KV binding.
    pub azure_key_vault: Option<KedaAzureKvBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KedaSecretRef {
    /// scaler-side parameter name.
    pub parameter: String,
    /// Secret name + key in the same namespace.
    pub secret_name: String,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KedaEnvRef {
    pub parameter: String,
    pub name: String,
    pub container_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KedaVaultBinding {
    pub address: String,
    pub authentication: String, // "token" | "kubernetes"
    pub mount: String,
    pub role: String,
    pub credential_secret_name: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KedaAzureKvBinding {
    pub vault_uri: String,
    pub tenant_id: String,
    pub client_id: String,
    pub secrets: Vec<String>,
}
