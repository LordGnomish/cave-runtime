//! Tenant model — tiers, isolation levels, and lifecycle states.
//!
//! Implements Principle 8 (Multi-Tenant Isolation), ADR-012, ADR-084.
//! Three isolation tiers: Soft (namespace), Hard (vcluster), Dedicated (cluster).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Tenant isolation tier. Determines resource boundaries and SLA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TenantTier {
    /// Namespace isolation. Shared cluster resources.
    /// SLA: best effort (Hetzner/Azure).
    Soft,
    /// vcluster isolation. Dedicated virtual control plane.
    /// SLA: 99.0% (Hetzner), 99.5% (Azure).
    Hard,
    /// Dedicated cluster or vcluster. Maximum isolation.
    /// SLA: 99.5% (Hetzner), 99.95% (Azure).
    Dedicated,
}

/// Tenant lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantState {
    /// Tenant created, resources being provisioned
    Provisioning,
    /// Tenant active and serving workloads
    Active,
    /// Tenant suspended (budget exceeded, compliance violation)
    Suspended,
    /// Tenant marked for deletion, in retention period
    Decommissioning,
    /// Tenant fully deleted after retention period
    Deleted,
}

/// Tenant environment within the tenant SDLC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TenantEnvironment {
    Dev,
    Staging,
    Prod,
}

/// Workload criticality classification for FinOps kill switch.
/// Implements Principle 12 (SLO-driven FinOps), ADR-096.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkloadCriticality {
    /// Never auto-suspended, even at budget overrun
    BusinessCritical,
    /// Auto-suspended at 150% of budget
    Standard,
    /// Auto-suspended at 120% of budget
    Batch,
}

/// Data classification for resources. Mandatory per ADR-102.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataClassification {
    /// May use any inference provider
    Public,
    /// May use any inference provider
    Internal,
    /// Azure OpenAI with DPA only, or Ollama
    Confidential,
    /// Ollama only (self-hosted, no external)
    Restricted,
}

/// Core tenant record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tenant {
    /// Unique tenant identifier (used in namespaces, labels, RLS)
    pub id: String,
    /// Display name
    pub name: String,
    /// Isolation tier
    pub tier: TenantTier,
    /// Current state
    pub state: TenantState,
    /// Cloud provider for this tenant's resources
    pub provider: crate::profile::Provider,
    /// Monthly budget in EUR
    pub monthly_budget_eur: Option<f64>,
    /// Default data classification
    pub default_classification: DataClassification,
    /// Default workload criticality
    pub default_criticality: WorkloadCriticality,
    /// Tenant-declared critical external FQDNs (Safe-Exit List, ADR-110)
    pub safe_exit_fqdns: Vec<String>,
    /// When the tenant was created
    pub created_at: DateTime<Utc>,
    /// When the tenant was last modified
    pub updated_at: DateTime<Utc>,
    /// Internal UUID for cross-system correlation
    pub uuid: Uuid,
}

/// Tenant-scoped rate limits per tier (from One-Prompt Kong section).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantRateLimits {
    /// Requests per second
    pub requests_per_second: u32,
    /// Burst allowance
    pub burst: u32,
    /// Daily request quota
    pub daily_quota: u64,
}

impl TenantRateLimits {
    pub fn for_tier(tier: TenantTier) -> Self {
        match tier {
            TenantTier::Soft => Self {
                requests_per_second: 100,
                burst: 200,
                daily_quota: 500_000,
            },
            TenantTier::Hard => Self {
                requests_per_second: 500,
                burst: 1000,
                daily_quota: 2_000_000,
            },
            TenantTier::Dedicated => Self {
                requests_per_second: 10_000, // Custom, placeholder
                burst: 20_000,
                daily_quota: u64::MAX, // Unlimited
            },
        }
    }
}

/// Egress quota state for a tenant. Implements ADR-110.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressState {
    pub tenant_id: String,
    /// Current egress bytes this period
    pub current_bytes: u64,
    /// Quota limit bytes
    pub quota_bytes: u64,
    /// Whether tenant is in egress quarantine
    pub quarantined: bool,
    /// When quarantine auto-restores (24h after trigger)
    pub quarantine_expires: Option<DateTime<Utc>>,
}

impl EgressState {
    /// Check if egress exceeds quota.
    pub fn exceeds_quota(&self) -> bool {
        self.current_bytes > self.quota_bytes
    }

    /// Minimum confidence threshold for autonomous quarantine action.
    /// Per One-Prompt: Soft = any, Hard = 0.7, Dedicated = 0.9.
    pub fn autonomy_confidence_threshold(tier: TenantTier) -> f64 {
        match tier {
            TenantTier::Soft => 0.0,
            TenantTier::Hard => 0.7,
            TenantTier::Dedicated => 0.9,
        }
    }
}

/// PR vcluster configuration (ADR-070).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrVclusterConfig {
    /// Max CPU (default: 2)
    pub max_cpu: u32,
    /// Max memory GiB (default: 4)
    pub max_memory_gib: u32,
    /// TTL hours (default: 4)
    pub ttl_hours: u32,
    /// Max per tenant (default: 5)
    pub max_per_tenant: u32,
}

impl Default for PrVclusterConfig {
    fn default() -> Self {
        Self {
            max_cpu: 2,
            max_memory_gib: 4,
            ttl_hours: 4,
            max_per_tenant: 5,
        }
    }
}

/// Tenant namespace naming convention.
pub fn tenant_namespace(tenant_id: &str, env: TenantEnvironment) -> String {
    format!("tenant-{tenant_id}-{env}", env = match env {
        TenantEnvironment::Dev => "dev",
        TenantEnvironment::Staging => "staging",
        TenantEnvironment::Prod => "prod",
    })
}

/// Tenant API subdomain: <tenant-id>.api.caveplatform.dev
pub fn tenant_api_subdomain(tenant_id: &str, domain: &str) -> String {
    format!("{tenant_id}.api.{domain}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_namespace_naming() {
        assert_eq!(
            tenant_namespace("acme", TenantEnvironment::Prod),
            "tenant-acme-prod"
        );
        assert_eq!(
            tenant_namespace("corp-x", TenantEnvironment::Dev),
            "tenant-corp-x-dev"
        );
    }

    #[test]
    fn test_rate_limits_soft() {
        let limits = TenantRateLimits::for_tier(TenantTier::Soft);
        assert_eq!(limits.requests_per_second, 100);
        assert_eq!(limits.burst, 200);
        assert_eq!(limits.daily_quota, 500_000);
    }

    #[test]
    fn test_rate_limits_dedicated_unlimited() {
        let limits = TenantRateLimits::for_tier(TenantTier::Dedicated);
        assert_eq!(limits.daily_quota, u64::MAX);
    }

    #[test]
    fn test_autonomy_thresholds() {
        assert_eq!(EgressState::autonomy_confidence_threshold(TenantTier::Soft), 0.0);
        assert_eq!(EgressState::autonomy_confidence_threshold(TenantTier::Hard), 0.7);
        assert_eq!(EgressState::autonomy_confidence_threshold(TenantTier::Dedicated), 0.9);
    }

    #[test]
    fn test_api_subdomain() {
        assert_eq!(
            tenant_api_subdomain("acme", "caveplatform.dev"),
            "acme.api.caveplatform.dev"
        );
    }

    #[test]
    fn test_pr_vcluster_defaults() {
        let cfg = PrVclusterConfig::default();
        assert_eq!(cfg.max_cpu, 2);
        assert_eq!(cfg.max_memory_gib, 4);
        assert_eq!(cfg.ttl_hours, 4);
        assert_eq!(cfg.max_per_tenant, 5);
    }
}
