//! Shared types used across all modules.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Canonical user identity — extracted from JWT, used across all modules.
/// Always use `cave_uid`, never the IdP `sub` claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaveIdentity {
    /// Platform-generated stable UUID (survives IdP migration)
    pub cave_uid: Uuid,
    /// Tenant scope
    pub tenant_id: String,
    /// Environment scope
    pub env: String,
    /// Platform roles
    pub roles: Vec<CaveRole>,
    /// Token expiry
    pub exp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaveRole {
    PlatformAdmin,
    PlatformViewer,
    TenantAdmin,
    TenantDeveloper,
    TenantViewer,
}

/// Module permission check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permission {
    pub module: String,
    pub action: String, // e.g., "flags:write", "vulns:triage", "scan:admin"
}

impl CaveIdentity {
    /// Check if this identity has the required permission
    pub fn has_permission(&self, module: &str, action: &str) -> bool {
        match self.roles.first() {
            Some(CaveRole::PlatformAdmin) => true,
            Some(CaveRole::TenantAdmin) => {
                // Tenant admins can do anything within their tenant scope
                !action.contains("platform:")
            }
            Some(CaveRole::TenantDeveloper) => {
                // Developers can read and write, but not admin
                !action.contains("admin") && !action.contains("platform:")
            }
            Some(CaveRole::TenantViewer) | Some(CaveRole::PlatformViewer) => {
                action.contains("read") || action.contains("list")
            }
            None => false,
        }
    }
}

/// Upstream tracking status for a feature
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamFeature {
    /// Source project (e.g., "unleash", "defectdojo")
    pub project: String,
    /// Upstream version where feature appeared
    pub upstream_version: String,
    /// GitHub issue/PR URL in upstream
    pub upstream_url: String,
    /// Our triage decision
    pub triage: UpstreamTriage,
    /// cave-runtime version where implemented (if adopted)
    pub implemented_in: Option<String>,
    /// Evaluation notes
    pub notes: String,
    /// When we detected this
    pub detected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum UpstreamTriage {
    /// Implement in cave-runtime
    Adopt,
    /// Track but don't implement yet
    Watch,
    /// Not relevant to our use case
    Skip,
}
